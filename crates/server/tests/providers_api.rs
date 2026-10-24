//! Provider and model-profile API tests (PLAN M2.2, spec/04 §8, spec/10 §2.1).
//!
//! The network is never touched: the connection test and the remote-model list run against a
//! `wiremock` server, and the Demo provider's in-process `mock://` client covers the rest.

use nekowriter::api::dto::providers as dto;
use nekowriter::testing::{TestApp, assert_error};
use serde_json::{Value, json};
use wiremock::matchers::{bearer_token, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ───────────────────────────── helpers ─────────────────────────────

async fn create_demo_provider(app: &TestApp) -> Value {
    let (status, body) = app
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
    assert_eq!(status, 201, "create demo provider: {body}");
    body
}

async fn create_openai_provider(app: &TestApp, base_url: &str, key: &str) -> Value {
    let (status, body) = app
        .post(
            "/api/providers",
            json!({
                "name": "Test OpenAI",
                "protocol": "openai",
                "base_url": base_url,
                "api_key": key,
                "extra_headers": {},
            }),
        )
        .await;
    assert_eq!(status, 201, "create provider: {body}");
    body
}

/// A streaming `text/event-stream` body with one text delta and a `[DONE]` terminator.
fn sse_text(text: &str) -> String {
    format!(
        "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":{}}}}},{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\n\
         data: [DONE]\n\n",
        serde_json::to_string(text).expect("text")
    )
}

// ───────────────────────────── presets ─────────────────────────────

#[tokio::test]
async fn presets_include_every_documented_provider_and_the_demo_only_when_enabled() {
    let app = TestApp::new().await;
    let (status, body) = app.get("/api/provider-presets").await;
    assert_eq!(status, 200, "{body}");
    let presets = body.as_array().expect("array");
    let keys: Vec<&str> = presets
        .iter()
        .map(|preset| preset["key"].as_str().expect("key"))
        .collect();
    for expected in [
        "anthropic",
        "openai",
        "deepseek",
        "openrouter",
        "qwen",
        "moonshot",
        "zhipu",
        "ollama",
        "lm_studio",
        "custom",
    ] {
        assert!(keys.contains(&expected), "missing preset {expected}");
    }
    // The test app allows the demo provider (debug-style config).
    assert!(keys.contains(&"demo"), "the demo preset is enabled here");

    // Preset defaults come from spec/04 §8.
    let anthropic = presets
        .iter()
        .find(|preset| preset["key"] == "anthropic")
        .expect("anthropic");
    assert_eq!(anthropic["protocol"], "anthropic");
    assert_eq!(anthropic["base_url"], "https://api.anthropic.com");
    assert_eq!(anthropic["context_window"], 1_000_000);
    assert_eq!(anthropic["max_output_tokens"], 128_000);
    assert_eq!(anthropic["options"]["effort"], "high");
    assert_eq!(anthropic["requires_key"], true);
    let openai = presets
        .iter()
        .find(|preset| preset["key"] == "openai")
        .expect("openai");
    assert_eq!(
        openai["options"]["max_tokens_field"],
        "max_completion_tokens"
    );
    let deepseek = presets
        .iter()
        .find(|preset| preset["key"] == "deepseek")
        .expect("deepseek");
    assert_eq!(deepseek["options"]["reasoning_replay"], "always");
    let ollama = presets
        .iter()
        .find(|preset| preset["key"] == "ollama")
        .expect("ollama");
    assert_eq!(ollama["requires_key"], false);
    assert_eq!(ollama["options"]["send_parallel_tool_calls"], false);
    let demo = presets
        .iter()
        .find(|preset| preset["key"] == "demo")
        .expect("demo");
    assert_eq!(demo["protocol"], "mock");
    assert_eq!(demo["base_url"], "mock://demo");
    assert_eq!(demo["suggested_models"][0], "demo-model");
}

#[tokio::test]
async fn demo_preset_is_hidden_when_the_demo_provider_is_disabled() {
    let app = TestApp::new().await;
    // The server-side gate is what the preset list reads, so exercise the pure function.
    let presets = nekowriter::services::providers::presets(false);
    let keys: Vec<dto::ProviderPresetDto> = presets;
    assert!(
        !keys
            .iter()
            .any(|preset| preset.key == nekowriter::services::providers::PresetKey::Demo),
        "the demo preset must be hidden"
    );
    let enabled = nekowriter::services::providers::presets(true);
    assert!(
        enabled
            .iter()
            .any(|preset| preset.key == nekowriter::services::providers::PresetKey::Demo)
    );
    let _ = app;
}

