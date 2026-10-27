//! One active relation per ordered pair in the **schema** (spec/02 §2, spec/06 §1.2, D-143).
//!
//! Three things have to hold, and only the first is obvious:
//!
//! 1. A fresh database rejects a second active relation for one ordered pair, whatever the kind.
//! 2. A database created **before** the rule existed is upgraded to it. The author's own database is
//!    the case that matters: `seaql_migrations` already records `m0001_init`, so only
//!    `m0003_relation_pair_unique` reaches it.
//! 3. The upgrade refuses to guess. A database that already holds a duplicate pair cannot have the
//!    index created, and the migration reports the offending ids instead of picking a survivor.
//!
//! The rule is enforced twice on purpose: the service returns a useful `conflict.exists`, and the
//! index catches every path that does not go through the service (a merge, a revert, a hand-written
//! statement).

use axum::http::StatusCode;
use nekowriter::testing::TestApp;
use sea_orm::ConnectionTrait;
use serde_json::json;

mod canon_fixture;

use canon_fixture::book;

/// Creates an entity and returns its id.
async fn character(app: &TestApp, book_id: i32, name: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": name, "summary": "s" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_i64().expect("id") as i32
}

/// A raw insert that bypasses the service, so only the index can reject it.
fn raw_relation_sql(book_id: i32, source: i32, target: i32, kind: &str) -> String {
    format!(
        "INSERT INTO relations(book_id, source_id, target_id, kind, inverse_kind, description, \
         status, created_at, updated_at) VALUES ({book_id}, {source}, {target}, '{kind}', '', '', \
         'active', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    )
}

/// The fresh schema carries the pair-keyed index: a second active row for one pair is refused by
/// SQLite itself, with a kind that differs.
#[tokio::test]
async fn a_fresh_database_rejects_a_second_active_relation_for_a_pair() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let source = character(&app, book_id, "Lin Fan").await;
    let target = character(&app, book_id, "Zhang San").await;
    app.execute(&raw_relation_sql(book_id, source, target, "ally"))
        .await;
    let clash = app
        .try_execute(&raw_relation_sql(book_id, source, target, "sworn ally"))
        .await;
    assert!(
        clash.is_err(),
        "the pair-keyed unique index refuses a second active relation for the pair"
    );

    // The reverse direction is still free (the pair is ordered).
    let reverse = app
        .try_execute(&raw_relation_sql(book_id, target, source, "mentor"))
        .await;
    assert!(
        reverse.is_ok(),
        "the reverse direction is a separate pair and stays allowed"
    );
}

