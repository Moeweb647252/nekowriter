//! The `deferred` research status in the schema (spec/02 §1–§3, D-138).
//!
//! Two things have to hold, and only one of them is obvious:
//!
//! 1. A fresh database accepts `status = 'deferred'` (the CHECK constraint of `0001_init.sql`).
//! 2. A database created **before** the status existed is upgraded, and the upgrade keeps every
//!    row that points at a research task. The author's own database is the case that matters:
//!    `seaql_migrations` already records `m0001_init`, so an edit to that file never runs on it —
//!    only a later migration file does.
//!
//! The upgrade rebuilds `research_tasks` (SQLite cannot alter a CHECK constraint in place), and
//! `agent_sessions.research_task_id` is `ON DELETE CASCADE`: with `foreign_keys=ON`, which
//! `Db::connect` sets on every connection, the `DROP TABLE` would silently delete the researcher
//! sessions and null `knowledge_entries.research_task_id`. The second test is what proves the
//! upgrade does not do that.

use axum::http::StatusCode;
use nekowriter::testing::TestApp;
use serde_json::json;

/// The pre-`deferred` schema of `research_tasks` plus the rows that reference a task: the shape
/// `0001_init.sql` created before this change, and the state a developer database is in.
const OLD_SCHEMA_AND_ROWS: &str = "
CREATE TABLE research_tasks (
  id               INTEGER PRIMARY KEY,
  book_id          INTEGER REFERENCES books(id) ON DELETE CASCADE,
  scope_id         INTEGER REFERENCES knowledge_scopes(id) ON DELETE SET NULL,
  scope_name       TEXT NOT NULL,
  topic            TEXT NOT NULL,
  requirements     TEXT NOT NULL DEFAULT '',
  language         TEXT NOT NULL,
  status           TEXT NOT NULL CHECK (status IN ('draft','ready','running','done','failed','cancelled')),
  session_id       INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
  report           TEXT NOT NULL DEFAULT '',
  entries_created  INTEGER NOT NULL DEFAULT 0,
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL
);
INSERT INTO research_tasks(id, scope_name, topic, language, status, created_at, updated_at)
VALUES (1, '明初历史', '宝钞与财政', 'zh-CN', 'ready', '2026-01-01T00:00:00Z',
        '2026-01-01T00:00:00Z');
INSERT INTO agent_sessions(book_id, role, kind, research_task_id, model_snapshot, system_prompt,
                           tools, status, created_at, updated_at)
