//! Session creation tests (PLAN M3.4, spec/03 §4, §10–§11).
//!
//! Covers model resolution, the frozen `model_snapshot` / `system_prompt` / `tools`, the kickoff
//! block lists per kind, the kickoff budget warning, and the request-building invariant that the
//! frozen head (model, system prompt, tools, options) never changes within a session.

mod agent_harness;

use std::collections::BTreeSet;

use agent_harness::{finish_step, runtime, tool_step};
use axum::http::StatusCode;
use nekowriter::db::json::SessionKind;
use nekowriter::testing::{TestApp, assert_error_detail};
use serde_json::json;

/// A book with a model on the Demo provider but **no** role assignment for the writer.
async fn app_without_writer_role() -> TestApp {
    let app = TestApp::new().await;
    let (status, provider) = app
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
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let provider_id = provider["id"].as_i64().expect("provider id") as i32;
    let (status, profile) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Demo model",
                "model": "demo-model",
                "context_window": 200_000,
                "max_output_tokens": 8_192,
                "effort_levels": [],
                "default_effort": null,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{profile}");
    app
}

/// spec/03 §4 step 1: no model for the role is `409 model.missing_role` with `details.role`.
#[tokio::test]
async fn a_missing_role_model_is_model_missing_role() {
    let app = app_without_writer_role().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.execute(&format!(
        "UPDATE chapters SET outline = '细纲' WHERE id = {chapter_id}"
    ))
    .await;

    let error = nekowriter::services::sessions::create(
        app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(book_id),
            chapter_id: Some(chapter_id),
            ..Default::default()
        },
    )
    .await
    .expect_err("no writer model");
    assert_eq!(error.code, nekowriter::error::ErrorCode::ModelMissingRole);
    let details = error.details.clone().expect("details");
    assert_eq!(details["role"], "writer");

    // The HTTP endpoint reports the same code.
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/write"), json!({}))
        .await;
    if status != StatusCode::NOT_FOUND {
        // `/write` arrives with M4; when it exists it must answer with the documented code.
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_error_detail(&body, "model.missing_role", "role");
    }
}

/// `book.model_overrides[role]` wins over `app_settings["roles.defaults"][role]`
/// (spec/03 §4 step 1).
#[tokio::test]
async fn the_book_override_wins_over_the_role_default() {
    let app = app_without_writer_role().await;
    // Two models on the same provider: the app default and the book override.
    let (status, providers) = app.get("/api/providers").await;
    assert_eq!(status, StatusCode::OK, "{providers}");
    let provider_id = providers[0]["id"].as_i64().expect("provider id") as i32;
    let (_, default_model) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Default model",
                "model": "default-model",
                "context_window": 100_000,
                "max_output_tokens": 4_096,
                "effort_levels": [],
                "default_effort": null,
            }),
        )
        .await;
    let (_, book_model) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Book model",
                "model": "book-model",
                "context_window": 200_000,
                "max_output_tokens": 8_192,
                "effort_levels": [],
                "default_effort": null,
            }),
        )
        .await;
    let default_id = default_model["id"].as_i64().expect("default id");
    let book_id_override = book_model["id"].as_i64().expect("book model id");
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": { "writer": default_id } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "model_overrides": { "writer": book_id_override } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;

    let (session_id, _warnings) = nekowriter::services::sessions::create(
        app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(book_id),
            chapter_id: Some(chapter_id),
            ..Default::default()
        },
    )
    .await
    .expect("create session");

    let (status, snapshot) = app.get(&format!("/api/sessions/{session_id}")).await;
    assert_eq!(status, StatusCode::OK, "{snapshot}");
    assert_eq!(
        snapshot["session"]["model_label"], "Book model",
        "the book override resolved"
    );
}

/// The snapshot is frozen at creation without the API key, and a later provider edit does not
/// change it; the key is resolved at call time so rotation works (spec/03 §4 step 1).
#[tokio::test]
async fn the_model_snapshot_is_frozen_without_the_key() {
    let runtime = runtime(vec![finish_step()]).await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Completed)
        .await;

    let (status, body) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The frozen prompt and tools are returned but never the snapshot itself in the DTO.
    assert!(!body["system_prompt"].as_str().expect("prompt").is_empty());
    assert!(
        body["session"].get("model_snapshot").is_none(),
        "the snapshot is not exposed verbatim"
    );
    // The snapshot row carries no API key (spec/03 §4 step 1).
    let snapshot = runtime.model_snapshot(session_id).await.to_string();
    assert!(
        !snapshot.to_lowercase().contains("api_key"),
        "the snapshot has no key: {snapshot}"
    );
    assert!(
        snapshot.contains("mock"),
        "the protocol is frozen: {snapshot}"
    );
    assert!(
        snapshot.contains("demo-model"),
        "the model is frozen: {snapshot}"
    );
}

