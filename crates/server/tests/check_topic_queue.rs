//! Check-topic queue tests (M20 20.4, spec/07 §8.1 step 6, spec/08 §5, D-168).
//!
//! A knowledge check's topics run one at a time, in id order, before conception. These tests
//! drive the real router and the scripted provider, plus a few direct row fixtures for states the
//! API cannot create (a failed model role, an abandoned check).

#[path = "agent_harness/mod.rs"]
mod agent_harness;

use std::time::Duration;

use agent_harness::{Runtime, refusal_step, runtime};
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

async fn topic_session_order(runtime: &Runtime, check_id: i32) -> Vec<(i32, i32)> {
    runtime
        .app
        .db()
        .conn()
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT t.id AS topic_id, s.id AS session_id FROM research_tasks t              JOIN agent_sessions s ON s.research_task_id = t.id              WHERE t.check_id = ? AND t.kind = 'topic' ORDER BY t.id",
            vec![check_id.into()],
        ))
        .await
        .expect("topic sessions")
        .into_iter()
        .map(|row| {
            (
                row.try_get("", "topic_id").expect("topic id"),
                row.try_get("", "session_id").expect("session id"),
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

async fn last_tool_ui(runtime: &Runtime, session_id: i32, name: &str) -> Value {
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
    let order = topic_session_order(&runtime, check_id).await;
    assert_eq!(order.len(), 3, "{order:?}");
    assert!(
        order.windows(2).all(|pair| pair[0].1 < pair[1].1),
        "sessions start in topic-id order: {order:?}"
    );

    wait_until(|| async { sessions(&runtime, book_id, "conception").await.len() == 1 }).await;
    let conception = sessions(&runtime, book_id, "conception").await;
    assert_eq!(conception.len(), 1);
    let text = kickoff(&runtime, conception[0].0).await;
    for topic in ["Topic one", "Topic two", "Topic three"] {
        assert!(
            text.contains(&format!("{topic}（History，done，0 条）")),
            "{topic} is not rendered as done with its entry count: {text}"
        );
    }
    assert_eq!(
        scalar(
            &runtime,
            &format!(
                "SELECT CAST(json_extract(conception, '$.start_after_knowledge_check') AS INTEGER) AS n                  FROM books WHERE id = {book_id}"
            ),
        )
        .await,
        0,
        "the auto-start flag is cleared in the same write"
    );
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
async fn continuing_a_failed_topic_starts_the_next_one() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [
                    { "topic": "Fails then continues", "requirements": "", "scope": "History" },
                    { "topic": "Next after continue", "requirements": "", "scope": "History" }
                ]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        refusal_step("the model refused this topic"),
        ScriptStep::new().tool("finish_research", json!({ "report": "continued" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "next done" })),
    ])
    .await;
    let (_book_id, check_id, check_session) =
        create_check_book(&runtime, "Continue topic", false).await;
    runtime.wait_any_status(check_session).await;
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "Fails then continues" && row.1 == "failed")
    })
    .await;
    let failed = topic_rows(&runtime, check_id)
        .await
        .into_iter()
        .find(|row| row.2 == "Fails then continues")
        .expect("failed topic")
        .0;
    let (status, body) = runtime
        .app
        .post(&format!("/api/research-tasks/{failed}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "Next after continue" && row.1 == "done")
    })
    .await;
}

#[tokio::test]
async fn an_author_topic_added_while_a_topic_runs_runs_after_it() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [
                    { "topic": "Running topic", "requirements": "", "scope": "History" },
                    { "topic": "Already queued", "requirements": "", "scope": "History" }
                ]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new()
            .delay(Duration::from_secs(1))
            .tool("finish_research", json!({ "report": "running done" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "queued done" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "added done" })),
    ])
    .await;
    let (book_id, check_id, check_session) =
        create_check_book(&runtime, "Add while running", false).await;
    runtime.wait_any_status(check_session).await;
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "Running topic" && row.1 == "running")
    })
    .await;
    let (status, body) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "check_id": check_id,
                "scope_name": "History",
                "topic": "Added while running",
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let rows = topic_rows(&runtime, check_id).await;
    assert_eq!(
        rows.iter().filter(|row| row.1 == "running").count(),
        1,
        "the earlier topic keeps the only running slot: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.2 == "Added while running" && row.1 == "ready"),
        "the added topic waits for the earlier ones: {rows:?}"
    );
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "Added while running" && row.1 == "done")
    })
    .await;
}