/// An `ended` row never occupies the active slot: the index is partial on `status = 'active'`.
#[tokio::test]
async fn the_index_only_covers_active_rows() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let source = character(&app, book_id, "Lin Fan").await;
    let target = character(&app, book_id, "Zhang San").await;
    app.execute(&format!(
        "INSERT INTO relations(book_id, source_id, target_id, kind, inverse_kind, description, \
         status, created_at, updated_at) VALUES ({book_id}, {source}, {target}, 'ally', '', '', \
         'ended', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    let active = app
        .try_execute(&raw_relation_sql(book_id, source, target, "sworn ally"))
        .await;
    assert!(
        active.is_ok(),
        "history never blocks an active relation for the pair"
    );
}

/// Builds a database in the state the author's is in: migrated to `m0001_init` only, with the old
/// `kind`-keyed indexes and one duplicate pair. Returns the directory and the path.
async fn pre_pair_rule_database(duplicate: bool) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("author.db");
    let db = nekowriter::db::Db::connect(&path).await.expect("connect");
    nekowriter_migration::up(db.conn()).await.expect("migrate");
    // Put the old indexes back and rewind the migration record: this is a database created before
    // the pair rule, which only a later migration file can reach.
    db.conn()
        .execute_unprepared(
            "DROP INDEX relations_active_shared;
             DROP INDEX relations_active_track;
             CREATE UNIQUE INDEX relations_active_shared ON relations(source_id, target_id, kind)
               WHERE status = 'active' AND track_id IS NULL;
             CREATE UNIQUE INDEX relations_active_track
               ON relations(track_id, source_id, target_id, kind)
               WHERE status = 'active' AND track_id IS NOT NULL;
             DELETE FROM seaql_migrations WHERE version = 'm0003_relation_pair_unique';",
        )
        .await
        .expect("rewind to the pre-pair-rule schema");

    // A book, two entities and one relation, inserted through raw SQL that matches the schema.
    db.conn()
        .execute_unprepared(
            "INSERT INTO books(title, writing_language, status, settings, created_at, updated_at) \
             VALUES ('Author Book', 'zh-CN', 'active', '{}', '2026-01-01T00:00:00Z', \
                     '2026-01-01T00:00:00Z');
             INSERT INTO entities(book_id, kind, name, created_at, updated_at) \
             VALUES (1, 'character', 'Lin Fan', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
                    (1, 'character', 'Zhang San', '2026-01-01T00:00:00Z', \
                     '2026-01-01T00:00:00Z');",
        )
        .await
        .expect("book and entities");
    // One active relation, plus a second one with different wording when `duplicate` is set: legal
    // under the old `kind`-keyed index, refused under the pair-keyed one — the author's situation.
    let mut relations = "INSERT INTO relations(book_id, source_id, target_id, kind, created_at, \
                         updated_at) VALUES (1, 1, 2, 'ally', '2026-01-01T00:00:00Z', \
                         '2026-01-01T00:00:00Z');"
        .to_owned();
    if duplicate {
        relations.push_str(
            "INSERT INTO relations(book_id, source_id, target_id, kind, created_at, updated_at) \
             VALUES (1, 1, 2, 'sworn ally', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');",
        );
    }
    db.conn()
        .execute_unprepared(&relations)
        .await
        .expect("the seed relations");
    drop(db);
    (dir, path)
}

/// The upgrade converts the indexes on a clean database and keeps the rows.
#[tokio::test]
async fn the_upgrade_converts_the_indexes_and_keeps_the_relations() {
    let (_dir, path) = pre_pair_rule_database(false).await;
    let db = nekowriter::db::Db::connect(&path).await.expect("reconnect");
    nekowriter_migration::up(db.conn()).await.expect("upgrade");

    // The row survived, and the pair rule now applies to a *different* wording.
    let rows = db
        .conn()
        .query_all_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT kind FROM relations",
        ))
        .await
        .expect("query");
    assert_eq!(rows.len(), 1, "the existing relation survives");
    assert_eq!(
        rows[0].try_get::<String>("", "kind").unwrap_or_default(),
        "ally"
    );
    assert!(
        db.conn()
            .execute_unprepared(
                "INSERT INTO relations(book_id, source_id, target_id, kind, inverse_kind, \
                                       description, status, created_at, updated_at) \
                 VALUES (1, 1, 2, 'sworn ally', '', '', 'active', '2026-01-01T00:00:00Z', \
                         '2026-01-01T00:00:00Z');"
            )
            .await
            .is_err(),
        "after the upgrade the pair rule holds for a re-worded kind"
    );

    // The migration is recorded once and a second run is a no-op.
    let count = db
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT count(*) AS n FROM seaql_migrations \
             WHERE version = 'm0003_relation_pair_unique'",
        ))
        .await
        .expect("query")
        .map(|row| row.try_get::<i64>("", "n").unwrap_or(0))
        .unwrap_or(0);
    assert_eq!(count, 1, "the upgrade is recorded exactly once");
    nekowriter_migration::up(db.conn())
        .await
        .expect("a second run is a no-op");
}

/// A database that already holds a duplicate pair cannot be upgraded, and the failure names the
/// rows instead of guessing. The alternative — picking a survivor — would rewrite the author's
/// canon silently.
#[tokio::test]
async fn the_upgrade_refuses_a_database_that_already_has_a_duplicate_pair() {
    let (_dir, path) = pre_pair_rule_database(true).await;
    let db = nekowriter::db::Db::connect(&path).await.expect("reconnect");
    let error = nekowriter_migration::up(db.conn())
        .await
        .expect_err("the upgrade must refuse a duplicate pair");
    let message = error.to_string();
    assert!(
        message.contains("0003_relation_pair_unique"),
        "the message names the migration: {message}"
    );
    assert!(
        message.contains("shared: #1 → #2"),
        "the message names the offending pair: {message}"
    );
    assert!(
        message.contains("ally") && message.contains("sworn ally"),
        "the message lists the colliding rows by kind and id: {message}"
    );

    // Nothing was touched: both rows are still there, unchanged.
    let rows = db
        .conn()
        .query_all_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT id, kind, status FROM relations ORDER BY id",
        ))
        .await
        .expect("query");
    assert_eq!(rows.len(), 2, "the migration never deletes a relation");
    assert_eq!(
        rows.iter()
            .filter(|row| row.try_get::<String>("", "status").unwrap_or_default() == "active")
            .count(),
        2,
        "and never silently ends one"
    );
}
