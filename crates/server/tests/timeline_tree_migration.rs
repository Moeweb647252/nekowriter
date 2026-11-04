//! Migration 0013 — the timeline tree and foreshadowing anchors (spec/02 §1–§5, D-170).
//!
//! The fixture is a database at the pre-0013 shape: two books with timeline rows, foreshadowing
//! text, and a change-log row for a timeline event whose id is higher than every live row. The
//! migration must create one contentless root per book, reparent every old row as an `occurred`
//! entry under it, keep the foreshadowing text unanchored, and leave 0001 untouched. A broken copy
//! must roll back to a retryable database, and a fresh database must have the same schema as the
//! upgraded one.

use std::collections::BTreeMap;

use nekowriter::db::Db;
use sea_orm::{ConnectionTrait, DbBackend, Statement};

const FIXTURE: &str = r#"
INSERT INTO books(id, title, idea, genre, writing_language, status, settings, model_overrides,
                  conception, created_at, updated_at)
VALUES
  (1, 'First', '', '', 'zh-CN', 'active', '{}', '{}', '{}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 'Second', '', '', 'en', 'active', '{}', '{}', '{}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO timeline_events
  (id, book_id, track_id, position, story_time, title, description, chapter_id, importance,
   created_at, updated_at)
VALUES
  (1, 1, NULL, 1024, 'Day 1', 'First event', 'first description', NULL, 2,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 1, NULL, 2048, 'Day 2', 'Second event', '', NULL, 3,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (3, 1, NULL, 3072, '', 'Third event', '', NULL, 1,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (4, 2, NULL, 1024, 'Year 2', 'Other book event', '', NULL, 2,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO foreshadowing
  (id, book_id, track_id, title, description, planned_payoff, status, planted_chapter_id,
   resolved_chapter_id, resolution, importance, created_at, updated_at)
VALUES
  (1, 1, NULL, 'Broken hilt', 'A hilt with no blade.', '卷三第 50—53 章：剑归原主。', 'open',
   NULL, NULL, '', 3, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 2, NULL, 'Old debt', '', 'The debt returns.', 'open', NULL, NULL, '', 2,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

-- A deleted event whose logged id is above every live row: the new roots must start above it too.
INSERT INTO change_log
  (id, book_id, track_id, chapter_id, session_id, actor, target_type, target_id, op, before, after,
   note, reverts_id, reverted_by_id, created_at)
VALUES
  (1, 1, NULL, NULL, NULL, 'user', 'timeline_event', 777, 'delete', NULL, NULL, '', NULL, NULL,
   '2026-01-01T00:00:00Z');
"#;

async fn scalar(conn: &sea_orm::DatabaseConnection, sql: &str) -> i64 {
    let row = conn
        .query_one_raw(Statement::from_string(DbBackend::Sqlite, sql))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i64>("", "n").unwrap_or(-1)
}

async fn column_count(conn: &sea_orm::DatabaseConnection, table: &str, column: &str) -> i64 {
    scalar(
        conn,
        &format!("SELECT count(*) AS n FROM pragma_table_info('{table}') WHERE name = '{column}'"),
    )
    .await
}

/// A fresh database migrated all the way.
async fn fresh_database() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Db::connect(&dir.path().join("fresh.db"))
        .await
        .expect("connect");
    nekowriter_migration::up(db.conn())
        .await
        .expect("migrate fresh");
    (dir, db)
}

/// A database at the pre-0013 shape with the fixture.
async fn pre_0013_database() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Db::connect(&dir.path().join("old.db"))
        .await
        .expect("connect");
    nekowriter_migration::up_to(db.conn(), 11)
        .await
        .expect("migrate to 0011");
    db.conn()
        .execute_unprepared(FIXTURE)
        .await
        .expect("fixture");
    assert_eq!(
        column_count(db.conn(), "timeline_events", "parent_id").await,
        0
    );
    assert_eq!(
        column_count(db.conn(), "foreshadowing", "payoff_event_id").await,
        0
    );
    (dir, db)
}

#[tokio::test]
async fn migration_creates_one_root_per_book_and_reparents_history() {
    let (_dir, db) = pre_0013_database().await;
    nekowriter_migration::up(db.conn())
        .await
        .expect("0013 applies");

    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0013_timeline_tree'"
        )
        .await,
        1
    );
    // One root per book, with empty content, no track/chapter/parent and position 0.
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM timeline_events WHERE kind = 'root'"
        )
        .await,
        2
    );
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM timeline_events \
             WHERE kind = 'root' AND (track_id IS NOT NULL OR chapter_id IS NOT NULL \
               OR parent_id IS NOT NULL OR position <> 0 OR title <> '' OR description <> '' \
               OR story_time <> '' OR status <> 'occurred')"
        )
        .await,
        0,
        "the root stores no content and anchors only the tree"
    );
    // The roots start above every live id and every logged timeline id (777).
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM timeline_events r \
             WHERE r.kind = 'root' AND r.id <= 777"
        )
        .await,
        0,
        "root ids are above the change log's highest timeline id"
    );
    // Every old row is a top-level occurred entry of its own book's root.
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM timeline_events e \
             JOIN timeline_events r ON r.book_id = e.book_id AND r.kind = 'root' \
             WHERE e.kind = 'entry' AND e.status = 'occurred' AND e.parent_id = r.id"
        )
        .await,
        4
    );
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM timeline_events WHERE kind = 'entry' AND parent_id IS NULL"
        )
        .await,
        0,
        "no entry is left without a parent"
    );
    // The old content survives unchanged.
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM timeline_events \
             WHERE id = 1 AND title = 'First event' AND position = 1024"
        )
        .await,
        1
    );
    // Foreshadowing keeps its text and is unanchored.
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM foreshadowing \
             WHERE payoff_event_id IS NULL AND payoff_position IS NULL"
        )
        .await,
        2
    );
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM foreshadowing WHERE title = 'Broken hilt' \
             AND planned_payoff = '卷三第 50—53 章：剑归原主。'"
        )
        .await,
        1
    );
    // A second root for a book violates the partial unique index.
    assert!(
        db.conn()
            .execute_unprepared(
                "INSERT INTO timeline_events \
                 (id, book_id, track_id, position, story_time, title, description, chapter_id, \
                  importance, kind, status, created_at, updated_at) \
                 VALUES (100000, 1, NULL, 0, '', '', '', NULL, 2, 'root', 'occurred', \
                         '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
            )
            .await
            .is_err(),
        "a book has exactly one root"
    );
}

