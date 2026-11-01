//! Opt-out knowledge visibility (M15 15.3, spec/07 §1, D-158).
//!
//! A scope is visible to every book unless that book has an exclusion row. The tests cover the
//! full rule: a new scope reaches every book, exclusions hide it from the API, kickoff and search
//! tools, unknown ids leave the set unchanged, author Run/Finalize clear the exclusion, and
//! deleting a scope cascades its exclusions away.

mod agent_write_harness;

use agent_write_harness::base::runtime;
use nekowriter_llm::mock::ScriptStep;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde_json::{Value, json};

async fn scope(runtime: &agent_write_harness::Runtime, name: &str, description: &str) -> i32 {
    let (status, body) = runtime
        .app
        .post(
            "/api/knowledge/scopes",
            json!({ "name": name, "description": description }),
        )
        .await;
    assert_eq!(status, 201, "scope: {body}");
    body["id"].as_i64().expect("scope id") as i32
}

async fn entry(runtime: &agent_write_harness::Runtime, scope_id: i32, title: &str) -> i32 {
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/knowledge/scopes/{scope_id}/entries"),
            json!({ "title": title, "content": "content" }),
        )
        .await;
    assert_eq!(status, 201, "entry: {body}");
    body["id"].as_i64().expect("entry id") as i32
}

async fn book(runtime: &agent_write_harness::Runtime, title: &str) -> i32 {
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
    assert_eq!(status, 201, "book: {body}");
    body["book"]["id"].as_i64().expect("book id") as i32
}

async fn put_exclusions(
    runtime: &agent_write_harness::Runtime,
    book_id: i32,
    ids: &[i32],
) -> Vec<Value> {
    let (status, body) = runtime
        .app
        .put(
            &format!("/api/books/{book_id}/scopes"),
            json!({ "excluded_scope_ids": ids }),
        )
        .await;
    assert_eq!(status, 200, "put exclusions: {body}");
    body.as_array().expect("scopes").clone()
}

async fn get_scopes(runtime: &agent_write_harness::Runtime, book_id: i32) -> Vec<Value> {
    let (status, body) = runtime
        .app
        .get(&format!("/api/books/{book_id}/scopes"))
        .await;
    assert_eq!(status, 200, "get scopes: {body}");
    body.as_array().expect("scopes").clone()
}

async fn kickoff_text(runtime: &agent_write_harness::Runtime, session_id: i32) -> String {
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

async fn exclusion_count(runtime: &agent_write_harness::Runtime, scope_id: i32) -> i64 {
    let row = runtime
        .app
        .state()
        .db_conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT count(*) AS n FROM book_scope_exclusions WHERE scope_id = ?",
            vec![scope_id.into()],
        ))
        .await
        .expect("query exclusion")
        .expect("row");
    row.try_get("", "n").expect("count")
}

async fn excluded_scope_ids(runtime: &agent_write_harness::Runtime, book_id: i32) -> Vec<i32> {
    let rows = runtime
        .app
        .state()
        .db_conn()
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT scope_id FROM book_scope_exclusions WHERE book_id = ? ORDER BY scope_id",
            vec![book_id.into()],
        ))
        .await
        .expect("query exclusions");
    rows.iter()
        .map(|row| row.try_get("", "scope_id").expect("scope id"))
        .collect()
}

async fn create_task(
    runtime: &agent_write_harness::Runtime,
    book_id: i32,
    scope_name: &str,
) -> i32 {
    let (status, body) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({ "book_id": book_id, "scope_name": scope_name, "topic": "Topic", "language": "en" }),
        )
        .await;
    assert_eq!(status, 201, "task: {body}");
    body["id"].as_i64().expect("task id") as i32
}

#[tokio::test]
async fn a_new_scope_is_visible_to_every_book_without_action() {
    let runtime = runtime(Vec::new()).await;
    let other = book(&runtime, "Other Book").await;
    let created = scope(&runtime, "New scope", "Created after both books").await;

    assert!(
        get_scopes(&runtime, runtime.book_id)
            .await
            .iter()
            .any(|item| item["id"] == created),
        "the first book sees the new scope"
    );
    assert!(
        get_scopes(&runtime, other)
            .await
            .iter()
            .any(|item| item["id"] == created),
        "the second book sees the new scope"
    );
}

#[tokio::test]
async fn an_exclusion_hides_a_scope_from_api_kickoff_and_search() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("search_knowledge", json!({ "query": "sword" })),
        ScriptStep::new().tool("read_knowledge_entry", json!({ "entry_id": 1 })),
    ])
    .await;
    let hidden = scope(&runtime, "Hidden scope", "Must not reach the book").await;
    entry(&runtime, hidden, "Secret entry").await;
    put_exclusions(&runtime, runtime.book_id, &[hidden]).await;

    assert!(
        get_scopes(&runtime, runtime.book_id)
            .await
            .iter()
            .all(|item| item["id"] != hidden),
        "GET /books/:id/scopes omits the excluded scope"
    );

    let session_id = agent_write_harness::start_planner(&runtime).await;
    runtime.wait_any_status(session_id).await;
    let kickoff = kickoff_text(&runtime, session_id).await;
    assert!(
        !kickoff.contains("Hidden scope"),
        "block.settings omits it: {kickoff}"
    );

    let session = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await
        .1;
    let calls = session["tool_calls"].as_array().expect("tool calls");
    let search = calls
        .iter()
        .find(|call| call["name"] == "search_knowledge")
        .expect("search call");
    assert!(search["is_error"].as_bool().unwrap_or(false), "{search}");
    assert!(
        search["result_text"]
            .as_str()
            .unwrap_or_default()
            .contains("没有可检索"),
        "search_knowledge sees no visible scope: {search}"
    );
    let read = calls
        .iter()
        .find(|call| call["name"] == "read_knowledge_entry")
        .expect("read call");
    assert!(read["is_error"].as_bool().unwrap_or(false), "{read}");
}

