//! Thinking-effort resolution at session creation (PLAN M19 19.5, spec/03 §4, spec/04 §5.1,
//! D-162).
//!
//! The resolver is one function for every creation path: the model chain first
//! (`book.models[role]` → global, plus the planner's writer fallback), then the effort candidates
//! (`book.efforts[role]` → global, plus the writer's entries only when the planner follows the
//! writer's model), then the profile default. A set candidate the resolved model cannot use is a
//! warning, never a failed session.
//!
//! Cases 1–11 of the plan's §4.4 table are numbered below. The wire tests at the bottom drive the
//! real runner against `wiremock` for both protocols.

use std::time::Duration;

use axum::http::StatusCode;
use nekowriter::db::json::SessionKind;
use nekowriter::services::sessions::CreateSession;
use nekowriter::testing::TestApp;
use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const WARNING: &str = "model.effort_unsupported";

/// A test app whose runners never touch the network. The provider rows still carry the protocol
/// and base URL that the snapshot freezes, so resolution is unchanged; only the outgoing model
/// call is replaced.
async fn app() -> TestApp {
    let mut app = TestApp::new().await;
    app.set_provider(nekowriter_llm::mock::ScriptedProvider::new(vec![
        nekowriter_llm::mock::ScriptStep::new()
            .text("ok")
            .stop(nekowriter_llm::StopReason::EndTurn),
    ]));
    app
}

async fn provider(app: &TestApp, name: &str, protocol: &str, base_url: &str) -> i64 {
    let (status, body) = app
        .post(
            "/api/providers",
            json!({ "name": name, "protocol": protocol, "base_url": base_url, "extra_headers": {} }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_i64().expect("provider id")
}

async fn demo_provider(app: &TestApp) -> i64 {
    provider(app, "Demo", "mock", "mock://demo").await
}

/// A real-protocol provider for resolution tests; the runner is scripted, so its unreachable base
/// URL is never dialled. The provider row still decides the protocol frozen into the snapshot.
async fn openai_provider(app: &TestApp) -> i64 {
    provider(app, "OpenAI", "openai", "http://127.0.0.1:9/v1").await
}

async fn profile(
    app: &TestApp,
    provider_id: i64,
    display_name: &str,
    levels: Value,
    default_effort: Value,
) -> i64 {
    let (status, body) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": display_name,
                "model": format!("model-{display_name}"),
                "context_window": 128_000,
                "max_output_tokens": 16_384,
                "effort_levels": levels,
                "default_effort": default_effort,
                "options": {},
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_i64().expect("profile id")
}

async fn book_and_chapter(app: &TestApp, title: &str) -> (i32, i32) {
    let book = app.create_book(title).await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.execute(&format!(
        "UPDATE chapters SET outline = 'outline' WHERE id = {chapter_id}"
    ))
    .await;
    (book_id, chapter_id)
}

async fn set_roles(app: &TestApp, roles: Value) {
    let (status, body) = app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn set_book_overrides(app: &TestApp, book_id: i32, overrides: Value) {
    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "model_overrides": overrides }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn create_write(app: &TestApp, book_id: i32, chapter_id: i32) -> (i32, Vec<String>) {
    nekowriter::services::sessions::create(
        app.state(),
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(book_id),
            chapter_id: Some(chapter_id),
            ..Default::default()
        },
    )
    .await
    .expect("create writer session")
}

async fn create_outline(app: &TestApp, book_id: i32, chapter_id: i32) -> (i32, Vec<String>) {
    nekowriter::services::sessions::create(
        app.state(),
        CreateSession {
            kind: SessionKind::ChapterOutline,
            book_id: Some(book_id),
            chapter_id: Some(chapter_id),
            ..Default::default()
        },
    )
    .await
    .expect("create planner session")
}

async fn snapshot(app: &TestApp, session_id: i32) -> Value {
    let row = app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT model_snapshot FROM agent_sessions WHERE id = ?",
            vec![session_id.into()],
        ))
        .await
        .expect("query snapshot")
        .expect("session row");
    let raw: String = row.try_get("", "model_snapshot").expect("snapshot column");
    serde_json::from_str(&raw).expect("snapshot JSON")
}

