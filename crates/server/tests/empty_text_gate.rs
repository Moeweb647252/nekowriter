//! A chapter whose paragraphs are all empty must not submit/approve (R10-03).

use axum::http::StatusCode;
use nekowriter::testing::{TestApp, assert_error};
use sea_orm::ConnectionTrait;
use serde_json::json;

async fn empty_paragraph_chapter(app: &TestApp) -> (i32, i32) {
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    let content = json!({
        "paragraphs": [{ "pid": "p1", "text": "" }],
        "next_pid": 2,
        "tail_open": false,
    })
    .to_string();
    app.execute(&format!(
        "UPDATE chapters SET content = '{}', word_count = 0, status = 'drafting' WHERE id = {chapter_id}",
        content.replace('\'', "''")
    ))
    .await;
    (book_id, chapter_id)
}

#[tokio::test]
async fn submit_refuses_a_chapter_with_only_empty_paragraphs() {
    let app = TestApp::new().await;
    let (_, chapter_id) = empty_paragraph_chapter(&app).await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "chapter.invalid_state");
    assert_eq!(
        app.db()
            .conn()
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                format!(
                    "SELECT count(*) AS n FROM chapter_versions WHERE chapter_id = {chapter_id}"
                ),
            ))
            .await
            .expect("query")
            .expect("row")
            .try_get::<i64>("", "n")
            .expect("count"),
        0
    );
}

#[tokio::test]
async fn approve_refuses_a_chapter_with_only_empty_paragraphs() {
    let app = TestApp::new().await;
    let (_, chapter_id) = empty_paragraph_chapter(&app).await;
    app.execute(&format!(
        "UPDATE chapters SET status = 'in_review' WHERE id = {chapter_id}"
    ))
    .await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "chapter.invalid_state");
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(chapter["status"], "in_review");
    assert!(chapter["official_version_id"].is_null());
}
