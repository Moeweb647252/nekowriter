//! Knowledge-first book creation and auto-start (M15 15.5, spec/08 §5, D-158).

#[path = "conception_harness/mod.rs"]
mod harness;

use std::time::Duration;

use harness::*;
use nekowriter_llm::mock::ScriptStep;
use serde_json::{Value, json};

async fn create_book(
    runtime: &Runtime,
    title: &str,
    knowledge_check: bool,
    auto_start: bool,
) -> Value {
    let (status, body) = runtime
        .app
        .post(
            "/api/books",
            json!({
                "title": title,
                "idea": "an idea",
                "genre": "xianxia",
                "writing_language": "zh-CN",
                "start_conception": true,
                "knowledge_check": knowledge_check,
                "auto_start_conception": auto_start,
            }),
        )
        .await;
    assert_eq!(status, 201, "create book: {body}");
    body
}

async fn sessions(runtime: &Runtime, book_id: i32, kind: &str) -> Vec<(i32, String)> {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    runtime
        .app
        .db()
        .conn()
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT id, status FROM agent_sessions WHERE book_id = ? AND kind = ? ORDER BY id",
            vec![book_id.into(), kind.into()],
        ))
        .await
        .expect("sessions")
        .into_iter()
        .map(|row| {
            (
                row.try_get("", "id").unwrap(),
                row.try_get("", "status").unwrap(),
            )
        })
        .collect()
}

async fn wait_for_conception(runtime: &Runtime, book_id: i32) {
    for _ in 0..300 {
        if !sessions(runtime, book_id, "conception").await.is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("auto-start did not create a conception session");
}

async fn check_task_count(app: &nekowriter::testing::TestApp, book_id: i32) -> i64 {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT count(*) AS n FROM research_tasks WHERE book_id = ? AND kind = 'preparation'",
            vec![book_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get("", "n").unwrap()
}

async fn kickoff(runtime: &Runtime, session_id: i32) -> String {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT content FROM agent_messages WHERE session_id = ? AND seq = 0",
            vec![session_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    let blocks: Vec<Value> =
        serde_json::from_str(&row.try_get::<String>("", "content").unwrap()).unwrap();
    blocks
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

async fn tools(runtime: &Runtime, session_id: i32) -> Vec<String> {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT tools FROM agent_sessions WHERE id = ?",
            vec![session_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    serde_json::from_str::<Vec<Value>>(&row.try_get::<String>("", "tools").unwrap())
        .unwrap()
        .into_iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn knowledge_check_starts_one_conception_session_with_scopes_and_report() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "finish_research",
            json!({ "report": "The base is sufficient." }),
        ),
        ScriptStep::new().text("Conception started."),
    ])
    .await;
    let (status, scope) = runtime
        .app
        .post(
            "/api/knowledge/scopes",
            json!({ "name": "Existing scope", "description": "A visible scope" }),
        )
        .await;
    assert_eq!(status, 201, "{scope}");
    let scope_id = scope["id"].as_i64().unwrap() as i32;
    let (status, _) = runtime
        .app
        .post(
            &format!("/api/knowledge/scopes/{scope_id}/entries"),
            json!({ "title": "Existing entry", "content": "content" }),
        )
        .await;
    assert_eq!(status, 201);

    let book = create_book(&runtime, "Knowledge-first book", true, true).await;
    let book_id = book["book"]["id"].as_i64().unwrap() as i32;
    let check_session = book["session_id"].as_i64().unwrap() as i32;
    assert!(book["research_task_id"].is_number(), "{book}");
    assert!(sessions(&runtime, book_id, "conception").await.is_empty());
    runtime.wait_any_status(check_session).await;
    wait_for_conception(&runtime, book_id).await;

    let conception = sessions(&runtime, book_id, "conception").await;
    assert_eq!(conception.len(), 1, "{conception:?}");
    let text = kickoff(&runtime, conception[0].0).await;
    assert!(text.contains("Existing scope"), "scopes in kickoff: {text}");
    assert!(
        text.contains("The base is sufficient."),
        "report in kickoff: {text}"
    );
    assert!(text.contains("【已有知识】"), "{text}");
    let names = tools(&runtime, conception[0].0).await;
    assert!(names.contains(&"search_knowledge".to_owned()), "{names:?}");
    assert!(
        names.contains(&"read_knowledge_entry".to_owned()),
        "{names:?}"
    );
}

#[tokio::test]
async fn auto_start_off_leaves_no_session_and_manual_start_works() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("finish_research", json!({ "report": "done" })),
    ])
    .await;
    let book = create_book(&runtime, "No auto-start", true, false).await;
    let book_id = book["book"]["id"].as_i64().unwrap() as i32;
    let check_session = book["session_id"].as_i64().unwrap() as i32;
    runtime.wait_any_status(check_session).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(sessions(&runtime, book_id, "conception").await.is_empty());
    let (status, body) = runtime
        .app
        .post(&format!("/api/books/{book_id}/conception/start"), json!({}))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(sessions(&runtime, book_id, "conception").await.len(), 1);
}

