//! PLAN M11 hardening (task-24): one open researcher session per research task.
//!
//! `research::run` resolves the task's scope and checks "no open session" in a transaction of its
//! own, then creates the session in a second transaction. Two concurrent `POST /run` calls could
//! therefore both pass that check and create two researcher sessions for one task, the second
//! overwriting `research_tasks.session_id` and orphaning the first. The check now also runs
//! inside session creation, in the creation transaction under the process-wide write lock
//! (D-109); the deterministic test below drives that path directly, and the concurrent API test
//! pins the observable HTTP behavior.

use std::time::Duration;

use axum::http::StatusCode;
use nekowriter::db::json::SessionKind;
use nekowriter::error::ErrorCode;
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter::testing::assert_error;
use nekowriter_llm::mock::ScriptStep;
use serde_json::json;

#[path = "agent_harness/mod.rs"]
mod agent_harness;

use agent_harness::runtime;

/// Creates research task on the harness book and returns its id.
async fn create_task(runtime: &agent_harness::Runtime) -> i32 {
    let (status, task) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": runtime.book_id,
                "scope_name": "背景设定",
                "topic": "边城城防",
                "requirements": "城门与轮值",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create task: {task}");
    task["id"].as_i64().expect("task id") as i32
}

/// The number of researcher sessions that belong to a task, read through the API.
async fn sessions_of(runtime: &agent_harness::Runtime, task_id: i32) -> usize {
    let (status, list) = runtime
        .app
        .get(&format!(
            "/api/books/{}/sessions?kind=research",
            runtime.book_id
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "list sessions: {list}");
    list.as_array()
        .expect("a session list")
        .iter()
        .filter(|session| session["research_task_id"] == json!(task_id))
        .count()
}

/// A second researcher session for a task that already has one is refused by session creation
/// itself — the only place the write lock can make the check and the insert atomic.
#[tokio::test]
async fn a_second_researcher_session_is_refused_by_creation() {
    // The delayed script keeps the first researcher running, so the task stays "open".
    let runtime = runtime(vec![ScriptStep::new().delay(Duration::from_secs(30))]).await;
    let task_id = create_task(&runtime).await;
    let session_id = nekowriter::services::research::run(runtime.app.state(), task_id)
        .await
        .expect("the first run starts a researcher session");

    let (_, task) = runtime
        .app
        .get(&format!("/api/research-tasks/{task_id}"))
        .await;
    let scope_id = task["scope_id"].as_i64().expect("the run attaches a scope") as i32;

    let result = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::Research,
            book_id: Some(runtime.book_id),
            chapter_id: None,
            research_task_id: Some(task_id),
            granularity: None,
            auto_pause: None,
            pause_after_outline: None,
            redraft: false,
            guidance: None,
            kickoff_extra: Some(json!({ "scope_id": scope_id })),
            pending_comment: None,
            chapter_guard: false,
            force_previous: false,
        },
    )
    .await;
    let error = result.expect_err("a second researcher session for one task must be refused");
    assert_eq!(error.code, ErrorCode::ResearchInvalidState);
    assert_eq!(
        error
            .details
            .as_ref()
            .and_then(|details| details["session_id"].as_i64()),
        Some(session_id as i64),
        "the 409 names the open session: {:?}",
        error.details
    );

    // The first session is still the task's only one.
    assert_eq!(sessions_of(&runtime, task_id).await, 1);
}

/// Two concurrent `POST /run` calls: exactly one starts a session, the other reports the
/// documented 409 `research.invalid_state` with its `session_id`.
#[tokio::test]
async fn concurrent_runs_create_exactly_one_researcher_session() {
    let runtime = runtime(vec![ScriptStep::new().delay(Duration::from_secs(30))]).await;
    let task_id = create_task(&runtime).await;
    let path = format!("/api/research-tasks/{task_id}/run");

    let (first, second) = tokio::join!(
        runtime.app.post(&path, json!({})),
        runtime.app.post(&path, json!({}))
    );
    let mut statuses = [first.0, second.0];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::CREATED, StatusCode::CONFLICT],
        "one run wins, one loses: first={:?} second={:?}",
        first.1,
        second.1
    );
    let conflict = if first.0 == StatusCode::CONFLICT {
        &first.1
    } else {
        &second.1
    };
    assert_error(conflict, "research.invalid_state");
    assert!(
        conflict["error"]["details"]["session_id"].is_i64(),
        "the 409 names the open session: {conflict}"
    );
    assert_eq!(sessions_of(&runtime, task_id).await, 1);
}
