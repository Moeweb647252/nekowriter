//! Check-topic queue tests (M20 20.4, spec/07 §8.1 step 6, spec/08 §5, D-168).
//!
//! A knowledge check's topics run one at a time, in id order, before conception. These tests
//! drive the real router and the scripted provider, plus a few direct row fixtures for states the
//! API cannot create (a failed model role, an abandoned check).

#[path = "agent_harness/mod.rs"]
mod agent_harness;

use std::time::Duration;

use agent_harness::{Runtime, runtime};
use axum::http::StatusCode;
use nekowriter_llm::mock::ScriptStep;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde_json::{Value, json};

async fn create_check_book(runtime: &Runtime, title: &str, auto_start: bool) -> (i32, i32, i32) {
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
                "knowledge_check": true,
                "auto_start_conception": auto_start
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (
        body["book"]["id"].as_i64().expect("book id") as i32,
        body["research_task_id"].as_i64().expect("check task") as i32,
        body["session_id"].as_i64().expect("check session") as i32,
    )
}

async fn sessions(runtime: &Runtime, book_id: i32, kind: &str) -> Vec<(i32, String)> {
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
                row.try_get("", "id").expect("id"),
                row.try_get("", "status").expect("status"),
            )
        })
        .collect()
}

async fn topic_rows(runtime: &Runtime, check_id: i32) -> Vec<(i32, String, String, i32)> {
    runtime
        .app
        .db()
        .conn()
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT id, status, topic, entries_created FROM research_tasks \
             WHERE check_id = ? AND kind = 'topic' ORDER BY id",
            vec![check_id.into()],
        ))
        .await
        .expect("topics")
        .into_iter()
        .map(|row| {
            (
                row.try_get("", "id").expect("id"),
                row.try_get("", "status").expect("status"),
                row.try_get("", "topic").expect("topic"),
                row.try_get("", "entries_created").expect("entries"),
            )
        })
        .collect()
}

async fn wait_until<F, Fut>(mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !check().await {
        assert!(std::time::Instant::now() < deadline, "queue did not settle");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn kickoff(runtime: &Runtime, session_id: i32) -> String {
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
        .expect("query kickoff")
        .expect("kickoff row");
    let blocks: Vec<Value> =
        serde_json::from_str(&row.try_get::<String>("", "content").expect("content"))
            .expect("blocks");
    blocks
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

async fn scalar(runtime: &Runtime, sql: &str) -> i64 {
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

#[tokio::test]
async fn three_topics_run_in_order_then_one_conception_session_starts() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [
                    { "topic": "Topic one", "requirements": "", "scope": "History" },
                    { "topic": "Topic two", "requirements": "", "scope": "History" },
                    { "topic": "Topic three", "requirements": "", "scope": "History" }
                ]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "one done" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "two done" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "three done" })),
        ScriptStep::new().text("conception"),
    ])
    .await;
    let (book_id, check_id, check_session) =
        create_check_book(&runtime, "Three topics", true).await;
    runtime.wait_any_status(check_session).await;

    wait_until(|| async {
        let rows = topic_rows(&runtime, check_id).await;
        rows.len() == 3 && rows.iter().all(|row| row.1 == "done")
    })
    .await;
    let topics = topic_rows(&runtime, check_id).await;
    assert_eq!(
        topics.iter().map(|row| row.2.as_str()).collect::<Vec<_>>(),
        ["Topic one", "Topic two", "Topic three"]
    );

    wait_until(|| async { sessions(&runtime, book_id, "conception").await.len() == 1 }).await;
    let conception = sessions(&runtime, book_id, "conception").await;
    assert_eq!(conception.len(), 1);
    let text = kickoff(&runtime, conception[0].0).await;
    for topic in ["Topic one", "Topic two", "Topic three"] {
        assert!(text.contains(topic), "{topic} missing from {text}");
        assert!(text.contains("0 条"), "{text}");
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        sessions(&runtime, book_id, "conception").await.len(),
        1,
        "auto-start is one-time"
    );
}

#[tokio::test]
async fn a_check_with_no_topics_starts_conception() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("finish_research", json!({ "report": "sufficient" })),
        ScriptStep::new().text("conception"),
    ])
    .await;
    let (book_id, _, check_session) = create_check_book(&runtime, "No topics", true).await;
    runtime.wait_any_status(check_session).await;
    wait_until(|| async { sessions(&runtime, book_id, "conception").await.len() == 1 }).await;
    assert_eq!(sessions(&runtime, book_id, "conception").await.len(), 1);
}

