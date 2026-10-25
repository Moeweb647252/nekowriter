//! M8 review, finding F3: the single-task response must carry the entries the task created once
//! it is done (spec/10 §2.5 "with the report and the created entries when done", PROJECT §8.10
//! "the report and created entries when done").
//!
//! Before the fix `ResearchTaskDto` carried only the `entries_created` count, so the task page
//! could show how many entries exist but not which ones.

use axum::http::StatusCode;
use serde_json::json;

#[path = "agent_harness/mod.rs"]
mod agent_harness;

use agent_harness::{runtime, tool_step};

async fn run_task(runtime: &agent_harness::Runtime, task_id: i32) -> i32 {
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
            return session_id;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session {session_id} did not finish"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn the_task_response_carries_the_entries_it_created_when_done() {
    let runtime = runtime(vec![
        tool_step(
            "save_knowledge_entry",
            json!({
                "title": "Gate rules",
                "content": "the gate closes at dusk",
                "source_urls": ["https://example.com/gates"],
            }),
        ),
        tool_step(
            "save_knowledge_entry",
            json!({
                "title": "Gate watch",
                "content": "the watch changes at midnight",
                "source_urls": ["https://example.com/watch"],
            }),
        ),
        tool_step("finish_research", json!({ "report": "done" })),
    ])
    .await;
    let (status, scope) = runtime
        .app
        .post(
            "/api/knowledge/scopes",
            json!({ "name": "Entries scope", "description": "for the entries test" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "scope: {scope}");
    let (status, task) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": runtime.book_id,
                "scope_name": "Entries scope",
                "topic": "gate rules",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "task: {task}");
    let task_id = task["id"].as_i64().expect("task id") as i32;
    assert!(
        task["entries"].as_array().is_some_and(Vec::is_empty),
        "a new task has no created entries: {task}"
    );

    run_task(&runtime, task_id).await;

    let (status, task) = runtime
        .app
        .get(&format!("/api/research-tasks/{task_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{task}");
    assert_eq!(task["status"], "done", "{task}");
    let entries = task["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("the done task must carry its created entries: {task}"));
    assert_eq!(entries.len(), 2, "{task}");
    // In id order, with the title the page links.
    assert_eq!(entries[0]["title"], "Gate rules");
    assert_eq!(entries[1]["title"], "Gate watch");
    assert!(entries[0]["id"].is_i64(), "{task}");
    // The stored total agrees (spec/07 §8 step 5).
    assert_eq!(task["entries_created"], 2, "{task}");
}
