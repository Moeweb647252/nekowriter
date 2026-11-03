//! Migration 0010 — configurable agent limits (spec/03 §5.7, spec/05 §8, D-164).
//!
//! The fixture is a database at the pre-0010 shape: 0001–0009 applied, the three new columns
//! dropped, and one pre-existing session row. The migration must add the columns with their old
//! defaults (5/40/25), leave a fresh database with the same effective schema, and roll a broken
//! copy back so it can be retried.

use nekowriter::db::Db;
use sea_orm::{ConnectionTrait, DbBackend, Statement};

const COLUMNS: [&str; 3] = [
    "tool_error_streak_limit",
    "research_max_tool_calls",
    "research_max_entries",
];
const DEFAULTS: [i64; 3] = [5, 40, 25];

async fn scalar(conn: &sea_orm::DatabaseConnection, sql: &str) -> i64 {
    let row = conn
        .query_one_raw(Statement::from_string(DbBackend::Sqlite, sql))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i64>("", "n").unwrap_or(-1)
}

async fn column_count(conn: &sea_orm::DatabaseConnection, column: &str) -> i64 {
    scalar(
        conn,
        &format!(
            "SELECT count(*) AS n FROM pragma_table_info('agent_sessions') WHERE name = '{column}'"
        ),
    )
    .await
}

async fn schema_details(conn: &sea_orm::DatabaseConnection) -> Vec<String> {
    let tables = conn
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type = 'table' \
             AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'seaql_%' ORDER BY name",
        ))
        .await
        .expect("tables");
    let mut details = Vec::new();
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
            details.push(format!(
                "{name}.column:{}:{}:{}:{}:{}",
                column.try_get::<String>("", "name").expect("column"),
                column.try_get::<String>("", "type").expect("type"),
                column.try_get::<i64>("", "notnull").expect("notnull"),
                column
                    .try_get::<String>("", "dflt_value")
                    .unwrap_or_default(),
                column.try_get::<i64>("", "pk").expect("pk"),
            ));
        }
    }
    details
}

/// The database after 0009 (0010 not applied), with one existing session row.
async fn pre_0010_database() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Db::connect(&dir.path().join("old.db"))
        .await
        .expect("connect");
    nekowriter_migration::up_to(db.conn(), 9)
        .await
        .expect("migrate to 0009");
    db.conn()
        .execute_unprepared(
            "INSERT INTO agent_sessions
               (id, role, kind, model_profile_id, model_snapshot, system_prompt, tools, status,
                counters, usage, created_at, updated_at)
             VALUES
               (1, 'writer', 'chapter_write', NULL, '{}', '', '[]', 'paused', '{}', '{}',
                '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');",
        )
        .await
        .expect("existing session");
    for column in COLUMNS {
        assert_eq!(
            column_count(db.conn(), column).await,
            0,
            "pre-0010 {column}"
        );
    }
    (dir, db)
}

#[tokio::test]
async fn migration_adds_the_columns_and_keeps_the_old_defaults() {
    let (_dir, db) = pre_0010_database().await;
    nekowriter_migration::up(db.conn())
        .await
        .expect("0010 applies");

    for column in COLUMNS {
        assert_eq!(column_count(db.conn(), column).await, 1, "{column} once");
    }
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0010_tool_error_limit'"
        )
        .await,
        1
    );
    let row = db
        .conn()
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT tool_error_streak_limit, research_max_tool_calls, research_max_entries \
             FROM agent_sessions WHERE id = 1",
        ))
        .await
        .expect("session")
        .expect("row");
    for (index, (column, expected)) in COLUMNS.iter().zip(DEFAULTS).enumerate() {
        let _ = index;
        assert_eq!(
            row.try_get::<i64>("", column).expect(column),
            expected,
            "{column} default for an existing session"
        );
    }
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM pragma_foreign_key_check"
        )
        .await,
        0,
        "no dangling foreign keys"
    );
}

#[tokio::test]
async fn fresh_and_upgraded_databases_have_the_same_schema_shape() {
    let (_old_dir, old) = pre_0010_database().await;
    nekowriter_migration::up(old.conn())
        .await
        .expect("upgrade old database");
    let old_details = schema_details(old.conn()).await;

    let fresh_dir = tempfile::tempdir().expect("fresh dir");
    let fresh = Db::connect(&fresh_dir.path().join("fresh.db"))
        .await
        .expect("connect fresh");
    nekowriter_migration::up(fresh.conn())
        .await
        .expect("migrate fresh database");

    assert_eq!(
        schema_details(fresh.conn()).await,
        old_details,
        "fresh and upgraded schemas disagree about a table or column"
    );
}

#[tokio::test]
async fn a_broken_copy_rolls_back_and_can_be_retried() {
    let (_dir, db) = pre_0010_database().await;
    db.conn()
        .execute_unprepared(
            "ALTER TABLE agent_sessions ADD COLUMN research_max_entries \
             INTEGER NOT NULL DEFAULT 25",
        )
        .await
        .expect("pre-seed the conflicting column");
    let baseline = schema_details(db.conn()).await;
    let before_sessions = scalar(db.conn(), "SELECT count(*) AS n FROM agent_sessions").await;

    let error = nekowriter_migration::up(db.conn())
        .await
        .expect_err("the broken copy must fail loudly");
    assert!(
        error.to_string().contains("duplicate column name"),
        "the first ALTER must fail, got: {error}"
    );
    assert_eq!(
        schema_details(db.conn()).await,
        baseline,
        "the failed migration changed the schema"
    );
    assert_eq!(
        scalar(db.conn(), "SELECT count(*) AS n FROM agent_sessions").await,
        before_sessions
    );
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0010_tool_error_limit'"
        )
        .await,
        0,
        "the failed migration must not be recorded"
    );
    // The third `ALTER` is the one that conflicts; the first two must have been rolled back.
    assert_eq!(column_count(db.conn(), "tool_error_streak_limit").await, 0);
    assert_eq!(column_count(db.conn(), "research_max_tool_calls").await, 0);
    assert_eq!(column_count(db.conn(), "research_max_entries").await, 1);

    db.conn()
        .execute_unprepared("ALTER TABLE agent_sessions DROP COLUMN research_max_entries")
        .await
        .expect("repair the copy");
    nekowriter_migration::up(db.conn())
        .await
        .expect("retry after repairing the copy");
    for column in COLUMNS {
        assert_eq!(
            column_count(db.conn(), column).await,
            1,
            "{column} after retry"
        );
    }
}
