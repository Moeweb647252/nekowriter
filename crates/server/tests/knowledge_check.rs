//! Knowledge-check backend (M20 20.3, spec/07 §8.1, spec/10 §2.2, D-168).
//!
//! These tests drive the real router and the scripted/Demo providers. They cover the planning
//! kickoff, the four-tool check freeze, the topic-session seven-tool freeze, `set_research_topics`
//! replacement and validation, the R8 old-session `set_target_scope` path and the existing
//! check lifecycle rules.

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

async fn frozen_tools(runtime: &agent_harness::Runtime, session_id: i32) -> Vec<String> {
    let row = runtime
        .app
        .state()
        .db_conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("SELECT tools FROM agent_sessions WHERE id = {session_id}"),
        ))
        .await
        .expect("query session tools")
        .expect("session row");
    let raw: String = row.try_get("", "tools").expect("tools");
    serde_json::from_str::<Vec<Value>>(&raw)
        .expect("tools json")
        .into_iter()
        .map(|tool| tool["name"].as_str().expect("tool name").to_owned())
        .collect()
}

async fn run_topic(runtime: &agent_harness::Runtime, scope_name: &str, topic: &str) -> i32 {
    let (status, task) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": runtime.book_id,
                "scope_name": scope_name,
                "topic": topic,
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let task_id = task["id"].as_i64().expect("task id") as i32;
    let (status, run) = runtime
        .app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    run["session_id"].as_i64().expect("session id") as i32
}

async fn scalar(runtime: &agent_harness::Runtime, sql: &str) -> i64 {
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(DatabaseBackend::Sqlite, sql))
        .await
        .expect("query")
        .expect("row");
    row.try_get("", "n").expect("n")
}

async fn wait_for_call(
    runtime: &agent_harness::Runtime,
    session_id: i32,
    name: &str,
) -> agent_harness::CallRow {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(call) = runtime
            .calls(session_id)
            .await
            .into_iter()
            .find(|call| call.name == name && call.status == "done")
        {
            return call;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session {session_id} never completed {name}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

async fn last_tool_ui(runtime: &agent_harness::Runtime, session_id: i32, name: &str) -> Value {
    let (_, session) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    session["tool_calls"]
        .as_array()
        .expect("tool calls")
        .iter()
        .rfind(|call| call["name"] == name)
        .unwrap_or_else(|| panic!("no {name} call"))["ui"]
        .clone()
}

async fn check_topic_rows(
    runtime: &agent_harness::Runtime,
    check_id: i32,
) -> Vec<(i32, String, String, String, String, Option<i32>)> {
    let rows = runtime
        .app
        .db()
        .conn()
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "SELECT id, topic, origin, status, scope_name, session_id FROM research_tasks                  WHERE check_id = {check_id} AND kind = 'topic' ORDER BY id"
            ),
        ))
        .await
        .expect("topics");
    rows.into_iter()
        .map(|row| {
            (
                row.try_get("", "id").expect("id"),
                row.try_get("", "topic").expect("topic"),
                row.try_get("", "origin").expect("origin"),
                row.try_get("", "status").expect("status"),
                row.try_get("", "scope_name").expect("scope"),
                row.try_get("", "session_id").expect("session"),
            )
        })
        .collect()
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

    let (status, existing) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": runtime.book_id,
                "scope_name": "Visible scope",
                "topic": "Existing topic",
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{existing}");
    let (status, second) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": runtime.book_id,
                "scope_name": "Visible scope",
                "topic": "Existing topic two",
                "requirements": "second detail",
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let (status, third) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": runtime.book_id,
                "scope_name": "Visible scope",
                "topic": "Existing topic three",
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{third}");
    runtime
        .app
        .execute(&format!(
            "INSERT INTO research_tasks(book_id, kind, origin, scope_name, topic, requirements, language, status, created_at, updated_at) \
             VALUES ({}, 'topic', 'conception', 'Visible scope', 'Conception draft', '', 'en', 'draft', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            runtime.book_id
        ))
        .await;

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
    assert!(text.contains("Existing topic"), "{text}");
    assert!(
        text.contains("Conception draft"),
        "conception drafts are shown because the check will not delete them: {text}"
    );
    assert!(
        text.contains("Existing topic（范围：Visible scope，状态：ready）\n- Existing topic two"),
        "the existing-topic list keeps one line per item: {text}"
    );
    assert!(
        text.contains("要求：second detail\n- Existing topic three"),
        "requirements stay on their own item line: {text}"
    );
    assert!(text.contains("最多列出 8 个主题"), "{text}");
    assert!(
        !text.contains("个条目。"),
        "there is no entry budget: {text}"
    );
}

