//! Migration 0014 — the step-pause flag (spec/02 §1–§2; D-173).
//!
//! The migration must add the column without losing a session and must leave every existing row
//! at 0. A fresh database and a database upgraded from 0013 must expose the same column.

use nekowriter::db::Db;
use sea_orm::{ConnectionTrait, DbBackend, Statement};

const SESSION_INSERT: &str = r#"
INSERT INTO agent_sessions
  (id, role, kind, model_snapshot, system_prompt, tools, status, phase, counters, usage,
   created_at, updated_at)
VALUES
  (1, 'writer', 'chapter_write',
   '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}',
   '', '[]', 'idle', 'none', '{}', '{}',
   '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
"#;

async fn scalar(conn: &sea_orm::DatabaseConnection, sql: &str) -> i64 {
    let row = conn
        .query_one_raw(Statement::from_string(DbBackend::Sqlite, sql))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i64>("", "n").unwrap_or(-1)
}

async fn pre_0014_database() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Db::connect(&dir.path().join("old.db"))
        .await
        .expect("connect");
    nekowriter_migration::up_to(db.conn(), 13)
        .await
        .expect("migrate to 0013");
    db.conn()
        .execute_unprepared(SESSION_INSERT)
        .await
        .expect("fixture");
    (dir, db)
}

#[tokio::test]
async fn migration_keeps_sessions_and_defaults_the_flag_to_zero() {
    let (_dir, db) = pre_0014_database().await;
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM pragma_table_info('agent_sessions') WHERE name = 'pause_after_step'"
        )
        .await,
        0,
        "the pre-0014 schema has no flag"
    );
    nekowriter_migration::up(db.conn())
        .await
        .expect("0014 applies");
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0014_step_pause'"
        )
        .await,
        1
    );
    assert_eq!(
        scalar(db.conn(), "SELECT count(*) AS n FROM agent_sessions").await,
        1,
        "the migration keeps every session"
    );
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM agent_sessions WHERE pause_after_step = 0"
        )
        .await,
        1,
        "every existing session has the default flag"
    );
}

#[tokio::test]
async fn fresh_and_upgraded_databases_expose_the_same_flag_column() {
    let dir = tempfile::tempdir().expect("temp dir");
    let fresh = Db::connect(&dir.path().join("fresh.db"))
        .await
        .expect("connect");
    nekowriter_migration::up(fresh.conn())
        .await
        .expect("migrate fresh");
    let (_old_dir, upgraded) = pre_0014_database().await;
    nekowriter_migration::up(upgraded.conn())
        .await
        .expect("upgrade");

    let fresh_type = scalar(
        fresh.conn(),
        "SELECT count(*) AS n FROM pragma_table_info('agent_sessions') WHERE name = 'pause_after_step' AND type = 'INTEGER' AND \"notnull\" = 1 AND dflt_value = '0'",
    )
    .await;
    let upgraded_type = scalar(
        upgraded.conn(),
        "SELECT count(*) AS n FROM pragma_table_info('agent_sessions') WHERE name = 'pause_after_step' AND type = 'INTEGER' AND \"notnull\" = 1 AND dflt_value = '0'",
    )
    .await;
    assert_eq!(fresh_type, 1);
    assert_eq!(upgraded_type, 1);
}

/// Execute a corrupted copy of 0014 in one transaction: the guard must roll the ALTER back, and a
/// retry of the real migration must succeed.
async fn rejected_copy(tamper: &str) -> (tempfile::TempDir, Db) {
    let (dir, db) = pre_0014_database().await;
    let sql = include_str!("../../migration/src/sql/0014_step_pause.sql").replace(
        "ALTER TABLE agent_sessions ADD COLUMN pause_after_step INTEGER NOT NULL DEFAULT 0;",
        &format!("ALTER TABLE agent_sessions ADD COLUMN pause_after_step INTEGER NOT NULL DEFAULT 0;\n{tamper}"),
    );
    let failed = db
        .write(move |tx| {
            Box::pin(async move {
                sea_orm::ConnectionTrait::execute_unprepared(tx, &sql).await?;
                Ok(())
            })
        })
        .await;
    assert!(failed.is_err(), "the guard rejects the corrupt copy");
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM pragma_table_info('agent_sessions') WHERE name = 'pause_after_step'"
        )
        .await,
        0,
        "the failed copy rolled the column back"
    );
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0014_step_pause'"
        )
        .await,
        0
    );
    nekowriter_migration::up(db.conn())
        .await
        .expect("the retry applies 0014");
    (dir, db)
}

#[tokio::test]
async fn a_lost_session_trips_the_count_guard_and_rolls_back() {
    let (_dir, db) = rejected_copy("DELETE FROM agent_sessions WHERE id = 1;").await;
    assert_eq!(
        scalar(db.conn(), "SELECT count(*) AS n FROM agent_sessions").await,
        1
    );
}

#[tokio::test]
async fn a_nonzero_existing_flag_trips_the_value_guard_and_rolls_back() {
    let (_dir, db) =
        rejected_copy("UPDATE agent_sessions SET pause_after_step = 1 WHERE id = 1;").await;
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM agent_sessions WHERE pause_after_step = 0"
        )
        .await,
        1
    );
}
