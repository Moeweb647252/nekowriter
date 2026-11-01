//! Knowledge-check backend (M15 15.4, spec/07 §8.1, spec/10 §2.2).
//!
//! These tests drive the real router and the scripted/Demo providers. The acceptance cases cover
//! the preparation task row, the kickoff inventory, `set_target_scope`'s three behaviours, the
//! no-target finish path and the AC8 tool-list freeze.

mod agent_harness;

use std::time::Duration;

use agent_harness::runtime;
use axum::http::StatusCode;
use nekowriter::testing::{TestApp, assert_error};
use nekowriter_llm::mock::ScriptStep;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde_json::{Value, json};

async fn scope(runtime: &agent_harness::Runtime, name: &str, description: &str) -> i32 {
    let (status, body) = runtime
        .app
        .post(
            "/api/knowledge/scopes",
            json!({ "name": name, "description": description }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "scope: {body}");
    body["id"].as_i64().expect("scope id") as i32
}

async fn entry(runtime: &agent_harness::Runtime, scope_id: i32, title: &str, content: &str) -> i32 {
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/knowledge/scopes/{scope_id}/entries"),
            json!({ "title": title, "content": content }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "entry: {body}");
    body["id"].as_i64().expect("entry id") as i32
}

async fn start_check(
    runtime: &agent_harness::Runtime,
    book_id: i32,
    requirements: &str,
) -> (i32, i32) {
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/knowledge-check"),
            json!({ "requirements": requirements }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "knowledge check: {body}");
    (
        body["task"]["id"].as_i64().expect("task id") as i32,
        body["session_id"].as_i64().expect("session id") as i32,
    )
}

async fn kickoff_text(runtime: &agent_harness::Runtime, session_id: i32) -> String {
    let row = runtime
        .app
        .state()
        .db_conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT content FROM agent_messages WHERE session_id = ? AND seq = 0",
            vec![session_id.into()],
        ))
        .await
        .expect("query kickoff")
        .expect("kickoff row");
    let raw: String = row.try_get("", "content").expect("content");
    let blocks: Vec<Value> = serde_json::from_str(&raw).expect("blocks");
    blocks
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    let row = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(DatabaseBackend::Sqlite, sql))
        .await
        .expect("query")
        .expect("row");
    row.try_get("", "n").expect("count")
}

async fn call(runtime: &agent_harness::Runtime, session_id: i32) -> Vec<(String, String, bool)> {
    runtime
        .calls(session_id)
        .await
        .into_iter()
        .map(|call| {
            (
                call.name,
                call.result_text.unwrap_or_default(),
                call.is_error,
            )
        })
        .collect()
}

async fn wait(runtime: &agent_harness::Runtime, session_id: i32) {
    runtime.wait_any_status(session_id).await;
}

