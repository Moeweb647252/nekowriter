//! Strict JSON decoding (M14 phase 2, plan §2.4–§2.5): a database row that no longer decodes is a
//! query error that names the column, never a silent default. A corrupt chapter must fail the
//! chapter read and must stop a write session from starting.

#![allow(dead_code)]

#[path = "agent_harness/mod.rs"]
mod base;

pub use base::*;

use axum::http::StatusCode;
use nekowriter::db::json::{Granularity, SessionKind};
use nekowriter::error::AppError;
use nekowriter::services::sessions::{self, CreateSession};
use serde_json::json;

/// Writes a chapter body the old fallback would have read as an empty chapter.
async fn corrupt_chapter_content(runtime: &Runtime, content: &str) {
    let sql = format!(
        "UPDATE chapters SET content = '{content}' WHERE id = {}",
        runtime.chapter_id
    );
    runtime
        .app
        .db()
        .write_unprepared(&sql)
        .await
        .expect("corrupt the stored chapter");
}

/// The GET path decodes through the entity, so the strict `JsonText` read fails the request.
#[tokio::test]
async fn corrupt_chapter_content_is_a_500_naming_the_column() {
    let runtime = runtime(Vec::new()).await;
    corrupt_chapter_content(&runtime, r#"{"paragraphs":3}"#).await;

    let (status, body) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let message = body.to_string();
    assert!(
        message.contains("content"),
        "the error names the column: {message}"
    );
    assert!(
        message.contains("corrupt JSON column"),
        "the error is the strict decode error: {message}"
    );
}

/// Starting a writer session reads the chapter; a corrupt row must stop it instead of starting with
/// an empty chapter.
#[tokio::test]
async fn a_write_session_refuses_to_start_on_a_corrupt_chapter() {
    let runtime = runtime(Vec::new()).await;
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/chapters/{}", runtime.chapter_id),
            json!({ "outline": "scene one", "reference_notes": "rain" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    corrupt_chapter_content(&runtime, r#"{"paragraphs":3}"#).await;

    let error: AppError = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            granularity: Some(Granularity::Paragraph),
            auto_pause: Some(true),
            ..CreateSession::default()
        },
    )
    .await
    .expect_err("a corrupt chapter must refuse the write session");
    assert!(
        error.to_string().contains("content"),
        "the refusal names the column: {error}"
    );
}

/// The column CHECK still rejects the empty text that serialization failure would produce.
#[tokio::test]
async fn an_empty_json_text_is_rejected_by_the_column_check() {
    let runtime = runtime(Vec::new()).await;
    let sql = format!(
        "UPDATE chapters SET content = '' WHERE id = {}",
        runtime.chapter_id
    );
    let error = runtime
        .app
        .db()
        .write_unprepared(&sql)
        .await
        .expect_err("the CHECK rejects empty text");
    assert!(
        error.to_string().contains("CHECK"),
        "the column check rejects empty text: {error}"
    );
}
