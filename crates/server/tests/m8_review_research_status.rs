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
use serde_json::json;

#[path = "agent_harness/mod.rs"]
mod agent_harness;

use agent_harness::{runtime, tool_step};

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