/// The `SessionDto.effort` of a session, straight from the HTTP endpoint.
async fn session_effort(app: &TestApp, session_id: i32) -> Value {
    let (status, body) = app.get(&format!("/api/sessions/{session_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["session"]["effort"].clone()
}

fn warning_count(warnings: &[String]) -> usize {
    warnings
        .iter()
        .filter(|warning| *warning == WARNING)
        .count()
}

// ───────────────────────────── 1–5: the candidate chain ─────────────────────────────

/// 1. No role choice → the profile's default `high`.
#[tokio::test]
async fn case_1_profile_default_applies_when_no_role_chooses() {
    let app = app().await;
    let provider_id = openai_provider(&app).await;
    let profile_id = profile(
        &app,
        provider_id,
        "Default high",
        json!(["low", "medium", "high"]),
        json!("high"),
    )
    .await;
    set_roles(&app, json!({ "writer": profile_id, "efforts": {} })).await;
    let (book_id, chapter_id) = book_and_chapter(&app, "One").await;
    let (session_id, warnings) = create_write(&app, book_id, chapter_id).await;

    let snapshot = snapshot(&app, session_id).await;
    assert_eq!(
        snapshot["options"]["reasoning_effort"], "high",
        "{snapshot}"
    );
    assert_eq!(warning_count(&warnings), 0, "{warnings:?}");
    assert_eq!(session_effort(&app, session_id).await, "high");
}

/// 2. The global writer choice applies when the book has no choice.
#[tokio::test]
async fn case_2_global_writer_effort_applies() {
    let app = app().await;
    let provider_id = openai_provider(&app).await;
    let profile_id = profile(
        &app,
        provider_id,
        "Global low",
        json!(["low", "medium", "high"]),
        Value::Null,
    )
    .await;
    set_roles(
        &app,
        json!({ "writer": profile_id, "efforts": { "writer": "low" } }),
    )
    .await;
    let (book_id, chapter_id) = book_and_chapter(&app, "One").await;
    let (session_id, warnings) = create_write(&app, book_id, chapter_id).await;

    let snapshot = snapshot(&app, session_id).await;
    assert_eq!(snapshot["options"]["reasoning_effort"], "low", "{snapshot}");
    assert_eq!(warning_count(&warnings), 0, "{warnings:?}");
}

/// 3. The book's `max` beats the global `low`.
#[tokio::test]
async fn case_3_book_effort_beats_global() {
    let app = app().await;
    let provider_id = openai_provider(&app).await;
    let profile_id = profile(
        &app,
        provider_id,
        "All levels",
        json!(["low", "medium", "high", "xhigh", "max"]),
        Value::Null,
    )
    .await;
    set_roles(
        &app,
        json!({ "writer": profile_id, "efforts": { "writer": "low" } }),
    )
    .await;
    let (book_id, chapter_id) = book_and_chapter(&app, "One").await;
    set_book_overrides(
        &app,
        book_id,
        json!({ "writer": profile_id, "efforts": { "writer": "max" } }),
    )
    .await;
    let (session_id, warnings) = create_write(&app, book_id, chapter_id).await;

    let snapshot = snapshot(&app, session_id).await;
    assert_eq!(snapshot["options"]["reasoning_effort"], "max", "{snapshot}");
    assert_eq!(warning_count(&warnings), 0, "{warnings:?}");
}

/// 4. A book level the resolved model cannot send falls through to the global `low` and warns.
#[tokio::test]
async fn case_4_unsupported_book_level_falls_through_and_warns() {
    let app = app().await;
    let provider_id = openai_provider(&app).await;
    let profile_id = profile(
        &app,
        provider_id,
        "Small set",
        json!(["low", "medium"]),
        Value::Null,
    )
    .await;
    set_roles(
        &app,
        json!({ "writer": profile_id, "efforts": { "writer": "low" } }),
    )
    .await;
    let (book_id, chapter_id) = book_and_chapter(&app, "One").await;
    set_book_overrides(
        &app,
        book_id,
        json!({ "writer": profile_id, "efforts": { "writer": "max" } }),
    )
    .await;
    let (session_id, warnings) = create_write(&app, book_id, chapter_id).await;

    let snapshot = snapshot(&app, session_id).await;
    assert_eq!(snapshot["options"]["reasoning_effort"], "low", "{snapshot}");
    assert_eq!(warning_count(&warnings), 1, "{warnings:?}");
    assert_eq!(session_effort(&app, session_id).await, "low");
}

/// 5. Every candidate unsupported and no default → no effort field, one warning.
#[tokio::test]
async fn case_5_no_candidate_survives_and_no_default() {
    let app = app().await;
    let provider_id = openai_provider(&app).await;
    let profile_id = profile(&app, provider_id, "Only low", json!(["low"]), Value::Null).await;
    set_roles(
        &app,
        json!({ "writer": profile_id, "efforts": { "writer": "max" } }),
    )
    .await;
    let (book_id, chapter_id) = book_and_chapter(&app, "One").await;
    set_book_overrides(
        &app,
        book_id,
        json!({ "writer": profile_id, "efforts": { "writer": "ultra" } }),
    )
    .await;
    let (session_id, warnings) = create_write(&app, book_id, chapter_id).await;

    let snapshot = snapshot(&app, session_id).await;
    assert!(
        snapshot["options"]["reasoning_effort"].is_null(),
        "no effort value: {snapshot}"
    );
    assert_eq!(warning_count(&warnings), 1, "{warnings:?}");
    assert_eq!(session_effort(&app, session_id).await, Value::Null);
}

// ───────────────────────────── 6–8: the planner fallback ─────────────────────────────

/// 6. A planner that follows the writer's model follows the writer's effort.
#[tokio::test]
async fn case_6_planner_following_the_writer_inherits_its_effort() {
    let app = app().await;
    let provider_id = openai_provider(&app).await;
    let profile_id = profile(
        &app,
        provider_id,
        "Writer",
        json!(["low", "high"]),
        Value::Null,
    )
    .await;
    set_roles(
        &app,
        json!({ "writer": profile_id, "efforts": { "writer": "low" } }),
    )
    .await;
    let (book_id, chapter_id) = book_and_chapter(&app, "One").await;
    let (session_id, warnings) = create_outline(&app, book_id, chapter_id).await;

    let snapshot = snapshot(&app, session_id).await;
    assert_eq!(snapshot["options"]["reasoning_effort"], "low", "{snapshot}");
    assert_eq!(warning_count(&warnings), 0, "{warnings:?}");
}

/// 7. A planner on its own model does **not** inherit the writer's effort; its default applies.
#[tokio::test]
async fn case_7_planner_on_own_model_ignores_the_writer_effort() {
    let app = app().await;
    let provider_id = openai_provider(&app).await;
    let planner_profile = profile(
        &app,
        provider_id,
        "Planner",
        json!(["low", "medium", "high"]),
        json!("medium"),
    )
    .await;
    let writer_profile = profile(
        &app,
        provider_id,
        "Writer",
        json!(["low", "high"]),
        Value::Null,
    )
    .await;
    set_roles(
        &app,
        json!({
            "planner": planner_profile,
            "writer": writer_profile,
            "efforts": { "writer": "low" },
        }),
    )
    .await;
    let (book_id, chapter_id) = book_and_chapter(&app, "One").await;
    let (session_id, warnings) = create_outline(&app, book_id, chapter_id).await;

    let snapshot = snapshot(&app, session_id).await;
    assert_eq!(
        snapshot["options"]["reasoning_effort"], "medium",
        "{snapshot}"
    );
    assert_eq!(warning_count(&warnings), 0, "{warnings:?}");
}

/// 8. A planner that follows the writer's model but sets its own effort wins with it.
#[tokio::test]
async fn case_8_planner_on_writer_model_uses_its_own_effort() {
    let app = app().await;
    let provider_id = openai_provider(&app).await;
    let profile_id = profile(
        &app,
        provider_id,
        "Writer",
        json!(["low", "medium", "high"]),
        Value::Null,
    )
    .await;
    set_roles(
        &app,
        json!({
            "writer": profile_id,
            "efforts": { "planner": "high", "writer": "low" },
        }),
    )
    .await;
    let (book_id, chapter_id) = book_and_chapter(&app, "One").await;
    let (session_id, warnings) = create_outline(&app, book_id, chapter_id).await;

    let snapshot = snapshot(&app, session_id).await;
    assert_eq!(
        snapshot["options"]["reasoning_effort"], "high",
        "{snapshot}"
    );
    assert_eq!(warning_count(&warnings), 0, "{warnings:?}");
}

// ───────────────────────────── 9: the wire field per protocol ─────────────────────────────

/// 9. The same level lands in `reasoning_effort` on OpenAI, `effort` on Anthropic, and nowhere on
/// the mock protocol; `SessionDto.effort` mirrors that.
#[tokio::test]
async fn case_9_the_same_level_uses_each_protocol_wire_field() {
    let app = app().await;
    let openai = provider(&app, "OpenAI", "openai", "http://127.0.0.1:9/v1").await;
    let anthropic = provider(&app, "Anthropic", "anthropic", "http://127.0.0.1:9").await;
    let demo = demo_provider(&app).await;
    let openai_profile = profile(&app, openai, "OpenAI high", json!(["high"]), Value::Null).await;
    let anthropic_profile = profile(
        &app,
        anthropic,
        "Anthropic high",
        json!(["high"]),
        Value::Null,
    )
    .await;
    // The mock protocol takes no levels at all; a role choice is still set so the case proves it
    // is ignored rather than stored.
    let demo_profile = profile(&app, demo, "Demo high", json!([]), Value::Null).await;

    for (label, profile_id) in [
        ("OpenAI", openai_profile),
        ("Anthropic", anthropic_profile),
        ("Demo", demo_profile),
    ] {
        let (book_id, chapter_id) = book_and_chapter(&app, label).await;
        set_book_overrides(
            &app,
            book_id,
            json!({ "writer": profile_id, "efforts": { "writer": "high" } }),
        )
        .await;
        let (session_id, _warnings) = create_write(&app, book_id, chapter_id).await;
        let snapshot = snapshot(&app, session_id).await;
        let effort = session_effort(&app, session_id).await;
        match label {
            "OpenAI" => {
                assert_eq!(
                    snapshot["options"]["reasoning_effort"], "high",
                    "{snapshot}"
                );
                assert_eq!(effort, "high");
            }
            "Anthropic" => {
                assert_eq!(snapshot["options"]["effort"], "high", "{snapshot}");
                assert_eq!(effort, "high");
            }
            _ => {
                assert!(
                    snapshot["options"]["reasoning_effort"].is_null(),
                    "{snapshot}"
                );
                assert!(snapshot["options"]["effort"].is_null(), "{snapshot}");
                assert_eq!(effort, Value::Null);
            }
        }
    }
}

// ───────────────────────────── 10–11: frozen snapshots ─────────────────────────────

/// 10. A rollover after the setting changed keeps the parent's level.
#[tokio::test]
async fn case_10_rollover_keeps_the_parent_effort() {
    let app = app().await;
    let provider_id = openai_provider(&app).await;
    let profile_id = profile(
        &app,
        provider_id,
        "Writer",
        json!(["low", "medium", "high"]),
        Value::Null,
    )
    .await;
    set_roles(
        &app,
        json!({ "writer": profile_id, "efforts": { "writer": "low" } }),
    )
    .await;
    let (book_id, chapter_id) = book_and_chapter(&app, "One").await;
    let (session_id, _warnings) = create_write(&app, book_id, chapter_id).await;

    // The author changes the setting while the session is open; the frozen snapshot must not move.
    set_roles(
        &app,
        json!({ "writer": profile_id, "efforts": { "writer": "high" } }),
    )
    .await;
    assert_eq!(
        snapshot(&app, session_id).await["options"]["reasoning_effort"],
        "low",
        "a setting change does not touch a running session"
    );

    let parent = nekowriter::db::entities::agent_sessions::Entity::find_by_id(session_id)
        .one(app.db().conn())
        .await
        .expect("load parent")
        .expect("parent row");
    nekowriter::agent::context::rollover(app.state(), &parent)
        .await
        .expect("rollover");

    let child_row = app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT id FROM agent_sessions WHERE parent_session_id = ?",
            vec![session_id.into()],
        ))
        .await
        .expect("query child")
        .expect("rollover child row");
    let child_id: i32 = child_row.try_get("", "id").expect("child id");
    assert_eq!(
        snapshot(&app, child_id).await["options"]["reasoning_effort"],
        "low",
        "the rollover child inherits the parent's frozen level"
    );
}