#[tokio::test]
async fn an_author_topic_starts_an_idle_queue() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "idle topic done" })),
    ])
    .await;
    let (book_id, check_id, check_session) = create_check_book(&runtime, "Idle queue", false).await;
    runtime.wait_any_status(check_session).await;
    let (status, body) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "check_id": check_id,
                "scope_name": "History",
                "topic": "Starts idle queue",
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "Starts idle queue" && row.1 == "done")
    })
    .await;
}

#[tokio::test]
async fn an_author_topic_runs_while_conception_runs() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("finish_research", json!({ "report": "sufficient" })),
        ScriptStep::new().text("conception"),
        ScriptStep::new().tool("finish_research", json!({ "report": "added done" })),
    ])
    .await;
    let (book_id, check_id, check_session) =
        create_check_book(&runtime, "Add while conception", true).await;
    runtime.wait_any_status(check_session).await;
    wait_until(|| async { sessions(&runtime, book_id, "conception").await.len() == 1 }).await;
    // The scripted provider is shared with the conception runner. Wait until it has taken its
    // own step so the topic's runner cannot consume the conception text step first.
    wait_until(|| async { runtime.scripted.requests().len() >= 2 }).await;
    let (status, body) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "check_id": check_id,
                "scope_name": "History",
                "topic": "Added during conception",
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "Added during conception" && row.1 == "done")
    })
    .await;
    assert_eq!(
        sessions(&runtime, book_id, "conception").await.len(),
        1,
        "the author's existing conception session is not duplicated"
    );
}

#[tokio::test]
async fn author_started_conception_does_not_stop_the_queue_or_duplicate() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [
                    { "topic": "First before manual conception", "requirements": "", "scope": "History" },
                    { "topic": "Second before manual conception", "requirements": "", "scope": "History" }
                ]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new()
            .delay(Duration::from_secs(2))
            .tool("finish_research", json!({ "report": "first done" })),
        ScriptStep::new().text("manual conception"),
        ScriptStep::new().tool("finish_research", json!({ "report": "second done" })),
    ])
    .await;
    let (book_id, check_id, check_session) =
        create_check_book(&runtime, "Manual conception queue", true).await;
    runtime.wait_any_status(check_session).await;
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "First before manual conception" && row.1 == "running")
    })
    .await;
    let (status, body) = runtime
        .app
        .post(&format!("/api/books/{book_id}/conception/start"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Consume the conception runner's own scripted step before the queue's next topic asks for
    // the shared provider step.
    wait_until(|| async { runtime.scripted.requests().len() >= 4 }).await;
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .filter(|row| row.1 == "done")
            .count()
            >= 2
    })
    .await;
    assert_eq!(
        sessions(&runtime, book_id, "conception").await.len(),
        1,
        "settling after an author-started conception starts no second session"
    );
}

#[tokio::test]
async fn auto_start_off_then_manual_conception_start_works() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [{ "topic": "Only topic", "requirements": "", "scope": "History" }]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "topic done" })),
        ScriptStep::new().text("conception"),
    ])
    .await;
    let (book_id, check_id, check_session) = create_check_book(&runtime, "Auto off", false).await;
    runtime.wait_any_status(check_session).await;
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.1 == "done")
    })
    .await;
    assert_eq!(
        sessions(&runtime, book_id, "conception").await.len(),
        0,
        "auto-start is off after the research settles"
    );
    let (status, body) = runtime
        .app
        .post(&format!("/api/books/{book_id}/conception/start"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sessions(&runtime, book_id, "conception").await.len(), 1);
}

