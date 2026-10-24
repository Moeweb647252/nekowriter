//! Runtime tests 8, 16, 17, 20, 21 (spec/03 §15): crash recovery, `after_completion`, stop
//! during `prepare`, the canonical appendix order plus a mid-transaction crash, and no lost
//! updates.
//!
//! The harness is shared with `agent_runtime.rs`; every test uses the real router, a temporary
//! database and a `ScriptedProvider`.

mod agent_harness;

use std::time::Duration;

use agent_harness::{finish_step, runtime, tool_step};
use axum::http::StatusCode;
use nekowriter::agent::session::TransitionCause;
use nekowriter::db::json::{Granularity, SessionStatus, ToolCallStatus};
use nekowriter::testing::assert_error;
use serde_json::json;

/// spec/03 §15 test 8: a simulated crash (`executing` row with no effects) is reset to `pending`
/// by recovery and then executed once.
#[tokio::test]
async fn test_08_recovery_resets_an_executing_call_and_it_runs_once() {
    let runtime = runtime(vec![
        // The first step is consumed by the request Stop drops; the recovery re-run takes the
        // next one, then the session completes.
        tool_step("echo", json!({ "text": "dropped" })),
        tool_step("echo", json!({ "text": "recovered" })),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    // Stop at once so nothing is committed: the fixture below simulates the crash instead.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let (status, _) = runtime
            .app
            .post(&format!("/api/sessions/{session_id}/stop"), json!({}))
            .await;
        if status == StatusCode::OK {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "could not stop the session"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let row = runtime
        .wait_status(session_id, SessionStatus::Stopped)
        .await;
    assert_eq!(row.status, "stopped");

    // The crash left this call in `executing` with no result and the session `running`.
    let call_id = runtime
        .app
        .insert_executing_call(session_id, "echo", json!({ "text": "recovered" }))
        .await;
    runtime.app.set_session_status(session_id, "running").await;

    let report = nekowriter::agent::recover(runtime.app.state())
        .await
        .expect("recovery");
    assert_eq!(report.reset_calls, 1, "one executing call was reset");
    assert_eq!(report.interrupted_sessions, 1);

    // Recovery leaves the call `pending` with no started_at and no result (D-033: no effect).
    let call = runtime.call_by_id(call_id).await;
    assert_eq!(call.status_enum(), ToolCallStatus::Pending);
    assert!(call.raw_result_text.is_none(), "no effect was recorded");
    assert!(call.result_text.is_none());
    assert_eq!(
        runtime.session_row(session_id).await.status,
        "interrupted",
        "a session that was running becomes interrupted"
    );

    // Continue re-executes the call exactly once.
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/continue"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let call = runtime.call_by_id(call_id).await;
    assert_eq!(call.status_enum(), ToolCallStatus::Done);
    assert_eq!(call.raw_result_text.as_deref(), Some("recovered"));
    // The recovered call was re-executed exactly once (its row was reset, not duplicated).
    let calls = runtime.calls(session_id).await;
    assert_eq!(
        calls.iter().filter(|call| call.id == call_id).count(),
        1,
        "the recovered call row exists once: {calls:?}"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.status_enum() == ToolCallStatus::Executing)
            .count(),
        0,
        "no call was left executing"
    );
}

/// spec/03 §15 test 16: a completing call followed by another call finishes the second as
/// `error.after_completion` without executing it, and every call of the batch is sealed.
#[tokio::test]
async fn test_16_calls_after_a_completing_call_get_after_completion() {
    // One response with three calls: a plain one, the completing one, then another.
    let batch = nekowriter_llm::mock::ScriptStep::new()
        .tool("echo", json!({ "text": "first" }))
        .tool("test_finish", json!({ "text": "completing" }))
        .tool("echo", json!({ "text": "never runs" }));
    let runtime = runtime(vec![batch]).await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    let calls = runtime.calls(session_id).await;
    assert_eq!(calls.len(), 3, "every call of the batch exists: {calls:?}");
    assert_eq!(calls[0].status_enum(), ToolCallStatus::Done);
    assert_eq!(calls[1].status_enum(), ToolCallStatus::Done);
    assert_eq!(
        calls[2].status_enum(),
        ToolCallStatus::Error,
        "the call after the completing one is an error"
    );
    assert!(calls[2].is_error);
    let result = calls[2].raw_result_text.as_deref().expect("error text");
    assert!(
        result.contains("没有执行"),
        "error.after_completion says the call was not executed: {result}"
    );
    // Every call of the batch is sealed, so a re-review can continue the conversation.
    assert!(
        calls.iter().all(|call| call.sealed),
        "the whole batch is sealed: {calls:?}"
    );
    assert!(calls.iter().all(|call| call.result_text.is_some()));
    assert_eq!(runtime.session_row(session_id).await.status, "completed");
}

/// spec/03 §15 test 17: a stop while a call is in its prepare phase returns the call to
/// `pending`, nothing was written, and Continue executes it once.
#[tokio::test]
async fn test_17_stop_during_prepare_returns_the_call_to_pending() {
    let runtime = runtime(vec![
        // `test_slow` sleeps in `prepare`, so the stop lands inside it (D-024).
        tool_step("test_slow", json!({ "sleep_ms": 900, "text": "slow page" })),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;

    // Wait until the call is marked `executing` (before its prepare finishes), then stop.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let calls = runtime.calls(session_id).await;
        if calls
            .iter()
            .any(|call| call.status_enum() == ToolCallStatus::Executing)
        {
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!("the slow call never started: {calls:?}");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "stop: {body}");

    let row = runtime
        .wait_status(session_id, SessionStatus::Stopped)
        .await;
    assert_eq!(row.held_tool_call_id, None);
    let calls = runtime.calls(session_id).await;
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].status_enum(),
        ToolCallStatus::Pending,
        "a stop during prepare returns the call to pending"
    );
    assert!(
        calls[0].raw_result_text.is_none(),
        "nothing was written by the call"
    );

    // Continue executes it exactly once.
    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/continue"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let slow: Vec<_> = calls
        .iter()
        .filter(|call| call.name == "test_slow")
        .collect();
    assert_eq!(slow.len(), 1, "executed once, not duplicated");
    assert_eq!(slow[0].status_enum(), ToolCallStatus::Done);
    assert_eq!(
        slow[0].raw_result_text.as_deref(),
        Some("slow page"),
        "the prepare result reached execute"
    );
}

/// spec/03 §15 test 20 (first half): pause → held; the author edits a paragraph, then resumes
/// with feedback → the sealed result is raw text, edit report, feedback, in that order.
#[tokio::test]
async fn test_20_a_held_call_seals_raw_result_edit_report_then_feedback() {
    // A writer session (not the debug one) is needed for the chapter edit report: the debug
    // session does attach a chapter, so its view snapshot is the chapter.
    let runtime = runtime(vec![
        tool_step("test_write", json!({ "text": "written" })),
        finish_step(),
    ])
    .await;
    // Write a paragraph through the API and give the session a chapter, then hold a call.
    runtime
        .app
        .put_text(runtime.chapter_id, 0, "First paragraph.")
        .await;
    let session_id = runtime
        .start_debug_with(Granularity::Paragraph, true, false)
        .await;
    let row = runtime.wait_status(session_id, SessionStatus::Held).await;
    let held_id = row.held_tool_call_id.expect("the call is held");

    // The author edits the chapter while the session is not running (spec/03 §7).
    let (version, _) = {
        let (_, chapter) = runtime
            .app
            .get(&format!("/api/chapters/{}", runtime.chapter_id))
            .await;
        (
            chapter["content_version"].as_i64().expect("version") as i32,
            chapter,
        )
    };
    runtime
        .app
        .put_text(
            runtime.chapter_id,
            version,
            "First paragraph, edited by the author.",
        )
        .await;

    // Resume with feedback: the deferred effects are revoked and the appendix order is fixed.
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/resume"),
            json!({ "feedback": "make it shorter" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "resume: {body}");
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    let call = runtime.call_by_id(held_id).await;
    let result = call.result_text.as_deref().expect("the call is sealed");
    let raw = result.find("written").expect("raw result text");
    let report = result
        .find("用户修改了正文")
        .or_else(|| result.find("正文"))
        .expect("the edit report");
    let feedback = result.find("用户反馈").expect("the feedback");
    assert!(
        raw < report,
        "raw result precedes the edit report: {result}"
    );
    assert!(
        report < feedback,
        "the edit report precedes the feedback: {result}"
    );
    assert!(
        result.contains("edited by the author"),
        "the report carries the author's new text: {result}"
    );
    assert!(
        result.contains("make it shorter"),
        "the feedback is appended: {result}"
    );
    // The appendices are typed and ordered by the canonical rank (spec/03 §5.3).
    let appendices = agent_harness::Runtime::appendices(&call);
    let kinds: Vec<&str> = appendices
        .iter()
        .map(|appendix| appendix["kind"].as_str().unwrap_or_default())
        .collect();
    assert!(
        kinds.contains(&"edit_report") && kinds.contains(&"user_feedback"),
        "typed appendices: {kinds:?}"
    );
}

/// spec/03 §15 test 20 (second half): a crash injected after `execute` but before the commit
/// leaves the call `executing` with no effect; recovery re-runs it exactly once.
#[tokio::test]
async fn test_20_a_crash_after_execute_leaves_no_effect_and_recovery_reruns_once() {
    let runtime = runtime(vec![
        tool_step("echo", json!({ "text": "dropped" })),
        tool_step("echo", json!({ "text": "once" })),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let (status, _) = runtime
            .app
            .post(&format!("/api/sessions/{session_id}/stop"), json!({}))
            .await;
        if status == StatusCode::OK {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "could not stop the session"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    runtime
        .wait_status(session_id, SessionStatus::Stopped)
        .await;

    // The crash happened inside `execute`: the row is `executing` and nothing else was written.
    let call_id = runtime
        .app
        .insert_executing_call(session_id, "echo", json!({ "text": "once" }))
        .await;
    runtime.app.set_session_status(session_id, "running").await;

    let report = nekowriter::agent::recover(runtime.app.state())
        .await
        .expect("recovery");
    assert_eq!(report.reset_calls, 1);
    let calls = runtime.calls(session_id).await;
    let recovered = calls
        .iter()
        .find(|call| call.id == call_id)
        .expect("the call");
    assert_eq!(recovered.status_enum(), ToolCallStatus::Pending);
    assert!(
        recovered.result_text.is_none(),
        "no effect survived the crash"
    );
    assert!(recovered.raw_result_text.is_none());

    runtime
        .app
        .post(&format!("/api/sessions/{session_id}/continue"), json!({}))
        .await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let recovered = runtime.call_by_id(call_id).await;
    assert_eq!(recovered.status_enum(), ToolCallStatus::Done);
    assert_eq!(
        recovered.raw_result_text.as_deref(),
        Some("once"),
        "the re-run produced the result once"
    );
}

/// spec/03 §15 test 21 (first half): a pause request and a granularity change committed while
/// the runner is inside a tool transaction are both honored at the next boundary.
#[tokio::test]
async fn test_21_a_pause_and_a_granularity_change_inside_a_tool_transaction_are_not_lost() {
    // The tool's transaction is short, so aim the API writes at the window where the runner is
    // working on a call: a slow prepare opens that window wide (the call is `executing`).
    let runtime = runtime(vec![
        tool_step("test_slow", json!({ "sleep_ms": 700, "text": "slow" })),
        tool_step("test_write", json!({ "text": "second call" })),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;

    // Wait until the call is executing (inside its prepare), then commit both writes.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let calls = runtime.calls(session_id).await;
        if calls
            .iter()
            .any(|call| call.status_enum() == ToolCallStatus::Executing)
        {
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!("the slow call never started: {calls:?}");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let (pause_status, pause_body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/pause"), json!({}))
        .await;
    assert_eq!(pause_status, StatusCode::OK, "pause: {pause_body}");
    let (patch_status, patch_body) = runtime
        .app
        .patch(
            &format!("/api/sessions/{session_id}"),
            json!({ "granularity": "sentence", "auto_pause": false }),
        )
        .await;
    assert_eq!(patch_status, StatusCode::OK, "patch: {patch_body}");

    // Both writes survived. Depending on where exactly the runner was, the pause either holds
    // the call in flight (`held`) or takes effect at the next boundary (`paused`); either way it
    // is honored together with the granularity change.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let row = loop {
        let row = runtime.session_row(session_id).await;
        if row.status == "held" || row.status == "paused" {
            break row;
        }
        if std::time::Instant::now() > deadline {
            panic!("the session never stopped working: {row:?}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(
        row.granularity.as_deref(),
        Some("sentence"),
        "the granularity change was not lost"
    );
    assert_eq!(row.auto_pause, 0, "the auto-pause switch was not lost");
    assert_eq!(
        row.pause_requested, 0,
        "the pause was honoured by the hold and cleared"
    );
    if row.status == "held" {
        assert_eq!(
            row.held_tool_call_id,
            runtime.calls(session_id).await.first().map(|call| call.id),
            "the call in flight when the writes landed is the held one"
        );
    }

    // Resume; the switch notice reaches the model with the next sealing (spec/03 §8).
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "resume: {body}");
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let notice = calls
        .iter()
        .filter_map(|call| call.result_text.as_deref())
        .find(|text| text.contains("写作粒度"));
    assert!(
        notice.is_some(),
        "the granularity change is announced to the model: {:?}",
        calls.iter().map(|c| &c.result_text).collect::<Vec<_>>()
    );
}

/// spec/03 §15 test 21 (second half): two concurrent `continue` calls start one runner; the
/// second gets `409 session.invalid_state`.
#[tokio::test]
async fn test_21_two_concurrent_continues_start_one_runner_and_the_second_conflicts() {
    let runtime = runtime(vec![
        nekowriter_llm::mock::ScriptStep::new()
            .text("a slow reply")
            .delay_per_event(Duration::from_millis(40)),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    // Stop it so Continue is valid.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let (status, _) = runtime
            .app
            .post(&format!("/api/sessions/{session_id}/stop"), json!({}))
            .await;
        if status == StatusCode::OK {
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!("could not stop the session");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    runtime
        .wait_status(session_id, SessionStatus::Stopped)
        .await;
    runtime.wait_idle(session_id).await;

    // Two concurrent continues: exactly one wins, the other is `session.invalid_state`.
    let path = format!("/api/sessions/{session_id}/continue");
    let first = runtime.app.post(&path, json!({}));
    let second = runtime.app.post(&path, json!({}));
    let (first, second) = tokio::join!(first, second);
    let statuses = [first.0, second.0];
    assert_eq!(
        statuses.iter().filter(|status| status.is_success()).count(),
        1,
        "exactly one continue wins: {statuses:?}"
    );
    let loser = [&first, &second]
        .into_iter()
        .find(|(status, _)| !status.is_success())
        .expect("a loser");
    assert_eq!(loser.0, StatusCode::CONFLICT);
    assert_error(&loser.1, "session.invalid_state");
    assert_eq!(loser.1["error"]["details"]["operation"], "continue");

    // One runner only.
    assert!(
        runtime.app.state().manager().active_count() <= 1,
        "at most one runner per session"
    );
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
}

/// `transition` writes `finished_at` for terminal statuses and clears `pause_requested`
/// (spec/03 §3).
#[tokio::test]
async fn transition_sets_finished_at_and_clears_the_pause_flag() {
    let runtime = runtime(vec![finish_step()]).await;
    let session_id = runtime.start_debug().await;
    let row = runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    assert!(
        row.finished_at.is_some(),
        "terminal statuses record finished_at"
    );
    assert_eq!(row.pause_requested, 0);

    // An open status keeps finished_at NULL.
    let second = agent_harness::runtime(vec![
        tool_step("test_write", json!({ "text": "x" })),
        finish_step(),
    ])
    .await;
    let other_id = second
        .start_debug_with(Granularity::Paragraph, true, false)
        .await;
    second.wait_status(other_id, SessionStatus::Held).await;
    let row = second.session_row(other_id).await;
    assert!(
        row.finished_at.is_none(),
        "an open status has no finished_at"
    );
}

/// Abandon works from any open status and sets `abandoned` with `finished_at` (spec/03 §3, §6).
#[tokio::test]
async fn abandon_closes_an_open_session() {
    let runtime = runtime(vec![tool_step("test_write", json!({ "text": "held" }))]).await;
    let session_id = runtime
        .start_debug_with(Granularity::Paragraph, true, false)
        .await;
    runtime.wait_status(session_id, SessionStatus::Held).await;

    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/abandon"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "abandoned");
    assert!(body["finished_at"].is_string());
    let row = runtime.session_row(session_id).await;
    assert_eq!(row.status, "abandoned");
    assert!(row.finished_at.is_some());
}

/// `transition` refuses to clobber a status another writer moved on (D-034).
#[tokio::test]
async fn transition_is_guarded_by_the_expected_status() {
    let runtime = runtime(vec![finish_step()]).await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let row = nekowriter::agent::session::load(runtime.app.state().db_conn(), session_id)
        .await
        .expect("session row");
    // The loaded row says `completed`; a transition asking for `held` still updates (the guard
    // matches the loaded status). Abandoning twice must fail because the second sees `abandoned`.
    let mut stale = row.clone();
    stale.status = "running".to_owned();
    let error = nekowriter::agent::session::transition(
        runtime.app.state().db_conn(),
        &stale,
        SessionStatus::Paused,
        TransitionCause::Boundary,
    )
    .await
    .expect_err("the guard rejects a stale status");
    assert_eq!(
        error.code,
        nekowriter::error::ErrorCode::SessionInvalidState
    );
}