async fn book(runtime: &agent_harness::Runtime, title: &str) -> i32 {
    let (status, body) = runtime
        .app
        .post(
            "/api/books",
            json!({
                "title": title,
                "idea": "",
                "genre": "xianxia",
                "writing_language": "zh-CN",
                "start_conception": false,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "book: {body}");
    body["book"]["id"].as_i64().expect("book id") as i32
}

#[tokio::test]
async fn kickoff_lists_visible_scopes_with_capped_titles_and_excluded_names() {
    let runtime = runtime(vec![ScriptStep::new().text("check")]).await;
    let visible = scope(&runtime, "Visible scope", "A visible description").await;
    for index in 0..52 {
        entry(
            &runtime,
            visible,
            &format!("Title {index:02}"),
            &format!("content {index}"),
        )
        .await;
    }
    let excluded = scope(&runtime, "Excluded scope", "Never listed").await;
    let (status, _) = runtime
        .app
        .put(
            &format!("/api/books/{}/scopes", runtime.book_id),
            json!({ "excluded_scope_ids": [excluded] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (_, session_id) = start_check(&runtime, runtime.book_id, "focus on daily life").await;
    wait(&runtime, session_id).await;
    let text = kickoff_text(&runtime, session_id).await;

    assert!(text.contains("【知识检查】"), "{text}");
    assert!(text.contains("Visible scope"), "{text}");
    assert!(text.contains("A visible description"), "{text}");
    assert!(text.contains("Title 00"), "{text}");
    assert!(text.contains("Title 49"), "{text}");
    assert!(!text.contains("Title 50"), "{text}");
    assert!(text.contains("另有 2 个条目标题未列出"), "{text}");
    assert!(
        text.contains("Excluded scope"),
        "excluded names are listed as excluded: {text}"
    );
    assert!(
        !text.contains("Never listed"),
        "a visible description of an excluded scope is not listed: {text}"
    );
    assert!(text.contains("focus on daily life"), "{text}");
}

#[tokio::test]
async fn search_knowledge_spans_the_books_visible_scopes_before_a_target() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("search_knowledge", json!({ "query": "sword" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "inventory checked" })),
    ])
    .await;
    let first = scope(&runtime, "First scope", "").await;
    let second = scope(&runtime, "Second scope", "").await;
    entry(&runtime, first, "First hit", "sword in the first scope").await;
    entry(&runtime, second, "Second hit", "sword in the second scope").await;

    let (_, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait(&runtime, session_id).await;
    let calls = call(&runtime, session_id).await;
    let search = calls
        .iter()
        .find(|call| call.0 == "search_knowledge")
        .expect("search call");
    assert!(!search.2, "{search:?}");
    assert!(search.1.contains("First hit"), "{search:?}");
    assert!(search.1.contains("Second hit"), "{search:?}");
}

#[tokio::test]
async fn save_without_a_target_reports_target_scope_unset() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "save_knowledge_entry",
            json!({
                "title": "No target",
                "content": "no target",
                "source_urls": []
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "no target" })),
    ])
    .await;
    let (_, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait(&runtime, session_id).await;
    let calls = call(&runtime, session_id).await;
    let save = calls
        .iter()
        .find(|call| call.0 == "save_knowledge_entry")
        .expect("save");
    assert!(save.2, "{save:?}");
    assert!(save.1.contains("目标知识范围"), "{save:?}");
}

#[tokio::test]
async fn existing_target_ignores_description_and_keeps_the_shared_scope() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_target_scope",
            json!({
                "name": "Existing target",
                "description": "attempted overwrite"
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "done" })),
    ])
    .await;
    let target = scope(&runtime, "Existing target", "original description").await;
    let (task_id, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait(&runtime, session_id).await;
    let (_, scope_body) = runtime
        .app
        .get(&format!("/api/knowledge/scopes/{target}"))
        .await;
    assert_eq!(scope_body["description"], "original description");
    let (_, task) = runtime
        .app
        .get(&format!("/api/research-tasks/{task_id}"))
        .await;
    assert_eq!(task["scope_id"], target);
}

#[tokio::test]
async fn new_target_without_description_is_validation() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("set_target_scope", json!({ "name": "Needs description" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "done" })),
    ])
    .await;
    let (_, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait(&runtime, session_id).await;
    let calls = call(&runtime, session_id).await;
    let target = calls
        .iter()
        .find(|call| call.0 == "set_target_scope")
        .expect("target");
    assert!(target.2, "{target:?}");
    assert!(target.1.contains("description is required"), "{target:?}");
}

#[tokio::test]
async fn new_target_with_description_is_visible_to_another_book() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_target_scope",
            json!({
                "name": "Brand new scope",
                "description": "created by the check"
            }),
        ),
        ScriptStep::new().tool(
            "save_knowledge_entry",
            json!({
                "title": "New entry",
                "content": "new content",
                "source_urls": []
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "created" })),
    ])
    .await;
    let other = book(&runtime, "Other book").await;
    let (_, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait(&runtime, session_id).await;
    let (_, scopes) = runtime.app.get("/api/knowledge/scopes").await;
    let created = scopes
        .as_array()
        .expect("scopes")
        .iter()
        .find(|scope| scope["name"] == "Brand new scope")
        .expect("new scope");
    let scope_id = created["id"].as_i64().unwrap() as i32;
    let (_, other_scopes) = runtime.app.get(&format!("/api/books/{other}/scopes")).await;
    assert!(
        other_scopes
            .as_array()
            .unwrap()
            .iter()
            .any(|scope| scope["id"] == scope_id)
    );
}