#[tokio::test]
async fn a_check_topic_added_while_planning_is_kept_by_the_check() {
    let runtime = runtime(vec![
        ScriptStep::new().delay(Duration::from_secs(1)).tool(
            "set_research_topics",
            json!({
                "topics": [{ "topic": "Agent planned", "requirements": "", "scope": "History" }]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "agent done" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "author done" })),
    ])
    .await;
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
    runtime.wait_any_status(check_session).await;
    let kept = last_tool_ui(&runtime, check_session, "set_research_topics").await["kept"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        kept.iter()
            .any(|row| row["topic"] == "Author while planning"),
        "the check's kept[] lists the planning-time author topic: {kept:?}"
    );
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "Author while planning" && row.1 == "done")
    })
    .await;
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
    // A real topic task's id, not a preparation task.
    let (status, topic) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "scope_name": "History",
                "topic": "Ordinary topic",
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{topic}");
    let (status, body) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "check_id": topic["id"],
                "scope_name": "History",
                "topic": "Bad",
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    // Another book's preparation task is not a valid check id for this book.
    let other = runtime.app.create_book("Other check book").await;
    let other_id = other["id"].as_i64().unwrap() as i32;
    let (status, other_check) = runtime
        .app
        .post(&format!("/api/books/{other_id}/knowledge-check"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{other_check}");
    let (status, body) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "check_id": other_check["task"]["id"],
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
async fn two_quick_topic_creates_start_exactly_one_topic_session() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new()
            .delay(Duration::from_secs(2))
            .tool("finish_research", json!({ "report": "first done" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "second done" })),
    ])
    .await;
    let (book_id, check_id, check_session) =
        create_check_book(&runtime, "Two triggers", false).await;
    runtime.wait_any_status(check_session).await;
    let first = runtime.app.post(
        "/api/research-tasks",
        json!({
            "book_id": book_id,
            "check_id": check_id,
            "scope_name": "History",
            "topic": "Trigger one",
            "language": "en"
        }),
    );
    let second = runtime.app.post(
        "/api/research-tasks",
        json!({
            "book_id": book_id,
            "check_id": check_id,
            "scope_name": "History",
            "topic": "Trigger two",
            "language": "en"
        }),
    );
    let ((first_status, first_body), (second_status, second_body)) = tokio::join!(first, second);
    assert_eq!(first_status, StatusCode::CREATED, "{first_body}");
    assert_eq!(second_status, StatusCode::CREATED, "{second_body}");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        sessions(&runtime, book_id, "research").await.len(),
        2,
        "the check plus exactly one topic session, not two topic sessions"
    );
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .filter(|row| row.1 == "done")
            .count()
            >= 2
    })
    .await;
}

#[tokio::test]
async fn rerunning_a_check_waits_for_a_running_topic_then_uses_its_new_list() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [
                    { "topic": "Running before rerun", "requirements": "", "scope": "History" },
                    { "topic": "Replaced by rerun", "requirements": "", "scope": "History" }
                ]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new()
            .delay(Duration::from_secs(2))
            .tool("finish_research", json!({ "report": "first done" })),
        ScriptStep::new().delay(Duration::from_secs(4)).tool(
            "set_research_topics",
            json!({
                "topics": [{ "topic": "Rerun replacement", "requirements": "", "scope": "History" }]
            }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "check rerun done" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "replacement done" })),
    ])
    .await;
    let (book_id, check_id, check_session) =
        create_check_book(&runtime, "Rerun check", false).await;
    runtime.wait_any_status(check_session).await;
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "Running before rerun" && row.1 == "running")
    })
    .await;
    let (status, body) = runtime
        .app
        .post(&format!("/api/research-tasks/{check_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "Running before rerun" && row.1 == "done")
    })
    .await;
    // The re-run is deliberately slower than the topic. Its `set_research_topics` has not run
    // yet, so the old waiting topic must still be ready and no new topic may have started.
    let waiting = topic_rows(&runtime, check_id).await;
    assert_eq!(
        waiting.iter().filter(|row| row.1 == "running").count(),
        0,
        "the queue waits while the re-run check is not done: {waiting:?}"
    );
    assert!(
        waiting
            .iter()
            .any(|row| row.2 == "Replaced by rerun" && row.1 == "ready"),
        "the old list is untouched until the re-run replaces it: {waiting:?}"
    );
    let check_status = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("SELECT status FROM research_tasks WHERE id = {check_id}"),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "status")
        .unwrap();
    assert_eq!(check_status, "running", "the re-run is still planning");

    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .any(|row| row.2 == "Rerun replacement" && row.1 == "done")
    })
    .await;
    let after = topic_rows(&runtime, check_id).await;
    assert!(
        !after.iter().any(|row| row.2 == "Replaced by rerun"),
        "the re-run's set_research_topics replaces the old waiting list: {after:?}"
    );
    assert_eq!(
        sessions(&runtime, book_id, "conception").await.len(),
        0,
        "auto-start is off"
    );
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
    assert_eq!(
        topic_rows(&runtime, check_id)
            .await
            .into_iter()
            .find(|row| row.0 == running_topic)
            .expect("abandoned topic")
            .1,
        "cancelled"
    );
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
        ScriptStep::new().tool("finish_research", json!({ "report": "first done" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "second done" })),
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
    let (status, second) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "check_id": check_id,
                "scope_name": "History",
                "topic": "Second waiting",
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");

    // Restore the role and run the first topic manually; its completion starts the second.
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
            .filter(|row| row.1 == "done")
            .count()
            >= 2
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
    assert_eq!(
        sessions(&runtime, book_id, "conception").await.len(),
        0,
        "an active book's check does not create a conception session"
    );
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("SELECT status FROM books WHERE id = {book_id}"),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "status").unwrap(), "active");
}