/// The kickoff of every kind is the block list of spec/03 §10, and `block.settings` carries the
/// cache breakpoint.
#[tokio::test]
async fn the_kickoff_blocks_follow_the_kind_table() {
    // The context module is the authority; assert the table itself.
    use nekowriter::agent::context::{block_is_cached, kickoff_blocks};

    for (kind, expected) in [
        (SessionKind::Conception, vec!["conception.kickoff"]),
        (
            SessionKind::Research,
            vec!["block.book_brief", "researcher.kickoff"],
        ),
        (
            SessionKind::ChapterOutline,
            vec![
                "block.settings",
                "block.canon_systems",
                "block.memory",
                "block.recent_chapters",
                "block.goals",
                "planner.kickoff",
            ],
        ),
        (
            SessionKind::ChapterWrite,
            vec![
                "block.settings",
                "block.canon_systems",
                "block.writing_skills",
                "block.memory",
                "block.recent_chapters",
                "block.goals",
                "block.chapter_draft",
                "writer.kickoff.chapter",
            ],
        ),
        (
            SessionKind::ChapterRevise,
            vec![
                "block.settings",
                "block.canon_systems",
                "block.writing_skills",
                "block.memory",
                "block.goals",
                "block.chapter_draft",
                "writer.kickoff.revise",
            ],
        ),
        (
            SessionKind::ChapterReview,
            vec![
                "block.settings",
                "block.canon_systems",
                "block.writing_skills",
                "block.memory",
                "block.goals",
                "block.chapter_draft",
                "reviewer.kickoff",
            ],
        ),
        (
            SessionKind::ChapterMaintenance,
            vec![
                "block.settings",
                "block.canon_systems",
                "block.memory",
                "block.cast_index",
                "block.goals",
                "block.chapter_draft",
                "archivist.kickoff.maintenance",
            ],
        ),
        (
            SessionKind::CanonSeed,
            vec![
                "block.settings",
                "block.canon_systems",
                "archivist.kickoff.seed",
            ],
        ),
        (
            SessionKind::ChapterResync,
            vec![
                "block.settings",
                "block.canon_systems",
                "block.memory",
                "block.cast_index",
                "block.chapter_diff",
                "archivist.kickoff.resync",
            ],
        ),
    ] {
        assert_eq!(kickoff_blocks(kind, false, false), expected, "{kind:?}");
        // A redraft uses the same drafting kickoff; its feedback comes through the task variable
        // (D-146).
        if kind == SessionKind::ChapterWrite {
            assert_eq!(
                kickoff_blocks(kind, true, false),
                kickoff_blocks(kind, false, false)
            );
        }
    }
    // Only `block.settings` carries a cache breakpoint (spec/03 §10). A skills block after the
    // settings block is outside the cached prefix; it earns no cache flag (design §6.1).
    assert!(block_is_cached("block.settings"));
    assert!(!block_is_cached("block.memory"));
    assert!(!block_is_cached("block.writing_skills"));
    assert!(!block_is_cached("writer.kickoff.chapter"));
}

/// A writer session's kickoff is one text block per kickoff block, persisted as `seq 0`, kind
/// `kickoff`, with the cache flag on the settings block (spec/03 §4 step 4, §10).
#[tokio::test]
async fn the_kickoff_message_holds_one_block_per_kickoff_block() {
    let runtime = runtime(vec![]).await;
    let (session_id, _warnings) = nekowriter::services::sessions::create(
        runtime.app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            ..Default::default()
        },
    )
    .await
    .expect("create session");
    let messages = runtime.messages(session_id).await;
    let kickoff = messages.first().expect("the kickoff is the first message");
    assert_eq!(kickoff.seq, 0);
    assert_eq!(kickoff.role, "user");
    assert_eq!(kickoff.kind, "kickoff");
    let blocks = kickoff.blocks();
    assert_eq!(
        blocks.len(),
        4,
        "settings · memory · draft · task: {blocks:?}"
    );
    assert!(
        matches!(
            &blocks[0],
            nekowriter::db::json::StoredBlock::Text {
                cache: Some(true),
                ..
            }
        ),
        "the settings block carries the cache breakpoint: {:?}",
        blocks[0]
    );
    for block in &blocks[1..] {
        assert!(
            matches!(
                block,
                nekowriter::db::json::StoredBlock::Text { cache: None, .. }
            ),
            "only the settings block is cached: {block:?}"
        );
    }
    // The rendered settings block contains the book title (the pack is Chinese, the data is not).
    let settings_text = match &blocks[0] {
        nekowriter::db::json::StoredBlock::Text { text, .. } => text.clone(),
        _ => String::new(),
    };
    assert!(
        settings_text.contains("Runtime Book"),
        "the settings block renders the book: {settings_text}"
    );
    // The task block names the chapter number.
    let task_text = match &blocks[2] {
        nekowriter::db::json::StoredBlock::Text { text, .. } => text.clone(),
        _ => String::new(),
    };
    assert!(
        task_text.contains('1'),
        "the task names the chapter: {task_text}"
    );
}

