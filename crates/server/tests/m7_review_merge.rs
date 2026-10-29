//! M7 review: entity merge and change-log revert keep `search_docs` in step (spec/07 §2,
//! spec/02 §5.11).

use axum::http::StatusCode;
use nekowriter::db::entities::search_docs;
use nekowriter::search::maintenance;
use nekowriter::testing::TestApp;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde_json::json;

async fn count(app: &TestApp, sql: &str) -> i64 {
    let row = app
        .db()
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql,
        ))
        .await
        .expect("count query")
        .expect("count row");
    row.try_get::<i64>("", "c").expect("count value")
}

async fn entity(app: &TestApp, book_id: i32, name: &str, summary: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": name, "summary": summary }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "entity: {body}");
    body["id"].as_i64().expect("entity id") as i32
}

async fn doc(app: &TestApp, ref_id: i32) -> Option<search_docs::Model> {
    search_docs::Entity::find()
        .filter(search_docs::COLUMN.kind.eq("entity"))
        .filter(search_docs::COLUMN.ref_id.eq(ref_id))
        .one(app.db().conn())
        .await
        .expect("doc query")
}

/// Merging B into A deletes B; B's document must go with it (spec/02 §5.11).
#[tokio::test]
async fn a_merged_away_entity_leaves_no_search_doc() {
    let app = TestApp::new().await;
    let book = app.create_book("Merge Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let kept = entity(&app, book_id, "Lin Fan", "the protagonist").await;
    let merged = entity(&app, book_id, "Lin Fanzi", "another name").await;
    assert!(doc(&app, merged).await.is_some());

    // `POST /api/entities/:id/merge` merges `from_id` into `:id`.
    let (status, body) = app
        .post(
            &format!("/api/entities/{kept}/merge"),
            json!({ "from_id": merged }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "merge: {body}");
    assert!(
        doc(&app, merged).await.is_none(),
        "the merged-away entity's document must be removed"
    );
    assert!(
        doc(&app, kept).await.is_some(),
        "the survivor keeps its doc"
    );
    assert_eq!(
        count(
            &app,
            "SELECT count(*) AS c FROM fts_book WHERE rowid NOT IN (SELECT id FROM search_docs)",
        )
        .await,
        0,
        "no fts row outlives its doc"
    );
}

/// Reverting a delete recreates the entity with a new id and re-indexes it; the old id's document
/// must not come back (spec/07 §2).
#[tokio::test]
async fn a_reverted_delete_reindexes_under_the_new_id() {
    let app = TestApp::new().await;
    let book = app.create_book("Revert Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let entity_id = entity(&app, book_id, "Lin Fan", "the protagonist").await;
    let (status, _) = app.delete(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(doc(&app, entity_id).await.is_none());
    // The delete change is the newest one for the target, so it can be reverted.
    let (status, changes) = app
        .get(&format!("/api/books/{book_id}/changes?target_type=entity"))
        .await;
    assert_eq!(status, StatusCode::OK, "{changes}");
    let delete_change = changes["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["op"] == "delete"))
        .expect("the delete change");
    let change_id = delete_change["id"].as_i64().expect("change id") as i32;
    let (status, body) = app
        .post(&format!("/api/changes/{change_id}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "revert: {body}");
    // The revert is its own change row; its `target_id` is the recreated entity's fresh id (a
    // `Snapshot` carries no id: the change row's `target_id` names the target).
    assert_eq!(
        body["op"],
        json!("create"),
        "the revert records a create: {body}"
    );
    let new_id = body["target_id"].as_i64().expect("the recreated entity id") as i32;
    assert_ne!(new_id, entity_id, "a reverted delete gets a fresh id");
    assert!(doc(&app, new_id).await.is_some(), "the new row is indexed");
    assert!(
        doc(&app, entity_id).await.is_none(),
        "the old id has no document"
    );
    assert_eq!(
        count(&app, "SELECT count(*) AS c FROM search_docs").await,
        1
    );
    assert_eq!(count(&app, "SELECT count(*) AS c FROM fts_book").await, 1);
}

/// Deleting a side track removes its private canon rows and their documents (spec/02 §5.11, D-030).
#[tokio::test]
async fn deleting_a_track_removes_its_private_canon_documents() {
    let app = TestApp::new().await;
    let book = app.create_book("Track Canon Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    // A private entity of an isolated track, indexed through the normal hook.
    app.execute(&format!(
        "INSERT INTO tracks(id, book_id, kind, title, canon_mode, ord, created_at, updated_at) \
         VALUES (900, {book_id}, 'side', 'Side', 'isolated', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'); \
         INSERT INTO entities(id, book_id, track_id, kind, name, summary, importance, status, \
                             is_protagonist, created_at, updated_at) \
         VALUES (901, {book_id}, 900, 'character', 'Secret Keeper', 'only in the side story', 3, 'active', 0, \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    nekowriter::db::Db::write(app.db(), move |tx| {
        Box::pin(async move {
            maintenance::sync_entity(tx, book_id, 901)
                .await
                .map_err(|err| sea_orm::DbErr::Custom(err.message))?;
            Ok(())
        })
    })
    .await
    .expect("sync");
    assert!(doc(&app, 901).await.is_some());

    app.execute("DELETE FROM tracks WHERE id = 900").await;
    assert!(
        doc(&app, 901).await.is_none(),
        "the track cascade removes the private document"
    );
    assert_eq!(
        count(
            &app,
            "SELECT count(*) AS c FROM fts_book WHERE rowid NOT IN (SELECT id FROM search_docs)",
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &app,
            "SELECT count(*) AS c FROM vec_book WHERE doc_id NOT IN (SELECT id FROM search_docs)",
        )
        .await,
        0
    );
}
