//! M8 review, finding F4: `finish_research` must report what **this session** saved or updated
//! (spec/05 §8), while the task's stored `entries_created` counts every run (spec/07 §8 step 5).
//!
//! The tool counted the task's entries across all runs for both, so a second run that saves
//! nothing told the model it had saved the first run's entries.

use axum::http::StatusCode;
use serde_json::{Value, json};

#[path = "agent_harness/mod.rs"]
mod agent_harness;

use agent_harness::{runtime, tool_step};

/// Creates a research task in a fresh scope and returns (task id, scope id).
async fn task_with_scope(runtime: &agent_harness::Runtime, scope_name: &str) -> (i32, i32) {
    let (status, scope) = runtime
        .app
        .post(
            "/api/knowledge/scopes",
            json!({ "name": scope_name, "description": "the review scope" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create scope: {scope}");
    let scope_id = scope["id"].as_i64().expect("scope id") as i32;
    let (status, task) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": runtime.book_id,
                "scope_name": scope_name,
                "topic": "the count topic",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create task: {task}");
    (task["id"].as_i64().expect("task id") as i32, scope_id)
}

/// Runs the task and returns the sealed tool calls of that session.
async fn run_and_seal(runtime: &agent_harness::Runtime, task_id: i32) -> Vec<Value> {
    let (status, run) = runtime
        .app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "run: {run}");
    let session_id = run["session_id"].as_i64().expect("session id") as i32;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let (_, snapshot) = runtime
            .app
            .get(&format!("/api/sessions/{session_id}"))
            .await;
        let status = snapshot["session"]["status"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if status != "running" && !status.is_empty() {
            return snapshot["tool_calls"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|call| call["status"] == "done" || call["status"] == "error")
                .collect();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session {session_id} did not finish"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn finish_research_reports_this_session_while_the_task_says_this_session_saved_nothing() {
    let runtime = runtime(vec![
        tool_step(
            "save_knowledge_entry",
            json!({
                "title": "First entry",
                "content": "alpha",
                "source_urls": ["https://example.com/a"],
            }),
        ),
        tool_step("finish_research", json!({ "report": "first run" })),
        tool_step("finish_research", json!({ "report": "second run" })),
    ])
    .await;
    let (task_id, _scope) = task_with_scope(&runtime, "Count scope").await;

    // Run 1 saves one entry and reports one (spec/05 §8).
    let first = run_and_seal(&runtime, task_id).await;
    let first_finish = first
        .iter()
        .find(|call| call["name"] == "finish_research")
        .unwrap_or_else(|| panic!("no finish_research in {first:?}"));
    let first_text = first_finish["result_text"].as_str().unwrap_or_default();
    assert!(
        first_text.contains('1'),
        "the first run saved one entry: {first_text}"
    );

    // The task stores the total across runs (spec/07 §8 step 5).
    let (_, task) = runtime
        .app
        .get(&format!("/api/research-tasks/{task_id}"))
        .await;
    assert_eq!(task["entries_created"], 1, "{task}");

    // Run 2 saves and updates nothing, so the result must report zero for **this session** even
    // though the task's total stays 1 (spec/05 §8).
    let second = run_and_seal(&runtime, task_id).await;
    let second_finish = second
        .iter()
        .find(|call| call["name"] == "finish_research")
        .unwrap_or_else(|| panic!("no finish_research in {second:?}"));
    let second_text = second_finish["result_text"].as_str().unwrap_or_default();
    assert!(
        second_text.contains("0 个条目"),
        "the second run saved nothing, so it must report 0: {second_text}"
    );

    let (_, task) = runtime
        .app
        .get(&format!("/api/research-tasks/{task_id}"))
        .await;
    assert_eq!(
        task["entries_created"], 1,
        "the stored total still counts every run: {task}"
    );
}