#[tokio::test]
async fn search_knowledge_spans_the_books_visible_scopes() {
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
async fn finishing_a_check_is_done_with_zero_entries() {
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
async fn a_topic_session_freezes_the_old_seven_tools_and_a_check_freezes_four() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("finish_research", json!({ "report": "done" })),
    ])
    .await;
    let topic_session = run_topic(&runtime, "Topic scope", "Topic").await;
    assert_eq!(
        frozen_tools(&runtime, topic_session).await,
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
    assert_eq!(
        frozen_tools(&runtime, check_session).await,
        vec![
            "search_knowledge",
            "read_knowledge_entry",
            "set_research_topics",
            "finish_research"
        ]
    );
    for session in [topic_session, check_session] {
        assert!(
            !frozen_tools(&runtime, session)
                .await
                .contains(&"set_target_scope".to_owned()),
            "no new session freezes set_target_scope"
        );
    }
}

/// R8: a session frozen before M20 keeps `set_target_scope` in its list, and the handler is still
/// registered, so the old call is permitted even though no new session freezes it.
#[tokio::test]
async fn an_old_frozen_session_can_still_call_set_target_scope() {
    use nekowriter::agent::tools::ToolRegistry;
    use nekowriter::agent::tools::registry::is_permitted;
    use nekowriter::db::json::{Phase, SessionKind};

    assert!(ToolRegistry::new().has("set_target_scope"));
    let frozen = vec!["set_target_scope".to_owned()];
    assert!(is_permitted(
        SessionKind::Research,
        Phase::None,
        "set_target_scope",
        &frozen
    ));
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
async fn set_research_topics_in_a_check_inserts_ready_topics_with_check_id() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [
                    { "topic": "First gap", "requirements": "collect one", "scope": "Existing scope" },
                    { "topic": "Second gap", "requirements": "collect two", "scope": "Brand new scope" }
                ]
            }),
        ),
        ScriptStep::new()
            .delay(Duration::from_secs(30))
            .tool("finish_research", json!({ "report": "planned" })),
    ])
    .await;
    let _existing = scope(&runtime, "Existing scope", "An existing scope").await;
    let scopes_before = scalar(&runtime, "SELECT count(*) AS n FROM knowledge_scopes").await;
    let revision_before = scalar(
        &runtime,
        &format!(
            "SELECT CAST(json_extract(conception, '$.topics_revision') AS INTEGER) AS n \
             FROM books WHERE id = {}",
            runtime.book_id
        ),
    )
    .await;
    let language = runtime
        .app
        .get(&format!("/api/books/{}", runtime.book_id))
        .await;
    let writing_language = language.1["writing_language"]
        .as_str()
        .unwrap_or("en")
        .to_owned();

    let (task_id, session_id) = start_check(&runtime, runtime.book_id, "").await;
    // The check's finish step is delayed, so the queue has not started yet. Poll for the tool's
    // rows, then read each row's status and scope_id from the same fresh query.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let ids: Vec<i32> = loop {
        let rows = check_topic_rows(&runtime, task_id).await;
        if rows.len() == 2 {
            break rows.into_iter().map(|row| row.0).collect();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the check never wrote its two topics: {rows:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let calls = call(&runtime, session_id).await;
    let set = calls
        .iter()
        .find(|call| call.0 == "set_research_topics")
        .expect("set call");
    assert!(!set.2, "{set:?}");

    let db = runtime.app.db().conn();
    for id in ids {
        let row = db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Sqlite,
                format!(
                    "SELECT check_id, kind, language, scope_id, book_id, status, origin, session_id                      FROM research_tasks WHERE id = {id}"
                ),
            ))
            .await
            .expect("task")
            .expect("task row");
        assert_eq!(row.try_get::<i32>("", "check_id").unwrap(), task_id);
        assert_eq!(row.try_get::<String>("", "kind").unwrap(), "topic");
        assert_eq!(
            row.try_get::<String>("", "language").unwrap(),
            writing_language
        );
        assert_eq!(row.try_get::<String>("", "origin").unwrap(), "conception");
        assert_eq!(row.try_get::<String>("", "status").unwrap(), "ready");
        assert_eq!(row.try_get::<Option<i32>>("", "scope_id").unwrap(), None);
        assert_eq!(row.try_get::<Option<i32>>("", "session_id").unwrap(), None);
        assert_eq!(row.try_get::<i32>("", "book_id").unwrap(), runtime.book_id);
    }
    assert_eq!(
        scalar(&runtime, "SELECT count(*) AS n FROM knowledge_scopes").await,
        scopes_before,
        "the check's set_research_topics never creates a scope"
    );
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/abandon"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        scalar(
            &runtime,
            &format!(
                "SELECT CAST(json_extract(conception, '$.topics_revision') AS INTEGER) AS n \
                 FROM books WHERE id = {}",
                runtime.book_id
            ),
        )
        .await,
        revision_before,
        "check topics do not bump topics_revision"
    );
}