/// 11. A setting change while a session runs changes neither its snapshot nor its DTO effort.
#[tokio::test]
async fn case_11_setting_changes_do_not_touch_a_running_session() {
    let app = app().await;
    let provider_id = openai_provider(&app).await;
    let profile_id = profile(
        &app,
        provider_id,
        "Writer",
        json!(["low", "medium", "high"]),
        Value::Null,
    )
    .await;
    set_roles(
        &app,
        json!({ "writer": profile_id, "efforts": { "writer": "low" } }),
    )
    .await;
    let (book_id, chapter_id) = book_and_chapter(&app, "One").await;
    let (session_id, _warnings) = create_write(&app, book_id, chapter_id).await;

    set_roles(
        &app,
        json!({ "writer": profile_id, "efforts": { "writer": "high" } }),
    )
    .await;
    assert_eq!(
        snapshot(&app, session_id).await["options"]["reasoning_effort"],
        "low"
    );
    assert_eq!(session_effort(&app, session_id).await, "low");
}

/// The test-only debug creation paths call the same resolver as production creation
/// (spec/03 §4: every creation path uses this one resolver).
#[tokio::test]
async fn debug_sessions_use_the_same_resolver() {
    let app = app().await;
    let provider_id = openai_provider(&app).await;
    let profile_id = profile(
        &app,
        provider_id,
        "Debug",
        json!(["low", "high"]),
        Value::Null,
    )
    .await;
    set_roles(
        &app,
        json!({ "writer": profile_id, "efforts": { "writer": "high" } }),
    )
    .await;
    let book = app.create_book("Debug").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (status, body) = app
        .post("/api/debug/sessions/echo", json!({ "book_id": book_id }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let session_id = body["session_id"].as_i64().expect("session id") as i32;
    assert_eq!(
        snapshot(&app, session_id).await["options"]["reasoning_effort"],
        "high"
    );
}

// ───────────────────────────── old JSON and save validation ─────────────────────────────

/// Rows written before 0009 have no `efforts`; they load with every choice unset and the profile
/// default still applies.
#[tokio::test]
async fn old_role_models_json_without_efforts_still_resolves() {
    let app = app().await;
    let provider_id = openai_provider(&app).await;
    let profile_id = profile(
        &app,
        provider_id,
        "Legacy",
        json!(["low", "medium"]),
        json!("medium"),
    )
    .await;
    let (book_id, chapter_id) = book_and_chapter(&app, "One").await;
    // Exactly the shape a pre-0009 row has: ids only, no `efforts` key.
    app.execute(&format!(
        "INSERT OR REPLACE INTO app_settings(key, value, updated_at) \
         VALUES ('roles.defaults', '{{\"writer\":{profile_id}}}', '2026-01-01T00:00:00Z')"
    ))
    .await;
    app.execute(&format!(
        "UPDATE books SET model_overrides = '{{\"writer\":{profile_id}}}' WHERE id = {book_id}"
    ))
    .await;
    let (session_id, warnings) = create_write(&app, book_id, chapter_id).await;
    assert_eq!(
        snapshot(&app, session_id).await["options"]["reasoning_effort"],
        "medium"
    );
    assert_eq!(warning_count(&warnings), 0, "{warnings:?}");
}

/// The save-validation errors of §2.3, on both endpoints, with the documented field keys.
#[tokio::test]
async fn role_efforts_are_validated_on_save() {
    let app = app().await;
    let provider_id = openai_provider(&app).await;
    let profile_id = profile(&app, provider_id, "Profile", json!(["low"]), Value::Null).await;
    let (book_id, _chapter_id) = book_and_chapter(&app, "One").await;

    // App settings: a level pattern violation names `efforts.writer`.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": { "writer": profile_id, "efforts": { "writer": "high effort" } } }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["error"]["details"]["fields"]["efforts.writer"],
        json!("an effort level must be 1-32 characters from A-Z, a-z, 0-9, '_' and '-'")
    );

    // The same rule on the book override.
    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "model_overrides": { "writer": profile_id, "efforts": { "writer": "bad!" } } }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"]["details"]["fields"]["efforts.writer"].is_string(),
        "{body}"
    );

    // A value outside the model's levels is accepted: it is checked at session creation, because
    // the model can change later.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": { "writer": profile_id, "efforts": { "writer": "max" } } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Values are trimmed before storage.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": { "writer": profile_id, "efforts": { "writer": "  max  " } } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["roles_defaults"]["efforts"]["writer"], "max");
}

