//! Harness for the writer-session tests (PLAN M4.2–M4.5, spec/03 §15).
//!
//! It reuses `agent_harness` (the temp database, the real router, the `ScriptedProvider`) and
//! adds what only a real `chapter_write` session needs: a session created through the production
//! path, so the frozen tool list, the kickoff and the phase machine are the shipped ones rather
//! than a debug stand-in.

// The harness is shared by several test binaries, and each uses a different subset of it.
#![allow(dead_code)]

#[path = "../agent_harness/mod.rs"]
pub mod base;

pub use base::*;

use nekowriter::db::json::{Granularity, Phase};
use nekowriter::services::sessions::{self, CreateSession};
use serde_json::json;

/// Starts a real `chapter_write` session (planning) for the harness chapter.
pub async fn start_writer(
    runtime: &Runtime,
    granularity: Granularity,
    auto_pause: bool,
    pause_after_outline: bool,
) -> i32 {
    start_writer_on(
        runtime,
        runtime.chapter_id,
        granularity,
        auto_pause,
        pause_after_outline,
        false,
    )
    .await
}

/// Starts a real `chapter_write` session for a chosen chapter.
pub async fn start_writer_on(
    runtime: &Runtime,
    chapter_id: i32,
    granularity: Granularity,
    auto_pause: bool,
    pause_after_outline: bool,
    redraft: bool,
) -> i32 {
    let (session_id, _warnings) = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: nekowriter::db::json::SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(chapter_id),
            research_task_id: None,
            granularity: Some(granularity),
            auto_pause: Some(auto_pause),
            pause_after_outline: Some(pause_after_outline),
            redraft,
            kickoff_extra: None,
        },
    )
    .await
    .expect("create writer session");
    session_id
}

/// A step that submits an outline, so the next step runs in `drafting`.
pub fn outline_step() -> nekowriter_llm::mock::ScriptStep {
    nekowriter_llm::mock::ScriptStep::new().tool(
        "submit_outline",
        json!({ "outline": "场景一：城门口", "reference_notes": "- 雨一直下" }),
    )
}

/// Runs a read-only query through the harness database and reads one typed column.
async fn scalar<T>(
    runtime: &Runtime,
    sql: &str,
    values: Vec<sea_orm::Value>,
    column: &str,
) -> Option<T>
where
    T: sea_orm::TryGetable,
{
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            sql,
            values,
        ))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<T>("", column).ok()
}

/// The chapter's stored content, decoded.
pub async fn chapter_content(
    runtime: &Runtime,
    chapter_id: i32,
) -> nekowriter::db::json::ChapterContent {
    let raw: String = scalar(
        runtime,
        "SELECT content FROM chapters WHERE id = ?",
        vec![chapter_id.into()],
        "content",
    )
    .await
    .unwrap_or_else(|| "{}".to_owned());
    nekowriter::db::json::from_json_or_default(&raw)
}

/// The chapter's `(status, word_count, content_version)`.
pub async fn chapter_row(runtime: &Runtime, chapter_id: i32) -> (String, i32, i32) {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT status, word_count, content_version FROM chapters WHERE id = ?",
            vec![chapter_id.into()],
        ))
        .await
        .expect("query")
        .expect("row");
    (
        row.try_get::<String>("", "status").expect("status"),
        row.try_get::<i32>("", "word_count").expect("word_count"),
        row.try_get::<i32>("", "content_version")
            .expect("content_version"),
    )
}

/// One column of the chapter row (`title`, `outline`, `reference_notes`, …).
pub async fn chapter_column(runtime: &Runtime, chapter_id: i32, column: &str) -> String {
    let sql = format!("SELECT {column} FROM chapters WHERE id = ?");
    scalar(runtime, &sql, vec![chapter_id.into()], column)
        .await
        .unwrap_or_default()
}