#[tokio::test]
async fn abandoned_check_starts_no_conception() {
    let runtime = runtime(vec![ScriptStep::new().delay(Duration::from_secs(20))]).await;
    let book = create_book(&runtime, "Abandoned check", true, true).await;
    let book_id = book["book"]["id"].as_i64().unwrap() as i32;
    let check_session = book["session_id"].as_i64().unwrap() as i32;
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{check_session}/abandon"), json!({}))
        .await;
    assert_eq!(status, 200, "{body}");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(sessions(&runtime, book_id, "conception").await.is_empty());
}

#[tokio::test]
async fn manually_started_conception_prevents_a_second_auto_start() {
    let runtime = runtime(vec![
        ScriptStep::new()
            .delay(Duration::from_secs(2))
            .tool("finish_research", json!({ "report": "done" })),
        ScriptStep::new().text("conception"),
    ])
    .await;
    let book = create_book(&runtime, "Manual race", true, true).await;
    let book_id = book["book"]["id"].as_i64().unwrap() as i32;
    let check_session = book["session_id"].as_i64().unwrap() as i32;
    let (status, body) = runtime
        .app
        .post(&format!("/api/books/{book_id}/conception/start"), json!({}))
        .await;
    assert_eq!(status, 200, "{body}");
    runtime.wait_any_status(check_session).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(sessions(&runtime, book_id, "conception").await.len(), 1);
}

#[tokio::test]
async fn missing_researcher_falls_back_to_conception_when_auto_start_is_on() {
    let runtime = runtime(vec![]).await;
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["researcher"] = Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "{body}");
    let book = create_book(&runtime, "No researcher", true, true).await;
    let book_id = book["book"]["id"].as_i64().unwrap() as i32;
    assert!(
        book["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w == "model.missing_role")
    );
    assert_eq!(check_task_count(&runtime.app, book_id).await, 0);
    assert_eq!(sessions(&runtime, book_id, "conception").await.len(), 1);
}

#[tokio::test]
async fn missing_conception_model_leaves_the_check_done_without_error() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("finish_research", json!({ "report": "done" })),
    ])
    .await;
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["conception"] = Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "{body}");
    let book = create_book(&runtime, "No conception", true, true).await;
    let book_id = book["book"]["id"].as_i64().unwrap() as i32;
    let check_session = book["session_id"].as_i64().unwrap() as i32;
    runtime.wait_any_status(check_session).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(sessions(&runtime, book_id, "conception").await.is_empty());
    let (status, task) = runtime
        .app
        .get(&format!(
            "/api/research-tasks/{}",
            book["research_task_id"].as_i64().unwrap()
        ))
        .await;
    assert_eq!(status, 200, "{task}");
    assert_eq!(task["status"], "done", "{task}");
}