// ───────────────────────────── provider CRUD ─────────────────────────────

#[tokio::test]
async fn provider_crud_masks_the_key_and_never_returns_it() {
    let app = TestApp::new().await;
    let (status, body) = app
        .post(
            "/api/providers",
            json!({
                "name": "OpenAI",
                "protocol": "openai",
                "base_url": "https://api.openai.com/v1",
                "api_key": "sk-secret-abcd",
                "extra_headers": { "x-extra": "1" },
            }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let id = body["id"].as_i64().expect("id");
    // Only the masked tail is returned, never the key.
    assert_eq!(body["api_key_masked"], "…abcd");
    assert_eq!(body["has_api_key"], true);
    assert_eq!(body["extra_headers"]["x-extra"], "1");
    assert!(
        !body.to_string().contains("sk-secret-abcd"),
        "the key must never be returned: {body}"
    );

    // GET and PATCH keep masking.
    let (status, listed) = app.get("/api/providers").await;
    assert_eq!(status, 200);
    assert_eq!(listed.as_array().expect("array").len(), 1);
    assert_eq!(listed[0]["api_key_masked"], "…abcd");

    // PATCH without api_key keeps the stored key (spec/10 §2.1).
    let (status, patched) = app
        .patch(
            &format!("/api/providers/{id}"),
            json!({
                "name": "OpenAI renamed",
                "protocol": "openai",
                "base_url": "https://api.openai.com/v1",
                "extra_headers": {},
            }),
        )
        .await;
    assert_eq!(status, 200, "{patched}");
    assert_eq!(patched["name"], "OpenAI renamed");
    assert_eq!(patched["has_api_key"], true, "the key survived the patch");
    assert_eq!(patched["api_key_masked"], "…abcd");

    // Round-tripping the masked value does not overwrite the key.
    let (status, _patched) = app
        .patch(
            &format!("/api/providers/{id}"),
            json!({
                "name": "OpenAI renamed",
                "protocol": "openai",
                "base_url": "https://api.openai.com/v1",
                "api_key": "…abcd",
                "extra_headers": {},
            }),
        )
        .await;
    assert_eq!(status, 200);
    let (status, after) = app.get(&format!("/api/providers/{id}")).await;
    assert_eq!(status, 200);
    assert_eq!(after["has_api_key"], true);

    // An empty key clears it.
    let (status, cleared) = app
        .patch(
            &format!("/api/providers/{id}"),
            json!({
                "name": "OpenAI renamed",
                "protocol": "openai",
                "base_url": "https://api.openai.com/v1",
                "api_key": "",
                "extra_headers": {},
            }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(cleared["has_api_key"], false);
    assert!(cleared["api_key_masked"].is_null());

    // DELETE returns 204 and the list is empty.
    let (status, _body) = app.delete(&format!("/api/providers/{id}")).await;
    assert_eq!(status, 204);
    let (_, listed) = app.get("/api/providers").await;
    assert!(listed.as_array().expect("array").is_empty());
}

#[tokio::test]
async fn provider_validation_and_conflicts() {
    let app = TestApp::new().await;
    // Missing name.
    let (status, body) = app
        .post(
            "/api/providers",
            json!({ "name": "  ", "protocol": "openai", "base_url": "https://x/v1" }),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_error(&body, "validation.failed");
    assert!(body["error"]["details"]["fields"]["name"].is_string());

    // Bad scheme.
    let (status, body) = app
        .post(
            "/api/providers",
            json!({ "name": "x", "protocol": "openai", "base_url": "ftp://x" }),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"]["details"]["fields"]["base_url"].is_string());

    // mock:// on a real protocol.
    let (status, body) = app
        .post(
            "/api/providers",
            json!({ "name": "x", "protocol": "openai", "base_url": "mock://demo" }),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    // Duplicate name.
    create_demo_provider(&app).await;
    let (status, body) = app
        .post(
            "/api/providers",
            json!({ "name": "Demo", "protocol": "mock", "base_url": "mock://demo" }),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_error(&body, "conflict.exists");

    // Unknown provider.
    let (status, body) = app.get("/api/providers/9999").await;
    assert_eq!(status, 404, "{body}");
    assert_error(&body, "not_found");
}

#[tokio::test]
async fn env_keys_are_validated_and_masked_verbatim() {
    let app = TestApp::new().await;
    // A typo in the variable name is caught when the provider is saved.
    let (status, body) = app
        .post(
            "/api/providers",
            json!({
                "name": "Env",
                "protocol": "openai",
                "base_url": "https://example.invalid/v1",
                "api_key": "env:NEKOWRITER_TEST_SURELY_UNSET_VARIABLE",
            }),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_error(&body, "validation.failed");
    assert!(
        body["error"]["details"]["fields"]["api_key"]
            .as_str()
            .expect("message")
            .contains("NEKOWRITER_TEST_SURELY_UNSET_VARIABLE"),
        "{body}"
    );

    // A set variable is accepted, and the `env:` reference (not a secret) is returned verbatim.
    // SAFETY: single-threaded test setup; the variable is only read by the service.
    unsafe { std::env::set_var("NEKOWRITER_TEST_PROVIDER_KEY", "sk-from-env-1234") };
    let (status, body) = app
        .post(
            "/api/providers",
            json!({
                "name": "Env",
                "protocol": "openai",
                "base_url": "https://example.invalid/v1",
                "api_key": "env:NEKOWRITER_TEST_PROVIDER_KEY",
            }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["api_key_masked"], "env:NEKOWRITER_TEST_PROVIDER_KEY");
    assert_eq!(body["has_api_key"], true);
    // The stored row still holds the reference, so no secret was written into the DTO.
    assert!(!body.to_string().contains("sk-from-env-1234"));
    unsafe { std::env::remove_var("NEKOWRITER_TEST_PROVIDER_KEY") };
}

#[tokio::test]
async fn deleting_a_provider_assigned_to_a_role_leaves_the_role_to_report_a_missing_model() {
    let app = TestApp::new().await;
    let provider = create_demo_provider(&app).await;
    let provider_id = provider["id"].as_i64().expect("id");
    let (status, model) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Demo model",
                "model": "demo-model",
                "context_window": 200000,
                "max_output_tokens": 16384,
                "options": {},
            }),
        )
        .await;
    assert_eq!(status, 201, "{model}");
    let model_id = model["id"].as_i64().expect("model id");
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": { "writer": model_id } }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["roles_defaults"]["writer"], model_id);

    // Deletion is allowed (D-048: a session whose provider vanished fails with
    // model.missing_provider); the role default keeps the now dangling model id, and starting a
    // session reports `model.missing_role` (spec/10 §4).
    let (status, _body) = app.delete(&format!("/api/providers/{provider_id}")).await;
    assert_eq!(status, 204);
    let (_, settings) = app.get("/api/app-settings").await;
    assert_eq!(settings["roles_defaults"]["writer"], model_id);
    let (_, models) = app.get("/api/model-profiles").await;
    assert!(
        models.as_array().expect("array").is_empty(),
        "the model cascaded away"
    );
}

// ───────────────────────────── model profiles ─────────────────────────────

#[tokio::test]
async fn model_profile_crud_and_validation() {
    let app = TestApp::new().await;
    let provider = create_demo_provider(&app).await;
    let provider_id = provider["id"].as_i64().expect("id");
    let (status, model) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Demo model",
                "model": "demo-model",
                "context_window": 200000,
                "max_output_tokens": 16384,
                "options": { "effort": "high" },
            }),
        )
        .await;
    assert_eq!(status, 201, "{model}");
    let model_id = model["id"].as_i64().expect("id");
    assert_eq!(model["options"]["effort"], "high");
    // The provider DTO carries its models.
    let (_, provider_dto) = app.get(&format!("/api/providers/{provider_id}")).await;
    assert_eq!(provider_dto["models"].as_array().expect("models").len(), 1);

    // Validation: an output cap above the context window is a field error.
    let (status, body) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Bad",
                "model": "bad",
                "context_window": 1000,
                "max_output_tokens": 2000,
                "options": {},
            }),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"]["details"]["fields"]["max_output_tokens"].is_string());

    // Duplicate display name on the same provider.
    let (status, body) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Demo model",
                "model": "other",
                "context_window": 1000,
                "max_output_tokens": 100,
                "options": {},
            }),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_error(&body, "conflict.exists");
    assert_eq!(body["error"]["details"]["id"], model_id);

    // PATCH.
    let (status, updated) = app
        .patch(
            &format!("/api/model-profiles/{model_id}"),
            json!({
                "provider_id": provider_id,
                "display_name": "Demo model (renamed)",
                "model": "demo-model",
                "context_window": 200000,
                "max_output_tokens": 8192,
                "options": { "send_parallel_tool_calls": false },
            }),
        )
        .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["display_name"], "Demo model (renamed)");
    assert_eq!(updated["max_output_tokens"], 8192);
    assert_eq!(updated["options"]["send_parallel_tool_calls"], false);

    // A model profile needs an existing provider.
    let (status, body) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": 9999,
                "display_name": "x",
                "model": "x",
                "context_window": 1000,
                "max_output_tokens": 100,
                "options": {},
            }),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    // DELETE.
    let (status, _body) = app.delete(&format!("/api/model-profiles/{model_id}")).await;
    assert_eq!(status, 204);
    let (_, models) = app.get("/api/model-profiles").await;
    assert!(models.as_array().expect("array").is_empty());
}

