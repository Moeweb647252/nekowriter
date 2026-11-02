//! Control state-table tests (PLAN M3.3, spec/03 §3, §6, spec/10 §2.7).
//!
//! Every operation is validated against the status table of spec/03 §3: an invalid one returns
//! `409 session.invalid_state` with `details.status` and `details.operation`. The tests drive the
//! real router and assert the persisted row after each legal operation.

mod agent_harness;

use std::time::Duration;

use agent_harness::{Runtime, finish_step, runtime, tool_step};
use axum::http::StatusCode;
use nekowriter::db::json::Granularity;
use nekowriter::testing::assert_error;
use serde_json::json;

/// Asserts a rejected operation with the documented code and details (spec/10 §4).
fn assert_invalid_operation(body: &serde_json::Value, operation: &str, status: &str) {
    assert_error(body, "session.invalid_state");
    assert_eq!(body["error"]["details"]["operation"], operation);
    assert_eq!(body["error"]["details"]["status"], status);
}

/// A held session rejects every operation that needs another status (spec/03 §3).
#[tokio::test]
async fn a_held_session_rejects_pause_continue_and_answer() {
    let runtime = runtime(vec![
        tool_step("test_write", json!({ "text": "held" })),
        finish_step(),
    ])
    .await;
    let session_id = runtime
        .start_debug_with(Granularity::Paragraph, true, false)
        .await;
    runtime.wait_any_status(session_id).await;
    let row = runtime.session_row(session_id).await;
    assert_eq!(row.status, "held", "auto_pause held the write call");

    for (operation, body) in [
        ("pause", json!({})),
        ("continue", json!({})),
        (
            "answer",
            json!({ "tool_call_id": row.held_tool_call_id, "payload": {} }),
        ),
    ] {
        let (status, response) = runtime
            .app
            .post(&format!("/api/sessions/{session_id}/{operation}"), body)
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{operation}: {response}");
        assert_invalid_operation(&response, operation, "held");
    }

    // Resume is legal and finalizes the call unchanged.
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Completed)
        .await;
}

/// A running session rejects resume, continue and answer, and accepts message and pause
/// (spec/03 §3).
#[tokio::test]
async fn a_running_session_accepts_message_and_pause_only() {
    // A first step that stalls keeps the session `running` for the whole test; it is stopped at
    // the end (well before the delay elapses).
    let runtime = runtime(vec![
        nekowriter_llm::mock::ScriptStep::new()
            .text("a stalled reply")
            .delay(Duration::from_secs(10)),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    // The session is created `running` and stays there until its request finishes.
    let row = runtime.session_row(session_id).await;
    assert_eq!(row.status, "running", "a new session starts running");

    // message while running: queued in the inbox (200).
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/message"),
            json!({ "text": "steer" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "message while running: {body}");

    // resume/continue/answer on a running session are invalid.
    for operation in ["resume", "continue"] {
        let (status, response) = runtime
            .app
            .post(
                &format!("/api/sessions/{session_id}/{operation}"),
                json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{operation}: {response}");
        assert_invalid_operation(&response, operation, "running");
    }
    let (status, response) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/answer"),
            json!({ "tool_call_id": 1, "payload": {} }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "answer: {response}");
    assert_invalid_operation(&response, "answer", "running");

    // The writer switches are legal on an open session (spec/03 §6).
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/sessions/{session_id}"),
            json!({ "granularity": "free", "pause_after_outline": false }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "patch: {body}");
    assert_eq!(body["granularity"], "free");
    assert_eq!(body["pause_after_outline"], false);
    assert_eq!(
        runtime.session_row(session_id).await.granularity.as_deref(),
        Some("free"),
        "the switch is persisted"
    );

    // Stop it (a running session accepts Stop) and let it reach `stopped`.
    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "stop while running");
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Stopped)
        .await;
}

/// Inserts a notice into the same inbox queue, bypassing the API (which only accepts steering
/// messages).
async fn insert_notice(runtime: &agent_harness::Runtime, session_id: i32, content: &str) {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    runtime
        .app
        .state()
        .db_conn()
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO agent_inbox (session_id, kind, content, created_at) \
             VALUES (?, 'notice', ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            vec![session_id.into(), content.into()],
        ))
        .await
        .expect("insert notice");
}

/// A running session exposes queued steering messages until the runner consumes them (spec/10
/// §2.7). The inbox contains only author messages, while `inbox_pending` counts every row.
#[tokio::test]
async fn a_running_session_snapshot_shows_steering_messages_until_consumed() {
    let runtime = runtime(vec![
        nekowriter_llm::mock::ScriptStep::new()
            .text("first reply")
            .delay(Duration::from_secs(5)),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    let initial = runtime.session_row(session_id).await;
    assert_eq!(initial.status, "running", "a new session starts running");
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/pause"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "pause: {body}");
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Paused)
        .await;
    for text in ["steer one", "steer two"] {
        let (status, body) = runtime
            .app
            .post(
                &format!("/api/sessions/{session_id}/message"),
                json!({ "text": text }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "message {text}: {body}");
    }
    insert_notice(&runtime, session_id, "notice only").await;

    let (status, snapshot) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "snapshot: {snapshot}");
    assert_eq!(
        snapshot["inbox_pending"], 3,
        "every unconsumed row counts: {snapshot}"
    );
    let inbox = snapshot["inbox"].as_array().expect("inbox");
    assert_eq!(
        inbox.len(),
        2,
        "notices are not author messages: {snapshot}"
    );
    assert_eq!(inbox[0]["kind"], "user_message", "{snapshot}");
    assert_eq!(inbox[0]["content"], "steer one", "{snapshot}");
    assert_eq!(inbox[1]["kind"], "user_message", "{snapshot}");
    assert_eq!(inbox[1]["content"], "steer two", "{snapshot}");

    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "resume: {body}");
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Completed)
        .await;
    let (status, snapshot) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "consumed snapshot: {snapshot}");
    assert_eq!(snapshot["inbox"], json!([]), "{snapshot}");
    assert_eq!(snapshot["inbox_pending"], 0, "{snapshot}");
    let messages = snapshot["messages"].as_array().expect("messages");
    let mut author_texts = Vec::new();
    for item in messages {
        if item["message"]["role"] != "user" {
            continue;
        }
        if let Some(blocks) = item["message"]["blocks"].as_array() {
            for block in blocks {
                if let Some(text) = block["text"].as_str() {
                    author_texts.push(text);
                }
            }
        }
    }
    let first = author_texts
        .iter()
        .position(|text| *text == "steer one")
        .expect("first steering text persisted");
    let second = author_texts
        .iter()
        .position(|text| *text == "steer two")
        .expect("second steering text persisted");
    assert!(
        first < second,
        "both steering texts persisted in event order: {author_texts:?}"
    );
}