#[tokio::test]
async fn a_check_set_keeps_author_patched_deferred_running_and_done_topics() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [
                    { "topic": "First", "requirements": "", "scope": "History" },
                    { "topic": "DeleteMe", "requirements": "", "scope": "History" }
                ]
            }),
        ),
        ScriptStep::new().delay(Duration::from_millis(1_500)).tool(
            "set_research_topics",
            json!({
                "topics": [{ "topic": "Replacement", "requirements": "", "scope": "History" }]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
    ])
    .await;
    let (check_id, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait_for_call(&runtime, session_id, "set_research_topics").await;
    let rows = check_topic_rows(&runtime, check_id).await;
    let first_id = rows.iter().find(|row| row.1 == "First").expect("first").0;
    // The author PATCH changes the row to `origin = author`; the other rows cover the statuses
    // the second check call must leave alone.
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/research-tasks/{first_id}"),
            json!({ "topic": "Patched first" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    runtime
        .app
        .execute(&format!(
            "INSERT INTO research_tasks(book_id, kind, origin, scope_name, topic, requirements, language, status, check_id, session_id, report, entries_created, created_at, updated_at) VALUES \
             ({book}, 'topic', 'author', 'History', 'Author added', '', 'en', 'ready', {check}, NULL, '', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'), \
             ({book}, 'topic', 'conception', 'History', 'Deferred one', '', 'en', 'deferred', {check}, NULL, '', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'), \
             ({book}, 'topic', 'conception', 'History', 'Running one', '', 'en', 'running', {check}, NULL, '', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'), \
             ({book}, 'topic', 'conception', 'History', 'Done one', '', 'en', 'done', {check}, NULL, 'done', 2, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            book = runtime.book_id,
            check = check_id
        ))
        .await;
    wait(&runtime, session_id).await;

    let rows = check_topic_rows(&runtime, check_id).await;
    let topics: Vec<&str> = rows.iter().map(|row| row.1.as_str()).collect();
    assert!(topics.contains(&"Patched first"), "{rows:?}");
    assert!(topics.contains(&"Author added"), "{rows:?}");
    assert!(topics.contains(&"Deferred one"), "{rows:?}");
    assert!(topics.contains(&"Running one"), "{rows:?}");
    assert!(topics.contains(&"Done one"), "{rows:?}");
    assert!(topics.contains(&"Replacement"), "{rows:?}");
    assert!(
        !topics.contains(&"DeleteMe"),
        "the untouched waiting agent topic is replaced: {rows:?}"
    );
    let kept = last_tool_ui(&runtime, session_id, "set_research_topics").await["kept"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let kept_topics: Vec<&str> = kept
        .iter()
        .filter_map(|row| row["topic"].as_str())
        .collect();
    assert!(kept_topics.contains(&"Patched first"), "{kept:?}");
    assert!(kept_topics.contains(&"Author added"), "{kept:?}");
    assert!(kept_topics.contains(&"Deferred one"), "{kept:?}");
    assert!(kept_topics.contains(&"Running one"), "{kept:?}");
    assert!(kept_topics.contains(&"Done one"), "{kept:?}");
    assert!(!kept_topics.contains(&"Replacement"), "{kept:?}");
    for (topic, status) in [
        ("Patched first", "ready"),
        ("Author added", "ready"),
        ("Deferred one", "deferred"),
        ("Running one", "running"),
        ("Done one", "done"),
    ] {
        let row = kept
            .iter()
            .find(|row| row["topic"] == topic)
            .unwrap_or_else(|| panic!("{topic} missing from kept: {kept:?}"));
        assert_eq!(row["status"], status, "{topic}: {kept:?}");
    }
}

#[tokio::test]
async fn nine_check_topics_are_validation_and_write_nothing() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": (0..9).map(|index| json!({
                    "topic": format!("Topic {index}"),
                    "requirements": "",
                    "scope": "History"
                })).collect::<Vec<_>>()
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
    ])
    .await;
    let (check_id, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait(&runtime, session_id).await;
    let calls = call(&runtime, session_id).await;
    let set = calls
        .iter()
        .find(|call| call.0 == "set_research_topics")
        .expect("set call");
    assert!(set.2, "{set:?}");
    assert!(set.1.contains("8"), "the limit is named: {set:?}");
    assert!(check_topic_rows(&runtime, check_id).await.is_empty());
}

