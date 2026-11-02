//! M8 review, finding F1: a research task must be `running` while its session is open
//! (spec/07 §8 step 5, spec/10 §2.5).
//!
//! `run` used to create the session without moving the task out of `ready`/`draft`, because the
//! only writer of that status sat in a creation-side-effect function that `create_inner` calls
//! only for chapter sessions. The consequences pinned here are the lifecycle and the guards: a
//! second `run` could start a second researcher session in the same scope, and PATCH/DELETE were
//! allowed while the session was open.

use std::time::Duration;

use axum::http::StatusCode;
use nekowriter::db::json::SessionStatus;
use nekowriter_llm::mock::ScriptStep;
use serde_json::json;

#[path = "agent_harness/mod.rs"]
mod agent_harness;

use agent_harness::{refusal_step, runtime, tool_step};

/// A research task linked to the harness book; `run` resolves its scope.
async fn book_task(runtime: &agent_harness::Runtime, scope_name: &str) -> i32 {
    let (status, task) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": runtime.book_id,
                "scope_name": scope_name,
                "topic": "the review topic",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create task: {task}");
    task["id"].as_i64().expect("task id") as i32
}

/// A scripted researcher whose first step is slow enough that the session is guaranteed to still
/// be open when `run` has returned.
fn slow_research_script() -> Vec<nekowriter_llm::mock::ScriptStep> {
    vec![
        tool_step("search_knowledge", json!({ "query": "gate" })).delay(Duration::from_secs(3)),
        tool_step(
            "finish_research",
            json!({ "report": "the review finished the research" }),
        ),
    ]
}