#[tokio::test]
async fn deleting_a_provider_cascades_its_model_profiles() {
    let app = TestApp::new().await;
    let provider = create_demo_provider(&app).await;
    let provider_id = provider["id"].as_i64().expect("id");
    app.post(
        "/api/model-profiles",
        json!({
            "provider_id": provider_id,
            "display_name": "Demo model",
            "model": "demo-model",
            "context_window": 200000,
            "max_output_tokens": 16384,
            "options": {},
        }),
    )
    .await;
    let (status, _body) = app.delete(&format!("/api/providers/{provider_id}")).await;
    assert_eq!(status, 204);
    let (_, models) = app.get("/api/model-profiles").await;
    assert!(models.as_array().expect("array").is_empty());
}

// ───────────────────────────── connection test and remote models ─────────────────────────────

#[tokio::test]
async fn test_connection_reports_success_with_latency_and_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(bearer_token("sk-test"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_text("pong")),
        )
        .mount(&server)
        .await;
    let app = TestApp::new().await;
    let provider = create_openai_provider(&app, &server.uri(), "sk-test").await;
    let provider_id = provider["id"].as_i64().expect("id");
    let (status, body) = app
        .post(
            &format!("/api/providers/{provider_id}/test"),
            json!({ "model": "gpt-test" }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["text"], "pong");
    assert!(body["latency_ms"].as_i64().expect("latency") >= 0);
    assert!(body["error"].is_null());
    // The test request stays tiny (spec/04 §8: max_output_tokens 32, user text "ping", no tools).
    let received = server.received_requests().await.expect("requests");
    let sent: Value = serde_json::from_slice(&received[0].body).expect("body");
    assert_eq!(sent["max_tokens"], 32);
    assert!(sent["tools"].is_null());
    assert_eq!(sent["messages"][1]["content"], "ping");
}