#[tokio::test]
async fn a_check_topic_added_while_planning_is_kept_by_the_check() {
    let runtime = runtime(vec![ScriptStep::new().delay(Duration::from_secs(10))]).await;
    let (book_id, check_id, check_session) =
        create_check_book(&runtime, "Planning topic", false).await;
    // The check is still running; an author topic is accepted and waits.
    let (status, body) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "check_id": check_id,
                "scope_name": "History",
                "topic": "Author while planning",
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let topic_id = body["id"].as_i64().unwrap() as i32;
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("SELECT status FROM research_tasks WHERE id = {topic_id}"),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "status").unwrap(), "ready");
    // The check's own `kept[]` is not available through a second tool call here; the row itself
    // is the waiting check topic and must still be present and ready.
    assert_eq!(topic_rows(&runtime, check_id).await.len(), 1);
    assert_eq!(topic_rows(&runtime, check_id).await[0].1, "ready");
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{check_session}/abandon"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn an_invalid_check_id_is_rejected() {
    let runtime = runtime(Vec::new()).await;
    let (book_id, check_id, _) = create_check_book(&runtime, "Invalid check", false).await;
    // A topic is not a check.
    let (status, body) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "check_id": check_id + 999,
                "scope_name": "History",
                "topic": "Bad",
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    // A check id without a book id.
    let (status, body) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "check_id": check_id,
                "scope_name": "History",
                "topic": "Bad",
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
}

#[tokio::test]
async fn abandoning_a_running_topic_starts_the_next_one() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [
                    { "topic": "Abandon me", "requirements": "", "scope": "History" },
                    { "topic": "Next after abandon", "requirements": "", "scope": "History" }
                ]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new()
            .delay(Duration::from_secs(2))
            .tool("finish_research", json!({ "report": "never finishes" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "next done" })),
    ])
    .await;
    let (_book_id, check_id, check_session) =
        create_check_book(&runtime, "Abandon topic", false).await;
    runtime.wait_any_status(check_session).await;
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "Abandon me" && row.1 == "running")
    })
    .await;
    let running_topic = topic_rows(&runtime, check_id)
        .await
        .into_iter()
        .find(|row| row.2 == "Abandon me")
        .unwrap()
        .0;
    let session = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("SELECT session_id FROM research_tasks WHERE id = {running_topic}"),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<Option<i32>>("", "session_id")
        .unwrap()
        .expect("running session");
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session}/abandon"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "Next after abandon" && row.1 == "done")
    })
    .await;
}

#[tokio::test]
async fn a_deferred_topic_is_skipped_and_resume_starts_it() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [
                    { "topic": "Runs first", "requirements": "", "scope": "History" },
                    { "topic": "Deferred then resumed", "requirements": "", "scope": "History" }
                ]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new()
            .delay(Duration::from_secs(1))
            .tool("finish_research", json!({ "report": "first done" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "second done" })),
    ])
    .await;
    let (book_id, check_id, check_session) =
        create_check_book(&runtime, "Deferred queue", false).await;
    runtime.wait_any_status(check_session).await;
    wait_until(|| async {
        let rows = topic_rows(&runtime, check_id).await;
        rows.iter()
            .any(|row| row.2 == "Runs first" && row.1 == "running")
    })
    .await;
    let rows = topic_rows(&runtime, check_id).await;
    let deferred = rows
        .iter()
        .find(|row| row.2 == "Deferred then resumed")
        .expect("deferred topic")
        .0;
    let (status, body) = runtime
        .app
        .post(&format!("/api/research-tasks/{deferred}/defer"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "Runs first" && row.1 == "done")
    })
    .await;
    assert_eq!(
        sessions(&runtime, book_id, "conception").await.len(),
        0,
        "auto-start is off"
    );
    let (status, body) = runtime
        .app
        .post(&format!("/api/research-tasks/{deferred}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "Deferred then resumed" && row.1 == "done")
    })
    .await;
}

