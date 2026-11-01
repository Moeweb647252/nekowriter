//! Migration 0008 — knowledge-first visibility and research-task ownership (spec/02 §1–§5,
//! D-158).
//!
//! The fixture is a database at the pre-0008 shape: `book_scopes` exists, `research_tasks` has
//! no `kind`/`origin`, and a row of every research status plus one research session is present.
//! The migration must preserve every task, back-fill only `draft` rows as `conception`, drop the
//! old attachment table and leave the new exclusion table empty. A fresh database is compared
//! against the upgraded one so the edited `0001` and the migration path cannot diverge.

use nekowriter::db::Db;
use sea_orm::{ConnectionTrait, DbBackend, Statement};

const OLD_RESEARCH_TASKS: &str = r#"
DROP TABLE IF EXISTS research_tasks;
CREATE TABLE research_tasks (
  id               INTEGER PRIMARY KEY,
  book_id          INTEGER REFERENCES books(id) ON DELETE CASCADE,
  scope_id         INTEGER REFERENCES knowledge_scopes(id) ON DELETE SET NULL,
  scope_name       TEXT NOT NULL,
  topic            TEXT NOT NULL,
  requirements     TEXT NOT NULL DEFAULT '',
  language         TEXT NOT NULL,
  status           TEXT NOT NULL CHECK (status IN ('draft','ready','deferred','running','done','failed','cancelled')),
  session_id       INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
  report           TEXT NOT NULL DEFAULT '',
  entries_created  INTEGER NOT NULL DEFAULT 0,
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL
);
"#;

const FIXTURE: &str = r#"
INSERT INTO books(id, title, idea, genre, writing_language, status, settings, model_overrides,
                  conception, created_at, updated_at)
VALUES
  (1, 'First', '', '', 'zh-CN', 'conceiving', '{}', '{}', '{}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 'Second', '', '', 'en', 'active', '{}', '{}', '{}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO knowledge_scopes(id, name, description, created_at, updated_at)
VALUES
  (1, 'History', 'Historical facts', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 'Lore', 'World lore', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (3, 'Private', 'Not attached', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

-- Attachments for the first book only; book 2 has none.
INSERT INTO book_scopes(book_id, scope_id) VALUES (1, 1), (1, 2);

INSERT INTO research_tasks
  (id, book_id, scope_id, scope_name, topic, requirements, language, status, session_id, report,
   entries_created, created_at, updated_at)
VALUES
  (1, 1, 1, 'History', 'Draft topic', '', 'zh-CN', 'draft', NULL, '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 1, 1, 'History', 'Ready topic', '', 'zh-CN', 'ready', NULL, '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (3, 1, 1, 'History', 'Deferred topic', '', 'zh-CN', 'deferred', NULL, '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (4, 1, 1, 'History', 'Running topic', '', 'zh-CN', 'running', NULL, '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (5, 1, 1, 'History', 'Done topic', '', 'zh-CN', 'done', NULL, 'done', 1,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (6, 1, 1, 'History', 'Failed topic', '', 'zh-CN', 'failed', NULL, '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (7, 1, 1, 'History', 'Cancelled topic', '', 'zh-CN', 'cancelled', NULL, '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO agent_sessions
  (id, book_id, role, kind, research_task_id, model_snapshot, system_prompt, tools, status, phase,
   counters, usage, created_at, updated_at)
VALUES
  (1, 1, 'researcher', 'research', 1, '{}', '', '[]', 'running', 'none', '{}', '{}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');


UPDATE research_tasks SET session_id = 1 WHERE id = 1;

"#;

async fn scalar(conn: &sea_orm::DatabaseConnection, sql: &str) -> i64 {
    let row = conn
        .query_one_raw(Statement::from_string(DbBackend::Sqlite, sql))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i64>("", "n").unwrap_or(-1)
}

async fn table_exists(conn: &sea_orm::DatabaseConnection, table: &str) -> bool {
    scalar(
        conn,
        &format!(
            "SELECT count(*) AS n FROM sqlite_master WHERE type = 'table' AND name = '{}'",
            table
        ),
    )
    .await
        == 1
}

async fn pre_0008_database() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Db::connect(&dir.path().join("old.db"))
        .await
        .expect("connect");
    nekowriter_migration::up(db.conn())
        .await
        .expect("migrate to 0008");
    db.conn()
        .execute_unprepared(&format!(
            "PRAGMA foreign_keys = OFF; \
             DROP TABLE IF EXISTS book_scope_exclusions; \
             DROP TABLE IF EXISTS book_scopes; \
             {OLD_RESEARCH_TASKS} \
             CREATE TABLE book_scopes ( \
               book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE, \
               scope_id INTEGER NOT NULL REFERENCES knowledge_scopes(id) ON DELETE CASCADE, \
               PRIMARY KEY (book_id, scope_id) \
             ); \
             PRAGMA foreign_keys = ON;"
        ))
        .await
        .expect("pre-0008 shape");
    db.conn()
        .execute_unprepared(FIXTURE)
        .await
        .expect("fixture");
    db.conn()
        .execute_unprepared("DELETE FROM seaql_migrations WHERE version = 'm0008_knowledge_first'")
        .await
        .expect("rewind to 0007");
    assert!(!table_exists(db.conn(), "book_scope_exclusions").await);
    assert!(table_exists(db.conn(), "book_scopes").await);
    (dir, db)
}

/// A stable description of table columns, foreign keys and index names — the parts entity_schema
/// compares — used to compare the fresh and upgraded paths.
async fn schema_shape(conn: &sea_orm::DatabaseConnection) -> Vec<String> {
    let tables = conn
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type = 'table' \
             AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'seaql_%' ORDER BY name",
        ))
        .await
        .expect("tables");
    let mut shape = Vec::new();
    for table in tables {
        let name: String = table.try_get("", "name").expect("name");
        let columns = conn
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("PRAGMA table_info(\"{name}\")"),
            ))
            .await
            .expect("columns");
        for column in columns {
            shape.push(format!(
                "{name}.column:{}:{}:{}:{}",
                column.try_get::<String>("", "name").expect("column"),
                column.try_get::<String>("", "type").expect("type"),
                column.try_get::<i64>("", "notnull").expect("notnull"),
                column.try_get::<i64>("", "pk").expect("pk"),
            ));
        }
        let foreign_keys = conn
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("PRAGMA foreign_key_list(\"{name}\")"),
            ))
            .await
            .expect("foreign keys");
        for key in foreign_keys {
            shape.push(format!(
                "{name}.fk:{}:{}:{}",
                key.try_get::<String>("", "from").expect("from"),
                key.try_get::<String>("", "table").expect("table"),
                key.try_get::<String>("", "to").expect("to"),
            ));
        }
        let indexes = conn
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("PRAGMA index_list(\"{name}\")"),
            ))
            .await
            .expect("indexes");
        for index in indexes {
            shape.push(format!(
                "{name}.index:{}:{}",
                index.try_get::<String>("", "name").expect("index name"),
                index.try_get::<i64>("", "unique").expect("unique"),
            ));
        }
    }
    shape
}