/// spec/07 §8 step 5: "Task status follows its session: `running` while the session is open".
/// `run` creates the open session, so immediately after it returns the task row must already say
/// `running`; every open-session guard of spec/10 §2.5 keys on this status.
#[tokio::test]
async fn a_task_is_running_while_its_session_is_open() {
    let runtime = runtime(slow_research_script()).await;
    let app = &runtime.app;
    let task_id = book_task(&runtime, "Review scope").await;

    let (status, body) = app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "run: {body}");
    let session_id = body["session_id"].as_i64().expect("session id") as i32;

    // The task and the list say `running` (spec/07 §8 step 5).
    let (status, task) = app.get(&format!("/api/research-tasks/{task_id}")).await;
    assert_eq!(status, StatusCode::OK, "{task}");
    assert_eq!(
        task["status"], "running",
        "the task is running while its session is open"
    );
    assert_eq!(task["session_id"], session_id);
    let (_, list) = app.get("/api/research-tasks").await;
    let listed = list
        .as_array()
        .expect("list")
        .iter()
        .find(|row| row["id"] == task_id)
        .unwrap_or_else(|| panic!("the task is missing from {list}"));
    assert_eq!(listed["status"], "running");

    // A second run is refused while the session is open and names the open session
    // (spec/07 §8 step 1, spec/10 §2.5). This is the guard that stops two researcher sessions
    // writing into one scope.
    let (status, body) = app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "second run: {body}");
    assert_eq!(body["error"]["code"], "research.invalid_state");
    assert_eq!(body["error"]["details"]["session_id"], session_id);

    // PATCH and DELETE are refused for the same reason (spec/10 §2.5).
    let (status, body) = app
        .patch(
            &format!("/api/research-tasks/{task_id}"),
            json!({ "topic": "changed" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "patch: {body}");
    assert_eq!(body["error"]["code"], "research.invalid_state");
    assert_eq!(body["error"]["details"]["session_id"], session_id);
    let (status, body) = app.delete(&format!("/api/research-tasks/{task_id}")).await;
    assert_eq!(status, StatusCode::CONFLICT, "delete: {body}");
    assert_eq!(body["error"]["code"], "research.invalid_state");

    // Abandoning the session cancels the task (spec/07 §8 step 5).
    let (status, _) = app
        .post(&format!("/api/sessions/{session_id}/abandon"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "abandon");
    let (_, task) = app.get(&format!("/api/research-tasks/{task_id}")).await;
    assert_eq!(task["status"], "cancelled", "{task}");
}

// ───────────────────── F9: a failed session fails the task and keeps it open ─────────────────────

/// spec/07 §8 step 5: a failed task keeps its session ("Continue sets it back to `running`"), and
/// spec/10 §2.5 still refuses PATCH/DELETE while that session is open. The task must therefore
/// read `failed`, not stay `running`.
#[tokio::test]
async fn a_failed_research_session_fails_the_task_and_keeps_it_open() {
    let runtime = runtime(vec![
        refusal_step("the model refused the turn"),
        ScriptStep::new()
            .tool(
                "finish_research",
                json!({ "report": "continued after failure" }),
            )
            .delay(Duration::from_secs(5)),
    ])
    .await;
    let app = &runtime.app;
    let task_id = book_task(&runtime, "Failure scope").await;

    let (status, body) = app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "run: {body}");
    let session_id = body["session_id"].as_i64().expect("session id") as i32;

    // Wait for the session to fail (a refusal is not retryable, spec/03 §5.5).
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let (_, snapshot) = app.get(&format!("/api/sessions/{session_id}")).await;
        let status = snapshot["session"]["status"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if status == "failed" {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the session did not fail; it is {status}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let (status, task) = app.get(&format!("/api/research-tasks/{task_id}")).await;
    assert_eq!(status, StatusCode::OK, "{task}");
    assert_eq!(task["status"], "failed", "{task}");
    assert_eq!(task["session_id"], session_id);

    // The failed session is still open: PATCH and DELETE are refused (spec/10 §2.5).
    let (status, body) = app
        .patch(
            &format!("/api/research-tasks/{task_id}"),
            json!({ "topic": "x" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, _) = app.delete(&format!("/api/research-tasks/{task_id}")).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Run now continues the failed session in place instead of refusing it (spec/07 §8 step 5).
    let (status, body) = app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["session_id"], session_id, "the same session continues");
    let (_, task) = app.get(&format!("/api/research-tasks/{task_id}")).await;
    assert_eq!(task["status"], "running", "{task}");
    assert_eq!(task["session_id"], session_id);
    assert_eq!(
        runtime.session_row(session_id).await.status,
        "running",
        "the session was continued, not replaced"
    );

    // Abandoning the failed session cancels the task (spec/07 §8 step 5).
    let (status, body) = app
        .post(&format!("/api/sessions/{session_id}/abandon"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "abandon: {body}");
    let (_, task) = app.get(&format!("/api/research-tasks/{task_id}")).await;
    assert_eq!(task["status"], "cancelled", "{task}");
}

/// The acceptance test of D-163: a failed research task continues its **own** session. The
/// material already fetched is still in the stored history (and its tool call is still sealed),
/// the task returns to `running`, and the saved knowledge entry is not duplicated.
#[tokio::test]
async fn a_failed_research_task_continues_its_own_session_without_losing_material() {
    let runtime = runtime(vec![
        tool_step(
            "save_knowledge_entry",
            json!({
                "title": "Gate rules",
                "content": "the gate closes at dusk",
                "source_urls": ["https://example.com/gates"],
            }),
        ),
        refusal_step("the provider timed out"),
        ScriptStep::new()
            .tool(
                "finish_research",
                json!({ "report": "continued from the stored history" }),
            )
            .delay(Duration::from_secs(5)),
    ])
    .await;
    let app = &runtime.app;
    let task_id = book_task(&runtime, "Resumable scope").await;

    let (status, body) = app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "run: {body}");
    let session_id = body["session_id"].as_i64().expect("session id") as i32;
    runtime
        .wait_status_within(session_id, SessionStatus::Failed, Duration::from_secs(20))
        .await;

    let (_, task) = app.get(&format!("/api/research-tasks/{task_id}")).await;
    assert_eq!(
        task["status"], "failed",
        "the task follows its session: {task}"
    );
    let scope_id = task["scope_id"].as_i64().expect("scope id");
    let before_messages = runtime.messages(session_id).await;
    let before_calls = runtime.calls(session_id).await;
    assert!(
        before_messages.len() >= 3,
        "the failed turn left its whole history: {before_messages:?}"
    );
    assert_eq!(before_calls.len(), 1, "{before_calls:?}");
    assert!(before_calls[0].sealed, "the fetched result is sealed");
    let (_, before_entries) = app
        .get(&format!("/api/knowledge/scopes/{scope_id}/entries"))
        .await;
    assert_eq!(
        before_entries["items"].as_array().map(Vec::len),
        Some(1),
        "{before_entries}"
    );

    // The same task id and the same session id: nothing is recreated.
    let (status, body) = app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "continue: {body}");
    assert_eq!(
        body["session_id"], session_id,
        "Continue must reuse the failed session"
    );
    let (_, task) = app.get(&format!("/api/research-tasks/{task_id}")).await;
    assert_eq!(task["status"], "running", "{task}");
    assert_eq!(task["session_id"], session_id);
    assert_eq!(runtime.session_row(session_id).await.status, "running");

    // The previous messages and the sealed tool call are still there; the new turn only appends.
    let during_messages = runtime.messages(session_id).await;
    for message in &before_messages {
        assert!(
            during_messages
                .iter()
                .any(|current| current.id == message.id),
            "message {} disappeared after Continue",
            message.id
        );
    }
    let during_calls = runtime.calls(session_id).await;
    assert!(
        during_calls
            .iter()
            .any(|call| call.id == before_calls[0].id && call.sealed),
        "the fetched tool result disappeared after Continue"
    );

    runtime
        .wait_status_within(
            session_id,
            SessionStatus::Completed,
            Duration::from_secs(20),
        )
        .await;
    let (_, task) = app.get(&format!("/api/research-tasks/{task_id}")).await;
    assert_eq!(task["status"], "done", "{task}");
    assert_eq!(task["entries_created"], 1);
    let (_, after_entries) = app
        .get(&format!("/api/knowledge/scopes/{scope_id}/entries"))
        .await;
    assert_eq!(
        after_entries["items"].as_array().map(Vec::len),
        Some(1),
        "Continue must not duplicate the entry: {after_entries}"
    );
}
