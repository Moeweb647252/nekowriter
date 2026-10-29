//! Harness for the conception tests (PLAN M9, spec/08 §5, spec/05 §7).
//!
//! It reuses `agent_harness` (the temp database, the real router, the `ScriptedProvider`) and
//! adds what a real conception session needs: a book created through the production path with
//! `start_conception`, readers for the setting materials, and the answer helper.

// The harness is shared by several test binaries, and each uses a different subset of it.
#![allow(dead_code)]

#[path = "../agent_harness/mod.rs"]
pub mod base;

pub use base::*;

use nekowriter_llm::mock::ScriptStep;
use serde_json::Value;

/// Creates a book with `start_conception: true` through the API and returns the response body
/// (`{book, session_id?, warnings}`). The runtime must have its scripted provider installed and
/// every role assigned first (that is what [`runtime`] does).
pub async fn create_conceiving_book(runtime: &Runtime, title: &str) -> Value {
    let (status, body) = runtime
        .app
        .post(
            "/api/books",
            serde_json::json!({
                "title": title,
                "idea": "一个失去师门的剑客，带着一封未拆的旧信走进边城。",
                "genre": "xianxia",
                "writing_language": "zh-CN",
                "start_conception": true,
            }),
        )
        .await;
    assert_eq!(status, 201, "create conceiving book: {body}");
    body
}

/// The book's conception session id (the response's `session_id`, or the open one from
/// `conception/start`).
pub async fn conception_session(runtime: &Runtime, book_id: i32) -> i32 {
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/start"),
            Value::Null,
        )
        .await;
    assert_eq!(status, 200, "conception/start: {body}");
    body["session_id"].as_i64().expect("session id") as i32
}

/// Waits for the session to reach `awaiting_input` and returns the awaiting call id.
pub async fn wait_for_form(runtime: &Runtime, session_id: i32) -> i32 {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let row = runtime.session_row(session_id).await;
        if row.status == "awaiting_input" {
            return row.held_tool_call_id.expect("the awaiting call id");
        }
        if matches!(
            row.status.as_str(),
            "idle" | "completed" | "failed" | "stopped"
        ) {
            panic!(
                "session {session_id} reached {} without asking the author",
                row.status
            );
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session {session_id} never awaited input; it is {}",
            row.status
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Answers the session's awaiting call through the API and returns the response status and body.
pub async fn answer(
    runtime: &Runtime,
    session_id: i32,
    call_id: i32,
    payload: Value,
) -> (axum::http::StatusCode, Value) {
    runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/answer"),
            serde_json::json!({ "tool_call_id": call_id, "payload": payload }),
        )
        .await
}

/// Answers a form and asserts success, returning the persisted call row after sealing.
pub async fn answer_ok(runtime: &Runtime, session_id: i32, call_id: i32, payload: Value) {
    let (status, body) = answer(runtime, session_id, call_id, payload).await;
    assert_eq!(status, 200, "answer: {body}");
}

/// Waits until the call with `name` is `done` and `sealed`, then returns its persisted row.
pub async fn wait_for_sealed(runtime: &Runtime, session_id: i32, name: &str) -> CallRow {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if let Some(row) = runtime
            .calls(session_id)
            .await
            .into_iter()
            .find(|call| call.name == name && call.sealed)
        {
            return row;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session {session_id} never sealed its {name} call"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// The first call of the session with that name, sealed or not.
pub async fn first_call(runtime: &Runtime, session_id: i32, name: &str) -> CallRow {
    runtime
        .calls(session_id)
        .await
        .into_iter()
        .find(|call| call.name == name)
        .unwrap_or_else(|| panic!("session {session_id} has no {name} call"))
}

/// The `(content, revision, approved_revision)` of one setting document.
pub async fn setting_doc(
    runtime: &Runtime,
    book_id: i32,
    kind: &str,
) -> (String, i32, Option<i32>) {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT content, revision, approved_revision FROM setting_docs \
             WHERE book_id = ? AND kind = ?",
            vec![book_id.into(), kind.to_owned().into()],
        ))
        .await
        .expect("query setting doc")
        .expect("setting doc row");
    (
        row.try_get::<String>("", "content").expect("content"),
        row.try_get::<i32>("", "revision").expect("revision"),
        row.try_get::<Option<i32>>("", "approved_revision")
            .expect("approved_revision"),
    )
}

/// The `(name, rank)` pairs of the book's character designs, in rank order.
pub async fn characters(runtime: &Runtime, book_id: i32) -> Vec<(String, i32)> {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    runtime
        .app
        .db()
        .conn()
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT name, rank FROM character_designs WHERE book_id = ? ORDER BY rank",
            vec![book_id.into()],
        ))
        .await
        .expect("query characters")
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "name").expect("name"),
                row.try_get::<i32>("", "rank").expect("rank"),
            )
        })
        .collect()
}

