//! Harness for the writer-session tests (PLAN M4.2–M4.5, spec/03 §15).
//!
//! It reuses `agent_harness` (the temp database, the real router, the `ScriptedProvider`) and
//! adds what only a real `chapter_write` session needs: a session created through the production
//! path, so the frozen tool list, the kickoff and the phase machine are the shipped ones rather
//! than a debug stand-in.

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