/// The frozen tool list of a session is the role's fixed list, in order, and a wrong phase
/// never changes it (spec/03 §1.2).
#[tokio::test]
async fn the_frozen_tool_list_is_the_role_list_in_order() {
    let runtime = runtime(vec![]).await;
    let (session_id, _warnings) = nekowriter::services::sessions::create(
        runtime.app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            ..Default::default()
        },
    )
    .await
    .expect("create session");
    let (status, body) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tools: Vec<&str> = body["tool_names"]
        .as_array()
        .expect("tool names")
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert_eq!(tools, nekowriter::agent::tools::WRITER_TOOLS.to_vec());
    // No duplicate names.
    let unique: BTreeSet<&str> = tools.iter().copied().collect();
    assert_eq!(unique.len(), tools.len());
}

/// The request built for a session is stable across steps: model, system prompt, tools and
/// options never change, and the messages only grow (spec/03 §11, §15 test 14).
#[tokio::test]
async fn the_request_head_is_identical_across_steps() {
    let runtime = runtime(vec![
        tool_step("echo", json!({ "text": "one" })),
        tool_step("echo", json!({ "text": "two" })),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Completed)
        .await;

    let bodies = runtime.scripted.bodies();
    assert!(bodies.len() >= 3, "several steps ran: {}", bodies.len());
    let first = &bodies[0];
    for later in &bodies[1..] {
        for key in ["model", "system", "tools", "max_tokens"] {
            assert_eq!(
                first.get(key),
                later.get(key),
                "{key} must not change within a session"
            );
        }
    }
    // Messages only grow, and each earlier request is a prefix of the next.
    for window in bodies.windows(2) {
        let earlier = window[0]["messages"].as_array().expect("messages");
        let later = window[1]["messages"].as_array().expect("messages");
        assert!(later.len() >= earlier.len());
        for (index, message) in earlier.iter().enumerate() {
            assert_eq!(message, &later[index], "message {index} must be unchanged");
        }
    }
}

/// Rebuilding the request from persisted state is deterministic (spec/03 §11: the request is
/// built from the stored rows only).
#[tokio::test]
async fn request_building_is_deterministic() {
    let runtime = runtime(vec![
        tool_step("echo", json!({ "text": "x" })),
        finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Completed)
        .await;

    let row = nekowriter::agent::session::load(runtime.app.state().db_conn(), session_id)
        .await
        .expect("session row");
    let prompts = runtime
        .app
        .state()
        .prompts()
        .for_book(runtime.app.state().db(), row.book_id)
        .await
        .expect("prompt set");
    let first =
        nekowriter::agent::context::build_request(runtime.app.state().db_conn(), &row, &prompts)
            .await
            .expect("build request");
    let second =
        nekowriter::agent::context::build_request(runtime.app.state().db_conn(), &row, &prompts)
            .await
            .expect("build request");
    // Compare the serialized bodies: identical model, prompts, tools and messages.
    let first_body = nekowriter_llm::openai::build_body(&first, &first.options);
    let second_body = nekowriter_llm::openai::build_body(&second, &second.options);
    assert_eq!(first_body, second_body);
}

/// A kickoff larger than half the context window still starts the session and reports
/// `context.large_kickoff` (spec/03 §10).
#[tokio::test]
async fn a_large_kickoff_warns_but_starts_the_session() {
    let runtime = runtime(vec![finish_step()]).await;
    // A tiny context window makes the ordinary kickoff exceed half of it.
    let (status, providers) = runtime.app.get("/api/providers").await;
    assert_eq!(status, StatusCode::OK, "{providers}");
    let provider_id = providers[0]["id"].as_i64().expect("provider id") as i32;
    let (status, profile) = runtime
        .app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Tiny",
                "model": "tiny",
                "context_window": 120,
                "max_output_tokens": 16,
                "effort_levels": [],
                "default_effort": null,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{profile}");
    let tiny_id = profile["id"].as_i64().expect("profile id");
    let (status, body) = runtime
        .app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": { "writer": tiny_id } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // A new book so the book override does not resolve first.
    let book = runtime.app.create_book("Tiny Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = runtime
        .app
        .get(&format!("/api/books/{book_id}/tracks"))
        .await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = runtime.app.create_chapter(track_id, "One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;

    let (session_id, warnings) = nekowriter::services::sessions::create(
        runtime.app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(book_id),
            chapter_id: Some(chapter_id),
            granularity: Some(nekowriter::db::json::Granularity::Paragraph),
            ..Default::default()
        },
    )
    .await
    .expect("the session still starts");
    assert!(
        warnings
            .iter()
            .any(|warning| warning == "context.large_kickoff"),
        "the budget warning is reported: {warnings:?}"
    );
    // The warning travels on the `session.updated` event and on the creation response DTO; it is
    // not a stored column, so a later GET does not repeat it (spec/03 §10).
    let (status, body) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["session"]["kind"], "chapter_write",
        "the session started despite the budget warning"
    );
    let _ = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/abandon"), json!({}))
        .await;
}

/// `cache_ttl: "auto"` is resolved at creation so it never changes within a session
/// (spec/03 §4 step 1, D-031).
#[tokio::test]
async fn the_cache_ttl_is_resolved_at_creation() {
    let runtime = runtime(vec![]).await;
    let (session_id, _warnings) = nekowriter::services::sessions::create(
        runtime.app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            ..Default::default()
        },
    )
    .await
    .expect("create session");
    let snapshot = runtime.model_snapshot(session_id).await;
    let ttl = snapshot["options"]["cache_ttl"]
        .as_str()
        .expect("a resolved cache_ttl");
    assert_ne!(ttl, "auto", "auto is resolved at creation: {snapshot}");
    // A write session with auto-pause off is a plain worker: exactly 5 minutes.
    assert_eq!(ttl, "5m", "the default worker TTL: {snapshot}");
    let _ = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/abandon"), json!({}))
        .await;

    // A planner that pauses after the outline is a holding session: exactly 1 hour (F11,
    // spec/04 §3).
    let (planner_paused, _) = nekowriter::services::sessions::create(
        runtime.app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: SessionKind::ChapterOutline,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            pause_after_outline: Some(true),
            ..Default::default()
        },
    )
    .await
    .expect("create paused planner");
    let snapshot = runtime.model_snapshot(planner_paused).await;
    assert_eq!(
        snapshot["options"]["cache_ttl"], "1h",
        "a paused planner keeps its prefix for an hour: {snapshot}"
    );
    let _ = runtime
        .app
        .post(
            &format!("/api/sessions/{planner_paused}/abandon"),
            json!({}),
        )
        .await;

    // Without the pause it is an ordinary worker again: 5 minutes.
    let (planner_instant, _) = nekowriter::services::sessions::create(
        runtime.app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: SessionKind::ChapterOutline,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            pause_after_outline: Some(false),
            ..Default::default()
        },
    )
    .await
    .expect("create instant planner");
    let snapshot = runtime.model_snapshot(planner_instant).await;
    assert_eq!(
        snapshot["options"]["cache_ttl"], "5m",
        "an unpaused planner is a worker: {snapshot}"
    );
    let _ = runtime
        .app
        .post(
            &format!("/api/sessions/{planner_instant}/abandon"),
            json!({}),
        )
        .await;
}

/// Every provider protocol freezes into the session's `model_snapshot` as the typed value; the
/// non-openai/anthropic case is `mock` (spec/03 §4 step 1).
#[tokio::test]
async fn a_session_freezes_each_provider_protocol() {
    for protocol in ["mock", "openai", "anthropic"] {
        let runtime = runtime(vec![finish_step()]).await;
        let (status, providers) = runtime.app.get("/api/providers").await;
        assert_eq!(status, StatusCode::OK, "{providers}");
        let provider_id = providers[0]["id"].as_i64().expect("provider id");
        runtime
            .app
            .execute(&format!(
                "UPDATE providers SET protocol = '{protocol}' WHERE id = {provider_id}"
            ))
            .await;

        let session_id = runtime.start_debug().await;
        let snapshot = runtime.model_snapshot(session_id).await;
        assert_eq!(
            snapshot["protocol"], protocol,
            "session creation froze the wrong protocol: {snapshot}"
        );
    }
}
