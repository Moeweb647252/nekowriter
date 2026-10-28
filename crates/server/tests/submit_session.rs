//! Submit-for-review ends an open, non-running writer session (spec/08 §1.1, task-20).
//!
//! The spec row reads "no `running` writer session (an open, non-running one is abandoned after
//! the UI confirms)". Only the `running` half was implemented, so a chapter whose writer was
//! paused/held/stopped could be submitted but never approved (`approve` refuses an open writer
//! session) — a reachable dead end with no way out except the agent panel.

mod review_harness;

use std::time::Duration;

use nekowriter::db::json::{Granularity, SessionKind, SessionStatus};
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter_llm::mock::ScriptStep;
use review_harness::{
    Runtime, chapter_state, prepare_in_review, review_step, reviews, runtime, session_column,
    start_reviewer,
};
use serde_json::json;

/// Starts a shipped drafting-only `chapter_write` session that holds on its first insert.
async fn start_writer_session(runtime: &Runtime) -> i32 {
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET outline = '场景一：城门口', reference_notes = '- 雨一直下' \
             WHERE id = {}",
            runtime.chapter_id
        ))
        .await;
    let (session_id, _warnings) = sessions::create(
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
    .expect("create writer session");
    session_id
}

/// The book settings without auto-maintenance, so an approval starts no archivist runner.
async fn disable_auto_maintenance(runtime: &Runtime) {
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/books/{}", runtime.book_id),
            json!({
                "settings": {
                    "granularity": "paragraph",
                    "auto_pause": false,
                    "pause_after_outline": true,
                    "target_words_min": 3000,
                    "target_words_max": 4000,
                    "recent_summary_count": 5,
                    "midterm_window": 10,
                    "auto_maintenance": false,
                },
            }),
        )
        .await;
    assert_eq!(status, 200, "book settings: {body}");
}

async fn chapter_open_sessions(runtime: &Runtime) -> Vec<serde_json::Value> {
    let (status, body) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(status, 200, "chapter: {body}");
    body["open_sessions"]
        .as_array()
        .expect("open_sessions")
        .clone()
}

/// A writer session held at an insert is abandoned by the manual submit, and the chapter can then
/// be approved (spec/08 §1.1).
#[tokio::test]
async fn submit_abandons_a_held_writer_session_and_approve_then_succeeds() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("insert_text", json!({ "text": "第一段。" })),
    ])
    .await;
    runtime
        .app
        .put_text(runtime.chapter_id, 0, "第一段。\n\n第二段。")
        .await;
    let writer = start_writer_session(&runtime).await;
    runtime
        .wait_status_within(writer, SessionStatus::Held, Duration::from_secs(10))
        .await;
    let open = chapter_open_sessions(&runtime).await;
    assert_eq!(open.len(), 1, "the held writer session is open: {open:?}");
    assert_eq!(open[0]["status"], "held");
    assert_eq!(open[0]["role"], "writer");

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/submit", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "submit: {body}");
    assert_eq!(body["status"], "in_review");
    let writer_status: Option<String> = session_column(&runtime, writer, "status").await;
    assert_eq!(
        writer_status.as_deref(),
        Some("abandoned"),
        "the open, non-running writer session is abandoned (spec/08 §1.1)"
    );
    assert!(
        chapter_open_sessions(&runtime).await.is_empty(),
        "no open writer session is left behind"
    );

    disable_auto_maintenance(&runtime).await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/approve", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(
        status, 200,
        "approval must not be blocked by the submitted session: {body}"
    );
    assert_eq!(body["chapter"]["status"], "official");
}

/// The same dead end exists on the revision path (`send_back` → stop → submit): the stopped
/// `chapter_revise` session is a writer session too, so the submit ends it.
#[tokio::test]
async fn submit_abandons_a_stopped_revision_session_and_approve_then_succeeds() {
    let runtime = runtime(vec![
        review_step("revise", "needs work", json!([]), "fix it"),
        ScriptStep::new().tool(
            "submit_revision_plan",
            json!({ "plan": "rewrite the second paragraph" }),
        ),
        ScriptStep::new()
            .tool("insert_text", json!({ "text": "改写后的第二段。" }))
            .delay(Duration::from_millis(3_000)),
    ])
    .await;
    prepare_in_review(&runtime).await;
    let reviewer = start_reviewer(&runtime).await;
    runtime
        .wait_status_within(reviewer, SessionStatus::Completed, Duration::from_secs(10))
        .await;
    let body = reviews(&runtime).await;
    let report = body["reports"][0]["id"].as_i64().expect("report id") as i32;
    let (status, started) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/send-back", runtime.chapter_id),
            json!({
                "feedback": "fix the second paragraph",
                "mode": "revise",
                "review_report_id": report,
            }),
        )
        .await;
    assert_eq!(status, 200, "send back: {started}");
    let revision = started["session_id"].as_i64().expect("session id") as i32;
    runtime
        .wait_status_within(revision, SessionStatus::Running, Duration::from_secs(10))
        .await;
    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{revision}/stop"), json!({}))
        .await;
    assert_eq!(status, 200, "stop the revision");
    runtime
        .wait_status_within(revision, SessionStatus::Stopped, Duration::from_secs(10))
        .await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/submit", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "submit: {body}");
    let revision_status: Option<String> = session_column(&runtime, revision, "status").await;
    assert_eq!(
        revision_status.as_deref(),
        Some("abandoned"),
        "a stopped revision session is an open writer session: submit ends it"
    );

    disable_auto_maintenance(&runtime).await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/approve", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "approve: {body}");
    assert_eq!(chapter_state(&runtime).await.0, "official");
}

/// A `running` writer session still blocks the manual submit with `409 chapter.session_exists`
/// and its session id (spec/08 §1.1) — the abandonment only covers quiescent sessions.
#[tokio::test]
async fn a_running_writer_session_still_blocks_submit() {
    let runtime = runtime(vec![
        ScriptStep::new()
            .tool("insert_text", json!({ "text": "第一段。" }))
            .delay(Duration::from_millis(3_000)),
    ])
    .await;
    runtime
        .app
        .put_text(runtime.chapter_id, 0, "第一段。")
        .await;
    let writer = start_writer_session(&runtime).await;
    runtime
        .wait_status_within(writer, SessionStatus::Running, Duration::from_secs(10))
        .await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/submit", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "submit: {body}");
    assert_eq!(body["error"]["code"], "chapter.session_exists");
    assert_eq!(
        body["error"]["details"]["session_id"], writer,
        "the block names the running session"
    );
    // The blocked submit left the session running and the chapter drafting.
    let writer_status: Option<String> = session_column(&runtime, writer, "status").await;
    assert_eq!(writer_status.as_deref(), Some("running"));
    assert_eq!(chapter_state(&runtime).await.0, "drafting");

    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{writer}/abandon"), json!({}))
        .await;
    assert_eq!(status, 200, "clean up the writer runner");
}
