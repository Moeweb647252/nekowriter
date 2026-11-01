//! M7 review: knowledge service and search-tool probes (spec/07 §1, spec/05 §3).

use axum::http::StatusCode;
use nekowriter::db::entities::knowledge_chunks;
use nekowriter::search::hybrid_knowledge;
use nekowriter::services::indexer;
use nekowriter::testing::TestApp;
use sea_orm::{ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter};
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

async fn scope(app: &TestApp, name: &str) -> i32 {
    let (status, body) = app
        .post("/api/knowledge/scopes", json!({ "name": name }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "scope: {body}");
    body["id"].as_i64().expect("scope id") as i32
}

async fn entry(app: &TestApp, scope_id: i32, title: &str, content: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/knowledge/scopes/{scope_id}/entries"),
            json!({ "title": title, "content": content }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "entry: {body}");
    body["id"].as_i64().expect("entry id") as i32
}

/// A cursor page must resume where it stopped and must not repeat or skip a row (spec/10 §1).
#[tokio::test]
async fn entry_pages_use_an_id_cursor_without_gaps_or_repeats() {
    let app = TestApp::new().await;
    let scope_id = scope(&app, "History").await;
    let mut created = Vec::new();
    for index in 1..=7 {
        created.push(entry(&app, scope_id, &format!("Entry {index}"), "content").await);
    }
    let mut seen: Vec<i32> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let path = match &cursor {
            None => format!("/api/knowledge/scopes/{scope_id}/entries?limit=3"),
            Some(cursor) => {
                format!("/api/knowledge/scopes/{scope_id}/entries?limit=3&cursor={cursor}")
            }
        };
        let (status, body) = app.get(&path).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for item in body["items"].as_array().expect("items") {
            seen.push(item["id"].as_i64().expect("id") as i32);
        }
        match body["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    assert_eq!(seen.len(), 7, "no row is skipped or repeated: {seen:?}");
    assert_eq!(seen, created, "ascending id order");
}

/// A scope's visible-name search restriction is enforced per session kind; a scope that exists but
/// is not attached is not searched (spec/07 §1, spec/05 §3).
#[tokio::test]
async fn knowledge_search_ignores_unattached_scopes_and_their_vectors() {
    let app = TestApp::new().await;
    let book = app.create_book("Scope Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let attached = scope(&app, "Attached").await;
    let hidden = scope(&app, "Hidden").await;
    entry(
        &app,
        attached,
        "Sword tomb",
        "The sword tomb holds a spirit.",
    )
    .await;
    entry(&app, hidden, "Sword vault", "The sword vault is sealed.").await;
    app.put(
        &format!("/api/books/{book_id}/scopes"),
        json!({ "excluded_scope_ids": [hidden] }),
    )
    .await;
    indexer::run_once(app.state()).await.expect("indexer");
    let visible = nekowriter::services::knowledge::visible_scope_ids(app.db().conn(), book_id)
        .await
        .expect("visible");
    assert_eq!(visible, vec![attached]);

    // The vector index holds both scopes' chunks; the query path must not merge them.
    assert_eq!(
        count(&app, "SELECT count(*) AS c FROM vec_knowledge").await,
        2,
        "both scopes are embedded"
    );
    let hits = hybrid_knowledge(app.db().conn(), "sword", None, &visible, 10)
        .await
        .expect("hybrid");
    println!("visible hits: {hits:?}");
    assert!(
        hits.hits.iter().all(|hit| hit.scope_id == attached),
        "an unattached scope must not be searched: {hits:?}"
    );
}

/// Detaching a scope only removes visibility: the scope, its entries, chunks and vectors survive
/// (spec/07 §1).
#[tokio::test]
async fn detaching_a_scope_keeps_its_rows() {
    let app = TestApp::new().await;
    let book = app.create_book("Detach Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let scope_id = scope(&app, "History").await;
    entry(&app, scope_id, "Tea", "# Tea\n\nSong tea houses.").await;
    app.put(
        &format!("/api/books/{book_id}/scopes"),
        json!({ "scope_ids": [scope_id] }),
    )
    .await;
    indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(
        count(&app, "SELECT count(*) AS c FROM vec_knowledge").await,
        1
    );

    let (status, body) = app
        .put(
            &format!("/api/books/{book_id}/scopes"),
            json!({ "excluded_scope_ids": [scope_id] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        count(&app, "SELECT count(*) AS c FROM knowledge_scopes").await,
        1,
        "the scope survives"
    );
    assert_eq!(
        knowledge_chunks::Entity::find()
            .count(app.db().conn())
            .await
            .expect("count"),
        1
    );
    assert_eq!(
        count(&app, "SELECT count(*) AS c FROM vec_knowledge").await,
        1,
        "the vector survives"
    );
    assert_eq!(
        count(&app, "SELECT count(*) AS c FROM fts_knowledge").await,
        1
    );
}

/// An entry's scope cannot change after creation: there is no scope field in the patch and a
/// cross-scope title reuse is allowed (spec/07 §1).
#[tokio::test]
async fn an_entry_scope_is_fixed_after_creation() {
    let app = TestApp::new().await;
    let first = scope(&app, "First").await;
    let second = scope(&app, "Second").await;
    let entry_id = entry(&app, first, "Tea", "Song tea houses.").await;
    // The patch DTO has no `scope_id`; an unknown field is ignored by serde's default behavior,
    // so the scope must stay put even when the client sends one.
    let (status, body) = app
        .patch(
            &format!("/api/knowledge/entries/{entry_id}"),
            json!({ "title": "Tea houses", "scope_id": second }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["scope_id"], json!(first), "the scope is unchanged");
    // The chunk keeps the original `scope_id`, so the vector partition does not move.
    let chunk_scope: i64 = count(
        &app,
        &format!("SELECT count(*) AS c FROM knowledge_chunks WHERE scope_id = {second}"),
    )
    .await;
    assert_eq!(chunk_scope, 0);
    assert_eq!(
        count(
            &app,
            &format!("SELECT count(*) AS c FROM knowledge_chunks WHERE scope_id = {first}"),
        )
        .await,
        1
    );
}

/// The chunker runs in the entry transaction: a `PATCH` that changes the content but fails on a
/// unique title must leave the old chunks exactly as they were (no partial re-chunk).
#[tokio::test]
async fn a_failed_entry_patch_changes_no_chunk() {
    let app = TestApp::new().await;
    let scope_id = scope(&app, "History").await;
    let first = entry(&app, scope_id, "Tea", "# Tea\n\nSong tea houses.").await;
    entry(&app, scope_id, "Tea houses", "other").await;
    let before: Vec<(i32, String, String)> = knowledge_chunks::Entity::find()
        .filter(knowledge_chunks::COLUMN.entry_id.eq(first))
        .all(app.db().conn())
        .await
        .expect("chunks")
        .into_iter()
        .map(|row| (row.id, row.text, row.content_hash))
        .collect();
    let (status, body) = app
        .patch(
            &format!("/api/knowledge/entries/{first}"),
            json!({ "title": "Tea houses", "content": "# New\n\nnew body" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let after: Vec<(i32, String, String)> = knowledge_chunks::Entity::find()
        .filter(knowledge_chunks::COLUMN.entry_id.eq(first))
        .all(app.db().conn())
        .await
        .expect("chunks")
        .into_iter()
        .map(|row| (row.id, row.text, row.content_hash))
        .collect();
    assert_eq!(before, after, "the patch rolled back with its chunks");
}