#[tokio::test]
async fn old_request_shape_still_starts_conception_immediately() {
    let runtime = runtime(vec![ScriptStep::new().text("conception")]).await;
    let (status, body) = runtime
        .app
        .post(
            "/api/books",
            json!({
                "title": "Old request",
                "idea": "idea",
                "genre": "xianxia",
                "writing_language": "zh-CN",
                "start_conception": true,
            }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let book_id = body["book"]["id"].as_i64().unwrap() as i32;
    assert_eq!(sessions(&runtime, book_id, "conception").await.len(), 1);
    assert_eq!(check_task_count(&runtime.app, book_id).await, 0);
}

#[tokio::test]
async fn an_abandoned_conception_session_prevents_auto_start() {
    let runtime = runtime(vec![
        ScriptStep::new()
            .delay(Duration::from_secs(2))
            .tool("finish_research", json!({ "report": "done" })),
        ScriptStep::new().text("manual conception"),
    ])
    .await;
    let book = create_book(&runtime, "Abandoned conception", true, true).await;
    let book_id = book["book"]["id"].as_i64().unwrap() as i32;
    let check_session = book["session_id"].as_i64().unwrap() as i32;
    let (status, manual) = runtime
        .app
        .post(&format!("/api/books/{book_id}/conception/start"), json!({}))
        .await;
    assert_eq!(status, 200, "{manual}");
    let manual_session = manual["session_id"].as_i64().unwrap() as i32;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{manual_session}/abandon"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    runtime.wait_any_status(check_session).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let conception = sessions(&runtime, book_id, "conception").await;
    assert_eq!(conception.len(), 1, "{conception:?}");
    assert_eq!(conception[0].1, "abandoned");
}

#[tokio::test]
async fn failed_check_starts_nothing() {
    let runtime = runtime(vec![ScriptStep::new().delay(Duration::from_secs(20))]).await;
    let book = create_book(&runtime, "Failed check", true, true).await;
    let book_id = book["book"]["id"].as_i64().unwrap() as i32;
    let check_session = book["session_id"].as_i64().unwrap() as i32;
    runtime
        .app
        .execute(&format!(
            "UPDATE agent_sessions SET status = 'failed' WHERE id = {check_session}"
        ))
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(sessions(&runtime, book_id, "conception").await.is_empty());
}

#[tokio::test]
async fn missing_researcher_with_auto_off_leaves_nothing_running() {
    let runtime = runtime(Vec::new()).await;
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["researcher"] = Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "{body}");
    let book = create_book(&runtime, "No researcher off", true, false).await;
    let book_id = book["book"]["id"].as_i64().unwrap() as i32;
    assert_eq!(check_task_count(&runtime.app, book_id).await, 0);
    assert!(sessions(&runtime, book_id, "conception").await.is_empty());
}

#[tokio::test]
async fn conception_search_knowledge_sees_only_visible_scopes() {
    let runtime = runtime(vec![
        ScriptStep::new()
            .delay(Duration::from_secs(1))
            .tool("search_knowledge", json!({ "query": "tea" })),
        ScriptStep::new().text("done"),
    ])
    .await;
    let (_, visible) = runtime
        .app
        .post(
            "/api/knowledge/scopes",
            json!({ "name": "Visible scope", "description": "" }),
        )
        .await;
    let visible_id = visible["id"].as_i64().unwrap() as i32;
    let (_, hidden) = runtime
        .app
        .post(
            "/api/knowledge/scopes",
            json!({ "name": "Hidden scope", "description": "" }),
        )
        .await;
    let hidden_id = hidden["id"].as_i64().unwrap() as i32;
    for (scope_id, title, content) in [
        (visible_id, "Visible tea", "tea in the visible scope"),
        (hidden_id, "Hidden tea", "tea in the hidden scope"),
    ] {
        let (status, body) = runtime
            .app
            .post(
                &format!("/api/knowledge/scopes/{scope_id}/entries"),
                json!({ "title": title, "content": content }),
            )
            .await;
        assert_eq!(status, 201, "{body}");
    }
    let (status, book) = runtime
        .app
        .post(
            "/api/books",
            json!({
                "title": "Conception search",
                "idea": "idea",
                "genre": "xianxia",
                "writing_language": "zh-CN",
                "start_conception": true,
            }),
        )
        .await;
    assert_eq!(status, 201, "{book}");
    let book_id = book["book"]["id"].as_i64().unwrap() as i32;
    let session_id = book["session_id"].as_i64().unwrap() as i32;
    let (status, _) = runtime
        .app
        .put(
            &format!("/api/books/{book_id}/scopes"),
            json!({ "excluded_scope_ids": [hidden_id] }),
        )
        .await;
    assert_eq!(status, 200);
    runtime.wait_any_status(session_id).await;
    let snapshot = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await
        .1;
    let search = snapshot["tool_calls"]
        .as_array()
        .expect("tool calls")
        .iter()
        .find(|call| call["name"] == "search_knowledge")
        .expect("search call");
    let text = search["result_text"].as_str().unwrap_or_default();
    assert!(text.contains("Visible tea"), "{text}");
    assert!(!text.contains("Hidden tea"), "{text}");
}

#[tokio::test]
async fn missing_researcher_and_conception_warns_once() {
    let runtime = runtime(Vec::new()).await;
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["researcher"] = Value::Null;
    roles["conception"] = Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "{body}");
    let book = create_book(&runtime, "Both missing", true, true).await;
    let warnings = book["warnings"].as_array().unwrap();
    assert_eq!(
        warnings
            .iter()
            .filter(|warning| *warning == "model.missing_role")
            .count(),
        1,
        "{warnings:?}"
    );
    let book_id = book["book"]["id"].as_i64().unwrap() as i32;
    assert!(sessions(&runtime, book_id, "conception").await.is_empty());
    assert_eq!(check_task_count(&runtime.app, book_id).await, 0);
}

#[tokio::test]
async fn rerunning_a_check_after_abandoned_conception_starts_nothing() {
    let runtime = runtime(vec![
        ScriptStep::new()
            .delay(Duration::from_secs(2))
            .tool("finish_research", json!({ "report": "first" })),
        ScriptStep::new().text("manual conception"),
        ScriptStep::new().tool("finish_research", json!({ "report": "second" })),
    ])
    .await;
    let book = create_book(&runtime, "Re-run after abandoned", true, true).await;
    let book_id = book["book"]["id"].as_i64().unwrap() as i32;
    let first_check = book["session_id"].as_i64().unwrap() as i32;
    let (status, manual) = runtime
        .app
        .post(&format!("/api/books/{book_id}/conception/start"), json!({}))
        .await;
    assert_eq!(status, 200, "{manual}");
    let manual_id = manual["session_id"].as_i64().unwrap() as i32;
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{manual_id}/abandon"), json!({}))
        .await;
    assert_eq!(status, 200, "{body}");
    runtime.wait_any_status(first_check).await;
    let (status, second) = runtime
        .app
        .post(&format!("/api/books/{book_id}/knowledge-check"), json!({}))
        .await;
    assert_eq!(status, 201, "{second}");
    let second_check = second["session_id"].as_i64().unwrap() as i32;
    runtime.wait_any_status(second_check).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let conception = sessions(&runtime, book_id, "conception").await;
    assert_eq!(conception.len(), 1, "{conception:?}");
    assert_eq!(conception[0].1, "abandoned");
}