#[tokio::test]
async fn an_excluded_scope_name_is_scope_excluded_and_writes_nothing() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [{ "topic": "Gap", "requirements": "", "scope": "Excluded scope" }]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
    ])
    .await;
    let excluded = scope(&runtime, "Excluded scope", "Not available").await;
    let (status, _) = runtime
        .app
        .put(
            &format!("/api/books/{}/scopes", runtime.book_id),
            json!({ "excluded_scope_ids": [excluded] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (check_id, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait(&runtime, session_id).await;
    let calls = call(&runtime, session_id).await;
    let set = calls
        .iter()
        .find(|call| call.0 == "set_research_topics")
        .expect("set call");
    assert!(set.2, "{set:?}");
    assert!(set.1.contains("Excluded scope"), "{set:?}");
    assert!(check_topic_rows(&runtime, check_id).await.is_empty());
}

#[tokio::test]
async fn check_topics_work_while_a_conception_session_runs() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [
                    { "topic": "Existing scope gap", "requirements": "", "scope": "Existing scope" },
                    { "topic": "New scope gap", "requirements": "", "scope": "New scope" }
                ]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
    ])
    .await;
    let _existing = scope(&runtime, "Existing scope", "").await;
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
    let scopes_before = scalar(&runtime, "SELECT count(*) AS n FROM knowledge_scopes").await;
    let revision_before = scalar(
        &runtime,
        &format!(
            "SELECT CAST(json_extract(conception, '$.topics_revision') AS INTEGER) AS n \
             FROM books WHERE id = {}",
            runtime.book_id
        ),
    )
    .await;
    let (check_id, session_id) = start_check(&runtime, runtime.book_id, "").await;
    wait(&runtime, session_id).await;
    let rows = check_topic_rows(&runtime, check_id).await;
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(
        scalar(&runtime, "SELECT count(*) AS n FROM knowledge_scopes").await,
        scopes_before,
        "new scope names are not created by planning"
    );
    assert_eq!(
        scalar(
            &runtime,
            &format!(
                "SELECT CAST(json_extract(conception, '$.topics_revision') AS INTEGER) AS n \
                 FROM books WHERE id = {}",
                runtime.book_id
            ),
        )
        .await,
        revision_before
    );
}
