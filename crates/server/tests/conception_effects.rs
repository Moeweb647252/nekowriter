//! Regression tests for the deferred-effects gap in the transactions that finalize an
//! answered or held call (spec/03 §5.4, §6).
//!
//! The M3 runtime review found that `answer_inner` applied only `effects.complete` and
//! `resume_inner` only `phase_to` + `complete`, so an awaiting call's phase change and the
//! completion's siblings/sealing/transition were silently dropped. No M3 tool set `await_input`,
//! so nothing could reach the answer path; `ask_user` and `request_review` are the first ones.
//!
//! The debug session freezes `test_ask` (D-072: only a debug session can), an awaiting tool that
//! also records a phase change and a completion, so both paths are observable without shipping a
//! synthetic tool.

#[path = "agent_harness/mod.rs"]
mod harness;

use harness::*;
use nekowriter::db::json::{Phase, SessionStatus};
use serde_json::json;

/// Answering a call whose recorded effects carry `phase_to` and `complete` applies **both**:
/// the phase changes, the acceptance logic runs (chapter in review + a draft version), the
/// session completes, and the answer's appendix ordering stays canonical.
#[tokio::test]
async fn answering_applies_every_recorded_effect() {
    let runtime = runtime(vec![
        tool_step(
            "test_ask",
            json!({ "text": "please answer", "phase": "drafting", "complete": true }),
        ),
        // Never reached: the completion ends the session. A pending sibling would be finished as
        // `error.after_completion`; a further model step would prove the session stayed open.
        text_step("the session should never ask again"),
    ])
    .await;
    let session_id = runtime.start_debug().await;

    // The awaiting call is the session's live call.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let call_id = loop {
        let row = runtime.session_row(session_id).await;
        if row.status == "awaiting_input" {
            break row.held_tool_call_id.expect("the awaiting call");
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session stayed {} and never awaited input",
            row.status
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    let call = runtime.call_by_id(call_id).await;
    assert_eq!(call.name, "test_ask");
    // The call itself is `awaiting_input`, not `done`: the effect and the outcome's
    // `await_input` must agree, or the session runs straight through (the M9.1 bug).
    assert_eq!(call.status, "awaiting_input");
    assert!(
        !call.sealed,
        "an awaiting call is not sealed until it is answered"
    );
    let effects = call.effects();
    assert_eq!(effects.phase_to, Some(Phase::Drafting));
    assert_eq!(
        effects.complete,
        Some(nekowriter::db::json::CompletionKind::Draft)
    );
    assert_eq!(
        effects.await_input,
        Some(nekowriter::db::json::AwaitInputKind::AskUser),
        "the recorded effect names the input it waits for"
    );

    // Answer it: this transaction finalizes the call and must apply the full recorded effects.
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/answer"),
            json!({ "tool_call_id": call_id, "payload": {} }),
        )
        .await;
    assert_eq!(status, 200, "answer: {body}");
    assert_eq!(
        body["status"], "completed",
        "the completion transitions the session, not back to running: {body}"
    );

    let row = runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    assert_eq!(row.phase, "drafting", "the recorded phase change applied");
    assert!(row.finished_at.is_some(), "a completed session is finished");

    // The completion handler ran: the chapter is in review with a `draft` version.
    let (chapter_status, _, _) = runtime.chapter_status().await;
    assert_eq!(chapter_status, "in_review");
    assert_eq!(
        runtime.version_kinds().await,
        vec!["draft".to_owned()],
        "the completion saved the draft version"
    );

    // The answered call is finalized once and sealed, with the answer's text composed.
    let sealed = runtime.call_by_id(call_id).await;
    assert_eq!(sealed.status, "done");
    assert!(sealed.sealed, "the completion sealed the batch");
    assert!(
        sealed.effects().phase_to.is_none(),
        "the effects are consumed"
    );
    let result = sealed.result_text.unwrap_or_default();
    assert!(
        result.contains("answered"),
        "the answer is the call's result: {result}"
    );
}

/// The held path applies the same completion semantics: approving a held call whose effects
/// complete the session completes it, instead of reopening a session that already finished
/// (spec/03 §5.4: the transaction that finalizes the call applies its effects).
#[tokio::test]
async fn resuming_a_held_completing_call_completes_the_session() {
    let runtime = runtime(vec![
        tool_step(
            "test_write",
            json!({ "text": "a paragraph", "complete": true }),
        ),
        text_step("the session should never ask again"),
    ])
    .await;
    // `auto_pause` holds every write call (spec/03 §5.2).
    let session_id = runtime
        .start_debug_with(nekowriter::db::json::Granularity::Paragraph, true, false)
        .await;
    let held = runtime.wait_status(session_id, SessionStatus::Held).await;
    let call_id = held.held_tool_call_id.expect("the held call");

    // Approve without feedback: the completion effect is kept and applied.
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/resume"), json!({}))
        .await;
    assert_eq!(status, 200, "resume: {body}");
    assert_eq!(
        body["status"], "completed",
        "the applied completion ends the session: {body}"
    );

    let row = runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    assert!(row.finished_at.is_some());
    let (chapter_status, _, _) = runtime.chapter_status().await;
    assert_eq!(chapter_status, "in_review");
    assert_eq!(runtime.version_kinds().await, vec!["draft".to_owned()]);
    let call = runtime.call_by_id(call_id).await;
    assert_eq!(call.status, "done");
    assert!(call.sealed, "the completion sealed the batch");
    assert!(
        call.effects().complete.is_none(),
        "the effects are consumed"
    );
}

/// An awaiting session's pending form never blocks the tools that ran before it: the answer's
/// effects apply in the same transaction that finalizes the call, so a second answer is refused.
#[tokio::test]
async fn answering_a_completed_call_is_refused() {
    let runtime = runtime(vec![
        tool_step("test_ask", json!({ "text": "a", "phase": "drafting" })),
        text_step("done"),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let call_id = loop {
        let row = runtime.session_row(session_id).await;
        if row.status == "awaiting_input" {
            break row.held_tool_call_id.expect("the awaiting call");
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session stayed {}",
            row.status
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/answer"),
            json!({ "tool_call_id": call_id, "payload": {} }),
        )
        .await;
    assert_eq!(status, 200, "first answer: {body}");
    // The session ran on: the phase applied, the awaiting tool answered, and the closing text
    // idled it (a writer session would nudge; the debug session is a writer kind, so it nudges
    // then fails — either way it is no longer awaiting the same call).
    let row = runtime.wait_any_status(session_id).await;
    assert_ne!(row.status, "awaiting_input");
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/answer"),
            json!({ "tool_call_id": call_id, "payload": {} }),
        )
        .await;
    assert_eq!(
        status, 409,
        "a finalized call is not answered twice: {body}"
    );
    assert_eq!(body["error"]["code"], "session.invalid_state");
}
