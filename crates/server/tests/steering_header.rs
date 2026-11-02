//! Steering messages carry their 【用户留言】 header (PLAN M17.1, D-160, spec/03 §5.3/§5.6).
//!
//! An author steering message is stored in `agent_inbox` with kind `user_message` and the author's
//! **raw** text. Every consumption path must render `notice.user_message` with the session's
//! `PromptSet` at the moment the row is consumed and deliver that rendered text:
//!
//! - path A: appended to the last tool result of the unsealed batch;
//! - path B: a new user message when there is no batch left to seal;
//! - path C: the user message built by text-only turn handling (§5.6).
//!
//! Runtime notices (`InboxKind::Notice`) are already rendered when queued and must pass through
//! unchanged. These tests also pin the book override, the render-failure fallback (the raw text is
//! never lost), several messages in one batch, and the paused-session feedback path.

mod agent_harness;

use std::time::{Duration, Instant};

use agent_harness::{Runtime, runtime};
use axum::http::StatusCode;
use nekowriter::db::json::{SessionKind, SessionStatus, StoredBlock, ToolCallStatus};
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter_llm::Message;
use nekowriter_llm::mock::ScriptStep;
use serde_json::{Value, json};

/// The header the prompt pack puts on every author message (prompts/notices.toml).
const HEADER: &str = "【用户留言】";

/// The exact text `notice.user_message` renders for `text` (the template is the header, a
/// newline, then the text).
fn rendered(text: &str) -> String {
    format!("{HEADER}\n{text}")
}

fn slow_step(text: &str, sleep_ms: u64) -> ScriptStep {
    ScriptStep::new().tool("test_slow", json!({ "text": text, "sleep_ms": sleep_ms }))
}