#[tokio::test]
async fn a_broken_copy_rolls_back_and_retries() {
    let (_dir, db) = pre_0013_database().await;
    db.conn()
        .execute_unprepared(
            "CREATE TRIGGER fail_root BEFORE INSERT ON timeline_events \
             WHEN NEW.kind = 'root' BEGIN SELECT RAISE(ABORT, 'root insert broken'); END;",
        )
        .await
        .expect("trigger");
    let error = nekowriter_migration::up(db.conn()).await;
    assert!(error.is_err(), "the broken copy fails");
    assert_eq!(
        column_count(db.conn(), "timeline_events", "parent_id").await,
        0,
        "the failed migration rolled the columns back"
    );
    assert_eq!(
        column_count(db.conn(), "foreshadowing", "payoff_event_id").await,
        0
    );
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0013_timeline_tree'"
        )
        .await,
        0
    );
    db.conn()
        .execute_unprepared("DROP TRIGGER fail_root")
        .await
        .expect("drop trigger");
    nekowriter_migration::up(db.conn())
        .await
        .expect("the repaired copy retries");
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM timeline_events WHERE kind = 'root'"
        )
        .await,
        2
    );
}

#[tokio::test]
async fn a_new_book_has_its_root() {
    let app = nekowriter::testing::TestApp::new().await;
    let book = app.create_book("Rooted").await;
    let book_id = book["id"].as_i64().expect("book id");
    let count = scalar(
        app.db().conn(),
        &format!(
            "SELECT count(*) AS n FROM timeline_events WHERE book_id = {book_id} AND kind = 'root'"
        ),
    )
    .await;
    assert_eq!(count, 1, "book creation inserts exactly one root");
}