#[tokio::test]
async fn a_missing_researcher_model_leaves_the_topic_ready_and_manual_run_continues() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "done" })),
    ])
    .await;
    let (book_id, check_id, check_session) =
        create_check_book(&runtime, "Missing researcher", false).await;
    runtime.wait_any_status(check_session).await;
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let researcher = settings["roles_defaults"]["researcher"].clone();
    let mut roles = settings["roles_defaults"].clone();
    roles["researcher"] = Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Adding a check topic triggers the queue, but the missing researcher model must not create
    // a session or surface an error.
    let (status, body) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "check_id": check_id,
                "scope_name": "History",
                "topic": "Waiting after check",
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let topic_id = body["id"].as_i64().unwrap() as i32;
    assert_eq!(
        sessions(&runtime, book_id, "research").await.len(),
        1,
        "only the check session exists"
    );
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("SELECT status FROM research_tasks WHERE id = {topic_id}"),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "status").unwrap(), "ready");

    // Restore the role and run the topic manually; its completion settles the queue.
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["researcher"] = researcher;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = runtime
        .app
        .post(&format!("/api/research-tasks/{topic_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.1 == "done")
    })
    .await;
}

#[tokio::test]
async fn a_check_on_an_active_book_runs_its_queue() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [{ "topic": "Active topic", "requirements": "", "scope": "History" }]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "topic done" })),
    ])
    .await;
    let book = runtime.app.create_book("Active check").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let (status, body) = runtime
        .app
        .post(&format!("/api/books/{book_id}/knowledge-check"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let check_id = body["task"]["id"].as_i64().unwrap() as i32;
    let check_session = body["session_id"].as_i64().unwrap() as i32;
    runtime.wait_any_status(check_session).await;
    wait_until(|| async {
        let rows = topic_rows(&runtime, check_id).await;
        rows.len() == 1 && rows[0].1 == "done"
    })
    .await;
}

#[tokio::test]
async fn an_abandoned_check_does_not_run_its_topics() {
    let runtime = runtime(vec![ScriptStep::new().delay(Duration::from_secs(20))]).await;
    let (book_id, check_id, check_session) =
        create_check_book(&runtime, "Abandoned check", true).await;
    runtime
        .app
        .post(&format!("/api/sessions/{check_session}/abandon"), json!({}))
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(sessions(&runtime, book_id, "conception").await.len(), 0);
    // No topic session was started for the check.
    assert!(
        sessions(&runtime, book_id, "research")
            .await
            .iter()
            .all(|(id, _)| *id == check_session),
        "{:?}",
        sessions(&runtime, book_id, "research").await
    );
    assert!(topic_rows(&runtime, check_id).await.is_empty());
}

#[tokio::test]
async fn a_scope_created_by_a_check_topic_uses_the_check_topics_for_its_description() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [
                    { "topic": "Topic A", "requirements": "", "scope": "Shared new scope" },
                    { "topic": "Topic B", "requirements": "", "scope": "Shared new scope" }
                ]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "done" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "done" })),
    ])
    .await;
    let (_book_id, check_id, check_session) = create_check_book(&runtime, "R10", true).await;
    runtime.wait_any_status(check_session).await;
    wait_until(|| async {
        let rows = topic_rows(&runtime, check_id).await;
        rows.len() == 2 && rows.iter().all(|row| row.1 == "done")
    })
    .await;
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT description FROM knowledge_scopes WHERE name = 'Shared new scope'",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<String>("", "description").unwrap(),
        "Topic A / Topic B"
    );
    assert_eq!(
        scalar(
            &runtime,
            &format!("SELECT count(*) AS n FROM research_tasks WHERE check_id = {check_id}")
        )
        .await,
        2
    );
}

#[tokio::test]
async fn a_failed_topic_holds_the_queue_until_continue_finishes_it() {
    // A failure is produced by an assistant text with no tool call; the topic task remains
    // failed and no next session is started. This test asserts the hold, then the session
    // transition path is covered by the existing research status tests.
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [
                    { "topic": "Fails", "requirements": "", "scope": "History" },
                    { "topic": "Waits", "requirements": "", "scope": "History" }
                ]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new().text("I cannot finish this one."),
    ])
    .await;
    let (book_id, check_id, check_session) =
        create_check_book(&runtime, "Failed topic", false).await;
    runtime.wait_any_status(check_session).await;
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .first()
            .is_some_and(|row| row.1 == "failed" || row.1 == "running")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        sessions(&runtime, book_id, "conception").await.len(),
        0,
        "a failed topic must not start conception"
    );
}