/// An idle session accepts a message, which starts a runner (spec/03 §3, §6).
#[tokio::test]
async fn an_idle_session_starts_on_a_message() {
    // A conception session ends a turn with text → idle (spec/03 §5.6). M3 creates one through
    // the service because the conception flow arrives with M9.
    let runtime = runtime(vec![
        nekowriter_llm::mock::ScriptStep::new().text("a conception reply"),
        finish_step(),
    ])
    .await;
    let session_id = nekowriter::services::sessions::create(
        runtime.app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: nekowriter::db::json::SessionKind::Conception,
            book_id: Some(runtime.book_id),
            ..Default::default()
        },
    )
    .await
    .expect("create conception session")
    .0;
    let row = runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Idle)
        .await;
    assert_eq!(row.status, "idle");

    // A message moves it back to running and spawns the runner.
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/message"),
            json!({ "text": "here is my answer" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "message to an idle session: {body}");
    assert_eq!(body["status"], "running");

    // The reply is text-only again, so it idles again; the message was delivered.
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Idle)
        .await;
    let messages = runtime.messages(session_id).await;
    assert!(
        messages
            .iter()
            .any(|message| message.role == "user" && message.text().contains("here is my answer")),
        "the author's reply reached the conversation: {messages:?}"
    );
}

/// `continue` is valid only for stopped, failed or interrupted (spec/03 §3).
#[tokio::test]
async fn continue_accepts_only_the_three_recoverable_statuses() {
    let runtime = runtime(vec![finish_step()]).await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Completed)
        .await;
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/continue"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_invalid_operation(&body, "continue", "completed");
}

/// `abandon` is valid from any open status and terminal sessions reject it (spec/03 §3).
#[tokio::test]
async fn abandon_is_valid_while_open_only() {
    let runtime = runtime(vec![finish_step()]).await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Completed)
        .await;
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/abandon"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_invalid_operation(&body, "abandon", "completed");
}

/// `message` to an `awaiting_input` session is rejected: the form must be answered
/// (spec/03 §6).
#[tokio::test]
async fn message_is_rejected_while_a_form_awaits_an_answer() {
    // M3 has no interactive tool, so the state is produced directly (the conception `ask_user`
    // flow of M9 is what normally reaches it).
    let runtime = runtime(vec![finish_step()]).await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Completed)
        .await;
    runtime
        .app
        .execute(&format!(
            "UPDATE agent_sessions SET status = 'awaiting_input', held_tool_call_id = NULL \
             WHERE id = {session_id}"
        ))
        .await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/message"),
            json!({ "text": "hello" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_invalid_operation(&body, "message", "awaiting_input");

    // Answering without a matching call is a validation failure, not a state one.
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/answer"),
            json!({ "tool_call_id": 12345, "payload": { "answers": [] } }),
        )
        .await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::CONFLICT,
        "{status} {body}"
    );
}

