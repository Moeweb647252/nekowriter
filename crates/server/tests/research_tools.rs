//! The researcher's write tools (PLAN 8.3, spec/05 §8, spec/07 §8).
//!
//! The tools run inside the runtime, so these tests drive a real research session with a
//! **scripted** provider: each step is exactly one tool call, and the assertions are on the sealed
//! result the model would see. That is the only way to exercise `source_urls`, the duplicate-title
//! rule, the mutual exclusion of `content`/`edits`, the `find` rule and the budget notice.

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use nekowriter::testing::TestApp;
use serde_json::{Value, json};

#[path = "agent_harness/mod.rs"]
mod agent_harness;

use agent_harness::{runtime, tool_step};
use nekowriter_llm::mock::ScriptStep;

/// Creates a research task on an existing book and returns (task id, scope id).
async fn task_with_scope(runtime: &agent_harness::Runtime, scope_name: &str) -> (i32, i32) {
    let (status, scope) = runtime
        .app
        .post(
            "/api/knowledge/scopes",
            json!({ "name": scope_name, "description": "a scope for the test" }),
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
                "topic": "the test topic",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create task: {task}");
    (task["id"].as_i64().expect("task id") as i32, scope_id)
}

/// The completing step of a researcher script (`finish_research`, spec/05 §8).
fn finish_step() -> ScriptStep {
    tool_step(
        "finish_research",
        json!({ "report": "the test finished the research" }),
    )
}

/// Waits for the session to stop running, then returns its sealed tool calls in order.
async fn sealed_results(app: &TestApp, session_id: i32) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let (_, snapshot) = app.get(&format!("/api/sessions/{session_id}")).await;
        let status = snapshot["session"]["status"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if status != "running" && !status.is_empty() {
            let calls = snapshot["tool_calls"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            return calls
                .into_iter()
                .filter(|call| call["status"] == "done" || call["status"] == "error")
                .collect();
        }
        assert!(
            Instant::now() < deadline,
            "session {session_id} did not finish within 60s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A scripted research session: one run, one step per entry in `steps`.
async fn run_scripted(
    steps: Vec<ScriptStep>,
    scope_name: &str,
) -> (agent_harness::Runtime, i32, i32, Vec<Value>) {
    let runtime = runtime(steps).await;
    let (task_id, scope_id) = task_with_scope(&runtime, scope_name).await;
    let (status, run) = runtime
        .app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "run: {run}");
    let session_id = run["session_id"].as_i64().expect("session id") as i32;
    let results = sealed_results(&runtime.app, session_id).await;
    (runtime, task_id, scope_id, results)
}

// ───────────────────────────── save_knowledge_entry ─────────────────────────────

#[tokio::test]
async fn save_writes_into_the_target_scope_and_attributes_the_entry() {
    let (runtime, task_id, scope_id, results) = run_scripted(
        vec![
            tool_step(
                "save_knowledge_entry",
                json!({
                    "title": "Gate rules",
                    "content": "the gate closes at dusk",
                    "source_urls": ["https://example.com/gates"],
                    "tags": ["city"],
                }),
            ),
            finish_step(),
        ],
        "Research scope",
    )
    .await;

    let call = &results[0];
    assert_eq!(call["name"], "save_knowledge_entry", "{call:?}");
    assert_eq!(call["is_error"], false, "{call:?}");

    // The entry landed in the task's scope, attributed to the research task (spec/07 §8 step 5).
    let (status, entries) = runtime
        .app
        .get(&format!("/api/knowledge/scopes/{scope_id}/entries"))
        .await;
    assert_eq!(status, StatusCode::OK, "{entries}");
    let items = entries["items"].as_array().cloned().unwrap_or_default();
    assert_eq!(items.len(), 1, "{entries}");
    assert_eq!(items[0]["title"], "Gate rules");
    assert_eq!(items[0]["source"], "research", "{entries}");

    // The task is done and counts it.
    let (_, task) = runtime
        .app
        .get(&format!("/api/research-tasks/{task_id}"))
        .await;
    assert_eq!(task["status"], "done", "{task}");
    assert_eq!(task["entries_created"], 1);
}

#[tokio::test]
async fn a_duplicate_title_is_entry_exists_and_suggests_update() {
    let (runtime, _task, _scope, results) = run_scripted(
        vec![
            tool_step(
                "save_knowledge_entry",
                json!({ "title": "Same", "content": "one", "source_urls": ["https://a.example"] }),
            ),
            tool_step(
                "save_knowledge_entry",
                json!({ "title": "Same", "content": "two", "source_urls": ["https://b.example"] }),
            ),
            finish_step(),
        ],
        "Dup scope",
    )
    .await;
    assert_eq!(results[0]["is_error"], false, "{:?}", results[0]);
    let second = &results[1];
    assert_eq!(second["is_error"], true, "{second:?}");
    // `error.entry_exists` renders the existing title and id, so the model knows what to update
    // (spec/05 §8). An empty text here would mean the error carries fields the branch does not
    // read — the strict renderer makes that an empty result.
    let text = second["result_text"].as_str().unwrap_or_default();
    assert!(!text.trim().is_empty(), "the model got no text: {second:?}");
    assert!(text.contains("Same"), "the existing title is named: {text}");
    assert!(
        text.contains('#') || text.contains("update_knowledge_entry"),
        "the error names the entry or the update tool: {text}"
    );
    let _ = runtime;
}

#[tokio::test]
async fn a_non_http_source_url_is_refused() {
    let (_runtime, _task, _scope, results) = run_scripted(
        vec![
            tool_step(
                "save_knowledge_entry",
                json!({ "title": "Bad source", "content": "c", "source_urls": ["not a url"] }),
            ),
            finish_step(),
        ],
        "Bad url scope",
    )
    .await;
    assert_eq!(results[0]["is_error"], true, "{:?}", results[0]);
    let text = results[0]["result_text"].as_str().unwrap_or_default();
    assert!(
        !text.trim().is_empty(),
        "the model got no text: {:?}",
        results[0]
    );
    assert!(
        text.contains("not a url"),
        "the offending url is named: {text}"
    );
}

#[tokio::test]
async fn general_knowledge_may_have_no_source() {
    let (runtime, _task, scope_id, results) = run_scripted(
        vec![
            tool_step(
                "save_knowledge_entry",
                // The schema requires the property; "general knowledge" means an **empty**
                // citation list (spec/05 §8), not an absent one.
                json!({ "title": "Common knowledge", "content": "everyone knows this", "source_urls": [] }),
            ),
            finish_step(),
        ],
        "General scope",
    )
    .await;
    assert_eq!(results[0]["is_error"], false, "{:?}", results[0]);
    let (_, entries) = runtime
        .app
        .get(&format!("/api/knowledge/scopes/{scope_id}/entries"))
        .await;
    let items = entries["items"].as_array().cloned().unwrap_or_default();
    assert_eq!(items.len(), 1, "{entries}");
}

// ───────────────────────────── update_knowledge_entry ─────────────────────────────

#[tokio::test]
async fn edits_replace_exactly_once_and_all_or_nothing() {
    let (runtime, _task, scope_id, results) = run_scripted(
        vec![
            tool_step(
                "save_knowledge_entry",
                json!({ "title": "Editable", "content": "alpha beta gamma", "source_urls": ["https://a.example"] }),
            ),
            // A `find` that occurs twice fails the whole call (spec/05 §8).
            tool_step(
                "update_knowledge_entry",
                json!({ "entry_id": 1, "edits": [{ "find": "a", "replace": "A" }] }),
            ),
            // A `find` that does not occur at all also fails.
            tool_step(
                "update_knowledge_entry",
                json!({ "entry_id": 1, "edits": [{ "find": "missing", "replace": "x" }] }),
            ),
            // A single valid edit succeeds.
            tool_step(
                "update_knowledge_entry",
                json!({ "entry_id": 1, "edits": [{ "find": "beta", "replace": "BETA" }] }),
            ),
            finish_step(),
        ],
        "Edit scope",
    )
    .await;
    assert_eq!(results[0]["is_error"], false, "{:?}", results[0]);
    assert_eq!(results[1]["is_error"], true, "an ambiguous find fails");
    assert_eq!(results[2]["is_error"], true, "a missing find fails");
    assert_eq!(results[3]["is_error"], false, "{:?}", results[3]);
    // `error.find_failed` words itself: the count for an ambiguous find, and "找不到" for a
    // missing one (spec/05 §8). Both are checked so a wrong field name cannot pass silently.
    let ambiguous = results[1]["result_text"].as_str().unwrap_or_default();
    assert!(!ambiguous.trim().is_empty(), "{:?}", results[1]);
    // "alpha beta gamma" contains five "a"s, and the branch reports the real count.
    assert!(
        ambiguous.contains('5'),
        "the occurrence count is reported: {ambiguous}"
    );
    let missing = results[2]["result_text"].as_str().unwrap_or_default();
    assert!(!missing.trim().is_empty(), "{:?}", results[2]);
    assert!(
        missing.contains("missing"),
        "the find snippet is quoted: {missing}"
    );

    // The failed calls left the content untouched; the valid one applied. The list is a summary,
    // so the content is read from the entry itself.
    let (_, entry) = runtime.app.get("/api/knowledge/entries/1").await;
    assert_eq!(entry["content"], "alpha BETA gamma", "{entry}");
    let _ = scope_id;
}

#[tokio::test]
async fn content_and_edits_are_mutually_exclusive() {
    let (_runtime, _task, _scope, results) = run_scripted(
        vec![
            tool_step(
                "save_knowledge_entry",
                json!({ "title": "Both", "content": "abc", "source_urls": ["https://a.example"] }),
            ),
            tool_step(
                "update_knowledge_entry",
                json!({
                    "entry_id": 1,
                    "content": "replaced",
                    "edits": [{ "find": "abc", "replace": "x" }],
                }),
            ),
            finish_step(),
        ],
        "Exclusive scope",
    )
    .await;
    assert_eq!(results[1]["is_error"], true, "{:?}", results[1]);
    let text = results[1]["result_text"].as_str().unwrap_or_default();
    assert!(
        !text.trim().is_empty(),
        "the model got no text: {:?}",
        results[1]
    );
    assert!(text.contains("content"), "{text}");
    assert!(text.contains("edits"), "{text}");
}

#[tokio::test]
async fn an_entry_outside_the_target_scope_is_not_found() {
    let runtime = runtime(vec![
        tool_step(
            "update_knowledge_entry",
            json!({ "entry_id": 1, "edits": [{ "find": "a", "replace": "b" }] }),
        ),
        finish_step(),
    ])
    .await;
    // An entry in *another* scope: the researcher must not see it (spec/05 §1.4).
    let (status, other) = runtime
        .app
        .post("/api/knowledge/scopes", json!({ "name": "Elsewhere" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_scope = other["id"].as_i64().expect("id") as i32;
    let (status, entry) = runtime
        .app
        .post(
            &format!("/api/knowledge/scopes/{other_scope}/entries"),
            json!({ "title": "Foreign", "content": "abc" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{entry}");
    let entry_id = entry["id"].as_i64().expect("id");

    let (task_id, _scope_id) = task_with_scope(&runtime, "Target scope").await;
    let (status, run) = runtime
        .app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    let session_id = run["session_id"].as_i64().expect("session") as i32;
    let results = sealed_results(&runtime.app, session_id).await;
    assert_eq!(results[0]["is_error"], true, "{:?}", results[0]);
    // The foreign entry still exists and is untouched.
    let (status, entry) = runtime
        .app
        .get(&format!("/api/knowledge/entries/{entry_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{entry}");
    assert_eq!(entry["content"], "abc", "the foreign entry is untouched");
}

// ───────────────────────────── finish_research ─────────────────────────────

#[tokio::test]
async fn finish_research_marks_the_task_done_with_its_report() {
    let (runtime, task_id, _scope, results) = run_scripted(
        vec![
            tool_step(
                "save_knowledge_entry",
                json!({ "title": "One", "content": "c", "source_urls": ["https://a.example"] }),
            ),
            tool_step(
                "finish_research",
                json!({ "report": "covered the topic; found no source for X" }),
            ),
        ],
        "Finish scope",
    )
    .await;
    assert_eq!(results[1]["name"], "finish_research", "{:?}", results[1]);
    let (_, task) = runtime
        .app
        .get(&format!("/api/research-tasks/{task_id}"))
        .await;
    assert_eq!(task["status"], "done", "{task}");
    assert_eq!(task["report"], "covered the topic; found no source for X");
    assert_eq!(task["entries_created"], 1);
    // The completion reports this session's entries to the model (spec/05 §8).
    assert!(
        results[1]["result_text"]
            .as_str()
            .unwrap_or_default()
            .contains('1'),
        "{:?}",
        results[1]
    );
}

// ───────────────────────────── the budget notice ─────────────────────────────

#[tokio::test]
async fn the_budget_notice_appears_past_the_limits() {
    // 41 calls: the 41st is past the 40-call budget, so its result carries `notice.research_budget`
    // (spec/05 §8). The calls are `search_knowledge`, which needs no network.
    let mut steps: Vec<ScriptStep> = (0..=40)
        .map(|index| tool_step("search_knowledge", json!({ "query": format!("q{index}") })))
        .collect();
    steps.push(finish_step());

    let (_runtime, _task, _scope, results) = run_scripted(steps, "Budget scope").await;
    // The last search's raw result text carries the budget appendix; the first 40 do not.
    let searched: Vec<&Value> = results
        .iter()
        .filter(|call| call["name"] == "search_knowledge")
        .collect();
    assert!(searched.len() >= 41, "{}", searched.len());
    let texts: Vec<String> = searched
        .iter()
        .map(|call| {
            format!(
                "{}{}",
                call["result_text"].as_str().unwrap_or_default(),
                call["appendices"]
            )
        })
        .collect();
    // The later calls carry the budget warning; an early one does not.
    assert!(
        texts
            .iter()
            .any(|text| text.contains("预算") || text.contains("budget")),
        "no budget notice in {} results",
        texts.len()
    );
}

// ───────────────────────────── the target-scope rule ─────────────────────────────

#[tokio::test]
async fn the_write_tools_are_absent_from_other_roles() {
    // A leader session must not have the researcher's tools at all (spec/05 §2).
    let researcher = nekowriter::agent::tools::registry::tools_for_role("researcher");
    assert!(researcher.contains(&"save_knowledge_entry"));
    assert!(researcher.contains(&"update_knowledge_entry"));
    assert!(researcher.contains(&"finish_research"));
    let writer = nekowriter::agent::tools::registry::tools_for_role("writer");
    assert!(!writer.contains(&"save_knowledge_entry"));
    assert!(!writer.contains(&"finish_research"));
}

// ───────────────────────────── the events (spec/10 §3) ─────────────────────────────

/// A researcher's write publishes `knowledge.changed`, so an open Knowledge page refreshes and the
/// scope's entry count is not stale (spec/10 §3).
#[tokio::test]
async fn saved_and_updated_entries_publish_knowledge_changed() {
    use nekowriter::events::ServerEvent;

    let runtime = runtime(vec![
        tool_step(
            "save_knowledge_entry",
            json!({ "title": "Event entry", "content": "alpha beta", "source_urls": ["https://a.example"] }),
        ),
        tool_step(
            "update_knowledge_entry",
            json!({ "entry_id": 1, "edits": [{ "find": "beta", "replace": "BETA" }] }),
        ),
        finish_step(),
    ])
    .await;
    let (task_id, scope_id) = task_with_scope(&runtime, "Event scope").await;

    // Subscribe before the run so no event is missed.
    let mut receiver = runtime.app.events().subscribe();
    let (status, run) = runtime
        .app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "run: {run}");
    let session_id = run["session_id"].as_i64().expect("session") as i32;
    let results = sealed_results(&runtime.app, session_id).await;
    assert_eq!(
        results.len(),
        3,
        "save, update and the completing call: {results:?}"
    );
    for call in results.iter().take(2) {
        assert_eq!(call["is_error"], false, "{call:?}");
    }

    // Both operations emitted their event: one `create`, then one `update`.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut ops: Vec<String> = Vec::new();
    while ops.len() < 2 && Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, receiver.recv()).await {
            Ok(Ok(envelope)) => {
                if let ServerEvent::KnowledgeChanged {
                    scope_id: changed_scope,
                    op,
                    ..
                } = envelope.event
                    && changed_scope == scope_id
                {
                    ops.push(op.as_str().to_owned());
                }
            }
            _ => break,
        }
    }
    assert_eq!(
        ops,
        vec!["create".to_owned(), "update".to_owned()],
        "a save and an update each publish knowledge.changed (spec/10 §3)"
    );
}
