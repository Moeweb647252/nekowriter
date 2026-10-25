//! M7 review: independent orphan-index-row probes (spec/02 §5.11, D-030).
//!
//! These delete rows through paths the shipped tests do not cover (a single entry, a chapter, a
//! track) and count `fts_*`/`vec_*` rows against their sources with own SQL.

use axum::http::StatusCode;
use nekowriter::services::indexer;
use nekowriter::testing::TestApp;
use sea_orm::ConnectionTrait;
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

async fn count_rows(app: &TestApp, table: &str) -> i64 {
    count(app, &format!("SELECT count(*) AS c FROM {table}")).await
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

#[tokio::test]
async fn deleting_entry_cascades_chunks_fts_and_vectors() {
    let app = TestApp::new().await;
    let scope_id = scope(&app, "History").await;
    let entry_id = entry(&app, scope_id, "Tea", "# Tea\n\nSong tea houses.").await;
    indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(count_rows(&app, "knowledge_chunks").await, 1);
    assert_eq!(count_rows(&app, "fts_knowledge").await, 1);
    assert_eq!(count_rows(&app, "vec_knowledge").await, 1);

    let (status, _) = app
        .delete(&format!("/api/knowledge/entries/{entry_id}"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    println!(
        "after entry delete: chunks={} fts={} vec={}",
        count_rows(&app, "knowledge_chunks").await,
        count_rows(&app, "fts_knowledge").await,
        count_rows(&app, "vec_knowledge").await
    );
    assert_eq!(count_rows(&app, "knowledge_chunks").await, 0);
    assert_eq!(
        count_rows(&app, "fts_knowledge").await,
        0,
        "no fts row outlives its chunk"
    );
    assert_eq!(
        count_rows(&app, "vec_knowledge").await,
        0,
        "no vec row outlives its chunk"
    );
}

#[tokio::test]
async fn deleting_a_chapter_cascades_its_summary_and_passage_docs() {
    let app = TestApp::new().await;
    let book = app.create_book("Orphan Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Sword Tomb").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.put_text(chapter_id, 0, "Lin Fan enters the tomb.\n\nHe waits.")
        .await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "submit: {body}");
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "approve: {body}");
    app.patch(
        &format!("/api/chapters/{chapter_id}"),
        json!({ "summary": "Lin Fan enters the tomb." }),
    )
    .await;
    indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(count_rows(&app, "search_docs").await, 2);
    assert_eq!(count_rows(&app, "fts_book").await, 2);
    assert_eq!(count_rows(&app, "vec_book").await, 2);

    let (status, body) = app.delete(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    println!(
        "after chapter delete: docs={} fts={} vec={}",
        count_rows(&app, "search_docs").await,
        count_rows(&app, "fts_book").await,
        count_rows(&app, "vec_book").await
    );
    assert_eq!(count_rows(&app, "search_docs").await, 0);
    assert_eq!(
        count_rows(&app, "fts_book").await,
        0,
        "no fts row outlives its doc"
    );
    assert_eq!(
        count_rows(&app, "vec_book").await,
        0,
        "no vec row outlives its doc"
    );
}

#[tokio::test]
async fn deleting_an_isolated_track_cascades_its_index_rows() {
    let app = TestApp::new().await;
    let book = app.create_book("Track Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    app.execute(&format!(
        "INSERT INTO tracks(id, book_id, kind, title, canon_mode, ord, created_at, updated_at) \
         VALUES (900, {book_id}, 'side', 'Side', 'isolated', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    app.execute(&format!(
        "INSERT INTO search_docs(id, book_id, track_id, kind, ref_id, title, body, content_hash, updated_at) \
         VALUES (901, {book_id}, 900, 'entity', 42, 'Secret', 'the secret keeper', 'h1', '2026-01-01T00:00:00Z'); \
         INSERT INTO fts_book(rowid, title, body) VALUES (901, 'secret', 'the secret keeper')"
    ))
    .await;
    indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(count_rows(&app, "vec_book").await, 1);

    app.execute("DELETE FROM tracks WHERE id = 900").await;
    println!(
        "after track delete: docs={} fts={} vec={}",
        count_rows(&app, "search_docs").await,
        count_rows(&app, "fts_book").await,
        count_rows(&app, "vec_book").await
    );
    assert_eq!(count_rows(&app, "search_docs").await, 0);
    assert_eq!(count_rows(&app, "fts_book").await, 0);
    assert_eq!(count_rows(&app, "vec_book").await, 0);
}
