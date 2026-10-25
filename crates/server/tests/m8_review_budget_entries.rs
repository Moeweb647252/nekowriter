//! M8 review: the research budget's **entry** side (spec/05 §8).
//!
//! The implementer's test covers the 40-call limit. The budget is "40 tool calls and 25 entries";
//! past either, every result carries `notice.research_budget`. This pins the entries > 25 branch,
//! which reads the task's entry count rather than the session counter.

use axum::http::StatusCode;
use serde_json::{Value, json};

#[path = "agent_harness/mod.rs"]
mod agent_harness;

use agent_harness::{runtime, tool_step};

async fn task_with_scope(runtime: &agent_harness::Runtime, scope_name: &str) -> i32 {
    let (status, scope) = runtime
        .app
        .post(
            "/api/knowledge/scopes",
            json!({ "name": scope_name, "description": "the budget scope" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "scope: {scope}");
    let (status, task) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": runtime.book_id,
                "scope_name": scope_name,
                "topic": "the budget topic",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "task: {task}");
    task["id"].as_i64().expect("task id") as i32
}

#[tokio::test]
async fn the_budget_notice_appears_past_twenty_five_entries() {
    // 26 saves: the 26th pushes the task past the 25-entry budget, so its result (or a later one)
    // carries the notice. The scope starts empty.
    let mut steps: Vec<nekowriter_llm::mock::ScriptStep> = (0..26)
        .map(|index| {
            tool_step(
                "save_knowledge_entry",
                json!({
                    "title": format!("Entry {index:02}"),
                    "content": format!("fact number {index}"),
                    "source_urls": ["https://example.com/source"],
                }),
            )
        })
        .collect();
    steps.push(tool_step(
        "finish_research",
        json!({ "report": "the budget test finished" }),
    ));

    let runtime = runtime(steps).await;
    let task_id = task_with_scope(&runtime, "Budget entries").await;
    let (status, run) = runtime
        .app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "run: {run}");
    let session_id = run["session_id"].as_i64().expect("session id") as i32;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let calls: Vec<Value> = loop {
        let (_, snapshot) = runtime
            .app
            .get(&format!("/api/sessions/{session_id}"))
            .await;
        let status = snapshot["session"]["status"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if status != "running" && !status.is_empty() {
            break snapshot["tool_calls"]
                .as_array()
                .cloned()
                .unwrap_or_default();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session {session_id} did not finish"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };

    let saves: Vec<&Value> = calls
        .iter()
        .filter(|call| call["name"] == "save_knowledge_entry")
        .collect();
    assert_eq!(saves.len(), 26, "all saves executed: {}", saves.len());
    let texts: Vec<String> = calls
        .iter()
        .map(|call| {
            format!(
                "{}{}",
                call["result_text"].as_str().unwrap_or_default(),
                call["appendices"]
            )
        })
        .collect();
    assert!(
        texts
            .iter()
            .any(|text| text.contains("预算") || text.contains("budget")),
        "no budget notice past 25 entries"
    );

    // The task stored all 26 entries and finished normally (the budget is soft).
    let (_, task) = runtime
        .app
        .get(&format!("/api/research-tasks/{task_id}"))
        .await;
    assert_eq!(task["status"], "done", "{task}");
    assert_eq!(task["entries_created"], 26, "{task}");
}