/// The `ui` payload of the first call of a session with that name.
pub async fn call_ui(runtime: &Runtime, session_id: i32, name: &str) -> serde_json::Value {
    let raw: Option<String> = scalar(
        runtime,
        "SELECT ui FROM agent_tool_calls WHERE session_id = ? AND name = ? ORDER BY id LIMIT 1",
        vec![session_id.into(), name.to_owned().into()],
        "ui",
    )
    .await;
    raw.and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// The phase a writer session is in.
pub async fn phase_of(runtime: &Runtime, session_id: i32) -> Phase {
    let row = runtime.session_row(session_id).await;
    Phase::from_str_opt(&row.phase).expect("phase")
}

/// Resumes every hold the session makes until it reaches a terminal status, collecting the held
/// calls in order. With `auto_pause` on, every write tool holds, so a test that wants the whole
/// script executed must approve each one (spec/03 §15 test 3).
pub async fn resume_until_done(
    runtime: &Runtime,
    session_id: i32,
    feedback: Option<&str>,
) -> Vec<i32> {
    let mut held = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let row = runtime.session_row(session_id).await;
        match row.status.as_str() {
            "held" => {
                let call_id = row.held_tool_call_id.expect("a held call id");
                held.push(call_id);
                let body = match feedback {
                    Some(text) => serde_json::json!({ "feedback": text }),
                    None => serde_json::json!({}),
                };
                let (status, response) = runtime
                    .app
                    .post(&format!("/api/sessions/{session_id}/resume"), body)
                    .await;
                assert_eq!(status, 200, "resume {call_id}: {response}");
            }
            "paused" => {
                let (status, response) = runtime
                    .app
                    .post(
                        &format!("/api/sessions/{session_id}/resume"),
                        serde_json::json!({}),
                    )
                    .await;
                assert_eq!(status, 200, "resume paused: {response}");
            }
            "completed" | "abandoned" | "superseded" => return held,
            _ => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "session {session_id} did not finish; it is {}",
                    row.status
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
    }
}

/// `POST /api/sessions/:id/resume`, returning the raw status and body.
pub async fn resume_raw(
    runtime: &Runtime,
    session_id: i32,
    feedback: Option<&str>,
) -> (axum::http::StatusCode, serde_json::Value) {
    let body = match feedback {
        Some(text) => serde_json::json!({ "feedback": text }),
        None => serde_json::json!({}),
    };
    runtime
        .app
        .post(&format!("/api/sessions/{session_id}/resume"), body)
        .await
}

/// The session DTO as the API returns it (`GET /api/sessions/:id`).
pub async fn session_dto(runtime: &Runtime, session_id: i32) -> serde_json::Value {
    let (status, body) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(status, 200, "session snapshot: {body}");
    body["session"].clone()
}

/// Waits until the session has held a call whose id is not `exclude` (a NEW hold after a resume;
/// the row keeps `held_tool_call_id` while the resume is in flight, so comparing ids is the only
/// reliable way to see the next hold).
pub async fn wait_for_new_hold(runtime: &Runtime, session_id: i32, exclude: i32) -> SessionRow {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let row = runtime.session_row(session_id).await;
        if row.status == "held" && row.held_tool_call_id.is_some_and(|id| id != exclude) {
            return row;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session {session_id} did not hold a new call; it is {} (held {})",
            row.status,
            row.held_tool_call_id
                .map_or_else(|| "-".to_owned(), |id| id.to_string())
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Waits until the session reaches `status`, returning its row.
pub async fn wait_for_status(runtime: &Runtime, session_id: i32, status: &str) -> SessionRow {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let row = runtime.session_row(session_id).await;
        if row.status == status {
            return row;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session {session_id} never reached {status}; it is {}",
            row.status
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// A runtime whose scripted provider fails every request with `error` (a provider that rejects the
/// request outright, so no scripted step is consumed).
pub async fn runtime_failing(error: nekowriter_llm::retry::LlmError) -> Runtime {
    let runtime = runtime(Vec::new()).await;
    let mut runtime = runtime;
    let scripted = runtime.app.set_provider(
        nekowriter_llm::mock::ScriptedProvider::new(Vec::new()).fail_when_exhausted(error),
    );
    runtime.scripted = scripted;
    runtime
}

/// A runtime whose scripted provider serves `steps` and then fails every further request with
/// `error` (a provider that rejects the request outright after the scripted ones).
pub async fn runtime_with_failure(
    steps: Vec<nekowriter_llm::mock::ScriptStep>,
    error: nekowriter_llm::retry::LlmError,
) -> Runtime {
    let mut runtime = runtime(Vec::new()).await;
    let scripted = runtime.app.set_provider(
        nekowriter_llm::mock::ScriptedProvider::new(steps).fail_when_exhausted(error),
    );
    runtime.scripted = scripted;
    runtime
}

/// Reads one integer from a query (`count(*) AS value`).
pub async fn query_i64(runtime: &Runtime, sql: &str, values: Vec<sea_orm::Value>) -> i64 {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            sql,
            values,
        ))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i64>("", "value").expect("value")
}