VALUES (NULL, 'researcher', 'research', 1, '{}', '', '[]', 'completed',
        '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO knowledge_scopes(name, description, created_at, updated_at)
VALUES ('明初历史', '', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO knowledge_entries(scope_id, title, content, source, research_task_id, created_at,
                              updated_at)
VALUES (1, '宝钞', '正文', 'research', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
";

#[tokio::test]
async fn a_fresh_database_accepts_the_deferred_status() {
    let app = TestApp::new().await;
    let (status, task) = app
        .post(
            "/api/research-tasks",
            json!({ "scope_name": "S", "topic": "t", "language": "en" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let id = task["id"].as_i64().expect("id");

    // The column CHECK is the real gate: writing the value directly proves the schema carries it,
    // not just the Rust enum (spec/02 §2, §3).
    app.execute(&format!(
        "UPDATE research_tasks SET status = 'deferred' WHERE id = {id}"
    ))
    .await;
    let (_, task) = app.get(&format!("/api/research-tasks/{id}")).await;
    assert_eq!(task["status"], "deferred", "{task}");
}

/// Rebuilds the `research_tasks` table of a migrated database into its pre-`deferred` shape, with
/// the session and the entry that reference the task. Returns the database's path.
async fn pre_deferred_database() -> (tempfile::TempDir, std::path::PathBuf) {
    use sea_orm::ConnectionTrait;

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("old.db");
    let db = nekowriter::db::Db::connect(&path).await.expect("connect");
    nekowriter_migration::up(db.conn()).await.expect("migrate");
    // Swap the migrated table for the pre-`deferred` one. Empty at this point, so the drop fires
    // no `ON DELETE` action and the fixture rows are inserted into the old shape afterwards.
    db.conn()
        .execute_unprepared("DROP TABLE research_tasks")
        .await
        .expect("drop the migrated table");
    db.conn()
        .execute_unprepared(OLD_SCHEMA_AND_ROWS)
        .await
        .expect("the pre-deferred schema and its rows");
    // Rewind the migration record to `m0001_init` only: that is the state a database created
    // before the deferred status existed is in, and the reason an edit to `0001` can never reach
    // it — the migrator runs pending files only. The later additive tables and columns are absent
    // in that state too, so drop the ones later migrations created; otherwise the replayed
    // migrator would meet its `CREATE TABLE` / `ADD COLUMN` on an already-migrated database.
    // 0009 is the only later migration that alters `model_profiles` (no rebuild recreates it), so
    // its two columns have to be removed here.
    db.conn()
        .execute_unprepared(
            "DROP TABLE IF EXISTS book_writing_skills; DROP TABLE IF EXISTS writing_skills; \
             DROP INDEX IF EXISTS timeline_events_root; \
             DROP INDEX IF EXISTS timeline_events_parent; \
             ALTER TABLE timeline_events DROP COLUMN parent_id; \
             ALTER TABLE timeline_events DROP COLUMN kind; \
             ALTER TABLE timeline_events DROP COLUMN status; \
             ALTER TABLE foreshadowing DROP COLUMN payoff_event_id; \
             ALTER TABLE foreshadowing DROP COLUMN payoff_position; \
             ALTER TABLE model_profiles DROP COLUMN effort_levels; \
             ALTER TABLE model_profiles DROP COLUMN default_effort; \
             DELETE FROM seaql_migrations WHERE version <> 'm0001_init'",
        )
        .await
        .expect("rewind to 0001");

    // The database records exactly the original migration.
    let record = db
        .conn()
        .query_all_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT version FROM seaql_migrations",
        ))
        .await
        .expect("query");
    assert_eq!(record.len(), 1, "exactly one applied migration");
    assert_eq!(
        record[0]
            .try_get::<String>("", "version")
            .unwrap_or_default(),
        "m0001_init"
    );
    // The old CHECK really does refuse the new value; otherwise this test would prove nothing.
    assert!(
        db.conn()
            .execute_unprepared("UPDATE research_tasks SET status = 'deferred' WHERE id = 1")
            .await
            .is_err(),
        "the pre-`deferred` CHECK must refuse the value the upgrade adds"
    );
    (dir, path)
}

#[tokio::test]
async fn the_upgrade_keeps_the_sessions_and_entries_that_point_at_a_task() {
    use sea_orm::ConnectionTrait;

    let (_dir, path) = pre_deferred_database().await;
    // The upgrade, exactly as startup runs it over the author's database.
    let db = nekowriter::db::Db::connect(&path).await.expect("reconnect");
    nekowriter_migration::up(db.conn()).await.expect("upgrade");

    // The upgraded database accepts the status now, and — the point of this test — the session and
    // the entry still point at the task. A rebuild with `foreign_keys=ON` would have deleted the
    // session and nulled the entry link, because SQLite fires `ON DELETE` actions on the drop.
    db.conn()
        .execute_unprepared("UPDATE research_tasks SET status = 'deferred' WHERE id = 1")
        .await
        .expect("the upgraded CHECK accepts deferred");
    let rows = db
        .conn()
        .query_all_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT (SELECT count(*) FROM agent_sessions WHERE research_task_id = 1) AS sessions, \
             (SELECT research_task_id FROM knowledge_entries WHERE title = '宝钞') AS entry_task",
        ))
        .await
        .expect("query");
    assert_eq!(rows.len(), 1, "one row");
    assert_eq!(
        rows[0].try_get::<i64>("", "sessions").unwrap_or(0),
        1,
        "the researcher session survives the upgrade"
    );
    assert_eq!(
        rows[0]
            .try_get::<Option<i32>>("", "entry_task")
            .ok()
            .flatten(),
        Some(1),
        "the knowledge entry keeps its task link"
    );
}

#[tokio::test]
async fn the_upgrade_is_repeatable_and_leaves_the_table_queryable() {
    use sea_orm::ConnectionTrait;

    // `startup` runs the migrator on every launch. A second call is a no-op, and the rebuilt table
    // keeps its shape: a rebuild that forgot a column would surface only when a query names it.
    let (_dir, path) = pre_deferred_database().await;
    let db = nekowriter::db::Db::connect(&path).await.expect("reconnect");
    for _ in 0..2 {
        nekowriter_migration::up(db.conn()).await.expect("upgrade");
    }
    // Every column of the entity (spec/02 §2) is present and the row survived the rebuild.
    let row = db
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT id, book_id, scope_id, scope_name, topic, requirements, language, status, \
             session_id, report, entries_created, created_at, updated_at \
             FROM research_tasks WHERE id = 1",
        ))
        .await
        .expect("query")
        .expect("the task row survives");
    assert_eq!(row.try_get::<i32>("", "id").unwrap_or(0), 1);
    assert_eq!(
        row.try_get::<String>("", "status").unwrap_or_default(),
        "ready"
    );
    // A second migration record was inserted once, not once per call.
    let count = db
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0002_research_deferred'",
        ))
        .await
        .expect("query")
        .map(|row| row.try_get::<i64>("", "n").unwrap_or(0))
        .unwrap_or(0);
    assert_eq!(count, 1, "the upgrade is recorded exactly once");
}
