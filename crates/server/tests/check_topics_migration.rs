//! Migration 0011 — knowledge-check research topics (spec/02 §1–§2, §5; D-168).
//!
//! The fixture is a database at the pre-0011 shape: a conceiving book, a done knowledge check,
//! three conception drafts and two topics that have already run. The migration must add
//! `research_tasks.check_id`, leave every row intact with `check_id IS NULL`, and expose the
//! self-referencing FK action: deleting the check sets its topics back to ordinary tasks.

use nekowriter::db::Db;
use sea_orm::{ConnectionTrait, DbBackend, Statement};

const FIXTURE: &str = r#"
INSERT INTO books(id, title, idea, genre, writing_language, status, settings, model_overrides,
                  conception, created_at, updated_at)
VALUES
  (1, 'Research first', 'a conceiving book', 'xianxia', 'zh-CN', 'conceiving', '{}', '{}', '{}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO research_tasks
  (id, book_id, scope_name, topic, kind, origin, requirements, language, status, report,
   entries_created, created_at, updated_at)
VALUES
  (1, 1, '', '', 'preparation', 'conception', '', 'zh-CN', 'done', 'the check report', 3,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 1, 'History', 'Draft one', 'topic', 'conception', '', 'zh-CN', 'draft', '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (3, 1, 'History', 'Draft two', 'topic', 'conception', '', 'zh-CN', 'draft', '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (4, 1, 'Lore', 'Draft three', 'topic', 'conception', '', 'zh-CN', 'draft', '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (5, 1, 'History', 'Run one', 'topic', 'author', '', 'zh-CN', 'done', 'done report', 2,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (6, 1, 'Lore', 'Run two', 'topic', 'conception', '', 'zh-CN', 'failed', '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
"#;

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
            "SELECT count(*) AS n FROM pragma_table_info('research_tasks') WHERE name = '{column}'"
        ),
    )
    .await
}

async fn index_count(conn: &sea_orm::DatabaseConnection) -> i64 {
    scalar(
        conn,
        "SELECT count(*) AS n FROM pragma_index_list('research_tasks') \
         WHERE name = 'research_tasks_check'",
    )
    .await
}

async fn research_schema(conn: &sea_orm::DatabaseConnection) -> String {
    let row = conn
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT group_concat(sql, char(10)) AS schema FROM sqlite_master \
             WHERE name IN ('research_tasks', 'research_tasks_check') ORDER BY name",
        ))
        .await
        .expect("schema")
        .expect("row");
    row.try_get::<String>("", "schema").unwrap_or_default()
}

async fn pre_0011_database() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Db::connect(&dir.path().join("old.db"))
        .await
        .expect("connect");
    nekowriter_migration::up_to(db.conn(), 10)
        .await
        .expect("migrate to 0010");
    db.conn()
        .execute_unprepared(FIXTURE)
        .await
        .expect("fixture");
    assert_eq!(column_count(db.conn(), "check_id").await, 0);
    (dir, db)
}

#[tokio::test]
async fn migration_adds_check_id_and_leaves_every_row_intact() {
    let (_dir, db) = pre_0011_database().await;
    nekowriter_migration::up(db.conn())
        .await
        .expect("0011 applies");

    assert_eq!(column_count(db.conn(), "check_id").await, 1);
    assert_eq!(index_count(db.conn()).await, 1);
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0011_check_topics'"
        )
        .await,
        1
    );
    assert_eq!(
        scalar(db.conn(), "SELECT count(*) AS n FROM research_tasks").await,
        6,
        "the row-count guard keeps every task"
    );
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM research_tasks WHERE check_id IS NOT NULL"
        )
        .await,
        0,
        "no pre-0011 task belonged to a check"
    );
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM pragma_foreign_key_check"
        )
        .await,
        0
    );
    // The check row and its report survive; the migration changes no data.
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM research_tasks WHERE id = 1 AND kind = 'preparation' \
             AND status = 'done' AND report = 'the check report'"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn deleting_the_check_sets_its_topics_to_null() {
    let (_dir, db) = pre_0011_database().await;
    nekowriter_migration::up(db.conn())
        .await
        .expect("0011 applies");
    db.conn()
        .execute_unprepared("UPDATE research_tasks SET check_id = 1 WHERE id IN (2, 5)")
        .await
        .expect("attach topics");
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM research_tasks WHERE check_id = 1"
        )
        .await,
        2
    );

    db.conn()
        .execute_unprepared("DELETE FROM research_tasks WHERE id = 1")
        .await
        .expect("delete the check");

    assert_eq!(
        scalar(db.conn(), "SELECT count(*) AS n FROM research_tasks").await,
        5
    );
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM research_tasks WHERE check_id IS NULL"
        )
        .await,
        5,
        "ON DELETE SET NULL makes the topics ordinary tasks"
    );
}

#[tokio::test]
async fn a_broken_copy_rolls_back_and_can_be_retried() {
    let (_dir, db) = pre_0011_database().await;
    // Pre-seed the *index*: the ALTER succeeds, the index creation fails, and the transaction
    // must roll the new column back with it.
    db.conn()
        .execute_unprepared("CREATE INDEX research_tasks_check ON research_tasks(book_id)")
        .await
        .expect("pre-seed the conflicting index");
    let baseline = research_schema(db.conn()).await;
    let before = scalar(db.conn(), "SELECT count(*) AS n FROM research_tasks").await;

    let error = nekowriter_migration::up(db.conn())
        .await
        .expect_err("the broken copy must fail loudly");
    assert!(
        error.to_string().contains("already exists"),
        "the index creation must fail, got: {error}"
    );
    assert_eq!(
        research_schema(db.conn()).await,
        baseline,
        "the failed migration changed the schema"
    );
    assert_eq!(
        scalar(db.conn(), "SELECT count(*) AS n FROM research_tasks").await,
        before
    );
    assert_eq!(
        column_count(db.conn(), "check_id").await,
        0,
        "the failed index creation rolled the column back"
    );
    assert_eq!(
        index_count(db.conn()).await,
        1,
        "the pre-seeded index is the only one"
    );
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0011_check_topics'"
        )
        .await,
        0,
        "the failed migration must not be recorded"
    );

    db.conn()
        .execute_unprepared("DROP INDEX research_tasks_check")
        .await
        .expect("repair the copy");
    nekowriter_migration::up(db.conn())
        .await
        .expect("retry after repairing the copy");
    assert_eq!(column_count(db.conn(), "check_id").await, 1);
    assert_eq!(index_count(db.conn()).await, 1);
}
