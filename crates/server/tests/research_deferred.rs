//! The deferred research status (spec/07 §8, spec/10 §2.5, D-138).
//!
//! A research task is the overview's required knowledge step (D-136), so a book with any `ready`
//! task is never prompted to write its next chapter. `deferred` is the release valve that says
//! "I know about this, not now": it keeps the task and its intent while dropping it out of the
//! runnable set. These tests pin the whole transition matrix in one table, so the rules are
//! legible: every allowed edge succeeds, every refused one is the documented
//! `409 research.invalid_state`.
//!
//! The task rows are forced into their source status with SQL where the API has no path to that
//! state (a `done` task needs a run, a `running` one needs a session); this is the same fixture
//! style the overview and M11 race tests use, and the guard under test reads the persisted row,
//! not the history that produced it.

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use nekowriter::events::{EventEnvelope, ServerEvent};
use nekowriter::testing::{TestApp, assert_error};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::TryRecvError;

// ───────────────────────────── fixtures ─────────────────────────────

/// An app with a Demo model assigned to the researcher role, so `run` can start a real session
/// without a network (spec/04 §7.2).
async fn demo_app() -> TestApp {
    let app = TestApp::new().await;
    let (status, provider) = app
        .post(
            "/api/providers",
            json!({
                "name": "Demo",
                "protocol": "mock",
                "base_url": "mock://demo",
                "extra_headers": {},
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create provider: {provider}");
    let provider_id = provider["id"].as_i64().expect("provider id") as i32;
    let (status, profile) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Demo model",
                "model": "demo-model",
                "context_window": 200_000,
                "max_output_tokens": 8_192,
                "effort_levels": [],
                "default_effort": null,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create profile: {profile}");
    let profile_id = profile["id"].as_i64().expect("profile id") as i32;
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": { "researcher": profile_id } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "assign researcher: {body}");
    app
}

/// Creates a task through the API and returns its id. Without a book the API needs a language and
/// the task starts `ready` (spec/10 §2.5).
async fn create_task(app: &TestApp) -> i32 {
    let (status, task) = app
        .post(
            "/api/research-tasks",
            json!({ "scope_name": "明初历史", "topic": "宝钞与财政", "language": "zh-CN" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create task: {task}");
    assert_eq!(task["status"], "ready", "{task}");
    task["id"].as_i64().expect("task id") as i32
}

/// Forces the task into a status the API has no path to. `running` and `failed` also get the open
/// session that state means (spec/07 §8 step 1), because `is_open` reads both columns.
async fn force_status(app: &TestApp, task_id: i32, status: &str) {
    if matches!(status, "running" | "failed") {
        app.execute(&format!(
            "INSERT INTO agent_sessions(book_id, role, kind, research_task_id, model_snapshot, \
             system_prompt, tools, status, created_at, updated_at) VALUES (NULL, 'researcher', \
             'research', {task_id}, '{{\"provider_id\":0,\"protocol\":\"mock\",\"base_url\":\"\",\"extra_headers\":{{}},\"model\":\"\",\"context_window\":0,\"max_output_tokens\":0,\"options\":null}}', '', '[]', '{status}', \
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ))
        .await;
        // One statement, so no id has to be read back between two writes.
        app.execute(&format!(
            "UPDATE research_tasks SET status = '{status}', \
             session_id = (SELECT max(id) FROM agent_sessions WHERE research_task_id = {task_id}) \
             WHERE id = {task_id}"
        ))
        .await;
    } else {
        app.execute(&format!(
            "UPDATE research_tasks SET status = '{status}' WHERE id = {task_id}"
        ))
        .await;
    }
}

/// The task's current DTO (spec/10 §2.5).
async fn task_dto(app: &TestApp, task_id: i32) -> Value {
    let (status, task) = app.get(&format!("/api/research-tasks/{task_id}")).await;
    assert_eq!(status, StatusCode::OK, "get task: {task}");
    task
}

/// Reads the events published while an API call ran. The publish happens after the commit and
/// before the response, so draining until the channel is empty is exact rather than timed.
async fn drain(receiver: &mut tokio::sync::broadcast::Receiver<EventEnvelope>) -> Vec<ServerEvent> {
    let mut events = Vec::new();
    tokio::task::yield_now().await;
    loop {
        match receiver.try_recv() {
            Ok(envelope) => events.push(envelope.event),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Lagged(_)) => continue,
            Err(TryRecvError::Closed) => break,
        }
    }
    events
}

// ───────────────────────────── the state matrix ─────────────────────────────

#[derive(Clone, Copy, Debug)]
enum Transition {
    Defer,
    Resume,
}

impl Transition {
    /// The endpoint path segment (spec/10 §2.5).
    fn path(self) -> &'static str {
        match self {
            Transition::Defer => "defer",
            Transition::Resume => "resume",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Expected {
    /// `200`, and the task ends in this status (the target, or the source on a repeat call).
    MovesTo(&'static str),
    /// `409 research.invalid_state` naming the task's current status.
    Refused,
    /// `409 research.invalid_state` naming the open session (the guard `run`/PATCH/DELETE share).
    OpenSession,
}

/// Every edge of the deferred state machine, in one table. The source statuses that carry an open
/// session are exactly `running` and `failed` (spec/07 §8 step 5); `ok` means the transition is
/// allowed and names the resulting status.
const MATRIX: &[(Transition, &str, Expected)] = &[
    // The author's two actions: "not now" and "actually, let me do this".
    (Transition::Defer, "ready", Expected::MovesTo("deferred")),
    (Transition::Resume, "deferred", Expected::MovesTo("ready")),
    // Repeating an action is harmless: each endpoint names a target state, so a double-click
    // returns the current row instead of an error (D-138).
    (Transition::Defer, "deferred", Expected::MovesTo("deferred")),
    (Transition::Resume, "ready", Expected::MovesTo("ready")),
    // A draft is not a confirmed task: the author confirms it to `ready` (PATCH, spec/10 §2.5) or
    // deletes it; deferring an unconfirmed topic has no meaning, and a draft never blocks writing.
    (Transition::Defer, "draft", Expected::Refused),
    (Transition::Resume, "draft", Expected::Refused),
    // Open sessions: the task has a session the author is already watching, so the state change
    // waits until that session ends (the same conflict PATCH and DELETE report).
    (Transition::Defer, "running", Expected::OpenSession),
    (Transition::Defer, "failed", Expected::OpenSession),
    (Transition::Resume, "running", Expected::OpenSession),
    (Transition::Resume, "failed", Expected::OpenSession),
    // Finished or abandoned work stays in the record: deferring it would claim a completed task is
    // still pending, or resurrect an intent the author dropped.
    (Transition::Defer, "done", Expected::Refused),
    (Transition::Defer, "cancelled", Expected::Refused),
    (Transition::Resume, "done", Expected::Refused),
    (Transition::Resume, "cancelled", Expected::Refused),
];

#[tokio::test]
async fn the_defer_and_resume_state_matrix() {
    let app = TestApp::new().await;
    for &(transition, from, expected) in MATRIX {
        // Each row gets its own task: the rows are independent state machines, not a sequence.
        let task_id = create_task(&app).await;
        let from = if from == "ready" {
            // The created task is already `ready`; forcing it would be a no-op write.
            "ready"
        } else {
            force_status(&app, task_id, from).await;
            from
        };
        let (status, body) = app
            .post(
                &format!("/api/research-tasks/{task_id}/{}", transition.path()),
                json!({}),
            )
            .await;
        let label = format!("{} of a {from} task", transition.path());

        match expected {
            Expected::MovesTo(target) => {
                assert_eq!(status, StatusCode::OK, "{label}: {body}");
                assert_eq!(body["status"], target, "{label} lands on {target}: {body}");
                // The response is the persisted row, not a computed one.
                let stored = task_dto(&app, task_id).await;
                assert_eq!(
                    stored["status"], target,
                    "{label} persists {target}: {stored}"
                );
            }
            Expected::Refused => {
                assert_eq!(status, StatusCode::CONFLICT, "{label}: {body}");
                assert_error(&body, "research.invalid_state");
                assert_eq!(
                    body["error"]["details"]["status"], from,
                    "the refusal names the current status: {body}"
                );
                // Nothing moved.
                let stored = task_dto(&app, task_id).await;
                assert_eq!(stored["status"], from, "{label} changes nothing: {stored}");
            }
            Expected::OpenSession => {
                assert_eq!(status, StatusCode::CONFLICT, "{label}: {body}");
                assert_error(&body, "research.invalid_state");
                let session_id = body["error"]["details"]["session_id"]
                    .as_i64()
                    .unwrap_or_else(|| panic!("{label} names the open session: {body}"));
                let stored = task_dto(&app, task_id).await;
                assert_eq!(stored["session_id"], session_id, "{label}: {stored}");
                assert_eq!(stored["status"], from, "{label} changes nothing: {stored}");
            }
        }
    }
}

#[tokio::test]
async fn a_wedged_running_task_without_a_session_is_still_refused() {
    // `running` normally carries a session, but the guard must not depend on that: without one the
    // refusal names the status instead of a session id.
    let app = TestApp::new().await;
    let task_id = create_task(&app).await;
    app.execute(&format!(
        "UPDATE research_tasks SET status = 'running' WHERE id = {task_id}"
    ))
    .await;
    let (status, body) = app
        .post(&format!("/api/research-tasks/{task_id}/defer"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "research.invalid_state");
    assert_eq!(body["error"]["details"]["status"], "running", "{body}");
}

// ───────────────────────────── the effect of a defer ─────────────────────────────

#[tokio::test]
async fn deferring_twice_changes_nothing_and_publishes_once() {
    let app = TestApp::new().await;
    let task_id = create_task(&app).await;
    let mut receiver = app.events().subscribe();

    let (status, first) = app
        .post(&format!("/api/research-tasks/{task_id}/defer"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["status"], "deferred");
    let events = drain(&mut receiver).await;
    let published = events
        .iter()
        .filter_map(|event| match event {
            ServerEvent::ResearchUpdated { task } if task.id == task_id => Some(task.status),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        published,
        vec![nekowriter::db::json::ResearchTaskStatus::Deferred],
        "the new status reaches the open pages (spec/10 §3)"
    );

    // The repeat call is a no-op: same row, same timestamp, no second event. A double-click is
    // harmless, which is the whole point of the target-state contract (D-138).
    let (status, second) = app
        .post(&format!("/api/research-tasks/{task_id}/defer"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["status"], "deferred");
    assert_eq!(
        second["updated_at"], first["updated_at"],
        "an idempotent repeat does not write: {second}"
    );
    assert!(
        drain(&mut receiver).await.is_empty(),
        "nothing changed, so nothing is announced"
    );
}

#[tokio::test]
async fn resume_returns_the_task_to_the_runnable_set() {
    let app = TestApp::new().await;
    let task_id = create_task(&app).await;
    app.post(&format!("/api/research-tasks/{task_id}/defer"), json!({}))
        .await;

    let (status, body) = app
        .post(&format!("/api/research-tasks/{task_id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ready", "{body}");

    // The list filter is typed by the same enum, so a resumed task is listed as `ready` again.
    let (_, ready) = app.get("/api/research-tasks?status=ready").await;
    assert!(
        ready
            .as_array()
            .expect("list")
            .iter()
            .any(|task| task["id"] == task_id),
        "the resumed task is runnable again: {ready}"
    );
}

#[tokio::test]
async fn the_list_can_filter_deferred_tasks() {
    let app = TestApp::new().await;
    let deferred = create_task(&app).await;
    create_task(&app).await;
    app.post(&format!("/api/research-tasks/{deferred}/defer"), json!({}))
        .await;

    let (status, list) = app.get("/api/research-tasks?status=deferred").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let list = list.as_array().expect("list");
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0]["id"], deferred);
    assert_eq!(list[0]["status"], "deferred");
}

#[tokio::test]
async fn a_deferred_task_can_be_run_without_resuming_it_first() {
    // The ruling (D-138): `run` accepts a deferred task, because forcing an un-defer step adds
    // friction without safety. The task goes straight to `running` with its session.
    let app = demo_app().await;
    let task_id = create_task(&app).await;
    let (status, _) = app
        .post(&format!("/api/research-tasks/{task_id}/defer"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().expect("session id") as i32;

    // The task left `deferred` for the session it is now running (spec/07 §8 step 5).
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let task = task_dto(&app, task_id).await;
        assert_ne!(
            task["status"], "deferred",
            "the run moved the task out of the deferred state: {task}"
        );
        assert_eq!(task["session_id"], session_id, "{task}");
        if task["status"] != "running" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the demo researcher did not finish: {task}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let task = task_dto(&app, task_id).await;
    assert_eq!(task["status"], "done", "{task}");
    assert_eq!(task["entries_created"], 2, "{task}");
}

#[tokio::test]
async fn deferring_a_missing_task_is_not_found() {
    let app = TestApp::new().await;
    for path in ["defer", "resume"] {
        let (status, body) = app
            .post(&format!("/api/research-tasks/999/{path}"), json!({}))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        assert_error(&body, "not_found");
    }
}