/// An unknown session id is `404 not_found` for every operation (spec/10 §4).
#[tokio::test]
async fn an_unknown_session_is_not_found() {
    let runtime = runtime(vec![]).await;
    for (operation, body) in [
        ("pause", json!({})),
        ("resume", json!({})),
        ("stop", json!({})),
        ("continue", json!({})),
        ("abandon", json!({})),
        ("message", json!({ "text": "x" })),
        ("answer", json!({ "tool_call_id": 1, "payload": {} })),
    ] {
        let (status, response) = runtime
            .app
            .post(&format!("/api/sessions/9999/{operation}"), body)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{operation}: {response}");
        assert_error(&response, "not_found");
    }
    let (status, body) = runtime.app.get("/api/sessions/9999").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, body) = runtime.app.get("/api/sessions/9999/messages").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// The session list filters by kind, status and chapter (spec/10 §2.7).
#[tokio::test]
async fn the_session_list_filters_work() {
    let runtime = runtime(vec![finish_step()]).await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Completed)
        .await;

    let (status, body) = runtime
        .app
        .get(&format!("/api/books/{}/sessions", runtime.book_id))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().expect("array").len(), 1);

    let (status, body) = runtime
        .app
        .get(&format!(
            "/api/books/{}/sessions?status=completed",
            runtime.book_id
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().expect("array").len(), 1);

    let (status, body) = runtime
        .app
        .get(&format!(
            "/api/books/{}/sessions?status=failed",
            runtime.book_id
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.as_array().expect("array").is_empty());

    let (status, body) = runtime
        .app
        .get(&format!(
            "/api/books/{}/sessions?chapter_id={}",
            runtime.book_id, runtime.chapter_id
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().expect("array").len(), 1);

    let (status, body) = runtime
        .app
        .get(&format!(
            "/api/books/{}/sessions?chapter_id=9999",
            runtime.book_id
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.as_array().expect("array").is_empty());
}

/// The message page returns older-first pages with a cursor (spec/10 §2.7).
#[tokio::test]
async fn the_message_page_is_cursor_paginated() {
    let runtime = runtime(vec![
        tool_step("echo", json!({ "text": "x" })),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Completed)
        .await;

    let (status, body) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}/messages?limit=1"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "the limit is honoured");
    let cursor = body["next_cursor"].as_str().expect("a cursor");

    let (status, older) = runtime
        .app
        .get(&format!(
            "/api/sessions/{session_id}/messages?limit=10&cursor={cursor}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{older}");
    let older_items = older["items"].as_array().expect("items");
    assert!(
        older_items
            .iter()
            .all(|item| item["message"]["seq"].as_i64().unwrap_or(i64::MAX)
                < cursor.parse::<i64>().unwrap_or(0)),
        "a page holds only older messages: {older_items:?}"
    );
}

/// Replaces a session's messages with `count` synthetic assistant messages, `seq` 0..count.
async fn seed_messages(runtime: &Runtime, session_id: i32, count: usize) {
    runtime
        .app
        .execute(&format!(
            "DELETE FROM agent_messages WHERE session_id = {session_id}"
        ))
        .await;
    let values = (0..count)
        .map(|seq| {
            format!(
                "({session_id}, {seq}, 'assistant', 'assistant', \
                 '[{{\"type\":\"text\",\"text\":\"m{seq}\"}}]', '2026-01-01T00:00:00Z')"
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    runtime
        .app
        .execute(&format!(
            "INSERT INTO agent_messages (session_id, seq, role, kind, content, created_at) \
             VALUES {values}"
        ))
        .await;
}

/// The snapshot reports the scrollback cursor only when its 200-message window is truncated
/// (spec/10 §2.7). The cursor is the decimal `seq` of the oldest returned message, which is
/// exactly the `seq < cursor` boundary of the message page; a complete 200-message history has
/// no cursor.
#[tokio::test]
async fn the_snapshot_reports_a_scrollback_cursor_only_when_truncated() {
    let runtime = runtime(vec![finish_step(), finish_step()]).await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Completed)
        .await;
    seed_messages(&runtime, session_id, 201).await;

    let (status, body) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 200, "the window is the last 200 messages");
    assert_eq!(
        messages[0]["message"]["seq"].as_i64(),
        Some(1),
        "the oldest message in the window is seq 1"
    );
    assert_eq!(body["next_cursor"].as_str(), Some("1"), "{body}");

    // The cursor starts the page just before the window: message 0 and nothing else.
    let (status, older) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}/messages?cursor=1"))
        .await;
    assert_eq!(status, StatusCode::OK, "{older}");
    let items = older["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{older}");
    assert_eq!(items[0]["message"]["seq"].as_i64(), Some(0));
    assert!(older["next_cursor"].is_null(), "{older}");

    // Exactly 200 messages is a complete history: there is nothing before the window.
    let full_session = runtime.start_debug().await;
    runtime
        .wait_status(full_session, nekowriter::db::json::SessionStatus::Completed)
        .await;
    seed_messages(&runtime, full_session, 200).await;
    let (status, body) = runtime
        .app
        .get(&format!("/api/sessions/{full_session}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["messages"].as_array().expect("messages").len(), 200);
    assert!(
        body["next_cursor"].is_null(),
        "a complete window has no cursor: {body}"
    );
}