#[tokio::test]
async fn excluded_target_is_refused_then_switching_keeps_earlier_entries() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("set_target_scope", json!({ "name": "Excluded target" })),
        ScriptStep::new().tool("set_target_scope", json!({ "name": "First target" })),
        ScriptStep::new().tool(
            "save_knowledge_entry",
            json!({
                "title": "First entry", "content": "first", "source_urls": []
            }),
        ),
        ScriptStep::new().tool("set_target_scope", json!({ "name": "Second target" })),
        ScriptStep::new().tool(
            "save_knowledge_entry",
            json!({
                "title": "Second entry", "content": "second", "source_urls": []
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "switched" })),
    ])
    .await;
    let excluded = scope(&runtime, "Excluded target", "").await;
    let first = scope(&runtime, "First target", "first target").await;
    let second = scope(&runtime, "Second target", "second target").await;
    let (status, _) = runtime
        .app
        .put(
            &format!("/api/books/{}/scopes", runtime.book_id),
            json!({ "excluded_scope_ids": [excluded] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait(&runtime, session_id).await;
    let calls = call(&runtime, session_id).await;
    let target = calls
        .iter()
        .find(|call| call.0 == "set_target_scope")
        .expect("target");
    assert!(target.2, "{target:?}");
    assert!(target.1.contains("排除"), "{target:?}");
    let (_, first_entries) = runtime
        .app
        .get(&format!("/api/knowledge/scopes/{first}/entries"))
        .await;
    let (_, second_entries) = runtime
        .app
        .get(&format!("/api/knowledge/scopes/{second}/entries"))
        .await;
    assert!(
        first_entries["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["title"] == "First entry")
    );
    assert!(
        second_entries["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["title"] == "Second entry")
    );
}

#[tokio::test]
async fn finishing_without_target_or_entries_is_done_with_zero() {
    let runtime = runtime(vec![ScriptStep::new().tool(
        "finish_research",
        json!({ "report": "the base is sufficient" }),
    )])
    .await;
    let (task_id, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait(&runtime, session_id).await;
    let (_, task) = runtime
        .app
        .get(&format!("/api/research-tasks/{task_id}"))
        .await;
    assert_eq!(task["status"], "done", "{task}");
    assert_eq!(task["entries_created"], 0, "{task}");
}

#[tokio::test]
async fn a_second_check_while_one_is_open_is_409_with_session_id() {
    let runtime = runtime(vec![
        ScriptStep::new()
            .delay(Duration::from_secs(20))
            .text("slow"),
    ])
    .await;
    let (_, first_session) = start_check(&runtime, runtime.book_id, "").await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{}/knowledge-check", runtime.book_id),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "research.invalid_state");
    assert_eq!(body["error"]["details"]["session_id"], first_session);
}

#[tokio::test]
async fn archived_book_is_409() {
    let runtime = runtime(Vec::new()).await;
    let (status, _) = runtime
        .app
        .patch(
            &format!("/api/books/{}", runtime.book_id),
            json!({ "status": "archived" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{}/knowledge-check", runtime.book_id),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "book.invalid_state");
}

#[tokio::test]
async fn missing_researcher_model_leaves_no_task_row() {
    let app = TestApp::new().await;
    let book = app.create_book("No researcher").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (status, body) = app
        .post(&format!("/api/books/{book_id}/knowledge-check"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "model.missing_role");
    assert_eq!(
        count(&app, "SELECT count(*) AS n FROM research_tasks").await,
        0,
        "the failed transaction leaves no task row"
    );
}

#[tokio::test]
async fn patch_of_a_preparation_task_rejects_topic_and_scope_name() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("finish_research", json!({ "report": "done" })),
    ])
    .await;
    let (task_id, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait(&runtime, session_id).await;
    for field in ["topic", "scope_name"] {
        let (status, body) = runtime
            .app
            .patch(
                &format!("/api/research-tasks/{task_id}"),
                json!({ field: "forbidden" }),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{field}: {body}");
        assert_error(&body, "validation.invalid_fields");
    }
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/research-tasks/{task_id}"),
            json!({ "requirements": "updated focus" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["requirements"], "updated focus");
}

#[tokio::test]
async fn a_topic_session_freezes_the_old_seven_tools_and_a_check_freezes_eight() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("finish_research", json!({ "report": "done" })),
    ])
    .await;
    let (status, task) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({ "book_id": runtime.book_id, "scope_name": "Topic scope", "topic": "Topic", "language": "en" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let task_id = task["id"].as_i64().unwrap() as i32;
    let (status, run) = runtime
        .app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    let topic_session = run["session_id"].as_i64().unwrap() as i32;
    let topic_row = runtime
        .app
        .state()
        .db_conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("SELECT tools FROM agent_sessions WHERE id = {topic_session}"),
        ))
        .await
        .unwrap()
        .expect("topic session");
    let topic: String = topic_row.try_get("", "tools").unwrap();
    let topic_tools: Vec<String> = serde_json::from_str::<Vec<Value>>(&topic)
        .unwrap()
        .into_iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        topic_tools,
        vec![
            "web_search",
            "web_fetch",
            "search_knowledge",
            "read_knowledge_entry",
            "save_knowledge_entry",
            "update_knowledge_entry",
            "finish_research"
        ]
    );

    let (_, check_session) = start_check(&runtime, runtime.book_id, "").await;
    let prep_row = runtime
        .app
        .state()
        .db_conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("SELECT tools FROM agent_sessions WHERE id = {check_session}"),
        ))
        .await
        .unwrap()
        .expect("check session");
    let prep: String = prep_row.try_get("", "tools").unwrap();
    let prep_tools: Vec<String> = serde_json::from_str::<Vec<Value>>(&prep)
        .unwrap()
        .into_iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(prep_tools.len(), 8);
    assert_eq!(prep_tools[4], "set_target_scope");
}

#[tokio::test]
async fn creating_and_deleting_a_check_while_conception_runs_is_not_settings_locked() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("finish_research", json!({ "report": "done" })),
    ])
    .await;
    // Put the book in conception and leave an open conception session, as the studio does while
    // the author is answering a form.
    runtime
        .app
        .execute(&format!(
            "UPDATE books SET status = 'conceiving' WHERE id = {}",
            runtime.book_id
        ))
        .await;
    runtime
        .app
        .execute(&format!(
            "INSERT INTO agent_sessions(book_id, role, kind, model_snapshot, system_prompt, tools, status, created_at, updated_at) \
             VALUES ({}, 'conception', 'conception', '{{}}', '', '[]', 'running', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            runtime.book_id
        ))
        .await;

    let (task_id, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait(&runtime, session_id).await;
    let (status, body) = runtime
        .app
        .delete(&format!("/api/research-tasks/{task_id}"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

#[tokio::test]
async fn finish_after_a_target_switch_counts_entries_in_both_scopes() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("set_target_scope", json!({ "name": "First target" })),
        ScriptStep::new().tool(
            "save_knowledge_entry",
            json!({
                "title": "First entry", "content": "first", "source_urls": []
            }),
        ),
        ScriptStep::new().tool("set_target_scope", json!({ "name": "Second target" })),
        ScriptStep::new().tool(
            "save_knowledge_entry",
            json!({
                "title": "Second entry", "content": "second", "source_urls": []
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "both targets" })),
    ])
    .await;
    scope(&runtime, "First target", "first").await;
    scope(&runtime, "Second target", "second").await;
    let (task_id, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait(&runtime, session_id).await;
    let session = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await
        .1;
    let finish = session["tool_calls"]
        .as_array()
        .expect("tool calls")
        .iter()
        .find(|call| call["name"] == "finish_research")
        .expect("finish call");
    assert!(
        finish["result_text"]
            .as_str()
            .unwrap_or_default()
            .contains("2 个条目"),
        "{finish}"
    );
    assert_eq!(finish["ui"]["entries"], 2, "{finish}");
    let (_, task) = runtime
        .app
        .get(&format!("/api/research-tasks/{task_id}"))
        .await;
    assert_eq!(task["entries_created"], 2, "{task}");
}

#[tokio::test]
async fn set_target_scope_publishes_research_updated_before_finish() {
    use nekowriter::db::json::ResearchTaskStatus;
    use nekowriter::events::{EventEnvelope, ServerEvent};
    use tokio::sync::broadcast::error::TryRecvError;

    let runtime = runtime(vec![
        ScriptStep::new().tool("set_target_scope", json!({ "name": "Existing target" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "done" })),
    ])
    .await;
    let mut receiver = runtime.app.events().subscribe();
    scope(&runtime, "Existing target", "existing").await;
    let (task_id, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait(&runtime, session_id).await;

    let mut saw_target = false;
    let mut saw_finish = false;
    loop {
        match receiver.try_recv() {
            Ok(EventEnvelope {
                event: ServerEvent::ResearchUpdated { task },
                ..
            }) if task.id == task_id => {
                match task.status {
                    ResearchTaskStatus::Running if task.scope_name == "Existing target" => {
                        saw_target = true;
                    }
                    ResearchTaskStatus::Done => saw_finish = true,
                    _ => {}
                }
                if saw_target && saw_finish {
                    break;
                }
            }
            Ok(_) => {}
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Lagged(_)) => continue,
            Err(TryRecvError::Closed) => break,
        }
    }
    assert!(
        saw_target,
        "set_target_scope must emit research.updated with the new target"
    );
    assert!(
        saw_finish,
        "finish must still emit its own research.updated"
    );
}

#[tokio::test]
async fn excluding_a_running_target_refuses_save_until_it_is_switched() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("set_target_scope", json!({ "name": "Running target" })),
        ScriptStep::new().delay(Duration::from_secs(2)).tool(
            "save_knowledge_entry",
            json!({
                "title": "Blocked entry", "content": "blocked", "source_urls": []
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "stopped" })),
    ])
    .await;
    let target = scope(&runtime, "Running target", "running").await;
    let (task_id, session_id) = start_check(&runtime, runtime.book_id, "").await;
    // Wait until the agent has selected the target, then the author unchecks it while the session
    // is still running.
    for _ in 0..200 {
        let (_, task) = runtime
            .app
            .get(&format!("/api/research-tasks/{task_id}"))
            .await;
        if task["scope_name"] == "Running target" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (status, _) = runtime
        .app
        .put(
            &format!("/api/books/{}/scopes", runtime.book_id),
            json!({ "excluded_scope_ids": [target] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    wait(&runtime, session_id).await;
    let calls = call(&runtime, session_id).await;
    let save = calls
        .iter()
        .find(|call| call.0 == "save_knowledge_entry")
        .expect("save call");
    assert!(save.2, "the excluded target must refuse the save: {save:?}");
    assert!(save.1.contains("排除"), "{save:?}");
}

#[tokio::test]
async fn excluding_a_running_target_refuses_update_until_it_is_switched() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("set_target_scope", json!({ "name": "Update target" })),
        ScriptStep::new().delay(Duration::from_secs(2)).tool(
            "update_knowledge_entry",
            json!({ "entry_id": 1, "content": "changed" }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "stopped" })),
    ])
    .await;
    let target = scope(&runtime, "Update target", "update").await;
    let entry_id = entry(&runtime, target, "Existing entry", "original").await;
    // The script above uses entry id 1; this test creates the first entry in a fresh database.
    assert_eq!(entry_id, 1);
    let (task_id, session_id) = start_check(&runtime, runtime.book_id, "").await;
    for _ in 0..200 {
        let (_, task) = runtime
            .app
            .get(&format!("/api/research-tasks/{task_id}"))
            .await;
        if task["scope_name"] == "Update target" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (status, _) = runtime
        .app
        .put(
            &format!("/api/books/{}/scopes", runtime.book_id),
            json!({ "excluded_scope_ids": [target] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    wait(&runtime, session_id).await;
    let calls = call(&runtime, session_id).await;
    let update = calls
        .iter()
        .find(|call| call.0 == "update_knowledge_entry")
        .expect("update call");
    assert!(
        update.2,
        "the excluded target must refuse the update: {update:?}"
    );
    assert!(update.1.contains("排除"), "{update:?}");
}

#[tokio::test]
async fn a_topic_session_can_finish_after_its_target_is_excluded() {
    let runtime = runtime(vec![
        ScriptStep::new()
            .delay(Duration::from_secs(2))
            .tool("finish_research", json!({ "report": "topic done" })),
    ])
    .await;
    let target = scope(&runtime, "Topic target", "topic").await;
    let (status, task) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({ "book_id": runtime.book_id, "scope_name": "Topic target", "topic": "Topic", "language": "en" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let task_id = task["id"].as_i64().unwrap() as i32;
    let (status, run) = runtime
        .app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    let session_id = run["session_id"].as_i64().unwrap() as i32;
    for _ in 0..200 {
        let (_, task) = runtime
            .app
            .get(&format!("/api/research-tasks/{task_id}"))
            .await;
        if task["scope_id"] == target {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (status, _) = runtime
        .app
        .put(
            &format!("/api/books/{}/scopes", runtime.book_id),
            json!({ "excluded_scope_ids": [target] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    wait(&runtime, session_id).await;
    let calls = call(&runtime, session_id).await;
    let finish = calls
        .iter()
        .find(|call| call.0 == "finish_research")
        .expect("finish call");
    assert!(
        !finish.2,
        "a topic session must still be able to finish: {finish:?}"
    );
    let (_, task) = runtime
        .app
        .get(&format!("/api/research-tasks/{task_id}"))
        .await;
    assert_eq!(task["status"], "done", "{task}");
}