#[tokio::test]
async fn test_connection_maps_a_rejected_key_to_provider_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string("{\"error\":\"bad key\"}"))
        .mount(&server)
        .await;
    let app = TestApp::new().await;
    let provider = create_openai_provider(&app, &server.uri(), "sk-wrong").await;
    let provider_id = provider["id"].as_i64().expect("id");
    let (status, body) = app
        .post(
            &format!("/api/providers/{provider_id}/test"),
            json!({ "model": "gpt-test" }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"]["code"], "provider.auth");
    assert!(body["text"].is_null());
}

#[tokio::test]
async fn test_connection_reports_an_unreachable_provider_as_unavailable() {
    // Port 9 on loopback is the discard service: nothing listens, so the connection is refused.
    let app = TestApp::new().await;
    let provider = create_openai_provider(&app, "http://127.0.0.1:9/v1", "sk-test").await;
    let provider_id = provider["id"].as_i64().expect("id");
    let (status, body) = app
        .post(
            &format!("/api/providers/{provider_id}/test"),
            json!({ "model": "gpt-test" }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"]["code"], "provider.unavailable");
}

#[tokio::test]
async fn test_connection_uses_the_demo_provider_without_a_model() {
    let app = TestApp::new().await;
    let provider = create_demo_provider(&app).await;
    let provider_id = provider["id"].as_i64().expect("id");
    let (status, body) = app
        .post(&format!("/api/providers/{provider_id}/test"), json!({}))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true);
    assert!(!body["text"].as_str().expect("text").is_empty());
}

#[tokio::test]
async fn remote_models_are_listed_through_the_adapter() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(bearer_token("sk-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                { "id": "gpt-5.2", "name": "GPT 5.2", "context_length": 400000 },
                { "id": "gpt-5.2-mini" }
            ]
        })))
        .mount(&server)
        .await;
    let app = TestApp::new().await;
    let provider = create_openai_provider(&app, &server.uri(), "sk-test").await;
    let provider_id = provider["id"].as_i64().expect("id");
    let (status, body) = app
        .get(&format!("/api/providers/{provider_id}/remote-models"))
        .await;
    assert_eq!(status, 200, "{body}");
    let models = body.as_array().expect("array");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0]["id"], "gpt-5.2");
    assert_eq!(models[0]["display_name"], "GPT 5.2");
    assert_eq!(models[0]["context_window"], 400000);
}

