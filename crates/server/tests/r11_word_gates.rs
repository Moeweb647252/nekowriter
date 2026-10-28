//! Round-11 reviewer probe (scratch, not committed): gate coverage for R10-03.

mod agent_write_harness;

use std::time::Duration;

use agent_write_harness::{runtime, start_writer};
use nekowriter::db::json::{Granularity, SessionKind, SessionStatus};
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter::testing::TestApp;
use serde_json::json;

async fn set_content(app: &TestApp, chapter_id: i32, text: &str) {
    app.execute(&format!(
        "UPDATE chapters SET content = '{{\"paragraphs\":[{{\"pid\":\"p1\",\"text\":\"{text}\"}}],\"next_pid\":2,\"tail_open\":false}}', word_count = 0, content_version = 0 WHERE id = {chapter_id}"
    ))
    .await;
}

/// The planned→drafting gate is a word-count gate: a punctuation-only edit keeps the chapter planned.
#[tokio::test]
async fn r11_planned_chapter_stays_planned_for_a_zero_word_punctuation_edit() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter").await;
    let chapter_id = chapter["id"].as_i64().expect("id") as i32;
    assert_eq!(chapter["status"], "planned");
    set_content(&app, chapter_id, "").await;

    let (status, body) = app
        .patch(
            &format!("/api/chapters/{chapter_id}/content"),
            json!({
                "base_version": 0,
                "ops": [{ "op": "insert", "after": "p1", "text": "……" }],
            }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(
        chapter["status"], "planned",
        "a zero-word edit must not move a planned chapter into drafting: {chapter}"
    );
    assert_eq!(chapter["word_count"], 0);
}

/// A legacy chapter with an empty paragraph plus one real word still submits (non-zero word count).
#[tokio::test]
async fn r11_legitimately_empty_ish_nonzero_chapter_submits() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter").await;
    let chapter_id = chapter["id"].as_i64().expect("id") as i32;
    app.execute(&format!(
        "UPDATE chapters SET content = '{{\"paragraphs\":[{{\"pid\":\"p1\",\"text\":\"\"}},{{\"pid\":\"p2\",\"text\":\"a\"}}],\"next_pid\":3,\"tail_open\":false}}', word_count = 1, status = 'drafting' WHERE id = {chapter_id}"
    ))
    .await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "in_review");
    assert_eq!(body["word_count"], 1);
}

/// `has_draft` is a word-count gate: an empty paragraph alone is not a draft.
#[tokio::test]
async fn r11_has_draft_ignores_an_empty_paragraph() {
    let rt = agent_write_harness::runtime(Vec::new()).await;
    rt
        .app
        .execute(&format!(
            "UPDATE chapters SET content = '{{\"paragraphs\":[{{\"pid\":\"p1\",\"text\":\"\"}}],\"next_pid\":2,\"tail_open\":false}}', word_count = 0 WHERE id = {}",
            rt.chapter_id
        ))
        .await;
    let session_id = start_writer(&rt, Granularity::Paragraph, false, false).await;
    let messages = rt.messages(session_id).await;
    let kickoff = messages.first().expect("kickoff").text();
    assert!(
        !kickoff.contains("本章已有部分正文"),
        "an empty paragraph must not be reported as a draft:\n{kickoff}"
    );

    let rt2 = agent_write_harness::runtime(Vec::new()).await;
    rt2
        .app
        .execute(&format!(
            "UPDATE chapters SET content = '{{\"paragraphs\":[{{\"pid\":\"p1\",\"text\":\"雨\"}}],\"next_pid\":2,\"tail_open\":false}}', word_count = 1 WHERE id = {}",
            rt2.chapter_id
        ))
        .await;
    let session_id = start_writer(&rt2, Granularity::Paragraph, false, false).await;
    let messages = rt2.messages(session_id).await;
    let kickoff = messages.first().expect("kickoff").text();
    assert!(
        kickoff.contains("本章已有部分正文"),
        "one real word is a draft:\n{kickoff}"
    );
}

/// `finish_revision` on an empty-only chapter is `error.draft_empty`.
#[tokio::test]
async fn r11_finish_revision_on_an_empty_paragraph_is_an_error() {
    use nekowriter_llm::mock::ScriptStep;
    let runtime = runtime(vec![
        ScriptStep::new().tool("submit_revision_plan", json!({ "plan": "rewrite" })),
        ScriptStep::new().tool("finish_revision", json!({ "summary": "nothing" })),
    ])
    .await;
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET content = '{{\"paragraphs\":[{{\"pid\":\"p1\",\"text\":\"\"}}],\"next_pid\":2,\"tail_open\":false}}', word_count = 0, status = 'revising' WHERE id = {}",
            runtime.chapter_id
        ))
        .await;
    let (session_id, _) = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterRevise,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            granularity: Some(Granularity::Paragraph),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create revision session");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let calls = runtime.calls(session_id).await;
        if let Some(call) = calls.iter().find(|call| call.name == "finish_revision") {
            assert!(call.is_error, "finish_revision must refuse an empty draft");
            assert!(
                call.raw_result_text
                    .clone()
                    .unwrap_or_default()
                    .contains("还没有任何正文"),
                "{:?}",
                call.raw_result_text
            );
            break;
        }
        if std::time::Instant::now() > deadline {
            let row = runtime.session_row(session_id).await;
            panic!("no finish_revision call; session is {}", row.status);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = SessionStatus::Completed;
}