/// Waits until one call is marked `executing` (the deterministic window in which a steering
/// message arrives while the batch is still unsealed).
async fn wait_for_executing(runtime: &Runtime, session_id: i32) -> i32 {
    let deadline = Instant::now() + agent_harness::wait_budget();
    loop {
        let calls = runtime.calls(session_id).await;
        if let Some(call) = calls
            .iter()
            .find(|call| call.status_enum() == ToolCallStatus::Executing)
        {
            return call.id;
        }
        assert!(
            Instant::now() <= deadline,
            "no call of session {session_id} entered executing"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// The raw, unconsumed rows of the inbox in arrival order: `(id, kind, content)`.
async fn pending_inbox(runtime: &Runtime, session_id: i32) -> Vec<(i32, String, String)> {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    runtime
        .app
        .db()
        .conn()
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT id, kind, content FROM agent_inbox \
             WHERE session_id = ? AND consumed_at IS NULL ORDER BY id",
            vec![session_id.into()],
        ))
        .await
        .expect("query pending inbox")
        .into_iter()
        .map(|row| {
            (
                row.try_get::<i32>("", "id").expect("id"),
                row.try_get::<String>("", "kind").expect("kind"),
                row.try_get::<String>("", "content").expect("content"),
            )
        })
        .collect()
}

/// True when some provider request carried `needle` as tool-result or user text.
fn delivered_in_requests(runtime: &Runtime, needle: &str) -> bool {
    runtime.scripted.requests().iter().any(|request| {
        request.messages.iter().any(|message| match message {
            Message::ToolResults { results } => {
                results.iter().any(|result| result.content.contains(needle))
            }
            Message::User { blocks } => blocks.iter().any(|block| block.text.contains(needle)),
            Message::Assistant { .. } => false,
        })
    })
}

/// The text blocks of a message, in order.
fn text_blocks(message: &agent_harness::MessageRow) -> Vec<String> {
    message
        .blocks()
        .into_iter()
        .filter_map(|block| match block {
            StoredBlock::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// The `inbox` appendices of a call, in order.
fn inbox_appendices(call: &agent_harness::CallRow) -> Vec<Value> {
    Runtime::appendices(call)
        .into_iter()
        .filter(|appendix| appendix["kind"] == "inbox")
        .collect()
}

// ───────────────────────────── path A ─────────────────────────────

/// Path A (spec/03 §5.3 slot 4): a message queued while a call is executing is delivered as the
/// `notice.user_message` appendix of the batch's last result, and the next request carries it.
#[tokio::test]
async fn path_a_a_steering_message_is_an_appendix_of_the_last_tool_result() {
    let runtime = runtime(vec![
        slow_step("slow page", 1_200),
        agent_harness::finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    wait_for_executing(&runtime, session_id).await;

    let text = "把雨写大一点。";
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/message"),
            json!({ "text": text }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "message while running: {body}");

    // Before consumption: the stored row and the queued-row DTO keep the author's raw text.
    let queued = pending_inbox(&runtime, session_id).await;
    assert_eq!(queued.len(), 1, "one steering message is queued");
    assert_eq!(queued[0].1, "user_message");
    assert_eq!(queued[0].2, text, "agent_inbox.content stays raw");
    let inbox_id = queued[0].0;
    let (status, snapshot) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "snapshot: {snapshot}");
    assert_eq!(snapshot["inbox"][0]["id"], inbox_id);
    assert_eq!(snapshot["inbox"][0]["content"], text);

    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    let calls = runtime.calls(session_id).await;
    let slow = calls
        .iter()
        .find(|call| call.name == "test_slow")
        .expect("the slow call");
    let expected = format!("slow page\n\n{}", rendered(text));
    assert_eq!(
        slow.result_text.as_deref(),
        Some(expected.as_str()),
        "the raw result, a blank line and the rendered notice"
    );
    let appendices = inbox_appendices(slow);
    assert_eq!(appendices.len(), 1, "one inbox appendix: {appendices:?}");
    assert_eq!(appendices[0]["text"], rendered(text));
    assert_eq!(appendices[0]["inbox_id"], inbox_id);
    assert!(
        delivered_in_requests(&runtime, &rendered(text)),
        "the next provider request carries the rendered appendix"
    );
    assert_eq!(runtime.inbox_pending(session_id).await, 0);
}

// ───────────────────────────── path B ─────────────────────────────

/// Path B (spec/03 §5.3): when the batch was already sealed by a request that never finished, the
/// report and the inbox become one new user message — the report first, the rendered message
/// second, as separate blocks.
#[tokio::test]
async fn path_b_a_steering_message_becomes_a_rendered_user_message() {
    let runtime = runtime(vec![
        agent_harness::tool_step("echo", json!({ "text": "first result" })),
        // A request that never finishes: Stop cancels it before anything is persisted.
        ScriptStep::new()
            .text("a partial reply that never lands")
            .delay(Duration::from_secs(5)),
        agent_harness::finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;

    // Wait for the second request (the one that never finishes), then stop it mid-flight.
    let deadline = Instant::now() + agent_harness::wait_budget();
    while runtime.scripted.requests().len() < 2 {
        assert!(
            Instant::now() <= deadline,
            "the second provider request never started"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "stop mid-stream");
    runtime
        .wait_status(session_id, SessionStatus::Stopped)
        .await;

    // The author edits the chapter and sends a message while stopped: both must ride in the same
    // new user message when the session continues.
    let chapter = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await
        .1;
    let version = chapter["content_version"].as_i64().expect("version") as i32;
    runtime
        .app
        .put_text(runtime.chapter_id, version, "作者在暂停期间写下的一段。")
        .await;
    let text = "接着写，别停。";
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/message"),
            json!({ "text": text }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "message while stopped: {body}");
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/continue"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "continue: {body}");
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    let messages = runtime.messages(session_id).await;
    let delivered = messages
        .iter()
        .find(|message| message.kind == "user" && message.text().contains(text))
        .expect("a user message carries the steering text");
    let blocks = text_blocks(delivered);
    assert_eq!(blocks.len(), 2, "two separate blocks: {blocks:?}");
    assert!(
        blocks[0].contains("【用户修改了正文】"),
        "the edit report comes first: {blocks:?}"
    );
    assert!(
        blocks[0].contains("作者在暂停期间写下的一段。"),
        "the report names the author's edit: {blocks:?}"
    );
    assert_eq!(blocks[1], rendered(text), "then the rendered notice");
    assert!(
        delivered_in_requests(&runtime, &rendered(text)),
        "the next provider request carries the user message"
    );
}

// ───────────────────────────── path C ─────────────────────────────

/// Path C (spec/03 §5.6): the reply to an idle conception session is a user message whose block
/// is the rendered `notice.user_message`.
#[tokio::test]
async fn path_c_a_conception_reply_is_a_rendered_user_message() {
    let runtime = runtime(vec![
        agent_harness::text_step("a conception reply"),
        agent_harness::text_step("another reply"),
    ])
    .await;
    let session_id = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::Conception,
            book_id: Some(runtime.book_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create conception session")
    .0;
    runtime.wait_status(session_id, SessionStatus::Idle).await;

    let text = "主角怕水，但从不承认。";
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/message"),
            json!({ "text": text }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "message to an idle session: {body}");
    runtime.wait_status(session_id, SessionStatus::Idle).await;

    let messages = runtime.messages(session_id).await;
    let user = messages
        .iter()
        .find(|message| message.kind == "user")
        .expect("the reply became a user message");
    let blocks = text_blocks(user);
    assert_eq!(blocks.len(), 1, "one rendered block: {blocks:?}");
    assert_eq!(blocks[0], rendered(text));
    assert!(
        delivered_in_requests(&runtime, &rendered(text)),
        "the next provider request carries the rendered notice"
    );
}

// ───────────────────────────── paused feedback ─────────────────────────────

/// `resume {feedback}` on a `paused` session queues a `user_message` inbox row (spec/03 §6) and
/// its next sealing renders the header.
#[tokio::test]
async fn paused_session_feedback_is_delivered_rendered() {
    let runtime = runtime(vec![
        slow_step("slow page", 1_200),
        agent_harness::finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    wait_for_executing(&runtime, session_id).await;

    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/pause"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "pause: {body}");
    runtime.wait_status(session_id, SessionStatus::Paused).await;

    let feedback = "这一段不要写死，留个口气。";
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/resume"),
            json!({ "feedback": feedback }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "resume: {body}");
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    let calls = runtime.calls(session_id).await;
    let slow = calls
        .iter()
        .find(|call| call.name == "test_slow")
        .expect("the slow call");
    let expected = format!("slow page\n\n{}", rendered(feedback));
    assert_eq!(
        slow.result_text.as_deref(),
        Some(expected.as_str()),
        "paused feedback is rendered like any other author message"
    );
}

// ───────────────────────────── runtime notices ─────────────────────────────

/// Runtime notices are rendered when queued and delivered as is — no 【用户留言】 is added.
#[tokio::test]
async fn runtime_notices_are_delivered_exactly_as_queued() {
    let runtime = runtime(vec![
        slow_step("slow page", 1_200),
        agent_harness::finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    wait_for_executing(&runtime, session_id).await;

    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/sessions/{session_id}"),
            json!({ "granularity": "sentence" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "patch granularity: {body}");

    let queued = pending_inbox(&runtime, session_id).await;
    assert_eq!(
        queued.len(),
        1,
        "the granularity notice is queued: {queued:?}"
    );
    assert_eq!(queued[0].1, "notice", "runtime notices keep their kind");
    assert!(
        queued[0].2.contains("【系统通知】"),
        "the notice was rendered when queued: {}",
        queued[0].2
    );
    let notice_text = queued[0].2.clone();

    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let slow = calls
        .iter()
        .find(|call| call.name == "test_slow")
        .expect("the slow call");
    let expected = format!("slow page\n\n{notice_text}");
    assert_eq!(
        slow.result_text.as_deref(),
        Some(expected.as_str()),
        "the notice text is delivered unchanged"
    );
    assert!(
        !slow
            .result_text
            .as_deref()
            .unwrap_or_default()
            .contains(HEADER),
        "a runtime notice never gains the author-message header"
    );
    let appendices = inbox_appendices(slow);
    assert_eq!(appendices.len(), 1);
    assert_eq!(appendices[0]["text"], notice_text);
}

// ───────────────────────────── overrides ─────────────────────────────

/// A book override of `notice.user_message` is the text delivered.
#[tokio::test]
async fn a_book_override_of_the_author_message_header_is_honoured() {
    let runtime = runtime(vec![
        agent_harness::text_step("a conception reply"),
        agent_harness::text_step("another reply"),
    ])
    .await;
    let (status, body) = runtime
        .app
        .put(
            "/api/prompts/notice.user_message",
            json!({
                "scope": "book",
                "book_id": runtime.book_id,
                "content": "【作者留言】\n{{ text }}",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save book override: {body}");

    let session_id = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::Conception,
            book_id: Some(runtime.book_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create conception session")
    .0;
    runtime.wait_status(session_id, SessionStatus::Idle).await;
    let text = "用第一人称写。";
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/message"),
            json!({ "text": text }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "message: {body}");
    runtime.wait_status(session_id, SessionStatus::Idle).await;

    let messages = runtime.messages(session_id).await;
    let user = messages
        .iter()
        .find(|message| message.kind == "user")
        .expect("a user message");
    assert_eq!(text_blocks(user), vec![format!("【作者留言】\n{text}")]);
}

/// A broken override falls back to the shipped default at render time (`prompt.override_failed`);
/// the author's text is delivered under the default header, never dropped.
#[tokio::test]
async fn a_broken_override_delivers_the_default_header_and_reports_the_fallback() {
    let runtime = runtime(vec![
        agent_harness::text_step("a conception reply"),
        agent_harness::text_step("another reply"),
    ])
    .await;
    // The save API validates against the sample contexts, so a broken override of this kind can
    // only reach the registry as a stored row (the same situation a pack change creates).
    runtime
        .app
        .execute(&format!(
            "INSERT INTO prompt_overrides(scope, book_id, key, content, updated_at) \
             VALUES ('book', {}, 'notice.user_message', '{{{{ text.missing }}}}', \
             '2026-01-01T00:00:00Z')",
            runtime.book_id
        ))
        .await;
    runtime.app.state().prompts().invalidate_all();

    // The registry already reports the fallback for the session's effective set.
    let prompts = runtime
        .app
        .state()
        .prompts()
        .for_book(runtime.app.db(), Some(runtime.book_id))
        .await
        .expect("prompt set");
    let probe = prompts
        .render("notice.user_message", &json!({ "text": "probe" }))
        .expect("the default renders");
    assert_eq!(
        probe.override_failed.as_deref(),
        Some("notice.user_message"),
        "the registry reports the failed override"
    );
    assert_eq!(probe.warnings(), vec!["prompt.override_failed".to_owned()]);

    let session_id = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::Conception,
            book_id: Some(runtime.book_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create conception session")
    .0;
    runtime.wait_status(session_id, SessionStatus::Idle).await;
    let text = "结尾再收一点。";
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/message"),
            json!({ "text": text }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "message: {body}");
    runtime.wait_status(session_id, SessionStatus::Idle).await;

    let messages = runtime.messages(session_id).await;
    let user = messages
        .iter()
        .find(|message| message.kind == "user")
        .expect("a user message");
    assert_eq!(
        text_blocks(user),
        vec![rendered(text)],
        "the shipped default header is delivered"
    );
}

// ───────────────────────────── several messages in one batch ─────────────────────────────

/// Several steering messages consumed by one sealing keep arrival order and each gets its own
/// header.
#[tokio::test]
async fn several_steering_messages_in_one_batch_keep_order_and_each_header() {
    let runtime = runtime(vec![
        slow_step("slow page", 2_000),
        agent_harness::finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    wait_for_executing(&runtime, session_id).await;

    let texts = ["第一条：多写雨。", "第二条：少写风。", "第三条：写城楼。"];
    for text in texts {
        let (status, body) = runtime
            .app
            .post(
                &format!("/api/sessions/{session_id}/message"),
                json!({ "text": text }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "message {text}: {body}");
    }
    let queued = pending_inbox(&runtime, session_id).await;
    assert_eq!(queued.len(), 3, "three rows are queued: {queued:?}");
    let inbox_ids: Vec<i32> = queued.iter().map(|item| item.0).collect();
    let raw: Vec<&str> = queued.iter().map(|item| item.2.as_str()).collect();
    assert_eq!(raw, texts, "every row keeps its raw text");

    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let slow = calls
        .iter()
        .find(|call| call.name == "test_slow")
        .expect("the slow call");
    let appendices = inbox_appendices(slow);
    let delivered: Vec<&str> = appendices
        .iter()
        .map(|appendix| appendix["text"].as_str().unwrap_or_default())
        .collect();
    let expected: Vec<String> = texts.iter().map(|text| rendered(text)).collect();
    assert_eq!(
        delivered,
        expected.iter().map(String::as_str).collect::<Vec<_>>(),
        "one rendered appendix per message, in arrival order"
    );
    let appendix_ids: Vec<i64> = appendices
        .iter()
        .map(|appendix| appendix["inbox_id"].as_i64().unwrap_or_default())
        .collect();
    assert_eq!(
        appendix_ids,
        inbox_ids.iter().map(|id| *id as i64).collect::<Vec<_>>()
    );
    let result_text = slow.result_text.as_deref().unwrap_or_default();
    assert_eq!(
        result_text.matches(HEADER).count(),
        3,
        "each message gets its own header: {result_text}"
    );
    let mut cursor = 0;
    for text in texts {
        let position = result_text[cursor..]
            .find(text)
            .unwrap_or_else(|| panic!("{text} missing from {result_text}"))
            + cursor;
        assert!(
            position >= cursor,
            "arrival order is preserved in {result_text}"
        );
        cursor = position + text.len();
    }
}
