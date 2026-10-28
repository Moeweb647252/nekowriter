//! The outline session, additive to the old write flow (PLAN M12.3, spec/03 §5.2, §15, D-146).
//!
//! Every test uses the real session-creation path and the `ScriptedProvider`, so the frozen tool
//! list, the kickoff, the staging of the outline and the approval transaction are the shipped
//! ones. The chapter is asserted byte-identical while the proposal is held; acceptance applies
//! exactly the proposal and completes the session without a further model request.

#![allow(dead_code)]

#[path = "agent_harness/mod.rs"]
mod base;

pub use base::*;

use axum::http::StatusCode;
use nekowriter::db::json::{Phase, SessionKind, SessionStatus};
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter_llm::mock::ScriptStep;
use serde_json::{Value, json};

/// The outline proposal the scripts submit.
fn outline_args(outline: &str, notes: &str, title: Option<&str>) -> Value {
    let mut value = json!({ "outline": outline, "reference_notes": notes });
    if let Some(title) = title {
        value["title"] = json!(title);
    }
    value
}

/// Starts a real `chapter_outline` session through the production path.
async fn start_outline(runtime: &Runtime, pause_after_outline: bool) -> i32 {
    let (session_id, _warnings) = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterOutline,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            pause_after_outline: Some(pause_after_outline),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create outline session");
    session_id
}

/// The chapter row as one comparable string, to prove the held proposal wrote nothing.
async fn chapter_snapshot(runtime: &Runtime) -> String {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT outline, reference_notes, title, content, content_version, status \
             FROM chapters WHERE id = ?",
            vec![runtime.chapter_id.into()],
        ))
        .await
        .expect("query chapter")
        .expect("chapter");
    format!(
        "{}|{}|{}|{}|{}|{}",
        row.try_get::<String>("", "outline").expect("outline"),
        row.try_get::<String>("", "reference_notes").expect("notes"),
        row.try_get::<String>("", "title").expect("title"),
        row.try_get::<String>("", "content").expect("content"),
        row.try_get::<i32>("", "content_version").expect("version"),
        row.try_get::<String>("", "status").expect("status")
    )
}