// ───────────────────────────── wire end to end ─────────────────────────────

/// Run one session against a live `wiremock` provider and return the first request body it
/// received. The runner is spawned by `services::sessions::create`; the helper polls until the
/// request arrives rather than sleeping for a fixed time.
async fn first_session_request(
    app: &TestApp,
    server: &MockServer,
    profile_id: i64,
    effort: &str,
) -> Value {
    let (book_id, chapter_id) = book_and_chapter(app, "Wire").await;
    set_book_overrides(
        app,
        book_id,
        json!({ "writer": profile_id, "efforts": { "writer": effort } }),
    )
    .await;
    let (session_id, _warnings) = create_write(app, book_id, chapter_id).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let requests = server.received_requests().await.expect("requests");
        if let Some(request) = requests.first() {
            let body: Value = serde_json::from_slice(&request.body).expect("request body");
            let _ = app
                .post(&format!("/api/sessions/{session_id}/abandon"), json!({}))
                .await;
            return body;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the runner did not send a request"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn the_openai_wire_request_carries_the_resolved_reasoning_effort() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(
                    "data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"pong\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n",
                ),
        )
        .mount(&server)
        .await;
    let app = app_with_real_provider().await;
    let provider_id = provider(&app, "Wire OpenAI", "openai", &server.uri()).await;
    let profile_id = profile(
        &app,
        provider_id,
        "Wire OpenAI profile",
        json!(["low", "medium", "high"]),
        Value::Null,
    )
    .await;

    let body = first_session_request(&app, &server, profile_id, "high").await;
    assert_eq!(body["reasoning_effort"], "high", "{body}");
}

#[tokio::test]
async fn the_anthropic_wire_request_carries_the_resolved_output_config_effort() {
    const ANTHROPIC_SSE: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(ANTHROPIC_SSE),
        )
        .mount(&server)
        .await;
    let app = app_with_real_provider().await;
    let provider_id = provider(&app, "Wire Anthropic", "anthropic", &server.uri()).await;
    let profile_id = profile(
        &app,
        provider_id,
        "Wire Anthropic profile",
        json!(["low", "medium", "high"]),
        Value::Null,
    )
    .await;

    let body = first_session_request(&app, &server, profile_id, "medium").await;
    assert_eq!(body["output_config"]["effort"], "medium", "{body}");
}

/// A `TestApp` whose runners use the real provider clients, for the wiremock tests above.
async fn app_with_real_provider() -> TestApp {
    TestApp::new().await
}
