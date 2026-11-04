//! Migration 0012 — the book-agent role, session kind and write mode (spec/02 §1–§3, D-169).
//!
//! The table rebuild has to preserve ten existing references. The fixture holds a session of
//! every old kind and one row in every referencing table, so a cascade or an accidental NULL is a
//! hard failure rather than a silently reduced database.

use nekowriter::db::json::{AgentRole, RoleModels, SessionKind, WriteMode};
use nekowriter::testing::TestApp;
use sea_orm::{ConnectionTrait, DbBackend, Statement};

const FIXTURE: &str = r#"
INSERT INTO books(id, title, writing_language, status, settings, created_at, updated_at)
VALUES (1, 'Book', 'en', 'active', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO tracks(id, book_id, kind, title, canon_mode, ord, created_at, updated_at)
VALUES (1, 1, 'main', 'Main', 'canon', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO chapters(id, book_id, track_id, ord, status, content, created_at, updated_at)
VALUES (1, 1, 1, 1, 'drafting', '{"paragraphs":[],"next_pid":1,"tail_open":false}',
        '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO research_tasks(id, book_id, scope_name, topic, language, status, created_at, updated_at)
VALUES (1, 1, 'S', 't', 'en', 'running', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

-- Every old kind and every old role. Session 11 carries a parent link to session 10.
INSERT INTO agent_sessions
  (id, book_id, role, kind, chapter_id, track_id, research_task_id, model_snapshot, system_prompt,
   tools, status, phase, granularity, auto_pause, pause_after_outline, pause_requested,
   base_content_version, counters, usage, created_at, updated_at)
VALUES
  (1, 1, 'conception',  'conception',          NULL, NULL, NULL, '{}', '', '[]', 'completed', 'none', NULL, 0, 0, 0, NULL, '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 1, 'researcher',  'research',            NULL, NULL, 1,    '{}', '', '[]', 'running',   'none', NULL, 0, 0, 0, NULL, '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (3, 1, 'planner',     'chapter_outline',     1,    1,    NULL, '{}', '', '[]', 'held',      'none', NULL, 0, 0, 0, NULL, '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (4, 1, 'writer',      'chapter_write',       1,    1,    NULL, '{}', '', '[]', 'paused',    'drafting', 'paragraph', 0, 0, 0, NULL, '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (5, 1, 'writer',      'chapter_revise',      1,    1,    NULL, '{}', '', '[]', 'paused',    'revising', 'paragraph', 0, 0, 0, NULL, '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (6, 1, 'reviewer',    'chapter_review',      1,    1,    NULL, '{}', '', '[]', 'completed', 'none', NULL, 0, 0, 0, NULL, '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (7, 1, 'archivist',   'chapter_maintenance', 1,    1,    NULL, '{}', '', '[]', 'completed', 'none', NULL, 0, 0, 0, NULL, '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (8, 1, 'archivist',   'canon_seed',          NULL, 1,    NULL, '{}', '', '[]', 'completed', 'none', NULL, 0, 0, 0, NULL, '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (9, 1, 'archivist',   'chapter_resync',      1,    1,    NULL, '{}', '', '[]', 'failed',    'none', NULL, 0, 0, 0, NULL, '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (10, 1, 'writer',     'chapter_write',       1,    1,    NULL, '{}', '', '[]', 'completed', 'drafting', 'paragraph', 0, 0, 0, NULL, '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (11, 1, 'writer',     'chapter_write',       1,    1,    NULL, '{}', '', '[]', 'running',   'drafting', 'paragraph', 0, 0, 0, NULL, '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (12, 1, 'writer',     'chapter_write',       1,    1,    NULL, '{}', '', '[]', 'idle',      'none', 'paragraph', 0, 0, 0, NULL, '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
UPDATE agent_sessions SET parent_session_id = 10 WHERE id = 11;

INSERT INTO agent_messages(id, session_id, seq, role, kind, content, created_at)
VALUES (1, 3, 0, 'user', 'kickoff', '[]', '2026-01-01T00:00:00Z'),
       (2, 10, 0, 'user', 'kickoff', '[]', '2026-01-01T00:00:00Z'),
       (3, 11, 0, 'user', 'kickoff', '[]', '2026-01-01T00:00:00Z');

INSERT INTO agent_tool_calls(id, session_id, message_id, call_index, provider_call_id, name, input,
                             status, created_at)
VALUES (1, 3, 1, 0, 'call_1', 'submit_outline', '{}', 'held', '2026-01-01T00:00:00Z'),
       (2, 10, 2, 0, 'call_2', 'insert_text', '{}', 'done', '2026-01-01T00:00:00Z');
UPDATE agent_sessions SET held_tool_call_id = 1 WHERE id = 3;

INSERT INTO agent_inbox(session_id, kind, content, created_at)
VALUES (3, 'user_message', 'keep going', '2026-01-01T00:00:00Z'),
       (3, 'notice', 'notice', '2026-01-01T00:00:00Z');

INSERT INTO chapter_versions(chapter_id, version_no, kind, content, word_count, session_id, created_at)
VALUES (1, 1, 'draft', '{"paragraphs":[],"next_pid":1,"tail_open":false}', 0, 10,
        '2026-01-01T00:00:00Z');

INSERT INTO setting_proposals(book_id, target_type, target_kind, edits, status, session_id, created_at)
VALUES (1, 'setting_doc', 'outline', '{"kind":"append","text":"x"}', 'pending', 7,
        '2026-01-01T00:00:00Z');

INSERT INTO change_log(book_id, track_id, session_id, actor, target_type, target_id, op, before, after, created_at)
VALUES (1, 1, 7, 'agent', 'setting_doc', 1, 'update', '{}', '{}', '2026-01-01T00:00:00Z');

INSERT INTO research_tasks(id, book_id, scope_name, topic, language, status, session_id, created_at, updated_at)
VALUES (2, 1, 'S', 'existing', 'en', 'running', 2, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO review_reports(chapter_id, content_version, session_id, attempt, verdict, summary, issues, feedback, created_at)
VALUES (1, 0, 6, 1, 'pass', 'ok', '[]', 'fine', '2026-01-01T00:00:00Z');

INSERT INTO chapter_feedback(chapter_id, content_version, review_report_id, text, mode, session_id, created_at)
VALUES (1, 0, 1, 'redo', 'revise', 5, '2026-01-01T00:00:00Z');
"#;

async fn scalar_i64(conn: &sea_orm::DatabaseConnection, sql: &str) -> i64 {
    let row = conn
        .query_one_raw(Statement::from_string(DbBackend::Sqlite, sql))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i64>("", "n").unwrap_or(-1)
}

async fn columns(conn: &sea_orm::DatabaseConnection, table: &str) -> Vec<String> {
    conn.query_all_raw(Statement::from_string(
        DbBackend::Sqlite,
        format!("PRAGMA table_info('{table}')"),
    ))
    .await
    .expect("table_info")
    .into_iter()
    .map(|row| row.try_get::<String>("", "name").expect("column"))
    .collect()
}

/// Every old column's value for every row, as SQLite's own text representation. Comparing the
/// complete table before and after the rebuild is what catches a column dropped from both the
/// CREATE TABLE and the INSERT ... SELECT lists.
async fn table_values(
    conn: &sea_orm::DatabaseConnection,
    columns: &[String],
) -> Vec<Vec<Option<String>>> {
    let projection = columns
        .iter()
        .enumerate()
        .map(|(index, column)| format!("quote({column}) AS c{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    conn.query_all_raw(Statement::from_string(
        DbBackend::Sqlite,
        format!("SELECT {projection} FROM agent_sessions ORDER BY id"),
    ))
    .await
    .expect("table values")
    .into_iter()
    .map(|row| {
        (0..columns.len())
            .map(|index| {
                row.try_get::<Option<String>>("", format!("c{index}").as_str())
                    .expect("value")
            })
            .collect()
    })
    .collect()
}

async fn pre_0012_database() -> (tempfile::TempDir, nekowriter::db::Db) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = nekowriter::db::Db::connect(&dir.path().join("old.db"))
        .await
        .expect("connect");
    nekowriter_migration::up_to(db.conn(), 11)
        .await
        .expect("migrate to 0011");
    db.conn()
        .execute_unprepared(FIXTURE)
        .await
        .expect("fixture");
    (dir, db)
}

async fn count_guard_tables(conn: &sea_orm::DatabaseConnection) -> Vec<(String, i64)> {
    let mut counts = Vec::new();
    for (table, where_clause) in [
        ("agent_sessions", ""),
        ("agent_messages", ""),
        ("agent_tool_calls", ""),
        ("agent_inbox", ""),
        ("chapter_versions", " WHERE session_id IS NOT NULL"),
        ("setting_proposals", " WHERE session_id IS NOT NULL"),
        ("change_log", " WHERE session_id IS NOT NULL"),
        ("research_tasks", " WHERE session_id IS NOT NULL"),
        ("review_reports", " WHERE session_id IS NOT NULL"),
        ("chapter_feedback", " WHERE session_id IS NOT NULL"),
        ("parent_sessions", " WHERE parent_session_id IS NOT NULL"),
    ] {
        let sql = if table == "parent_sessions" {
            "SELECT count(*) AS n FROM agent_sessions WHERE parent_session_id IS NOT NULL"
                .to_owned()
        } else {
            format!("SELECT count(*) AS n FROM {table}{where_clause}")
        };
        counts.push((table.to_owned(), scalar_i64(conn, &sql).await));
    }
    counts
}

#[tokio::test]
async fn a_fresh_database_accepts_the_book_agent_and_rejects_bad_write_modes() {
    let app = TestApp::new().await;
    app.execute(
        "INSERT INTO agent_sessions(book_id, role, kind, model_snapshot, system_prompt, tools, \
         status, write_mode, created_at, updated_at) \
         VALUES (NULL, 'assistant', 'book_agent', '{}', '', '[]', 'idle', 'review', \
         '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .await;
    assert!(
        app.try_execute(
            "INSERT INTO agent_sessions(role, kind, model_snapshot, system_prompt, tools, status, \
         write_mode, created_at, updated_at) VALUES ('writer', 'chapter_write', '{}', '', '[]', \
         'idle', 'full', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        )
        .await
        .is_err()
    );
    assert!(
        app.try_execute(
            "INSERT INTO agent_sessions(role, kind, model_snapshot, system_prompt, tools, status, \
         write_mode, created_at, updated_at) VALUES ('assistant', 'book_agent', '{}', '', '[]', \
         'idle', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        )
        .await
        .is_err()
    );
    assert!(
        app.try_execute(
            "INSERT INTO agent_sessions(role, kind, model_snapshot, system_prompt, tools, status, \
         write_mode, created_at, updated_at) VALUES ('assistant', 'book_agent', '{}', '', '[]', \
         'idle', 'bogus', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn pre_0012_rows_survive_with_write_mode_null() {
    let (_dir, db) = pre_0012_database().await;
    let old_columns = columns(db.conn(), "agent_sessions").await;
    assert!(!old_columns.contains(&"write_mode".to_owned()));
    let old_values = table_values(db.conn(), &old_columns).await;
    let before = count_guard_tables(db.conn()).await;

    nekowriter_migration::up(db.conn())
        .await
        .expect("0012 applies");
    // S-1: the rebuilt order is spec/02's final order, not "old order plus write_mode at the
    // end"; pin it so a future rebuild cannot shuffle columns by accident.
    let expected_columns = [
        "id",
        "book_id",
        "role",
        "kind",
        "chapter_id",
        "track_id",
        "research_task_id",
        "model_profile_id",
        "model_snapshot",
        "system_prompt",
        "tools",
        "status",
        "phase",
        "granularity",
        "auto_pause",
        "pause_after_outline",
        "write_mode",
        "tool_error_streak_limit",
        "research_max_tool_calls",
        "research_max_entries",
        "pause_requested",
        "held_tool_call_id",
        "base_content_version",
        "pending_comment",
        "view_snapshot",
        "counters",
        "usage",
        "error",
        "parent_session_id",
        "created_at",
        "updated_at",
        "finished_at",
        "canon_systems",
    ];
    assert_eq!(
        columns(db.conn(), "agent_sessions").await,
        expected_columns,
        "the rebuilt column order is spec/02's final order"
    );
    assert_eq!(
        table_values(db.conn(), &old_columns).await,
        old_values,
        "every old column's value must survive the rebuild"
    );
    assert_eq!(
        count_guard_tables(db.conn()).await,
        before,
        "reference counts"
    );
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM agent_sessions WHERE write_mode IS NOT NULL"
        )
        .await,
        0,
        "every old row gets NULL"
    );
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM pragma_foreign_key_check"
        )
        .await,
        0
    );
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0012_book_agent'"
        )
        .await,
        1
    );
    // A representative row keeps every carried value, including the 0006 JSON and 0010 limits.
    let row = db.conn().query_one_raw(Statement::from_string(
        DbBackend::Sqlite,
        "SELECT role, kind, status, canon_systems, tool_error_streak_limit, research_max_tool_calls, \
         research_max_entries FROM agent_sessions WHERE id = 3",
    )).await.expect("query").expect("row");
    assert_eq!(row.try_get::<String>("", "role").unwrap(), "planner");
    assert_eq!(
        row.try_get::<String>("", "kind").unwrap(),
        "chapter_outline"
    );
    assert_eq!(row.try_get::<String>("", "status").unwrap(), "held");
    assert_eq!(
        row.try_get::<Option<String>>("", "canon_systems").unwrap(),
        None
    );
    assert_eq!(
        row.try_get::<i64>("", "tool_error_streak_limit").unwrap(),
        5
    );
    assert_eq!(
        row.try_get::<i64>("", "research_max_tool_calls").unwrap(),
        40
    );
    assert_eq!(row.try_get::<i64>("", "research_max_entries").unwrap(), 25);
}

#[tokio::test]
async fn fresh_and_upgraded_databases_have_the_same_schema() {
    let (_dir, old) = pre_0012_database().await;
    nekowriter_migration::up(old.conn()).await.expect("upgrade");
    let fresh_dir = tempfile::tempdir().expect("fresh dir");
    let fresh = nekowriter::db::Db::connect(&fresh_dir.path().join("fresh.db"))
        .await
        .expect("connect fresh");
    nekowriter_migration::up(fresh.conn())
        .await
        .expect("migrate fresh");

    let old_columns = columns(old.conn(), "agent_sessions").await;
    assert_eq!(old_columns, columns(fresh.conn(), "agent_sessions").await);
    assert_eq!(old_columns.len(), 33, "one explicit column per table field");
    assert!(old_columns.contains(&"write_mode".to_owned()));
    let old_indexes: Vec<String> = old
        .conn()
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'agent_sessions' ORDER BY name",
        ))
        .await
        .expect("indexes")
        .into_iter()
        .map(|row| row.try_get("", "name").unwrap())
        .collect();
    assert_eq!(
        old_indexes,
        [
            "agent_sessions_book",
            "agent_sessions_chapter",
            "agent_sessions_status"
        ]
    );
}

#[tokio::test]
async fn a_broken_copy_rolls_back_and_can_be_retried() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = nekowriter::db::Db::connect(&dir.path().join("broken.db"))
        .await
        .expect("connect");
    nekowriter_migration::up_to(db.conn(), 11)
        .await
        .expect("migrate to 0011");
    db.conn()
        .execute_unprepared(FIXTURE)
        .await
        .expect("fixture");
    let before = count_guard_tables(db.conn()).await;
    db.conn()
        .execute_unprepared(
            "PRAGMA foreign_keys = OFF; \
             INSERT INTO agent_messages(id, session_id, seq, role, kind, content, created_at) \
             VALUES (99, 999, 0, 'user', 'notice', '[]', '2026-01-01T00:00:00Z'); \
             PRAGMA foreign_keys = ON;",
        )
        .await
        .expect("inject orphan");
    let with_orphan = count_guard_tables(db.conn()).await;
    assert!(
        nekowriter_migration::up(db.conn()).await.is_err(),
        "broken copy fails"
    );
    assert_eq!(count_guard_tables(db.conn()).await, with_orphan);
    assert!(
        !columns(db.conn(), "agent_sessions")
            .await
            .contains(&"write_mode".to_owned())
    );
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0012_book_agent'"
        )
        .await,
        0
    );
    db.conn()
        .execute_unprepared("DELETE FROM agent_messages WHERE id = 99")
        .await
        .expect("remove orphan");
    nekowriter_migration::up(db.conn())
        .await
        .expect("retry succeeds");
    assert_eq!(count_guard_tables(db.conn()).await, before);
    assert!(
        columns(db.conn(), "agent_sessions")
            .await
            .contains(&"write_mode".to_owned())
    );
}

#[tokio::test]
async fn old_role_models_load_without_assistant() {
    let old: RoleModels =
        serde_json::from_str(r#"{"writer":3,"efforts":{"writer":"low"}}"#).expect("old role JSON");
    assert_eq!(old.writer, Some(3));
    assert_eq!(old.assistant, None);
    assert_eq!(old.efforts.writer.as_deref(), Some("low"));
    assert_eq!(old.efforts.assistant, None);
    let _ = (
        AgentRole::Assistant,
        SessionKind::BookAgent,
        WriteMode::Review,
    );
}

/// The per-table guards, not just the final foreign-key check: delete one referencing row inside
/// the rebuild window on a scratch connection and prove the matching count guard rolls back.
#[tokio::test]
async fn a_lost_referencing_row_fails_the_per_table_count_guard() {
    let sql = include_str!("../../migration/src/sql/0012_book_agent.sql");
    let cases = [
        (
            "agent_sessions",
            "DELETE FROM agent_sessions WHERE id = 12;",
        ),
        ("agent_messages", "DELETE FROM agent_messages WHERE id = 3;"),
        (
            "agent_tool_calls",
            "DELETE FROM agent_tool_calls WHERE id = 1;",
        ),
        ("agent_inbox", "DELETE FROM agent_inbox WHERE rowid = 1;"),
        (
            "chapter_versions",
            "DELETE FROM chapter_versions WHERE rowid = 1;",
        ),
        (
            "setting_proposals",
            "DELETE FROM setting_proposals WHERE rowid = 1;",
        ),
        ("change_log", "DELETE FROM change_log WHERE rowid = 1;"),
        ("research_tasks", "DELETE FROM research_tasks WHERE id = 1;"),
        (
            "review_reports",
            "DELETE FROM review_reports WHERE rowid = 1;",
        ),
        (
            "chapter_feedback",
            "DELETE FROM chapter_feedback WHERE rowid = 1;",
        ),
        (
            "parent_sessions",
            "UPDATE agent_sessions SET parent_session_id = NULL WHERE id = 11;",
        ),
    ];
    for (table, deletion) in cases {
        let (_dir, db) = pre_0012_database().await;
        let pool = db.conn().get_sqlite_connection_pool();
        let mut conn = pool.acquire().await.expect("scratch connection");
        let broken = sql.replace(
            "-- Every reference must survive the rebuild.",
            &format!("{deletion}\n-- Every reference must survive the rebuild."),
        );
        let result = sea_orm::sqlx::raw_sql(sea_orm::sqlx::AssertSqlSafe(broken))
            .execute(&mut *conn)
            .await;
        assert!(result.is_err(), "{table} count guard must fail");
        assert!(
            !columns(db.conn(), "agent_sessions")
                .await
                .contains(&"write_mode".to_owned()),
            "{table}: failed rebuild must roll back"
        );
    }
}
