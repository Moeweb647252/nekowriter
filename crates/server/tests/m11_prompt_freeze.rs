//! PLAN M11.5 smoke step 8, the half that had no regression test: "Override `writer.system` for
//! this book → the next session uses it; sessions created before the change keep the old prompt."
//!
//! The freeze site is `sessions::create_inner`, which renders the role's system prompt and tool
//! definitions once and stores them in `agent_sessions.system_prompt` / `.tools`. Requests are then
//! built from that column (`context::build_request`), never from the live registry, which is what
//! makes the cached provider prefix (tools → system → messages) stable for the life of a session
//! (spec/03 §1.3: "the system prompt and tool definitions are frozen per session. Editing prompts
//! affects only new sessions.").
//!
//! Until this test existed nothing created a session around an override: `prompts_registry.rs`
//! writes `prompt_overrides` rows and asserts rendering, but never a session created before and
//! after a change. `agent_runtime.rs` only asserted that `system_prompt` is non-empty, which a
//! frozen prompt and a live one both satisfy.

use axum::http::StatusCode;
use nekowriter::db::json::Granularity;
use serde_json::json;

#[path = "agent_write_harness/mod.rs"]
mod agent_write_harness;

use agent_write_harness::{runtime, start_writer_on};

/// The book-level `writer.system` override, distinct from the shipped template.
const OVERRIDE: &str = "OVERRIDDEN WRITER SYSTEM PROMPT — a book-level test override.";

/// A session created before the override keeps its frozen prompt; one created after uses it.
#[tokio::test]
async fn an_override_reaches_new_sessions_and_never_rewrites_an_existing_one() {
    let runtime = runtime(Vec::new()).await;
    let chapter_id = runtime.chapter_id;

    // A real writer session, created through the production path and frozen before the override.
    let before = start_writer_on(
        &runtime,
        chapter_id,
        Granularity::Paragraph,
        false,
        false,
        false,
    )
    .await;
    let (status, frozen) = runtime.app.get(&format!("/api/sessions/{before}")).await;
    assert_eq!(status, StatusCode::OK, "{frozen}");
    let prompt_before = frozen["system_prompt"]
        .as_str()
        .expect("the snapshot carries the frozen prompt")
        .to_owned();
    assert!(
        !prompt_before.contains("OVERRIDDEN WRITER SYSTEM PROMPT"),
        "the before-session must start on the shipped prompt"
    );

    // Install the book-level override through the API.
    let (status, body) = runtime
        .app
        .put(
            "/api/prompts/writer.system",
            json!({ "scope": "book", "book_id": runtime.book_id, "content": OVERRIDE }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save the override: {body}");

    // Invariant 5 (spec/02 §5): one open writer session per chapter, so the first is abandoned
    // before the second starts. Its frozen prompt stays in the row either way.
    let (status, abandoned) = runtime
        .app
        .post(&format!("/api/sessions/{before}/abandon"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "abandon: {abandoned}");

    // The earlier session still reports exactly what it froze.
    let (status, unchanged) = runtime.app.get(&format!("/api/sessions/{before}")).await;
    assert_eq!(status, StatusCode::OK, "{unchanged}");
    assert_eq!(
        unchanged["system_prompt"].as_str(),
        Some(prompt_before.as_str()),
        "editing a prompt must not rewrite a session that already froze one (spec/03 §1.3)"
    );
    assert!(
        !unchanged["system_prompt"]
            .as_str()
            .unwrap_or_default()
            .contains("OVERRIDDEN WRITER SYSTEM PROMPT"),
        "the old session kept the old prompt: {unchanged}"
    );

    // A session created after the change freezes the override.
    let after = start_writer_on(
        &runtime,
        chapter_id,
        Granularity::Paragraph,
        false,
        false,
        false,
    )
    .await;
    let (status, fresh) = runtime.app.get(&format!("/api/sessions/{after}")).await;
    assert_eq!(status, StatusCode::OK, "{fresh}");
    let prompt_after = fresh["system_prompt"]
        .as_str()
        .expect("the new snapshot carries its prompt");
    assert!(
        prompt_after.contains("OVERRIDDEN WRITER SYSTEM PROMPT"),
        "a session created after the override uses it: {prompt_after}"
    );
    assert_ne!(
        prompt_after, prompt_before,
        "the two sessions froze different prompts"
    );
}

/// The override reaches the provider request, and a session keeps using the prompt it froze even
/// after the override changes again: the live registry is not consulted per request.
///
/// This is the assertion that distinguishes a frozen prompt from a live one. A session is started
/// under one override, the override is then replaced by a second one, and the next request of that
/// session must still carry the first — spec/03 §1.3 ("editing prompts affects only new sessions")
/// and the cached-prefix contract (tools → system → messages) both depend on it.
#[tokio::test]
async fn a_running_session_keeps_the_prompt_it_froze_when_the_override_changes() {
    let runtime = runtime(vec![
        // Two turns, so the session makes a second request after the override changes.
        nekowriter_llm::mock::ScriptStep::new().text("first reply"),
        nekowriter_llm::mock::ScriptStep::new().text("second reply"),
    ])
    .await;
    const FIRST: &str = "FIRST FROZEN PROMPT — installed before the session started.";
    const SECOND: &str = "SECOND LIVE PROMPT — installed while the session was running.";
    let (status, body) = runtime
        .app
        .put(
            "/api/prompts/writer.system",
            json!({ "scope": "book", "book_id": runtime.book_id, "content": FIRST }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save the first override: {body}");

    let session_id = start_writer_on(
        &runtime,
        runtime.chapter_id,
        Granularity::Paragraph,
        false,
        false,
        false,
    )
    .await;
    // Wait for the first request, then change the override underneath the session.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while runtime.scripted.bodies().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the runner never made its first request"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let (status, body) = runtime
        .app
        .put(
            "/api/prompts/writer.system",
            json!({ "scope": "book", "book_id": runtime.book_id, "content": SECOND }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "replace the override: {body}");

    // The session's next request still carries the first prompt. (The session ends with a
    // text-only reply, so two requests are all it makes; the second one is the assertion.)
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while runtime.scripted.bodies().len() < 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "the runner never made its second request: {:?}",
            runtime.scripted.bodies().len()
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let bodies = runtime.scripted.bodies();
    let system_of = |body: &serde_json::Value| -> String {
        body["messages"]
            .as_array()
            .and_then(|messages| {
                messages
                    .iter()
                    .find(|message| message["role"] == json!("system"))
            })
            .and_then(|message| message["content"].as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let first = system_of(&bodies[0]);
    let second = system_of(&bodies[1]);
    assert!(
        first.contains("FIRST FROZEN PROMPT"),
        "the first request carries the override that was live at creation: {first}"
    );
    assert_eq!(
        first, second,
        "the second request carries the same frozen prompt, not the newly saved override"
    );
    assert!(
        !second.contains("SECOND LIVE PROMPT"),
        "a saved override must not reach a session that already froze its prompt: {second}"
    );
    let _ = session_id;
}
