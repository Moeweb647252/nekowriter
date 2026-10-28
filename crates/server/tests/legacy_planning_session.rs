//! A `chapter_write` session that was open in `planning` at the upgrade keeps working under its
//! frozen prompt (PLAN M12.4 AC5, spec/03 §1.1, §5.4, D-146).
//!
//! The production path no longer creates this phase; the fixture sets the test-only
//! `legacy_planning_fixture` flag, which freezes the old 20-tool list and starts in `planning` —
//! the exact shape a session row created before the split has.

#![allow(dead_code)]

#[path = "agent_harness/mod.rs"]
mod base;

pub use base::*;

use axum::http::StatusCode;
use nekowriter::db::json::{Granularity, SessionKind, SessionStatus};
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter_llm::mock::ScriptStep;
use serde_json::{Value, json};

fn outline_step(outline: &str) -> ScriptStep {
    ScriptStep::new().tool(
        "submit_outline",
        json!({ "outline": outline, "reference_notes": "- 要点" }),
    )
}

fn insert_step(text: &str) -> ScriptStep {
    ScriptStep::new().tool("insert_text", json!({ "text": text, "chapter_end": true }))
}

async fn start_legacy(runtime: &Runtime, pause_after_outline: bool) -> i32 {
    let (session_id, _) = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            granularity: Some(Granularity::Paragraph),
            auto_pause: Some(false),
            pause_after_outline: Some(pause_after_outline),
            legacy_planning_fixture: true,
            // The exact text captured from the pre-split pack at 2d4b7af (F3).
            legacy_system_prompt: Some(
                include_str!("fixtures/legacy_writer_system.txt").to_owned(),
            ),
            legacy_kickoff_text: Some(
                include_str!("fixtures/legacy_writer_kickoff.txt").to_owned(),
            ),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create legacy planning session");
    session_id
}

fn appendices(call: &CallRow) -> Vec<Value> {
    serde_json::from_str(&call.appendices).expect("appendices")
}

/// The legacy session keeps the planning arm: submit_outline writes the chapter immediately,
/// holds under pause-after-outline, approves with the writer acceptance notice, drafts and
/// completes.
#[tokio::test]
async fn a_legacy_planning_session_plans_holds_approves_and_drafts() {
    let runtime = runtime(vec![
        outline_step("旧流程的章纲"),
        insert_step("旧流程的正文。"),
        ScriptStep::new().tool("finish_draft", json!({})),
    ])
    .await;
    let session_id = start_legacy(&runtime, true).await;
    let held = runtime.wait_status(session_id, SessionStatus::Held).await;
    assert_eq!(held.phase, "planning");
    let calls = runtime.calls(session_id).await;
    assert_eq!(calls.len(), 1);
    let effects: Value =
        serde_json::from_str(calls[0].effects.as_deref().unwrap_or("{}")).expect("effects");
    assert_eq!(
        effects["phase_to"], "drafting",
        "the legacy tool still defers the phase change"
    );
    assert!(
        effects["pending_outline"].is_null(),
        "no staging in legacy mode"
    );

    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Wait for the drafting steps to complete.
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let first = calls
        .iter()
        .find(|call| call.name == "submit_outline")
        .expect("outline call");
    let texts: Vec<String> = appendices(first)
        .iter()
        .filter(|appendix| appendix["kind"] == "effect_notice")
        .map(|appendix| appendix["text"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert!(
        texts.iter().any(|text| text.contains("章纲已确认")),
        "the legacy writer acceptance notice: {texts:?}"
    );
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(chapter["outline"], "旧流程的章纲");
    assert_eq!(chapter["status"], "in_review");
    assert_eq!(
        runtime.session_row(session_id).await.phase,
        "drafting",
        "the phase advanced at approval"
    );
}

/// Feedback on a held legacy outline gives the writer rejection notice (not the planner's), and
/// the session stays in `planning` so it can resubmit.
#[tokio::test]
async fn legacy_feedback_uses_the_writer_rejection_notice() {
    let runtime = runtime(vec![outline_step("第一版"), outline_step("第二版")]).await;
    let session_id = start_legacy(&runtime, true).await;
    runtime.wait_status(session_id, SessionStatus::Held).await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/resume"),
            json!({ "feedback": "改得更紧一些" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let held = runtime.wait_status(session_id, SessionStatus::Held).await;
    assert_eq!(held.phase, "planning", "feedback keeps the legacy phase");
    let calls = runtime.calls(session_id).await;
    let first = &calls[0];
    let texts: Vec<String> = appendices(first)
        .iter()
        .map(|appendix| appendix["text"].as_str().unwrap_or_default().to_owned())
        .collect();
    let all = texts.join("\n");
    assert!(
        all.contains("作者没有通过这份章纲"),
        "the legacy writer rejection notice: {all}"
    );
    assert!(
        !all.contains("作者没有接受这份细纲"),
        "the planner's notice is not used for a legacy session: {all}"
    );
    assert!(!all.contains("完成被撤回"), "{all}");
    assert!(all.contains("改得更紧一些"), "{all}");
}

/// The legacy fixture really freezes the pre-split system prompt and kickoff, not the current
/// pack (AC5/R10, F3): a silent switch to current prompts fails this test.
#[tokio::test]
async fn the_legacy_fixture_freezes_the_pre_split_prompt_and_kickoff() {
    let runtime = runtime(vec![outline_step("细纲")]).await;
    let session_id = start_legacy(&runtime, true).await;
    let (_, snapshot) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    let old_system = include_str!("fixtures/legacy_writer_system.txt");
    let old_kickoff = include_str!("fixtures/legacy_writer_kickoff.txt");
    assert_eq!(snapshot["system_prompt"], old_system);
    assert!(
        snapshot["system_prompt"]
            .as_str()
            .unwrap_or_default()
            .contains("规划阶段")
    );
    let kickoff = snapshot["messages"][0]["message"]["blocks"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert_eq!(kickoff, old_kickoff);
    assert!(
        kickoff.contains("当前处于【规划阶段】") && !kickoff.contains("当前处于【正文阶段】"),
        "the pre-split kickoff: {kickoff}"
    );
    // The current pack renders a different prompt, so equality above cannot pass by accident.
    let current = runtime
        .app
        .state()
        .prompts()
        .for_book(runtime.app.db(), Some(runtime.book_id))
        .await
        .expect("current prompts")
        .render("writer.system", &json!({}))
        .expect("render writer.system");
    assert_ne!(current.text, old_system);
    runtime.wait_status(session_id, SessionStatus::Held).await;
}

/// The legacy fixture really freezes the old 20-tool list, `submit_outline` included.
#[tokio::test]
async fn the_legacy_fixture_freezes_the_old_tool_list() {
    let runtime = runtime(vec![outline_step("细纲")]).await;
    let session_id = start_legacy(&runtime, true).await;
    runtime.wait_status(session_id, SessionStatus::Held).await;
    let (_, snapshot) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    let names: Vec<&str> = snapshot["tool_names"]
        .as_array()
        .expect("tool names")
        .iter()
        .map(|name| name.as_str().unwrap_or_default())
        .collect();
    assert_eq!(names.len(), 20, "{names:?}");
    assert!(names.contains(&"submit_outline"));
}