/// The book's research tasks as `(topic, scope_name, status)` in id order.
pub async fn research_tasks(runtime: &Runtime, book_id: i32) -> Vec<(String, String, String)> {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    runtime
        .app
        .db()
        .conn()
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT topic, scope_name, status FROM research_tasks WHERE book_id = ? ORDER BY id",
            vec![book_id.into()],
        ))
        .await
        .expect("query research tasks")
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "topic").expect("topic"),
                row.try_get::<String>("", "scope_name").expect("scope"),
                row.try_get::<String>("", "status").expect("status"),
            )
        })
        .collect()
}

/// The book's `ConceptionState`.
pub async fn conception_state(
    runtime: &Runtime,
    book_id: i32,
) -> nekowriter::db::json::ConceptionState {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT conception FROM books WHERE id = ?",
            vec![book_id.into()],
        ))
        .await
        .expect("query conception state")
        .expect("book row");
    serde_json::from_str(&row.try_get::<String>("", "conception").expect("conception"))
        .unwrap_or_default()
}

/// The book's status column.
pub async fn book_status(runtime: &Runtime, book_id: i32) -> String {
    let (_, body) = runtime.app.get(&format!("/api/books/{book_id}")).await;
    body["status"].as_str().expect("status").to_owned()
}

/// The change-log rows of a session as `(target_type, op, actor)`.
pub async fn session_changes(runtime: &Runtime, session_id: i32) -> Vec<(String, String, String)> {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    runtime
        .app
        .db()
        .conn()
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT target_type, op, actor FROM change_log WHERE session_id = ? ORDER BY id",
            vec![session_id.into()],
        ))
        .await
        .expect("query changes")
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "target_type").expect("target"),
                row.try_get::<String>("", "op").expect("op"),
                row.try_get::<String>("", "actor").expect("actor"),
            )
        })
        .collect()
}

/// A step that asks two questions (the Demo provider's first conception turn).
pub fn ask_step() -> ScriptStep {
    tool_step(
        "ask_user",
        serde_json::json!({
            "questions": [
                {
                    "id": "tone",
                    "question": "整体基调更偏向哪种？",
                    "options": [
                        { "label": "热血爽快" },
                        { "label": "沉郁厚重" },
                    ],
                },
                {
                    "id": "ending",
                    "question": "结局的走向？",
                    "options": [{ "label": "开放" }, { "label": "悲剧" }],
                    "multi_select": false,
                },
            ],
        }),
    )
}

/// A `request_review` step.
pub fn review_step(message: &str) -> ScriptStep {
    tool_step("request_review", serde_json::json!({ "message": message }))
}

/// Valid answers for [`ask_step`].
pub fn ask_answer() -> Value {
    serde_json::json!({
        "answers": [
            { "id": "tone", "selected": ["沉郁厚重"], "other": "像旧信一样安静" },
            { "id": "ending", "selected": ["开放"] },
        ],
    })
}

/// The `ui` payload of one call.
pub async fn call_ui(runtime: &Runtime, call_id: i32) -> Value {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let raw = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT ui FROM agent_tool_calls WHERE id = ?",
            vec![call_id.into()],
        ))
        .await
        .expect("query ui")
        .expect("call row")
        .try_get::<Option<String>>("", "ui")
        .expect("ui column");
    raw.and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or(Value::Null)
}

/// The book's conception session id, read from the database (for tests whose book was created
/// before the response's `session_id` was observable).
pub async fn conception_session_of_book(runtime: &Runtime, book_id: i32) -> i32 {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT id AS value FROM agent_sessions \
             WHERE book_id = ? AND kind = 'conception' ORDER BY id DESC LIMIT 1",
            vec![book_id.into()],
        ))
        .await
        .expect("query session")
        .expect("session row")
        .try_get::<i32>("", "value")
        .expect("session id")
}