#[tokio::test]
async fn an_abandoned_check_does_not_run_its_topics() {
    let runtime = runtime(vec![ScriptStep::new().delay(Duration::from_secs(20))]).await;
    let (book_id, check_id, check_session) =
        create_check_book(&runtime, "Abandoned check", true).await;
    let (status, body) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "check_id": check_id,
                "scope_name": "History",
                "topic": "Survives abandon",
                "language": "en"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{check_session}/abandon"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(sessions(&runtime, book_id, "conception").await.len(), 0);
    assert!(
        sessions(&runtime, book_id, "research")
            .await
            .iter()
            .all(|(id, _)| *id == check_session),
        "no topic session starts after the check is abandoned: {:?}",
        sessions(&runtime, book_id, "research").await
    );
    let rows = topic_rows(&runtime, check_id).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].1, "ready", "{rows:?}");
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
async fn existing_scope_descriptions_survive_and_excluded_scopes_are_reincluded() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "set_research_topics",
            json!({
                "topics": [
                    { "topic": "Existing desc topic", "requirements": "", "scope": "Existing desc" },
                    { "topic": "Excluded desc topic", "requirements": "", "scope": "Excluded desc" }
                ]
            }),
        ),
        ScriptStep::new()
            .delay(Duration::from_secs(2))
            .tool("finish_research", json!({ "report": "planned" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "existing done" })),
        ScriptStep::new().tool("finish_research", json!({ "report": "excluded done" })),
    ])
    .await;
    for (name, description) in [
        ("Existing desc", "Keep this description"),
        ("Excluded desc", "Excluded description"),
    ] {
        let (status, body) = runtime
            .app
            .post(
                "/api/knowledge/scopes",
                json!({ "name": name, "description": description }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    let (book_id, check_id, check_session) =
        create_check_book(&runtime, "Existing scopes", true).await;
    wait_until(|| async { topic_rows(&runtime, check_id).await.len() == 2 }).await;
    let scope_id = scalar(
        &runtime,
        "SELECT id AS n FROM knowledge_scopes WHERE name = 'Excluded desc'",
    )
    .await;
    let (status, _) = runtime
        .app
        .put(
            &format!("/api/books/{book_id}/scopes"),
            json!({ "excluded_scope_ids": [scope_id] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    runtime.wait_any_status(check_session).await;
    wait_until(|| async {
        topic_rows(&runtime, check_id)
            .await
            .iter()
            .filter(|row| row.1 == "done")
            .count()
            >= 2
    })
    .await;
    assert_eq!(
        scalar(
            &runtime,
            "SELECT count(*) AS n FROM knowledge_scopes WHERE name = 'Existing desc'              AND description = 'Keep this description'"
        )
        .await,
        1,
        "an existing description is never rewritten"
    );
    assert_eq!(
        scalar(
            &runtime,
            &format!(
                "SELECT count(*) AS n FROM book_scope_exclusions WHERE book_id = {book_id}                  AND scope_id = {scope_id}"
            ),
        )
        .await,
        0,
        "the run re-includes the scope for the book"
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