#[tokio::test]
async fn migration_upgrades_a_pre_0008_database_and_preserves_research_tasks() {
    let (_dir, db) = pre_0008_database().await;
    let before = scalar(db.conn(), "SELECT count(*) AS n FROM research_tasks").await;

    nekowriter_migration::up(db.conn())
        .await
        .expect("the 0008 migration applies");

    let after = scalar(db.conn(), "SELECT count(*) AS n FROM research_tasks").await;
    assert_eq!(before, 7);
    assert_eq!(after, before, "the research_tasks count guard holds");
    assert!(
        !table_exists(db.conn(), "book_scopes").await,
        "book_scopes must be gone"
    );
    assert!(
        table_exists(db.conn(), "book_scope_exclusions").await,
        "the exclusion table must exist"
    );
    assert_eq!(
        scalar(db.conn(), "SELECT count(*) AS n FROM book_scope_exclusions").await,
        0,
        "the upgrade creates no exclusions"
    );
    assert!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM pragma_foreign_key_check"
        )
        .await
            == 0,
        "the upgraded database has no dangling foreign keys"
    );

    let rows = db
        .conn()
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT id, status, kind, origin FROM research_tasks ORDER BY id",
        ))
        .await
        .expect("tasks");
    let expected = [
        (1, "draft", "topic", "conception"),
        (2, "ready", "topic", "author"),
        (3, "deferred", "topic", "author"),
        (4, "running", "topic", "author"),
        (5, "done", "topic", "author"),
        (6, "failed", "topic", "author"),
        (7, "cancelled", "topic", "author"),
    ];
    assert_eq!(rows.len(), expected.len());
    for (row, (id, status, kind, origin)) in rows.iter().zip(expected) {
        assert_eq!(row.try_get::<i32>("", "id").unwrap(), id);
        assert_eq!(row.try_get::<String>("", "status").unwrap(), status);
        assert_eq!(row.try_get::<String>("", "kind").unwrap(), kind);
        assert_eq!(row.try_get::<String>("", "origin").unwrap(), origin);
    }
}

#[tokio::test]
async fn fresh_and_upgraded_databases_have_the_same_schema_shape() {
    let (_old_dir, old) = pre_0008_database().await;
    nekowriter_migration::up(old.conn())
        .await
        .expect("upgrade old database");
    let old_shape = schema_shape(old.conn()).await;

    let fresh_dir = tempfile::tempdir().expect("fresh dir");
    let fresh = Db::connect(&fresh_dir.path().join("fresh.db"))
        .await
        .expect("connect fresh");
    nekowriter_migration::up(fresh.conn())
        .await
        .expect("migrate fresh database");
    let fresh_shape = schema_shape(fresh.conn()).await;

    assert_eq!(
        fresh_shape, old_shape,
        "the edited 0001 and the 0008 upgrade path must reach the same schema"
    );
}