#[tokio::test]
async fn a_fresh_database_matches_the_upgraded_schema() {
    let (_fresh_dir, fresh) = fresh_database().await;
    let (_up_dir, upgraded) = pre_0013_database().await;
    nekowriter_migration::up(upgraded.conn())
        .await
        .expect("0013 applies");

    let fresh_schema = schema_dump(fresh.conn()).await;
    let upgraded_schema = schema_dump(upgraded.conn()).await;
    assert_eq!(
        fresh_schema, upgraded_schema,
        "the upgraded schema equals a fresh database"
    );

    for table in ["timeline_events", "foreshadowing"] {
        assert_eq!(
            table_details(fresh.conn(), table).await,
            table_details(upgraded.conn(), table).await,
            "{table} details differ between fresh and upgraded databases"
        );
    }
}

fn normalize_schema_sql(sql: &str) -> String {
    let without_comments: String = sql
        .lines()
        .map(|line| line.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    without_comments
        .replace('"', "")
        .replace("IF NOT EXISTS ", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" ,", ",")
        .replace(" )", ")")
}

async fn schema_dump(conn: &sea_orm::DatabaseConnection) -> BTreeMap<String, (String, String)> {
    conn.query_all_raw(Statement::from_string(
        DbBackend::Sqlite,
        "SELECT type, name, sql FROM sqlite_master \
         WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' \
           AND name <> 'seaql_migrations' ORDER BY type, name",
    ))
    .await
    .expect("schema dump")
    .into_iter()
    .map(|row| {
        (
            row.try_get::<String>("", "name").unwrap_or_default(),
            (
                row.try_get::<String>("", "type").unwrap_or_default(),
                normalize_schema_sql(&row.try_get::<String>("", "sql").unwrap_or_default()),
            ),
        )
    })
    .collect()
}

async fn table_details(conn: &sea_orm::DatabaseConnection, table: &str) -> Vec<String> {
    let mut details = Vec::new();
    for column in conn
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            format!("PRAGMA table_info(\"{table}\")"),
        ))
        .await
        .expect("columns")
    {
        details.push(format!(
            "column:{}:{}:{}:{}:{}",
            column.try_get::<String>("", "name").expect("name"),
            column.try_get::<String>("", "type").expect("type"),
            column.try_get::<i64>("", "notnull").expect("notnull"),
            column
                .try_get::<String>("", "dflt_value")
                .unwrap_or_default(),
            column.try_get::<i64>("", "pk").expect("pk"),
        ));
    }
    for key in conn
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            format!("PRAGMA foreign_key_list(\"{table}\")"),
        ))
        .await
        .expect("foreign keys")
    {
        details.push(format!(
            "fk:{}:{}:{}:{}:{}",
            key.try_get::<String>("", "from").expect("from"),
            key.try_get::<String>("", "table").expect("table"),
            key.try_get::<String>("", "to").expect("to"),
            key.try_get::<String>("", "on_update").expect("on_update"),
            key.try_get::<String>("", "on_delete").expect("on_delete"),
        ));
    }
    for index in conn
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            format!("PRAGMA index_list(\"{table}\")"),
        ))
        .await
        .expect("indexes")
    {
        details.push(format!(
            "index:{}:{}:{}",
            index.try_get::<String>("", "name").expect("index name"),
            index.try_get::<i64>("", "unique").expect("unique"),
            index.try_get::<i64>("", "partial").expect("partial"),
        ));
    }
    details.sort();
    details
}
