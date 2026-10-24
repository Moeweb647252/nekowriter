//! Harness for the review tests (PLAN M6.1, spec/08 §3, spec/03 §6).
//!
//! It reuses `agent_harness` (the temp database, the real router, the `ScriptedProvider`) and adds
//! what a real `chapter_review` session needs: a production-path session creation, a chapter that
//! is `in_review` with text, and readers for the persisted review rows.

// The harness is shared by several test binaries, and each uses a different subset of it.
#![allow(dead_code)]

#[path = "../agent_harness/mod.rs"]
pub mod base;

pub use base::*;

use nekowriter::db::json::SessionKind;
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter_llm::mock::ScriptStep;
use serde_json::{Value, json};

/// Starts a real `chapter_review` session for the harness chapter.
pub async fn start_reviewer(runtime: &Runtime) -> i32 {
    let (session_id, _warnings) = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterReview,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create reviewer session");
    session_id
}

/// Writes three paragraphs and marks the chapter `in_review`, so the review operations run.
pub async fn prepare_in_review(runtime: &Runtime) -> i32 {
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    let version = chapter["content_version"].as_i64().expect("version") as i32;
    runtime
        .app
        .put_text(
            runtime.chapter_id,
            version,
            "第一段，雨落在城门口。\n\n第二段，他握紧了手中的旧剑。\n\n第三段，钟声敲了七下。",
        )
        .await;
    runtime
        .app
        .set_chapter_status(runtime.chapter_id, "in_review")
        .await;
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    chapter["content_version"].as_i64().expect("version") as i32
}

/// A `submit_review` step (spec/05 §5).
pub fn review_step(verdict: &str, summary: &str, issues: Value, feedback: &str) -> ScriptStep {
    ScriptStep::new().tool(
        "submit_review",
        json!({
            "verdict": verdict,
            "summary": summary,
            "issues": issues,
            "feedback": feedback,
        }),
    )
}

/// One issue of a scripted review.
pub fn issue(pids: Value, quote: &str) -> Value {
    json!({
        "severity": "minor",
        "category": "prose",
        "pids": pids,
        "quote": quote,
        "problem": "the transition is missing",
        "suggestion": "add a line of market noise",
    })
}

/// `GET /api/chapters/:id/reviews`.
pub async fn reviews(runtime: &Runtime) -> Value {
    let (status, body) = runtime
        .app
        .get(&format!("/api/chapters/{}/reviews", runtime.chapter_id))
        .await;
    assert_eq!(status, 200, "reviews: {body}");
    body
}

/// All sessions of one kind for the harness chapter, oldest first, as `(id, status)`.
pub async fn sessions_of_kind(runtime: &Runtime, kind: &str) -> Vec<(i32, String)> {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    runtime
        .app
        .db()
        .conn()
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT id, status FROM agent_sessions WHERE chapter_id = ? AND kind = ? ORDER BY id",
            vec![runtime.chapter_id.into(), kind.to_owned().into()],
        ))
        .await
        .expect("query sessions")
        .into_iter()
        .map(|row| {
            (
                row.try_get::<i32>("", "id").expect("id"),
                row.try_get::<String>("", "status").expect("status"),
            )
        })
        .collect()
}

/// One session column (e.g. `pending_comment`, `base_content_version`).
pub async fn session_column<T>(runtime: &Runtime, session_id: i32, column: &str) -> Option<T>
where
    T: sea_orm::TryGetable,
{
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let sql = format!("SELECT {column} FROM agent_sessions WHERE id = ?");
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            &sql,
            vec![session_id.into()],
        ))
        .await
        .expect("query")
        .expect("session row");
    row.try_get::<T>("", column).ok()
}

/// The joined kickoff text of a session (spec/03 §10).
pub async fn kickoff_text(runtime: &Runtime, session_id: i32) -> String {
    runtime
        .messages(session_id)
        .await
        .into_iter()
        .find(|message| message.kind == "kickoff")
        .map(|message| message.text())
        .unwrap_or_default()
}
