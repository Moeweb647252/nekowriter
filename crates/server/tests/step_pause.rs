//! Step-pause and keep-write resume (M25 25.2, plan §5.1).

mod agent_write_harness;

use std::time::Duration;

use agent_write_harness::{chapter_content, runtime, start_writer};
use axum::http::StatusCode;
use nekowriter::db::json::{Granularity, SessionStatus};
use nekowriter_llm::mock::ScriptStep;
use serde_json::{Value, json};

fn insert_step(text: &str, complete: bool) -> ScriptStep {
    let mut args = json!({ "text": text });
    if complete {
        args["chapter_end"] = json!(true);
    }
    ScriptStep::new()
        .delay(Duration::from_millis(500))
        .tool("insert_text", args)
}

async fn pause(
    runtime: &agent_write_harness::Runtime,
    id: i32,
    body: Value,
) -> (StatusCode, Value) {
    runtime
        .app
        .post(&format!("/api/sessions/{id}/pause"), body)
        .await
}

async fn resume(
    runtime: &agent_write_harness::Runtime,
    id: i32,
    body: Value,
) -> (StatusCode, Value) {
    runtime
        .app
        .post(&format!("/api/sessions/{id}/resume"), body)
        .await
}

/// A plain insert under a step pause lands, then the session pauses before the next request.
#[tokio::test]
async fn step_pause_finishes_a_plain_insert_then_pauses() {
    let runtime = runtime(vec![
        insert_step("第一段。", false),
        ScriptStep::new().text("后续"),
    ])
    .await;
    let id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let (status, body) = pause(&runtime, id, json!({ "after_step": true })).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let row = runtime.wait_status(id, SessionStatus::Paused).await;
    assert_eq!(row.pause_requested, 0, "leaving running clears the pause");
    assert_eq!(
        row.pause_after_step, 0,
        "leaving running clears the step flag"
    );
    assert_eq!(
        chapter_content(&runtime, runtime.chapter_id)
            .await
            .paragraphs
            .len(),
        1
    );
}

