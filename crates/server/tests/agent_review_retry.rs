//! Regression tests from the M3 milestone review, round 2: `SessionDto.retrying`.

mod agent_harness;

use std::time::Duration;

use agent_harness::Runtime;
use nekowriter::events::ServerEvent;
use nekowriter::testing::TestApp;
use nekowriter_llm::mock::ScriptedProvider;
use nekowriter_llm::retry::LlmError;
use serde_json::json;

/// The same fixture as `agent_harness::runtime`, but with a pre-built scripted provider so a test
/// can make it fail before it answers. Mirrors the shared helper rather than changing it: the
/// harness module is edited by other milestones.
async fn runtime_with_provider(provider: ScriptedProvider) -> Runtime {
    let mut app = TestApp::new().await;
    let scripted = app.set_provider(provider);

    let book = app.create_book("Retry Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;

    let (status, provider_row) = app
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
    assert_eq!(status, 201, "create provider: {provider_row}");
    let provider_id = provider_row["id"].as_i64().expect("provider id") as i32;

    let (status, profile) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Demo model",
                "model": "demo-model",
                "context_window": 200_000,
                "max_output_tokens": 8_192,
            }),
        )
        .await;
    assert_eq!(status, 201, "create model profile: {profile}");
    let profile_id = profile["id"].as_i64().expect("profile id") as i32;

    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({
                "roles_defaults": {
                    "conception": profile_id,
                    "researcher": profile_id,
                    "writer": profile_id,
                    "reviewer": profile_id,
                    "archivist": profile_id,
                },
            }),
        )
        .await;
    assert_eq!(status, 200, "assign roles: {body}");

    Runtime {
        app,
        scripted,
        book_id,
        track_id,
        chapter_id,
    }
}

/// spec/04 §6: "`session.updated` carries `retrying: {attempt, wait_seconds, reason}` so the UI
/// can show 'Provider overloaded — retrying in 8 s'." spec/10 §2.8 lists the field on
/// `SessionDto`.
///
/// The retry loop logs the retry and sleeps, but never publishes a `session.updated` carrying the
/// retry state, and `session::to_dto` hardcodes `retrying: None`, so a `SessionDto` can never
/// carry a value: the agent panel cannot show that a retry is in flight, and the author sees a
/// session that looks hung for up to 30 s (the last backoff step).
#[tokio::test]
async fn a_retry_publishes_retrying_on_session_updated() {
    // Fail the first two attempts with a retryable error, then answer normally. The backoff for
    // attempt 1 is ~2 s, which leaves a wide window to observe the event.
    let provider = ScriptedProvider::new(vec![
        agent_harness::tool_step("echo", json!({ "text": "eventually" })),
        agent_harness::finish_step(),
    ])
    .failing(LlmError::Overloaded, 2);
    let runtime = runtime_with_provider(provider).await;

    let mut receiver = runtime.app.events().subscribe();
    let session_id = runtime.start_debug().await;

    // Watch the session events for one carrying a non-null `retrying`.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut saw_retrying = false;
    while std::time::Instant::now() < deadline {
        let remaining = deadline - std::time::Instant::now();
        let event = tokio::time::timeout(remaining, receiver.recv()).await;
        let Ok(Ok(envelope)) = event else { break };
        if let ServerEvent::SessionUpdated { session, .. } = envelope.event
            && session.id == session_id
            && session.retrying.is_some()
        {
            saw_retrying = true;
            break;
        }
    }
    assert!(
        saw_retrying,
        "no session.updated carried `retrying` while the provider was being retried \
         (spec/04 §6, spec/10 §2.8): the UI cannot show the retry"
    );
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Completed)
        .await;
}