/// Approves a held call with no feedback.
async fn approve(runtime: &Runtime, session_id: i32) -> Value {
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Sends feedback on a held call.
async fn feedback(runtime: &Runtime, session_id: i32, text: &str) -> Value {
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/resume"),
            json!({ "feedback": text }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test]
async fn hold_then_approve_applies_outline_notes_and_title_and_completes() {
    let runtime = runtime(vec![ScriptStep::new().tool(
        "submit_outline",
        outline_args("场景一：城门口", "- 雨一直下", Some("城门")),
    )])
    .await;
    // Give the chapter no title so the proposal's title has somewhere to go.
    let (status, _) = runtime
        .app
        .patch(
            &format!("/api/chapters/{}", runtime.chapter_id),
            json!({ "title": "" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Capture the full chapter row (outline, notes, title, content, version, status) before the
    // session exists at all: this is the assertion that a held proposal wrote nothing (F16).
    let before = chapter_snapshot(&runtime).await;
    let session_id = start_outline(&runtime, true).await;
    let held = runtime.wait_status(session_id, SessionStatus::Held).await;
    assert_eq!(held.phase, "none");
    assert_eq!(held.pause_after_outline, 1);
    // The chapter is byte-identical while the proposal is held, and still `planned`.
    let (chapter_status, _, _) = runtime.chapter_status().await;
    assert_eq!(chapter_status, "planned");
    assert_eq!(chapter_snapshot(&runtime).await, before);

    let calls = runtime.calls(session_id).await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "submit_outline");
    assert_eq!(calls[0].status, "held");
    let effects: Value =
        serde_json::from_str(calls[0].effects.as_deref().unwrap_or("{}")).expect("effects JSON");
    assert_eq!(effects["pending_outline"]["outline"], "场景一：城门口");
    assert_eq!(effects["pending_outline"]["title"], "城门");
    assert_eq!(effects["complete"], "outline");

    let requests_before = runtime.scripted.requests().len();
    let body = approve(&runtime, session_id).await;
    assert_eq!(body["status"], "completed", "{body}");
    // No further model request: the approval transaction applies the proposal and completes.
    assert_eq!(runtime.scripted.requests().len(), requests_before);

    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(chapter["outline"], "场景一：城门口");
    assert_eq!(chapter["reference_notes"], "- 雨一直下");
    assert_eq!(chapter["title"], "城门");
    assert_eq!(chapter["status"], "planned");
}

#[tokio::test]
async fn an_existing_title_is_not_overwritten_by_the_proposal() {
    let runtime = runtime(vec![ScriptStep::new().tool(
        "submit_outline",
        outline_args("细纲", "- 要点", Some("提议的标题")),
    )])
    .await;
    let session_id = start_outline(&runtime, true).await;
    runtime.wait_status(session_id, SessionStatus::Held).await;
    approve(&runtime, session_id).await;
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(chapter["title"], "Chapter One", "the author's title wins");
}

#[tokio::test]
async fn feedback_rejects_the_proposal_and_reports_edits_then_feedback_in_order() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "submit_outline",
            outline_args("第一版细纲", "- 要点一", None),
        ),
        ScriptStep::new().tool(
            "submit_outline",
            outline_args("第二版细纲", "- 要点二", None),
        ),
    ])
    .await;
    // An accepted outline already exists: feedback must leave it untouched.
    let (status, _) = runtime
        .app
        .patch(
            &format!("/api/chapters/{}", runtime.chapter_id),
            json!({ "outline": "作者原有的细纲", "reference_notes": "- 原有要点" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let session_id = start_outline(&runtime, true).await;
    runtime.wait_status(session_id, SessionStatus::Held).await;

    // The author edits the outline while the proposal is held.
    let (status, _) = runtime
        .app
        .patch(
            &format!("/api/chapters/{}", runtime.chapter_id),
            json!({ "outline": "作者在等待时改过的细纲" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Feedback revokes the effects; the planner keeps working and resubmits (second script step).
    feedback(&runtime, session_id, "场景再压紧一点").await;
    runtime.wait_status(session_id, SessionStatus::Held).await;

    let calls = runtime.calls(session_id).await;
    assert_eq!(calls.len(), 2);
    let first = &calls[0];
    assert_eq!(first.status, "done");
    assert!(!first.is_error);
    let appendices: Vec<Value> = serde_json::from_str(&first.appendices).expect("appendices");
    let all_text: String = appendices
        .iter()
        .map(|a| a["text"].as_str().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(all_text.contains("作者没有接受这份细纲"), "{all_text}");
    assert!(all_text.contains("作者在等待时改过的细纲"), "{all_text}");
    assert!(all_text.contains("场景再压紧一点"), "{all_text}");
    assert!(
        !all_text.contains("完成被撤回"),
        "no completion_revoked: {all_text}"
    );
    // The composed result carries them in the canonical slot order (spec/03 §5.3).
    let composed = first.result_text.clone().unwrap_or_default();
    let notice_at = composed
        .find("作者没有接受这份细纲")
        .unwrap_or_else(|| panic!("no rejection notice in {composed}"));
    let edit_at = composed
        .find("作者在等待时改过的细纲")
        .unwrap_or_else(|| panic!("no edit report in {composed}"));
    let feedback_at = composed
        .find("场景再压紧一点")
        .unwrap_or_else(|| panic!("no feedback in {composed}"));
    assert!(
        notice_at < edit_at && edit_at < feedback_at,
        "notice < edit report < feedback: {composed}"
    );
    // The chapter kept the author's edit (the rejected proposal was discarded).
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(chapter["outline"], "作者在等待时改过的细纲");

    // Resubmission works: approve the second proposal.
    approve(&runtime, session_id).await;
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(chapter["outline"], "第二版细纲");
    assert_eq!(chapter["reference_notes"], "- 要点二");
}

#[tokio::test]
async fn flag_off_accepts_immediately_and_completes_without_a_request() {
    let runtime = runtime(vec![ScriptStep::new().tool(
        "submit_outline",
        outline_args("立即生效的细纲", "- 要点", None),
    )])
    .await;
    let session_id = start_outline(&runtime, false).await;
    let row = runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    assert_eq!(row.phase, "none");
    assert_eq!(runtime.scripted.requests().len(), 1);
    let (status, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(chapter["outline"], "立即生效的细纲");
    assert_eq!(chapter["status"], "planned");
}

#[tokio::test]
async fn abandon_while_held_keeps_the_previous_outline() {
    let runtime = runtime(vec![ScriptStep::new().tool(
        "submit_outline",
        outline_args("会被放弃的细纲", "- 要点", None),
    )])
    .await;
    let (status, _) = runtime
        .app
        .patch(
            &format!("/api/chapters/{}", runtime.chapter_id),
            json!({ "outline": "原有细纲", "reference_notes": "- 原有要点" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let session_id = start_outline(&runtime, true).await;
    runtime.wait_status(session_id, SessionStatus::Held).await;
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/abandon"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "abandoned");
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(chapter["outline"], "原有细纲");
    assert_eq!(chapter["reference_notes"], "- 原有要点");
    assert_eq!(chapter["status"], "planned");
}

/// The outline session's frozen list is the planner's: no drafting tool and no `submit_revision_plan`.
#[tokio::test]
async fn the_outline_session_freezes_the_planner_tool_list() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("submit_outline", outline_args("细纲", "- 要点", None)),
    ])
    .await;
    let session_id = start_outline(&runtime, true).await;
    runtime.wait_status(session_id, SessionStatus::Held).await;
    let (status, snapshot) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{snapshot}");
    let names: Vec<&str> = snapshot["tool_names"]
        .as_array()
        .expect("tool names")
        .iter()
        .map(|name| name.as_str().unwrap_or_default())
        .collect();
    assert_eq!(names.len(), 12, "{names:?}");
    assert!(names.contains(&"submit_outline"));
    assert!(names.contains(&"read_draft"));
    assert!(!names.contains(&"insert_text"));
    assert!(!names.contains(&"submit_revision_plan"));
    assert_eq!(snapshot["session"]["role"], "planner");
    assert_eq!(snapshot["session"]["kind"], "chapter_outline");
    let _ = Phase::None;
}

/// spec/03 §15 test 18: the author edits the chapter while `submit_outline` is held, then
/// approves → the chapter takes exactly the proposal (the interim edit is replaced), the held call
/// finalises with the staged wording, and the session completes (F4).
#[tokio::test]
async fn author_edits_while_held_are_replaced_by_the_approved_proposal() {
    let runtime = runtime(vec![ScriptStep::new().tool(
        "submit_outline",
        outline_args("提议细纲", "- 提议要点", Some("提议标题")),
    )])
    .await;
    let session_id = start_outline(&runtime, true).await;
    runtime.wait_status(session_id, SessionStatus::Held).await;
    // The author writes the chapter by hand while the proposal waits.
    let (status, _) = runtime
        .app
        .patch(
            &format!("/api/chapters/{}", runtime.chapter_id),
            json!({
                "outline": "作者在等待时改的细纲",
                "reference_notes": "- 作者等待时的要点",
                "title": "作者标题"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    approve(&runtime, session_id).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    // Exactly the proposal: the interim author edit to the outline is replaced, while the
    // author's non-empty title wins.
    assert_eq!(chapter["outline"], "提议细纲");
    assert_eq!(chapter["reference_notes"], "- 提议要点");
    assert_eq!(chapter["title"], "作者标题");
    assert_eq!(chapter["status"], "planned");

    let calls = runtime.calls(session_id).await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].status, "done");
    assert!(!calls[0].is_error);
    let result = calls[0].result_text.clone().unwrap_or_default();
    assert!(
        result.contains("作者接受后才会生效"),
        "the held call finalises with the staged wording: {result}"
    );
    assert!(!result.contains("章纲与参考要点已保存"), "{result}");
}

/// The planner's retrieval path runs end-to-end in a `chapter_outline` session: a real retrieval
/// call succeeds, the kickoff is the planner's, and the accepted proposal follows it (F5).
#[tokio::test]
async fn the_planner_retrieves_then_submits_in_one_session() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("get_entity", json!({ "entity": "林凡" })),
        ScriptStep::new().tool(
            "submit_outline",
            outline_args("检索后的细纲", "- 检索后的要点", None),
        ),
    ])
    .await;
    let (status, _) = runtime
        .app
        .post(
            &format!("/api/books/{}/entities", runtime.book_id),
            json!({ "kind": "character", "name": "林凡", "summary": "主角", "importance": 5 }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = start_outline(&runtime, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    let calls = runtime.calls(session_id).await;
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[0].name, "get_entity");
    assert!(!calls[0].is_error);
    let retrieved = calls[0].result_text.clone().unwrap_or_default();
    assert!(
        retrieved.contains("林凡"),
        "the retrieval result: {retrieved}"
    );
    assert_eq!(calls[1].name, "submit_outline");

    let (_, snapshot) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    let kickoff: String = snapshot["messages"][0]["message"]["blocks"]
        .as_array()
        .expect("blocks")
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(kickoff.contains("【细纲任务】"), "{kickoff}");
    assert!(kickoff.contains("submit_outline"), "{kickoff}");
    let tools: Vec<&str> = snapshot["tool_names"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|name| name.as_str().unwrap_or_default())
        .collect();
    assert!(tools.contains(&"get_entity"));
    assert!(!tools.contains(&"insert_text"));
}

/// Approving an outline advances the stored view snapshot past the agent's own proposal, so the
/// finalised call carries no edit report naming the proposal as an author edit (F17). Deleting
/// `keep_view_snapshot_outline_in_sync` makes this test fail.
#[tokio::test]
async fn approving_an_outline_does_not_report_the_proposal_as_an_author_edit() {
    let runtime = runtime(vec![ScriptStep::new().tool(
        "submit_outline",
        outline_args("提议细纲", "- 提议要点", None),
    )])
    .await;
    let session_id = start_outline(&runtime, true).await;
    runtime.wait_status(session_id, SessionStatus::Held).await;
    approve(&runtime, session_id).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let result = calls[0].result_text.clone().unwrap_or_default();
    assert!(
        !result.contains("【用户修改了正文】"),
        "the proposal is not an author edit: {result}"
    );
    assert!(
        !result.contains("〔作者修改后的章纲〕"),
        "the proposal is not an author outline edit: {result}"
    );
    assert!(result.contains("作者接受后才会生效"), "{result}");
}

/// A planner sees an existing chapter's reference notes even when the chapter has neither a
/// draft nor an outline (R3 suspicion 1): the planner may edit notes, so hiding them risks
/// overwriting facts it was never shown.
#[tokio::test]
async fn the_planner_kickoff_shows_reference_notes_without_an_outline() {
    let runtime =
        runtime(vec![ScriptStep::new().tool(
            "submit_outline",
            outline_args("新细纲", "- 新要点", None),
        )])
        .await;
    let (status, _) = runtime
        .app
        .patch(
            &format!("/api/chapters/{}", runtime.chapter_id),
            json!({ "reference_notes": "- 作者早已记下的事实" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let session_id = start_outline(&runtime, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let (_, snapshot) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    let kickoff: String = snapshot["messages"][0]["message"]["blocks"]
        .as_array()
        .expect("blocks")
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(kickoff.contains("作者早已记下的事实"), "{kickoff}");
    assert!(kickoff.contains("<reference_notes>"), "{kickoff}");
}