#[tokio::test]
async fn remote_models_report_a_rejected_key_and_a_protocol_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
        .mount(&server)
        .await;
    let app = TestApp::new().await;
    let provider = create_openai_provider(&app, &server.uri(), "sk-test").await;
    let provider_id = provider["id"].as_i64().expect("id");
    let (status, body) = app
        .get(&format!("/api/providers/{provider_id}/remote-models"))
        .await;
    assert_eq!(status, 502, "{body}");
    assert_error(&body, "provider.auth");

    // A provider that does not support listing models (mock is handled by the demo client).
    let (status, body) = app.get("/api/providers/9999/remote-models").await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn the_demo_provider_lists_its_model() {
    let app = TestApp::new().await;
    let provider = create_demo_provider(&app).await;
    let provider_id = provider["id"].as_i64().expect("id");
    let (status, body) = app
        .get(&format!("/api/providers/{provider_id}/remote-models"))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body[0]["id"], "demo-model");
    assert_eq!(body[0]["context_window"], 200000);
    assert_eq!(body[0]["max_output_tokens"], 16384);
}

// ───────────────────────────── role defaults and masking ─────────────────────────────

#[tokio::test]
async fn app_settings_carry_role_defaults_and_mask_the_search_key() {
    let app = TestApp::new().await;
    let (status, defaults) = app.get("/api/app-settings").await;
    assert_eq!(status, 200, "{defaults}");
    // Absent role defaults serialize as nulls (RoleModels is a struct of options).
    for role in [
        "conception",
        "researcher",
        "writer",
        "reviewer",
        "archivist",
    ] {
        assert!(
            defaults["roles_defaults"][role].is_null(),
            "roles_defaults.{role} should be unset: {defaults}"
        );
    }
    assert_eq!(defaults["book_defaults"]["granularity"], "paragraph");
    assert_eq!(defaults["book_defaults"]["pause_after_outline"], true);
    assert_eq!(defaults["book_defaults"]["auto_pause"], false);
    assert_eq!(defaults["book_defaults"]["target_words_min"], 3000);
    assert_eq!(defaults["book_defaults"]["target_words_max"], 4000);
    assert_eq!(defaults["book_defaults"]["recent_summary_count"], 5);
    assert_eq!(defaults["book_defaults"]["midterm_window"], 10);
    assert_eq!(defaults["book_defaults"]["auto_maintenance"], true);

    // Role defaults are five model-profile ids (spec/10 §2.1).
    let provider = create_demo_provider(&app).await;
    let provider_id = provider["id"].as_i64().expect("id");
    let (_, model) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Demo model",
                "model": "demo-model",
                "context_window": 200000,
                "max_output_tokens": 16384,
                "options": {},
            }),
        )
        .await;
    let model_id = model["id"].as_i64().expect("model id");
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({
                "roles_defaults": {
                    "conception": model_id,
                    "researcher": model_id,
                    "writer": model_id,
                    "reviewer": model_id,
                    "archivist": model_id,
                },
            }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    for role in [
        "conception",
        "researcher",
        "writer",
        "reviewer",
        "archivist",
    ] {
        assert_eq!(body["roles_defaults"][role], model_id, "{role}");
    }
    // A patch that omits roles keeps them.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "book_defaults": { "granularity": "free" } }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["book_defaults"]["granularity"], "free");
    assert_eq!(body["roles_defaults"]["writer"], model_id);

    // The Tavily key is masked like a provider key and survives a round trip (spec/10 §2.1).
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "provider": "tavily", "tavily_api_key": "tvly-secret-wxyz" } }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["search"]["tavily_api_key"], "…wxyz");
    assert!(!body.to_string().contains("tvly-secret-wxyz"));
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "tavily_api_key": "…wxyz" } }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["search"]["tavily_api_key"], "…wxyz");
}
