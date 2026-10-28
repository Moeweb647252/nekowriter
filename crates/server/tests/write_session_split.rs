//! The split write session: drafting-only, outline-driven, redraft by feedback, rollover chain
//! (PLAN M12.4, spec/03 §1, §4, §9–§10, §15, D-146).

#![allow(dead_code)]

#[path = "agent_harness/mod.rs"]
mod base;

pub use base::*;

use axum::http::StatusCode;
use nekowriter::db::json::{SessionKind, SessionStatus};
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter_llm::mock::ScriptStep;
use serde_json::{Value, json};

fn outline_args(outline: &str, notes: &str) -> Value {
    json!({ "outline": outline, "reference_notes": notes })
}

fn insert_step(text: &str, chapter_end: bool) -> ScriptStep {
    ScriptStep::new().tool(
        "insert_text",
        json!({ "text": text, "chapter_end": chapter_end }),
    )
}

/// Sets the chapter's accepted outline directly (the write precondition reads only this column).
async fn set_outline(runtime: &Runtime, outline: &str) {
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/chapters/{}", runtime.chapter_id),
            json!({ "outline": outline, "reference_notes": "- 要点" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Starts a real outline session that submits `outline` and (with `pause`) holds it; approves it.
async fn approved_outline(runtime: &Runtime, _outline: &str) -> i32 {
    let (session_id, _) = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterOutline,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            pause_after_outline: Some(true),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create outline session");
    runtime.wait_status(session_id, SessionStatus::Held).await;
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    session_id
}

/// The joined kickoff text of a session's first message.
fn kickoff_text(runtime: &Runtime, messages: &[MessageRow]) -> String {
    let _ = runtime;
    messages
        .first()
        .expect("kickoff")
        .blocks()
        .iter()
        .filter_map(|block| match block {
            nekowriter::db::json::StoredBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Starts a write session through the production endpoint.
async fn start_write(runtime: &Runtime, auto_pause: bool) -> i32 {
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/write", runtime.chapter_id),
            json!({ "granularity": "paragraph", "auto_pause": auto_pause }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["session_id"].as_i64().expect("session id") as i32
}

/// Runs a write session to completion with one paragraph and a confirming `finish_draft`.
async fn run_write_to_completion(runtime: &Runtime) -> i32 {
    let session_id = start_write(runtime, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    session_id
}

/// AC1: outline accepted, then a drafting session without `submit_outline`, whose kickoff carries
/// the outline, notes and granularity rule; a scripted `submit_outline` is refused with no effect.
#[tokio::test]
async fn ac1_outline_then_write_without_submit_outline() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("submit_outline", outline_args("第一版细纲", "- 要点一")),
        // The write session tries the retired tool; it must be refused before executing.
        ScriptStep::new().tool(
            "submit_outline",
            outline_args("不应写入的细纲", "- 不应写入"),
        ),
        insert_step("第一段正文。", true),
        ScriptStep::new().tool("finish_draft", json!({})),
    ])
    .await;
    let planner = approved_outline(&runtime, "第一版细纲").await;
    assert_eq!(
        runtime.session_row(planner).await.status,
        "completed",
        "the approved outline session completed"
    );
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(chapter["outline"], "第一版细纲");
    assert_eq!(
        chapter["status"], "planned",
        "accepting an outline never changes the status"
    );

    let writer = start_write(&runtime, false).await;
    let row = runtime.wait_status(writer, SessionStatus::Completed).await;
    assert_eq!(row.phase, "drafting");
    assert_eq!(row.pause_after_outline, 0);

    // The frozen list has no `submit_outline`, and its kickoff carries the accepted outline.
    let (status, snapshot) = runtime.app.get(&format!("/api/sessions/{writer}")).await;
    assert_eq!(status, StatusCode::OK, "{snapshot}");
    let names: Vec<&str> = snapshot["tool_names"]
        .as_array()
        .expect("tool names")
        .iter()
        .map(|name| name.as_str().unwrap_or_default())
        .collect();
    assert_eq!(names.len(), 19, "{names:?}");
    assert!(!names.contains(&"submit_outline"));
    let messages = runtime.messages(writer).await;
    let kickoff = kickoff_text(&runtime, &messages);
    assert!(kickoff.contains("第一版细纲"), "{kickoff}");
    assert!(kickoff.contains("- 要点一"), "{kickoff}");
    assert!(kickoff.contains("当前写作粒度"), "{kickoff}");
    assert!(!kickoff.contains("【规划阶段】"), "{kickoff}");

    // The refused call is an error result with no effect on the chapter.
    let calls = runtime.calls(writer).await;
    let refused = calls
        .iter()
        .find(|call| call.name == "submit_outline")
        .expect("the refused call");
    assert!(
        refused.is_error,
        "submit_outline is refused in a write session"
    );
    assert_eq!(refused.status, "error");
    let effects: Value =
        serde_json::from_str(refused.effects.as_deref().unwrap_or("{}")).expect("effects JSON");
    assert!(
        effects["pending_outline"].is_null(),
        "the refused call staged nothing: {effects}"
    );
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(chapter["outline"], "第一版细纲", "the call had no effect");
    assert_eq!(chapter["status"], "in_review");
}

/// AC2: re-outlining after prose leaves the text, the versions and the write session's messages
/// byte-identical (D-146 Q5).
#[tokio::test]
async fn ac2_reoutline_after_prose_leaves_the_history_untouched() {
    let runtime = runtime(vec![
        // The write session: one paragraph, then the confirming completion.
        insert_step("已经写好的正文。", true),
        ScriptStep::new().tool("finish_draft", json!({})),
        // The re-outline session.
        ScriptStep::new().tool("submit_outline", outline_args("重做后的细纲", "- 新要点")),
    ])
    .await;
    set_outline(&runtime, "原始细纲").await;
    let writer = run_write_to_completion(&runtime).await;
    let (_, before_chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    let before_messages: Vec<String> = runtime
        .messages(writer)
        .await
        .into_iter()
        .map(|message| format!("{}|{}|{}", message.seq, message.role, message.content))
        .collect();
    let before_versions = runtime.version_kinds().await;
    assert_eq!(before_chapter["status"], "in_review");

    // Re-outline the in-review chapter; the outline session has no chapter side effects except
    // the accepted proposal.
    let planner = start_outline_session(&runtime, false).await;
    runtime.wait_status(planner, SessionStatus::Completed).await;
    let (_, after_chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(after_chapter["outline"], "重做后的细纲");
    assert_eq!(after_chapter["content"], before_chapter["content"]);
    assert_eq!(
        after_chapter["content_version"],
        before_chapter["content_version"]
    );
    assert_eq!(
        after_chapter["status"], "in_review",
        "the outline session never changes the status"
    );
    assert_eq!(runtime.version_kinds().await, before_versions);
    let after_messages: Vec<String> = runtime
        .messages(writer)
        .await
        .into_iter()
        .map(|message| format!("{}|{}|{}", message.seq, message.role, message.content))
        .collect();
    assert_eq!(
        after_messages, before_messages,
        "the write history is untouched"
    );
}

async fn start_outline_session(runtime: &Runtime, pause: bool) -> i32 {
    let (session_id, _) = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterOutline,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            pause_after_outline: Some(pause),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create outline session");
    session_id
}

/// AC3: the two steps have separate sessions and separate usage; the book total is their sum.
#[tokio::test]
async fn ac3_the_two_steps_have_separate_histories_and_usage() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("submit_outline", outline_args("细纲", "- 要点")),
        insert_step("正文。", true),
        ScriptStep::new().tool("finish_draft", json!({})),
    ])
    .await;
    let planner = start_outline_session(&runtime, false).await;
    runtime.wait_status(planner, SessionStatus::Completed).await;
    let writer = run_write_to_completion(&runtime).await;

    let (_, planner_snapshot) = runtime.app.get(&format!("/api/sessions/{planner}")).await;
    let (_, writer_snapshot) = runtime.app.get(&format!("/api/sessions/{writer}")).await;
    assert_ne!(planner, writer);
    assert_eq!(planner_snapshot["session"]["role"], "planner");
    assert_eq!(writer_snapshot["session"]["role"], "writer");
    let planner_requests = planner_snapshot["session"]["usage"]["requests"]
        .as_i64()
        .expect("planner requests");
    let writer_requests = writer_snapshot["session"]["usage"]["requests"]
        .as_i64()
        .expect("writer requests");
    assert_eq!(planner_requests, 1);
    assert!(writer_requests >= 1, "{writer_requests}");
    let (_, overview) = runtime
        .app
        .get(&format!("/api/books/{}/overview", runtime.book_id))
        .await;
    assert_eq!(
        overview["usage"]["requests"].as_i64(),
        Some(planner_requests + writer_requests),
        "the book total is the sum of the two sessions: {overview}"
    );
}

/// Redraft is a write session in `drafting` whose kickoff carries the author's feedback; it
/// drafts from the current accepted outline. Without an outline the send-back refuses.
#[tokio::test]
async fn redraft_starts_a_drafting_session_with_the_feedback() {
    let runtime = runtime(vec![
        insert_step("旧正文。", true),
        ScriptStep::new().tool("finish_draft", json!({})),
        // The redraft's drafting steps.
        insert_step("重写后的正文。", true),
        ScriptStep::new().tool("finish_draft", json!({})),
    ])
    .await;
    set_outline(&runtime, "现行细纲").await;
    run_write_to_completion(&runtime).await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/send-back", runtime.chapter_id),
            json!({ "feedback": "节奏太慢，重写。", "mode": "redraft" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let session_id = body["session_id"].as_i64().expect("session id") as i32;
    let (_, snapshot) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(snapshot["session"]["kind"], "chapter_write");
    assert_eq!(snapshot["session"]["phase"], "drafting");
    assert_eq!(snapshot["session"]["pause_after_outline"], false);
    let kickoff = kickoff_text(&runtime, &runtime.messages(session_id).await);
    assert!(kickoff.contains("作者要求重写本章"), "{kickoff}");
    assert!(kickoff.contains("节奏太慢，重写。"), "{kickoff}");
    assert!(kickoff.contains("现行细纲"), "{kickoff}");
    // The text was archived and cleared, a backup version exists, status is drafting.
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(chapter["status"], "drafting");
    assert_eq!(
        chapter["content"]["paragraphs"].as_array().map(Vec::len),
        Some(0)
    );
    assert!(runtime.version_kinds().await.contains(&"backup".to_owned()));
}

/// A redraft without an outline is `409 chapter.outline_missing` (D-146).
#[tokio::test]
async fn redraft_without_an_outline_is_refused() {
    let runtime = runtime(vec![
        insert_step("旧正文。", true),
        ScriptStep::new().tool("finish_draft", json!({})),
    ])
    .await;
    set_outline(&runtime, "细纲").await;
    run_write_to_completion(&runtime).await;
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET outline = '' WHERE id = {}",
            runtime.chapter_id
        ))
        .await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/send-back", runtime.chapter_id),
            json!({ "feedback": "重写。", "mode": "redraft" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "chapter.outline_missing");
}

/// A plain drafting rollover has `feedback = null`, so it does not print the redraft sentence
/// (the rollover chain rule of D-146; Fact 8's defect).
#[tokio::test]
async fn a_plain_drafting_rollover_has_no_redraft_sentence() {
    let runtime = runtime(vec![
        insert_step("第一段。", false),
        insert_step("第二段。", false),
        ScriptStep::new().tool("finish_draft", json!({})),
        ScriptStep::new().tool("finish_draft", json!({})),
    ])
    .await;
    set_outline(&runtime, "细纲").await;
    let writer = start_write(&runtime, true).await;
    let held = runtime.wait_status(writer, SessionStatus::Held).await;
    let _ = held;
    // Push the session near the context limit while it is held, then approve: the next boundary
    // rolls over before asking the model again.
    runtime
        .app
        .execute(&format!(
            "UPDATE agent_sessions SET usage = '{{\"requests\":120,\"input_tokens\":165000,\
             \"cached_input_tokens\":0,\"cache_write_tokens\":0,\"output_tokens\":9000,\
             \"reasoning_tokens\":0,\"last_prompt_tokens\":165000,\"estimated\":false}}' \
             WHERE id = {writer}"
        ))
        .await;
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{writer}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    runtime.wait_status(writer, SessionStatus::Superseded).await;
    let (_, snapshot) = runtime.app.get(&format!("/api/sessions/{writer}")).await;
    let child = snapshot["session"]["child_session_id"]
        .as_i64()
        .expect("the rollover child") as i32;
    let kickoff = kickoff_text(&runtime, &runtime.messages(child).await);
    assert!(
        !kickoff.contains("本章是按作者的意见重写的"),
        "a plain drafting rollover has no redraft feedback:\n{kickoff}"
    );
}