/// A completing insert is held even under a step pause.
#[tokio::test]
async fn step_pause_holds_a_completing_insert() {
    let runtime = runtime(vec![
        ScriptStep::new()
            .delay(Duration::from_millis(500))
            .tool("finish_draft", json!({})),
    ])
    .await;
    runtime
        .app
        .put_text(runtime.chapter_id, 0, &"甲".repeat(3500))
        .await;
    let id = start_writer(&runtime, Granularity::Chapter, false, false).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let (status, body) = pause(&runtime, id, json!({ "after_step": true })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = runtime.wait_status(id, SessionStatus::Held).await;
    assert!(
        row.held_tool_call_id.is_some(),
        "the completing call is held"
    );
    assert!(
        !chapter_content(&runtime, runtime.chapter_id)
            .await
            .paragraphs
            .is_empty(),
        "the pre-existing draft remains"
    );
    let (_, snapshot) = runtime.app.get(&format!("/api/sessions/{id}")).await;
    let calls = snapshot["tool_calls"].as_array().expect("calls");
    assert!(
        calls.iter().any(|call| call["completes"] == true),
        "the held call reports completes: {snapshot}"
    );
}

/// A hold pause sent while a step pause is pending upgrades it and holds the next call.
#[tokio::test]
async fn hold_pause_upgrades_a_step_pause() {
    let runtime = runtime(vec![insert_step("第一段。", false)]).await;
    let id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    let (status, _) = pause(&runtime, id, json!({ "after_step": true })).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = pause(&runtime, id, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = runtime.wait_status(id, SessionStatus::Held).await;
    assert!(row.held_tool_call_id.is_some());
    assert!(
        chapter_content(&runtime, runtime.chapter_id)
            .await
            .paragraphs
            .is_empty()
    );
}

/// A step pause sent after a hold pause leaves the hold pause in charge.
#[tokio::test]
async fn step_pause_does_not_weaken_a_hold_pause() {
    let runtime = runtime(vec![insert_step("第一段。", false)]).await;
    let id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    let (status, _) = pause(&runtime, id, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = pause(&runtime, id, json!({ "after_step": true })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = runtime.wait_status(id, SessionStatus::Held).await;
    assert!(row.held_tool_call_id.is_some());
}

/// Resuming a running session with a pause clears both flags and queues the feedback.
#[tokio::test]
async fn resume_clears_a_running_pause_and_queues_feedback() {
    let runtime = runtime(vec![insert_step("第一段。", false)]).await;
    let id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    let (status, _) = pause(&runtime, id, json!({ "after_step": true })).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = resume(&runtime, id, json!({ "feedback": "请继续" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = runtime.session_row(id).await;
    assert_eq!(row.pause_requested, 0);
    assert_eq!(row.pause_after_step, 0);
    let inbox = runtime.app.get(&format!("/api/sessions/{id}")).await.1;
    assert!(
        inbox["inbox"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["kind"] == "user_message"))
    );
}

/// Running without a pending pause is a 409.
#[tokio::test]
async fn resume_running_without_a_pause_is_conflict() {
    let runtime = runtime(vec![insert_step("第一段。", false)]).await;
    let id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    let (status, body) = resume(&runtime, id, json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

/// Keep-write applies the held write and appends feedback instead of discarding it.
#[tokio::test]
async fn keep_write_applies_the_staged_write_with_feedback() {
    let runtime = runtime(vec![insert_step("保留段落。", false)]).await;
    let id = start_writer(&runtime, Granularity::Paragraph, true, false).await;
    let row = runtime.wait_status(id, SessionStatus::Held).await;
    assert!(row.held_tool_call_id.is_some());
    let (status, body) = resume(
        &runtime,
        id,
        json!({ "feedback": "保留这一段，后面更紧凑", "keep_write": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        chapter_content(&runtime, runtime.chapter_id)
            .await
            .paragraphs
            .len(),
        1,
        "keep_write applies the staged content"
    );
}

/// Keep-write on a completing call revokes the completion, applies the text, and puts the
/// completion notice before the feedback in the sealed result.
#[tokio::test]
async fn keep_write_revokes_a_completion_and_keeps_the_text() {
    let runtime = runtime(vec![
        ScriptStep::new().delay(Duration::from_millis(500)).tool(
            "insert_text",
            json!({ "text": "保留结尾。", "chapter_end": true }),
        ),
    ])
    .await;
    runtime
        .app
        .put_text(runtime.chapter_id, 0, &"甲".repeat(3500))
        .await;
    let id = start_writer(&runtime, Granularity::Chapter, true, false).await;
    let row = runtime.wait_status(id, SessionStatus::Held).await;
    assert!(row.held_tool_call_id.is_some());
    let call_id = row.held_tool_call_id.unwrap();
    let before = runtime.call_by_id(call_id).await.effects();
    assert!(before.complete.is_some(), "the held call completes");
    let (status, body) = resume(
        &runtime,
        id,
        json!({ "feedback": "保留这一版", "keep_write": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let call = runtime.call_by_id(call_id).await;
    let text = call.result_text.unwrap_or_default();
    let completion = text.find("完成").unwrap_or(0);
    let feedback = text.find("保留这一版").unwrap_or(text.len());
    assert!(
        completion <= feedback,
        "completion notice precedes feedback: {text}"
    );
    let row = runtime.session_row(id).await;
    assert_eq!(row.status, "running", "the completion was revoked");
    assert_eq!(
        chapter_content(&runtime, runtime.chapter_id)
            .await
            .paragraphs
            .len(),
        2
    );
}

/// An empty pause body keeps the old hold-pause meaning.
#[tokio::test]
async fn empty_pause_body_is_a_hold_pause() {
    let runtime = runtime(vec![insert_step("第一段。", false)]).await;
    let id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    let (status, body) = runtime
        .app
        .request("POST", &format!("/api/sessions/{id}/pause"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = runtime.wait_status(id, SessionStatus::Held).await;
    assert!(row.held_tool_call_id.is_some());
}

/// H-1: a stale runner copy that saw the pause is not allowed to pause after a concurrent resume
/// cleared it; the guarded update misses and returns `None`.
#[tokio::test]
async fn guarded_pause_misses_after_a_resume_clears_it() {
    use nekowriter::agent::session::{self as agent_session, TransitionCause};
    use nekowriter::db::entities::agent_sessions;
    use nekowriter::db::json::SessionStatus;
    use sea_orm::EntityTrait;

    let runtime = runtime(vec![insert_step("第一段。", false)]).await;
    let id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    let (status, _) = pause(&runtime, id, json!({ "after_step": true })).await;
    assert_eq!(status, StatusCode::OK);
    let stale = agent_sessions::Entity::find_by_id(id)
        .one(runtime.app.db().conn())
        .await
        .expect("query")
        .expect("session");
    let (status, _) = resume(&runtime, id, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let missed = agent_session::transition_if_pause_requested(
        runtime.app.db().conn(),
        &stale,
        SessionStatus::Paused,
        TransitionCause::PauseBeforeRequest,
    )
    .await
    .expect("guarded transition");
    assert!(
        missed.is_none(),
        "the cleared pause must miss the guarded update"
    );
    let current = runtime.session_row(id).await;
    assert_ne!(current.status, "paused");
}

/// M-2: a running-pause resume queues exactly one inbox row and emits a session.updated event.
#[tokio::test]
async fn resume_running_pause_queues_one_message_and_emits_session_updated() {
    use nekowriter::db::entities::agent_inbox;
    use nekowriter::events::ServerEvent;
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

    let runtime = runtime(vec![insert_step("第一段。", false)]).await;
    let id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    let _ = pause(&runtime, id, json!({ "after_step": true })).await;
    let mut events = runtime.app.state().events().subscribe();
    let (status, _) = resume(&runtime, id, json!({ "feedback": "只发一次" })).await;
    assert_eq!(status, StatusCode::OK);
    runtime
        .wait_event(&mut events, Duration::from_secs(5), |event| {
            matches!(event, ServerEvent::SessionUpdated { session, .. } if session.id == id)
        })
        .await;
    let count = agent_inbox::Entity::find()
        .filter(agent_inbox::COLUMN.session_id.eq(id))
        .filter(
            agent_inbox::COLUMN
                .kind
                .eq(nekowriter::db::json::InboxKind::UserMessage),
        )
        .count(runtime.app.db().conn())
        .await
        .expect("inbox count");
    assert_eq!(count, 1, "exactly one feedback message is queued");
}

/// M-1: editing the draft while the write is held makes the keep-write apply a no-op (D-141).
#[tokio::test]
async fn keep_write_drops_a_superseded_staged_write() {
    let runtime = runtime(vec![insert_step("代理段落。", false)]).await;
    let id = start_writer(&runtime, Granularity::Paragraph, true, false).await;
    let row = runtime.wait_status(id, SessionStatus::Held).await;
    assert!(row.held_tool_call_id.is_some());
    runtime
        .app
        .put_text(runtime.chapter_id, 0, "作者改写后的段落。")
        .await;
    let (status, _) = resume(
        &runtime,
        id,
        json!({ "feedback": "保留我的版本，继续", "keep_write": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs[0].text, "作者改写后的段落。");
}

/// S-1: entering `running` from `paused` clears both pause flags, including a ling step flag.
#[tokio::test]
async fn resume_from_paused_clears_a_lingering_step_flag() {
    use nekowriter::db::entities::agent_sessions;
    use sea_orm::{EntityTrait, QueryFilter};

    let runtime = runtime(vec![insert_step("第一段。", false)]).await;
    let id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let _ = pause(&runtime, id, json!({ "after_step": true })).await;
    runtime.wait_status(id, SessionStatus::Paused).await;
    agent_sessions::Entity::update_many()
        .set(agent_sessions::ActiveModel {
            pause_after_step: sea_orm::Set(true),
            ..Default::default()
        })
        .filter(agent_sessions::COLUMN.id.eq(id))
        .exec(runtime.app.db().conn())
        .await
        .expect("seed a leaked flag");
    let (status, body) = resume(&runtime, id, json!({ "feedback": "继续" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = runtime.session_row(id).await;
    assert_eq!(row.pause_requested, 0);
    assert_eq!(row.pause_after_step, 0, "set_status clears both flags");
}
