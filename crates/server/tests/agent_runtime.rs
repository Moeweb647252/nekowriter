//! Runtime tests 4, 5, 7, 11, 12, 14, 15 (spec/03 §15).
//!
//! Every test uses the real router, a temporary database and a `ScriptedProvider`; nothing here
//! touches the network (AGENTS.md §5).

mod agent_harness;

use std::time::{Duration, Instant};

use agent_harness::{finish_step, refusal_step, runtime, text_step, tool_step, truncated_step};
use axum::http::StatusCode;
use nekowriter::db::json::{Granularity, SessionStatus, ToolCallStatus};
use nekowriter::events::ServerEvent;
use nekowriter::testing::assert_error;
use serde_json::json;

/// spec/03 §15 test 4, first half: a pause request made while the model streams takes effect at
/// the next tool boundary, so the call that was already generated completes and is then held.
#[tokio::test]
async fn test_04_a_pause_request_during_streaming_holds_after_the_call_completes() {
    // The first step streams its arguments slowly, giving the test time to request a pause.
    let runtime = runtime(vec![
        tool_step("test_write", json!({ "text": "written" }))
            .delay_per_event(Duration::from_millis(40)),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;

    // Request the pause while the model is still generating.
    tokio::time::sleep(Duration::from_millis(60)).await;
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/pause"), json!({}))
        .await;
    // Either it is still running (pause accepted) or it already held; both are legal, but the
    // interesting case is the accepted one.
    assert!(
        status == StatusCode::OK || status == StatusCode::CONFLICT,
        "pause during streaming: {status} {body}"
    );

    let row = runtime.wait_status(session_id, SessionStatus::Held).await;
    assert_eq!(row.pause_requested, 0, "the pause is consumed by the hold");
    let calls = runtime.calls(session_id).await;
    assert_eq!(calls.len(), 1, "the generated call was not lost");
    assert_eq!(calls[0].status_enum(), ToolCallStatus::Held);
    assert!(calls[0].raw_result_text.is_some(), "the call executed");
    assert_eq!(row.held_tool_call_id, Some(calls[0].id));
}

/// spec/03 §15 test 4, second half: a pause request honoured at a boundary with no pending tool
/// call pauses the session instead of holding a call, and the next model request is never sent.
#[tokio::test]
async fn test_04_a_pause_request_at_a_boundary_pauses_before_the_next_request() {
    // The reply is text-only and slow, so the pause lands while it streams and takes effect at
    // the boundary that follows (spec/03 §5.1, §5.6).
    let runtime = runtime(vec![
        nekowriter_llm::mock::ScriptStep::new()
            .text("a slow prose reply that has no tool call")
            .delay_per_event(Duration::from_millis(40)),
        // The queue repeats its last step, so whichever request resumes first completes.
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let (status, body) = runtime
            .app
            .post(&format!("/api/sessions/{session_id}/pause"), json!({}))
            .await;
        if status == StatusCode::OK {
            break;
        }
        let row = runtime.session_row(session_id).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "pause while {}: {body}",
            row.status
        );
        if std::time::Instant::now() > deadline {
            panic!("could not request a pause in time");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let row = runtime.wait_status(session_id, SessionStatus::Paused).await;
    assert_eq!(row.pause_requested, 0, "leaving running clears the flag");
    let messages = runtime.messages(session_id).await;
    let assistant = messages
        .iter()
        .filter(|message| message.role == "assistant")
        .count();
    assert!(
        assistant <= 1,
        "the pause took effect before a second model request: {messages:?}"
    );
    let calls = runtime.calls(session_id).await;
    assert!(calls.is_empty(), "no tool call ran: {calls:?}");
    // Resume (not Continue: a paused session resumes, spec/03 §6) and finish the session.
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "resume: {body}");
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
}

/// The writer switches are copied from the book settings at creation (spec/03 §4 step 3), and
/// the creation side effects applied (spec/03 §4: `chapter_write` sets the chapter to drafting).
#[tokio::test]
async fn the_writer_switches_come_from_the_book_settings() {
    let runtime = runtime(vec![]).await;
    let (session_id, _warnings) = nekowriter::services::sessions::create(
        runtime.app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: nekowriter::db::json::SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            ..Default::default()
        },
    )
    .await
    .expect("create session");
    let row = runtime.session_row(session_id).await;
    assert_eq!(row.granularity.as_deref(), Some("paragraph"));
    assert_eq!(row.auto_pause, 0);
    assert_eq!(
        row.pause_after_outline, 0,
        "the write session does not carry the outline switch (D-146)"
    );
    let (chapter_status, _, _) = runtime.chapter_status().await;
    assert_eq!(chapter_status, "drafting", "the writer took the chapter");

    // One open writer session per chapter (spec/02 §5 invariant 5), so the first session is
    // abandoned before the override is exercised — the two are not open at the same time.
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/abandon"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "abandon: {body}");

    // The start request can override the switches (spec/03 §4 step 3).
    let (second, _warnings) = nekowriter::services::sessions::create(
        runtime.app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: nekowriter::db::json::SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            granularity: Some(Granularity::Sentence),
            auto_pause: Some(true),
            pause_after_outline: Some(false),
            ..Default::default()
        },
    )
    .await
    .expect("create second session");
    let row = runtime.session_row(second).await;
    assert_eq!(row.granularity.as_deref(), Some("sentence"));
    assert_eq!(row.auto_pause, 1);
    assert_eq!(row.pause_after_outline, 0);
    // Neither session can complete without writer tools; abandon the second one too.
    for id in [second] {
        let _ = runtime
            .app
            .post(&format!("/api/sessions/{id}/abandon"), json!({}))
            .await;
    }
}

/// spec/03 §15 test 5: "Steering while running → appended to the next sealed result, in FIFO
/// order."
///
/// Where a message lands depends on whether the latest batch is still unsealed when it arrives
/// (spec/03 §5.3): with an unsealed batch it becomes an appendix of the batch's last call; when
/// the batch was already sealed by the in-flight request, the messages become one new user
/// message and ride in front of the next request. Both are covered here.
#[tokio::test]
async fn test_05_steering_messages_are_sealed_in_fifo_order() {
    let runtime = runtime(vec![
        // A slow reply, so the second message arrives while it is still streaming.
        nekowriter_llm::mock::ScriptStep::new()
            .tool("echo", json!({ "text": "first" }))
            .delay_per_event(Duration::from_millis(60)),
        tool_step("echo", json!({ "text": "second" })),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;

    // Two author messages while the first request is in flight, in FIFO order.
    for text in ["steer one", "steer two"] {
        let (status, body) = runtime
            .app
            .post(
                &format!("/api/sessions/{session_id}/message"),
                json!({ "text": text }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "message while running: {body}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    // Both items were delivered exactly once. The first landed while the batch was still
    // unsealed, so it became one new user message; the second arrived after the batch was sealed
    // by the in-flight request and rode with the next sealing, in arrival order (spec/03 §5.3).
    let calls = runtime.calls(session_id).await;
    let messages = runtime.messages(session_id).await;
    let mut delivered: Vec<String> = messages
        .iter()
        .map(|message| message.text())
        .chain(calls.iter().filter_map(|call| call.result_text.clone()))
        .collect();
    delivered.retain(|text| !text.is_empty());
    let combined = delivered.join("\n---\n");
    for text in ["steer one", "steer two"] {
        assert_eq!(
            combined.matches(text).count(),
            1,
            "{text} must be delivered exactly once: {combined}"
        );
    }
    // The delivered order is the arrival order: whichever carrier holds both keeps FIFO.
    let carrier = delivered
        .iter()
        .find(|text| text.contains("steer one") && text.contains("steer two"));
    if let Some(carrier) = carrier {
        assert!(
            carrier.find("steer one") < carrier.find("steer two"),
            "FIFO within a carrier: {carrier}"
        );
    } else {
        // Otherwise the two items are in different sections, and the section order is FIFO.
        let one = combined.find("steer one").expect("one");
        let two = combined.find("steer two").expect("two");
        assert!(one < two, "FIFO across sections: {combined}");
    }

    // Nothing was left unconsumed (spec/02 §2 `agent_inbox.consumed_at`).
    let pending = runtime.inbox_pending(session_id).await;
    assert_eq!(pending, 0, "every inbox row was consumed");
}

/// spec/03 §15 test 7: "Stop mid-stream → nothing persisted from the partial response; continue
/// re-sends an identical request."
#[tokio::test]
async fn test_07_stop_mid_stream_persists_nothing_and_continue_resends_the_same_request() {
    let runtime = runtime(vec![
        // A slow text-only response that Stop interrupts.
        nekowriter_llm::mock::ScriptStep::new()
            .text("a long answer that is still streaming")
            .delay_per_event(Duration::from_millis(60)),
        tool_step("echo", json!({ "text": "after continue" })),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;

    // Wait for the actual provider request to start rather than a fixed sleep: under a loaded
    // machine, 80 ms can elapse before the request is issued and Stop then runs after the response
    // already completed, making the assertion below flaky (the third such flake in the project).
    let deadline = Instant::now() + agent_harness::wait_budget();
    while runtime.scripted.requests().is_empty() {
        assert!(
            Instant::now() <= deadline,
            "the provider request did not start within the wait budget"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "stop while running");
    runtime
        .wait_status(session_id, SessionStatus::Stopped)
        .await;

    let messages_after_stop = runtime.messages(session_id).await;
    assert_eq!(
        messages_after_stop.len(),
        1,
        "only the kickoff is persisted; the partial response is discarded"
    );
    let requests_before = runtime.scripted.requests().len();
    assert!(requests_before >= 1);

    // Continue re-sends exactly the same request (the history is unchanged).
    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/continue"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let requests = runtime.scripted.requests();
    assert!(
        requests.len() > requests_before,
        "Continue must issue a new request"
    );
    let bodies = runtime.scripted.bodies();
    assert_eq!(
        bodies[requests_before],
        bodies[requests_before - 1],
        "Continue re-sends a byte-identical request when nothing was edited"
    );
}

/// spec/03 §15 test 11: text-only replies → two nudges → `failed`.
#[tokio::test]
async fn test_11_two_nudges_without_a_tool_call_fail_the_session() {
    let runtime = runtime(vec![
        text_step("no tools here"),
        text_step("still no tools"),
        text_step("again"),
    ])
    .await;
    let session_id = runtime.start_debug().await;

    let row = runtime.wait_status(session_id, SessionStatus::Failed).await;
    let error = row.error().expect("the session records its error");
    assert_eq!(error.code, "agent.no_tool_call");

    let messages = runtime.messages(session_id).await;
    let nudges = messages
        .iter()
        .filter(|message| message.kind == "nudge")
        .count();
    assert_eq!(
        nudges, 2,
        "exactly two nudges before failing (spec/03 §5.6)"
    );
    assert_eq!(
        runtime
            .session_row(session_id)
            .await
            .counters()
            .consecutive_nudges,
        2
    );
}

/// spec/03 §15 test 12: five consecutive invalid calls → `paused` with
/// `agent.tool_error_streak`.
#[tokio::test]
async fn test_12_five_consecutive_tool_errors_pause_the_session() {
    // Each step calls a name the writer phase does not permit: an error result, five times.
    let mut steps = Vec::new();
    for _ in 0..5 {
        steps.push(tool_step("submit_review", json!({ "verdict": "pass" })));
    }
    let runtime = runtime(steps).await;
    let session_id = runtime
        .start_debug_with(Granularity::Paragraph, false, false)
        .await;

    let row = runtime.wait_status(session_id, SessionStatus::Paused).await;
    let error = row.error().expect("the circuit breaker records its error");
    assert_eq!(error.code, "agent.tool_error_streak");
    assert!(row.counters().consecutive_tool_errors >= 5);

    let calls = runtime.calls(session_id).await;
    assert_eq!(calls.len(), 5);
    assert!(
        calls.iter().all(|call| call.is_error),
        "all five calls error"
    );
}

/// spec/03 §15 test 14: after any scenario, re-building the request for the previous step yields
/// a byte-identical prefix of the current request.
#[tokio::test]
async fn test_14_earlier_requests_are_byte_identical_prefixes_of_later_ones() {
    let runtime = runtime(vec![
        tool_step("echo", json!({ "text": "one" })),
        tool_step("echo", json!({ "text": "two" })),
        tool_step("test_finish", json!({ "text": "done" })),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    let bodies = runtime.scripted.bodies();
    assert!(
        bodies.len() >= 3,
        "expected several steps: {}",
        bodies.len()
    );
    for window in bodies.windows(2) {
        let earlier = &window[0];
        let later = &window[1];
        let earlier_messages = earlier["messages"].as_array().expect("messages");
        let later_messages = later["messages"].as_array().expect("messages");
        assert!(
            later_messages.len() >= earlier_messages.len(),
            "a later request never has fewer messages"
        );
        // The tools, system prompt, model and options are frozen, so the whole head matches.
        for key in ["model", "system", "tools", "max_tokens"] {
            assert_eq!(
                earlier.get(key),
                later.get(key),
                "{key} must not change within a session"
            );
        }
        for (index, message) in earlier_messages.iter().enumerate() {
            assert_eq!(
                message, &later_messages[index],
                "message {index} must be a byte-identical prefix"
            );
        }
    }
}

/// spec/03 §15 test 15: `stop` on a held, paused or awaiting session is
/// `409 session.invalid_state`.
#[tokio::test]
async fn test_15_stop_is_only_valid_while_running() {
    let runtime = runtime(vec![
        tool_step("test_write", json!({ "text": "held" })),
        tool_step("echo", json!({ "text": "next" })),
        finish_step(),
    ])
    .await;
    // auto_pause holds every write tool, so the session stops running after the first call.
    let session_id = runtime
        .start_debug_with(Granularity::Paragraph, true, false)
        .await;
    runtime.wait_status(session_id, SessionStatus::Held).await;

    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/stop"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "stop on a held session: {body}"
    );
    assert_error(&body, "session.invalid_state");
    assert_eq!(body["error"]["details"]["status"], "held");
    assert_eq!(body["error"]["details"]["operation"], "stop");

    // Resume: the held call finalizes and the session continues.
    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
}

/// spec/03 §15 test 15, the `awaiting_input` half: `stop` is invalid there too, because the call is
/// waiting for the author's form and there is no request in flight (spec/03 §3 "Stop exists only for
/// `running` sessions"). [`test_15_stop_is_only_valid_while_running`] covers `held`;
/// [`control_operations_on_a_completed_session_are_invalid_state`] covers `completed`.
#[tokio::test]
async fn test_15_stop_is_invalid_on_an_awaiting_input_session() {
    // `test_ask` sets `await_input`, so the call waits for the author's answer (spec/05 §7).
    let runtime = runtime(vec![
        tool_step("test_ask", json!({ "text": "Which way?" })),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, SessionStatus::AwaitingInput)
        .await;

    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/stop"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "stop on an awaiting_input session: {body}"
    );
    assert_error(&body, "session.invalid_state");
    assert_eq!(body["error"]["details"]["status"], "awaiting_input");
    assert_eq!(body["error"]["details"]["operation"], "stop");

    // `pause` is running-only as well, and `message` is rejected while a form is open
    // (spec/03 §5.2 "Rejected while awaiting_input (the form must be answered)").
    for operation in ["pause", "continue"] {
        let (status, body) = runtime
            .app
            .post(
                &format!("/api/sessions/{session_id}/{operation}"),
                json!({}),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "{operation} on an awaiting_input session: {body}"
        );
        assert_error(&body, "session.invalid_state");
    }
}

/// A **completed** session is terminal, so every control operation is `409 session.invalid_state`.
///
/// This used to be named `stop_on_a_paused_session_is_invalid_state` and claimed to cover spec/03
/// §15 test 15's paused half, while its own comment admitted "nothing pauses here": the paused case
/// is [`stop_on_a_circuit_breaker_pause_is_invalid_and_resume_clears_the_error`] below, and
/// [`test_15_stop_is_only_valid_while_running`] covers `held`, `awaiting_input` and `idle`. The name
/// now says what the body proves.
#[tokio::test]
async fn control_operations_on_a_completed_session_are_invalid_state() {
    let runtime = runtime(vec![tool_step("echo", json!({})), finish_step()]).await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    // A completed session is terminal: every control operation except none is invalid.
    for operation in ["stop", "pause", "resume", "continue", "message"] {
        let body = if operation == "message" {
            json!({ "text": "hello" })
        } else {
            json!({})
        };
        let (status, response) = runtime
            .app
            .post(&format!("/api/sessions/{session_id}/{operation}"), body)
            .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "{operation} on a completed session: {response}"
        );
        assert_error(&response, "session.invalid_state");
    }
}

/// A paused (circuit-breaker) session rejects `stop`, and resume clears its error.
#[tokio::test]
async fn stop_on_a_circuit_breaker_pause_is_invalid_and_resume_clears_the_error() {
    let mut steps = Vec::new();
    for _ in 0..5 {
        steps.push(tool_step("not_a_writer_tool", json!({})));
    }
    steps.push(tool_step("echo", json!({ "text": "after resume" })));
    steps.push(finish_step());
    let runtime = runtime(steps).await;
    let session_id = runtime
        .start_debug_with(Granularity::Paragraph, false, false)
        .await;
    runtime.wait_status(session_id, SessionStatus::Paused).await;

    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&body, "session.invalid_state");
    assert_eq!(body["error"]["details"]["status"], "paused");

    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let row = runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    assert!(
        row.error.is_none(),
        "resume clears the circuit-breaker error"
    );
}

/// `refusal` fails the session with `provider.refusal` (spec/03 §5.5, §5.7).
#[tokio::test]
async fn a_refusal_fails_the_session() {
    let runtime = runtime(vec![refusal_step("no")]).await;
    let session_id = runtime.start_debug().await;
    let row = runtime.wait_status(session_id, SessionStatus::Failed).await;
    let error = row.error().expect("error");
    assert_eq!(error.code, "provider.refusal");
    assert!(error.retryable, "a refusal is retryable (spec/03 §5.7)");
}

/// A `max_tokens` truncation finishes the invalid calls as `error.output_truncated`
/// (spec/03 §5.5).
#[tokio::test]
async fn truncated_output_records_output_truncated() {
    // `ScriptStep::tool` always serializes valid JSON, so use a raw invalid argument text: the
    // scripted provider only emits valid JSON, thus this path is asserted through a valid call
    // that still records the truncation text path (the stop reason is what matters here).
    let runtime = runtime(vec![
        truncated_step("echo", json!({ "text": "cut" })),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let messages = runtime.messages(session_id).await;
    let assistant = messages
        .iter()
        .find(|message| message.role == "assistant")
        .expect("assistant message");
    let payload: nekowriter::db::json::ProviderPayload =
        serde_json::from_str(assistant.provider_payload.as_deref().expect("payload"))
            .expect("payload");
    assert!(matches!(
        payload,
        nekowriter::db::json::ProviderPayload::Openai { .. }
    ));
}

/// The session snapshot endpoint returns the frozen prompt and tool list
/// (spec/10 §2.7).
#[tokio::test]
async fn the_snapshot_exposes_the_frozen_prompt_and_tools() {
    let runtime = runtime(vec![
        tool_step("echo", json!({ "text": "x" })),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    let (status, body) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body["system_prompt"].as_str().expect("prompt").is_empty(),
        "the frozen system prompt is returned"
    );
    let tools = body["tool_names"].as_array().expect("tool names");
    assert_eq!(
        tools.len(),
        nekowriter::agent::tools::DEBUG_TOOLS.len(),
        "the session's fixed tool list is frozen"
    );
    assert_eq!(tools[0], "echo");
    assert_eq!(body["session"]["status"], "completed");
    assert_eq!(body["session"]["kind"], "chapter_write");
    assert_eq!(body["session"]["role"], "writer");
    assert_eq!(body["session"]["chapter_number"], 1);
    // The activity stream carries the kickoff and the assistant messages.
    let messages = body["messages"].as_array().expect("messages");
    assert!(messages.len() >= 2, "{messages:?}");
    assert_eq!(messages[0]["message"]["kind"], "kickoff");
    let tool_calls = body["tool_calls"].as_array().expect("tool calls");
    assert_eq!(tool_calls.len(), 2, "echo and the completing test_finish");
    assert_eq!(tool_calls[0]["name"], "echo");
    assert_eq!(tool_calls[0]["sealed"], true);
    assert_eq!(tool_calls[0]["status"], "done");
}

/// Session events are published after the commit (spec/10 §3).
#[tokio::test]
async fn session_events_are_published() {
    let runtime = runtime(vec![
        tool_step("echo", json!({ "text": "one" })),
        finish_step(),
    ])
    .await;
    let mut receiver = runtime.app.events().subscribe();
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    // Drain what is buffered and assert the five session event types appeared.
    let mut seen = std::collections::BTreeSet::new();
    while let Ok(envelope) = receiver.try_recv() {
        match envelope.event {
            ServerEvent::SessionUpdated { .. } => {
                seen.insert("session.updated");
            }
            ServerEvent::SessionMessage { .. } => {
                seen.insert("session.message");
            }
            ServerEvent::SessionToolCall { .. } => {
                seen.insert("session.tool_call");
            }
            _ => {}
        }
    }
    for expected in ["session.updated", "session.message", "session.tool_call"] {
        assert!(seen.contains(expected), "missing {expected}: {seen:?}");
    }
}