#[tokio::test]
async fn unknown_exclusion_ids_in_a_mixed_list_are_404_and_change_nothing() {
    let runtime = runtime(Vec::new()).await;
    let first = scope(&runtime, "First", "Initially excluded").await;
    let second = scope(&runtime, "Second", "Would be newly excluded").await;
    put_exclusions(&runtime, runtime.book_id, &[first]).await;
    assert_eq!(
        excluded_scope_ids(&runtime, runtime.book_id).await,
        vec![first],
        "the initial stored set is exactly [a]"
    );

    let (status, body) = runtime
        .app
        .put(
            &format!("/api/books/{}/scopes", runtime.book_id),
            json!({ "excluded_scope_ids": [second, 999_999] }),
        )
        .await;
    assert_eq!(status, 404, "unknown id in a mixed list: {body}");
    assert_eq!(
        excluded_scope_ids(&runtime, runtime.book_id).await,
        vec![first],
        "the stored set is still exactly [a]"
    );
    let scopes = get_scopes(&runtime, runtime.book_id).await;
    assert!(
        scopes.iter().any(|item| item["id"] == second),
        "the valid b in the rejected list was not applied: {scopes:?}"
    );
}

#[tokio::test]
async fn run_clears_the_target_scope_exclusion() {
    let runtime = runtime(Vec::new()).await;
    let target = scope(&runtime, "Run target", "Target of a book task").await;
    let task = create_task(&runtime, runtime.book_id, "Run target").await;
    put_exclusions(&runtime, runtime.book_id, &[target]).await;
    assert_eq!(
        exclusion_count(&runtime, target).await,
        1,
        "the Run target starts excluded"
    );

    let (status, body) = runtime
        .app
        .post(&format!("/api/research-tasks/{task}/run"), json!({}))
        .await;
    assert_eq!(status, 201, "run: {body}");
    assert_eq!(
        exclusion_count(&runtime, target).await,
        0,
        "Run deletes the exclusion row"
    );
    assert!(
        get_scopes(&runtime, runtime.book_id)
            .await
            .iter()
            .any(|item| item["id"] == target),
        "Run clears the target exclusion"
    );
}

#[tokio::test]
async fn finalize_clears_a_topic_scope_exclusion() {
    let runtime = runtime(Vec::new()).await;
    // No conception model: the conceiving book has no session, so finalize is deterministic.
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["conception"] = Value::Null;
    roles["archivist"] = Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "drop roles: {body}");

    let (status, response) = runtime
        .app
        .post(
            "/api/books",
            json!({
                "title": "Finalize Book",
                "idea": "idea",
                "genre": "xianxia",
                "writing_language": "zh-CN",
                "start_conception": true,
            }),
        )
        .await;
    assert_eq!(status, 201, "book: {response}");
    let book_id = response["book"]["id"].as_i64().expect("book id") as i32;
    for (kind, content) in [
        ("outline", "# 大纲\n\n入城。"),
        ("world", "# 世界观\n\n剑气。"),
        ("protagonist", "# 主角\n\n沈砚。"),
    ] {
        let (status, body) = runtime
            .app
            .put(
                &format!("/api/books/{book_id}/setting-docs/{kind}"),
                json!({ "content": content, "base_revision": 0, "note": "" }),
            )
            .await;
        assert_eq!(status, 200, "seed {kind}: {body}");
    }
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "沈砚", "role_label": "主角", "summary": "游侠", "profile": "# 沈砚", "note": "" }),
        )
        .await;
    assert_eq!(status, 201, "character: {body}");

    let scope_id = scope(&runtime, "Finalize target", "Topic target").await;
    let task = create_task(&runtime, book_id, "Finalize target").await;
    let (_, task_body) = runtime
        .app
        .get(&format!("/api/research-tasks/{task}"))
        .await;
    assert_eq!(
        task_body["status"], "draft",
        "the task is a conception draft"
    );
    put_exclusions(&runtime, book_id, &[scope_id]).await;
    assert_eq!(
        exclusion_count(&runtime, scope_id).await,
        1,
        "the finalize target starts excluded"
    );

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            Value::Null,
        )
        .await;
    assert_eq!(status, 200, "finalize: {body}");
    assert_eq!(
        exclusion_count(&runtime, scope_id).await,
        0,
        "Finalize deletes the exclusion row"
    );
    assert!(
        get_scopes(&runtime, book_id)
            .await
            .iter()
            .any(|item| item["id"] == scope_id),
        "Finalize clears the topic's scope exclusion"
    );
}

#[tokio::test]
async fn deleting_a_scope_cascades_its_exclusions() {
    let runtime = runtime(Vec::new()).await;
    let scope_id = scope(&runtime, "Doomed", "Will be deleted").await;
    put_exclusions(&runtime, runtime.book_id, &[scope_id]).await;
    assert_eq!(exclusion_count(&runtime, scope_id).await, 1);

    let (status, body) = runtime
        .app
        .delete(&format!("/api/knowledge/scopes/{scope_id}"))
        .await;
    assert_eq!(status, 204, "delete scope: {body}");
    assert_eq!(exclusion_count(&runtime, scope_id).await, 0, "cascade");
}
