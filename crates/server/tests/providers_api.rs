//! Provider and model-profile API tests (PLAN M2.2, spec/04 §8, spec/10 §2.1).
//!
//! The network is never touched: the connection test and the remote-model list run against a
//! `wiremock` server, and the Demo provider's in-process `mock://` client covers the rest.

use nekowriter::api::dto::providers as dto;
use nekowriter::testing::{TestApp, assert_error};
use serde_json::{Value, json};
use wiremock::matchers::{bearer_token, body_string_contains, method, path};
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

/// `conflict.exists` carries `details.id` (spec/10 §4), so the UI can point at the row that
/// already holds the name. Provider names were the one place that answered `details: null`.
#[tokio::test]
async fn a_duplicate_provider_name_reports_the_existing_id() {
    let app = TestApp::new().await;
    let first = create_demo_provider(&app).await;
    let first_id = first["id"].as_i64().expect("id");

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
    assert_eq!(status, 409, "{body}");
    assert_error(&body, "conflict.exists");
    assert_eq!(body["error"]["details"]["id"], first_id, "{body}");

    // PATCH has the same rule, and must not count the row against itself.
    let other = create_openai_provider(&app, "https://example.invalid/v1", "sk-test").await;
    let other_id = other["id"].as_i64().expect("id");
    let (status, body) = app
        .patch(
            &format!("/api/providers/{other_id}"),
            json!({
                "name": "Demo",
                "protocol": "openai",
                "base_url": "https://example.invalid/v1",
                "extra_headers": {},
            }),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_error(&body, "conflict.exists");
    assert_eq!(body["error"]["details"]["id"], first_id, "{body}");
    let (status, body) = app
        .patch(
            &format!("/api/providers/{other_id}"),
            json!({
                "name": "Test OpenAI",
                "protocol": "openai",
                "base_url": "https://example.invalid/v1",
                "extra_headers": {},
            }),
        )
        .await;
    assert_eq!(status, 200, "a provider keeps its own name: {body}");
}

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
    // A real wire protocol: the Demo provider sends no request at all, so it currently accepts no
    // model option (D-132), and this test is about the options round trip.
    let provider = create_openai_provider(&app, "https://example.invalid/v1", "sk-test").await;
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
                "options": { "reasoning_replay": "always" },
            }),
        )
        .await;
    assert_eq!(status, 201, "{model}");
    let model_id = model["id"].as_i64().expect("id");
    assert_eq!(model["options"]["reasoning_replay"], "always");
    assert_eq!(
        model["ignored_options"],
        json!([]),
        "nothing is inert: {model}"
    );
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

    // The options must round-trip through `ModelOptions` (spec/04 §5). An unknown value used to
    // be stored verbatim; deserializing it at session creation then failed and reset *every*
    // option to its default, silently dropping `reasoning_replay` (and with it the DeepSeek/Kimi
    // thinking-mode replay of spec/04 §2).
    for options in [
        json!({ "reasoning_replay": "sometimes" }),
        json!({ "effort": "ultra" }),
        json!({ "max_tokens_field": "nonsense" }),
        json!({ "thinking_display": "verbose" }),
        json!({ "cache_ttl": "2h" }),
    ] {
        let (status, body) = app
            .post(
                "/api/model-profiles",
                json!({
                    "provider_id": provider_id,
                    "display_name": format!("Bad options {options}"),
                    "model": "demo-model",
                    "context_window": 1000,
                    "max_output_tokens": 100,
                    "options": options,
                }),
            )
            .await;
        assert_eq!(status, 400, "options {options}: {body}");
        assert!(
            body["error"]["details"]["fields"]["options"].is_string(),
            "options {options} must name the options field: {body}"
        );
    }

    // Valid options survive a round trip unchanged, so the check above rejects only what the
    // adapters cannot read and never normalizes a value the author set (spec/04 §5). The set is
    // split by protocol: storing an Anthropic-only field on this mock provider is now refused
    // (D-132), and the mock protocol answers to the OpenAI column.
    let openai_options = json!({
        "reasoning_replay": "always",
        "reasoning_effort": "medium",
        "max_tokens_field": "max_completion_tokens",
        "send_parallel_tool_calls": false,
        "stream_usage": false,
        "temperature": 0.7,
        "extra_body": { "top_k": 40 },
        "cache_ttl": "1h",
    });
    let (status, round_tripped) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Every OpenAI option",
                "model": "demo-model",
                "context_window": 1000,
                "max_output_tokens": 100,
                "options": openai_options,
            }),
        )
        .await;
    assert_eq!(status, 201, "{round_tripped}");
    assert_eq!(
        round_tripped["options"], openai_options,
        "options must round-trip"
    );
    let (status, _body) = app
        .delete(&format!(
            "/api/model-profiles/{}",
            round_tripped["id"].as_i64().expect("id")
        ))
        .await;
    assert_eq!(status, 204);

    let anthropic_options = json!({
        "reasoning_replay": "always",
        "effort": "xhigh",
        "thinking_display": "omitted",
        "cache_ttl": "1h",
        "refusal_fallback": true,
        "eager_input_streaming": false,
    });
    let (status, anthropic_provider) = app
        .post(
            "/api/providers",
            json!({
                "name": "Test Anthropic",
                "protocol": "anthropic",
                "base_url": "https://api.anthropic.test",
                "api_key": "sk-ant-test",
            }),
        )
        .await;
    assert_eq!(status, 201, "{anthropic_provider}");
    let anthropic_provider_id = anthropic_provider["id"].as_i64().expect("id");
    let (status, round_tripped) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": anthropic_provider_id,
                "display_name": "Every Anthropic option",
                "model": "claude-test",
                "context_window": 1000,
                "max_output_tokens": 100,
                "options": anthropic_options,
            }),
        )
        .await;
    assert_eq!(status, 201, "{round_tripped}");
    assert_eq!(
        round_tripped["options"], anthropic_options,
        "options must round-trip"
    );
    assert_eq!(round_tripped["ignored_options"], json!([]));
    let (status, _body) = app
        .delete(&format!(
            "/api/model-profiles/{}",
            round_tripped["id"].as_i64().expect("id")
        ))
        .await;
    assert_eq!(status, 204);

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
    assert!(
        models.as_array().expect("array").is_empty(),
        "the two round-trip profiles were already deleted: {models}"
    );
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

/// The connection test reports on the saved profile, so it must send that profile's
/// `ModelOptions` (spec/04 §5). The OpenAI preset's own reason for
/// `max_tokens_field: max_completion_tokens` is a model that rejects `max_tokens`; a test that
/// always sends the default reports a false failure for exactly the correct configuration.
#[tokio::test]
async fn test_connection_uses_the_saved_profile_options() {
    let server = MockServer::start().await;
    // Priority 1: the body a connection test used to send (`max_tokens`) fails, exactly like a
    // real model that only accepts `max_completion_tokens`.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_string_contains("\"max_tokens\":"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            "{\"error\":{\"message\":\"Unsupported parameter: 'max_tokens' is not supported with this model.\",\"type\":\"invalid_request_error\"}}",
        ))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_text("pong")),
        )
        .with_priority(2)
        .mount(&server)
        .await;

    let app = TestApp::new().await;
    let provider = create_openai_provider(&app, &server.uri(), "sk-test").await;
    let provider_id = provider["id"].as_i64().expect("id");
    let (status, profile) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "GPT",
                "model": "gpt-test",
                "context_window": 400000,
                "max_output_tokens": 128000,
                "options": { "max_tokens_field": "max_completion_tokens" },
            }),
        )
        .await;
    assert_eq!(status, 201, "{profile}");

    // The saved profile's options are used, so the provider accepts the request.
    let (status, body) = app
        .post(&format!("/api/providers/{provider_id}/test"), json!({}))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true, "{body}");
    let received = server.received_requests().await.expect("requests");
    let sent: Value = serde_json::from_slice(&received[0].body).expect("body");
    assert_eq!(sent["max_completion_tokens"], 32, "body: {sent}");
    assert!(sent.get("max_tokens").is_none(), "body: {sent}");
    // The request stays tiny (spec/04 §8).
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

// ───────────────────────────── demo protocol gate ─────────────────────────────

/// A `mock` provider is only usable with the demo provider enabled (spec/01 §6). Creating one is
/// already gated, but the gate has to cover a *stored* row too: a debug build writes the row, and
/// a later `--enable-demo-provider`-less run must not act on it.
///
/// The config helper cannot be exercised through HTTP, because
/// `Config::demo_provider_enabled()` ORs in `cfg!(debug_assertions)`, which is always true in a
/// test binary — the whole "disabled" state is unreachable through the router here. The gate is
/// therefore tested where it lives: on the row-level check every use funnels through.
#[test]
fn a_stored_demo_provider_is_only_usable_when_the_demo_provider_is_enabled() {
    let row = nekowriter::db::entities::providers::Model {
        id: 1,
        name: "Demo".to_owned(),
        protocol: "mock".to_owned(),
        base_url: "mock://demo".to_owned(),
        api_key: None,
        extra_headers: "{}".to_owned(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let error = nekowriter::services::providers::client_for(&row, false)
        .expect_err("the demo gate must reject a stored mock row when the flag is off");
    assert!(
        matches!(error.code, nekowriter::error::ErrorCode::ValidationFailed),
        "unexpected code: {:?}",
        error.code
    );
    assert!(
        nekowriter::services::providers::client_for(&row, true).is_ok(),
        "an enabled demo provider must build a client"
    );

    // An ordinary provider is unaffected by the flag.
    let openai = nekowriter::db::entities::providers::Model {
        protocol: "openai".to_owned(),
        base_url: "https://example.invalid/v1".to_owned(),
        ..row
    };
    assert!(nekowriter::services::providers::client_for(&openai, false).is_ok());
}

// ───────────────────────────── option → protocol validation (D-132) ─────────────────────────────

/// Creates an OpenAI-protocol provider pointing nowhere in particular.
async fn create_openai_provider_named(app: &TestApp, name: &str) -> i64 {
    let (status, body) = app
        .post(
            "/api/providers",
            json!({
                "name": name,
                "protocol": "openai",
                "base_url": "https://example.invalid/v1",
                "api_key": "sk-test",
            }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_i64().expect("id")
}

async fn create_anthropic_provider_named(app: &TestApp, name: &str) -> i64 {
    let (status, body) = app
        .post(
            "/api/providers",
            json!({
                "name": name,
                "protocol": "anthropic",
                "base_url": "https://api.anthropic.test",
                "api_key": "sk-ant-test",
            }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_i64().expect("id")
}

/// The bug report in one test: the author's profile #4 stores the **Anthropic** `effort` on an
/// **OpenAI**-protocol provider. The OpenAI adapter sends only `reasoning_effort`, so the request
/// carried no reasoning parameter, the turn succeeded, and nothing reported the mismatch.
///
/// Saving such a profile is now a `400 validation.failed` naming the field and the protocol.
#[tokio::test]
async fn an_anthropic_option_on_an_openai_provider_is_rejected_on_save() {
    let app = TestApp::new().await;
    let provider_id = create_openai_provider_named(&app, "Magpie").await;

    // The exact options blob of the author's profile #4.
    let (status, body) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "DeepSeek V4 Pro",
                "model": "deepseek-v4-pro",
                "context_window": 200000,
                "max_output_tokens": 16384,
                "options": {
                    "reasoning_replay": "always",
                    "max_tokens_field": "max_tokens",
                    "effort": "high",
                    "thinking_display": "omitted",
                    "cache_ttl": "1h",
                },
            }),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_error(&body, "validation.failed");
    let fields = &body["error"]["details"]["fields"];
    for field in ["effort", "thinking_display"] {
        let message = fields[field]
            .as_str()
            .unwrap_or_else(|| panic!("{field} must be named: {body}"));
        assert!(
            message.contains("protocol openai"),
            "the message names the protocol: {message}"
        );
    }
    assert_eq!(
        fields["effort"].as_str(),
        Some(
            "effort cannot be sent by a provider with protocol openai; use reasoning_effort instead"
        ),
        "{body}"
    );
    // The reachable options are not reported, so the message points only at the defect.
    assert!(fields.get("reasoning_replay").is_none(), "{body}");
    assert!(fields.get("cache_ttl").is_none(), "{body}");

    // The same intent under the OpenAI name is accepted.
    let (status, accepted) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "DeepSeek V4 Pro",
                "model": "deepseek-v4-pro",
                "context_window": 200000,
                "max_output_tokens": 16384,
                "options": {
                    "reasoning_replay": "always",
                    "reasoning_effort": "high",
                    "cache_ttl": "1h",
                },
            }),
        )
        .await;
    assert_eq!(status, 201, "{accepted}");
    assert_eq!(accepted["options"]["reasoning_effort"], "high");
    assert_eq!(accepted["ignored_options"], json!([]), "{accepted}");
}

/// The mirror image: an OpenAI-only option on an Anthropic provider.
#[tokio::test]
async fn an_openai_option_on_an_anthropic_provider_is_rejected_on_save() {
    let app = TestApp::new().await;
    let provider_id = create_anthropic_provider_named(&app, "Anthropic").await;
    let (status, body) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Claude",
                "model": "claude-test",
                "context_window": 1000000,
                "max_output_tokens": 128000,
                "options": {
                    "reasoning_effort": "high",
                    "temperature": 0.5,
                    "effort": "high",
                },
            }),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_error(&body, "validation.failed");
    let fields = &body["error"]["details"]["fields"];
    for field in ["reasoning_effort", "temperature"] {
        let message = fields[field]
            .as_str()
            .unwrap_or_else(|| panic!("{field} must be named: {body}"));
        assert!(
            message.contains("protocol anthropic"),
            "the message names the protocol: {message}"
        );
    }
    assert!(fields.get("effort").is_none(), "{body}");

    // PATCH is subject to the same rule; it is not a create-only check.
    let (status, profile) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Claude",
                "model": "claude-test",
                "context_window": 1000000,
                "max_output_tokens": 128000,
                "options": { "effort": "high" },
            }),
        )
        .await;
    assert_eq!(status, 201, "{profile}");
    let profile_id = profile["id"].as_i64().expect("id");
    let (status, body) = app
        .patch(
            &format!("/api/model-profiles/{profile_id}"),
            json!({
                "provider_id": provider_id,
                "display_name": "Claude",
                "model": "claude-test",
                "context_window": 1000000,
                "max_output_tokens": 128000,
                "options": { "effort": "high", "reasoning_effort": "high" },
            }),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"]["details"]["fields"]["reasoning_effort"].is_string());

    // Moving the profile to another provider re-validates against the new protocol.
    let openai_id = create_openai_provider_named(&app, "OpenAI").await;
    let (status, body) = app
        .patch(
            &format!("/api/model-profiles/{profile_id}"),
            json!({
                "provider_id": openai_id,
                "display_name": "Claude",
                "model": "gpt-test",
                "context_window": 400000,
                "max_output_tokens": 128000,
                "options": { "effort": "high" },
            }),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"]["details"]["fields"]["effort"].as_str(),
        Some(
            "effort cannot be sent by a provider with protocol openai; use reasoning_effort instead"
        ),
        "{body}"
    );
    // The stranded field removed and the name fixed, the move is accepted.
    let (status, moved) = app
        .patch(
            &format!("/api/model-profiles/{profile_id}"),
            json!({
                "provider_id": openai_id,
                "display_name": "Claude",
                "model": "gpt-test",
                "context_window": 400000,
                "max_output_tokens": 128000,
                "options": { "reasoning_effort": "high" },
            }),
        )
        .await;
    assert_eq!(status, 200, "{moved}");
    assert_eq!(moved["provider_id"], openai_id);
    assert_eq!(moved["ignored_options"], json!([]), "{moved}");
}

/// A row written **before** this rule must stay readable: failing the read would lock the author
/// out of the settings page that has to fix it. The mismatch is reported instead.
#[tokio::test]
async fn an_already_stored_mismatch_is_readable_and_reported() {
    let app = TestApp::new().await;
    let provider_id = create_openai_provider_named(&app, "Magpie").await;
    // Simulate the pre-fix row: the API refuses to write it, so write it the way the old version
    // did, straight into the table.
    let now = chrono::Utc::now();
    let stored = serde_json::json!({
        "reasoning_replay": "always",
        "max_tokens_field": "max_tokens",
        "effort": "high",
        "thinking_display": "omitted",
        "cache_ttl": "1h",
    });
    use nekowriter::db::entities::model_profiles;
    use sea_orm::{ActiveModelTrait, Set};
    let id = model_profiles::ActiveModel {
        provider_id: Set(i32::try_from(provider_id).expect("provider id")),
        display_name: Set("DeepSeek V4 Pro".to_owned()),
        model: Set("deepseek-v4-pro".to_owned()),
        context_window: Set(200_000),
        max_output_tokens: Set(16_384),
        options: Set(stored.to_string()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(app.db().conn())
    .await
    .expect("insert the legacy row")
    .id;

    // The list, the single GET and the provider GET all stay 200 and report the mismatch.
    for (status, body) in [
        app.get("/api/model-profiles").await,
        app.get(&format!("/api/model-profiles/{id}")).await,
    ] {
        assert_eq!(status, 200, "{body}");
        let profile = if body.is_array() {
            body[0].clone()
        } else {
            body.clone()
        };
        assert_eq!(
            profile["ignored_options"],
            json!(["effort", "thinking_display"]),
            "the inert options are named: {body}"
        );
        // The stored blob is returned untouched: reading never rewrites the author's configuration.
        assert_eq!(profile["options"], stored, "{body}");
    }
    let (status, provider) = app.get(&format!("/api/providers/{provider_id}")).await;
    assert_eq!(status, 200, "{provider}");
    assert_eq!(
        provider["models"][0]["ignored_options"],
        json!(["effort", "thinking_display"]),
        "{provider}"
    );

    // The startup scan finds the same row, so the mismatch is in the log even when Settings is
    // never opened.
    let mismatches = nekowriter::services::providers::stored_option_mismatches(app.db().conn())
        .await
        .expect("scan");
    assert_eq!(mismatches.len(), 1, "{mismatches:?}");
    let (mismatch_id, profile_name, provider_name, options) = &mismatches[0];
    assert_eq!(*mismatch_id, id);
    assert_eq!(profile_name, "DeepSeek V4 Pro");
    assert_eq!(provider_name, "Magpie");
    assert_eq!(
        options,
        &vec!["effort".to_owned(), "thinking_display".to_owned()]
    );

    // And the fix is one PATCH through the UI: replace `effort` with `reasoning_effort` and drop
    // the other Anthropic field.
    let (status, fixed) = app
        .patch(
            &format!("/api/model-profiles/{id}"),
            json!({
                "provider_id": provider_id,
                "display_name": "DeepSeek V4 Pro",
                "model": "deepseek-v4-pro",
                "context_window": 200000,
                "max_output_tokens": 16384,
                "options": {
                    "reasoning_replay": "always",
                    "reasoning_effort": "high",
                    "cache_ttl": "1h",
                },
            }),
        )
        .await;
    assert_eq!(status, 200, "{fixed}");
    assert_eq!(fixed["ignored_options"], json!([]), "{fixed}");
    assert_eq!(fixed["options"]["reasoning_effort"], "high");
}

/// The option table the settings page renders comes from the adapters (`crates/llm`), so the UI
/// cannot drift from what the request body can carry.
#[tokio::test]
async fn the_option_specs_endpoint_publishes_the_protocol_table() {
    let app = TestApp::new().await;
    let (status, body) = app.get("/api/provider-presets/options").await;
    assert_eq!(status, 200, "{body}");
    let specs = body.as_array().expect("array");
    let spec = |key: &str| {
        specs
            .iter()
            .find(|spec| spec["key"] == key)
            .unwrap_or_else(|| panic!("missing spec {key}"))
    };

    // The pair the bug report is about.
    assert_eq!(spec("effort")["protocols"], json!(["anthropic"]));
    assert_eq!(
        spec("effort")["values"],
        json!(["low", "medium", "high", "xhigh", "max"])
    );
    assert_eq!(spec("effort")["kind"], "choice");
    assert_eq!(spec("reasoning_effort")["protocols"], json!(["openai"]));
    // Free-form on purpose: gateways accept values OpenAI does not document.
    assert_eq!(spec("reasoning_effort")["kind"], "text");
    assert_eq!(spec("reasoning_effort")["values"], json!([]));

    // Read by both adapters (verified in `crates/llm/tests/model_options.rs`).
    for key in ["reasoning_replay", "cache_ttl"] {
        assert_eq!(
            spec(key)["protocols"],
            json!(["openai", "anthropic"]),
            "{key} is read by both adapters"
        );
    }
    // The Demo provider sends no request, so no option may be offered for it (D-132).
    for spec in specs {
        assert!(
            !spec["protocols"]
                .as_array()
                .expect("protocols")
                .iter()
                .any(|protocol| protocol == "mock"),
            "the mock protocol must carry no option: {spec}"
        );
    }
    for key in [
        "thinking_display",
        "refusal_fallback",
        "eager_input_streaming",
    ] {
        assert_eq!(spec(key)["protocols"], json!(["anthropic"]), "{key}");
    }
    for key in [
        "temperature",
        "send_parallel_tool_calls",
        "stream_usage",
        "max_tokens_field",
        "extra_body",
    ] {
        assert_eq!(spec(key)["protocols"], json!(["openai"]), "{key}");
    }
    // Every option is described exactly once, so the UI cannot render a duplicate or miss one.
    let mut keys: Vec<&str> = specs
        .iter()
        .map(|spec| spec["key"].as_str().expect("key"))
        .collect();
    keys.sort_unstable();
    let mut expected = vec![
        "cache_ttl",
        "eager_input_streaming",
        "effort",
        "extra_body",
        "max_tokens_field",
        "reasoning_effort",
        "reasoning_replay",
        "refusal_fallback",
        "send_parallel_tool_calls",
        "stream_usage",
        "temperature",
        "thinking_display",
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected);
}

/// A shipped preset carrying an option its own protocol cannot send would be a default defect, and
/// worse than the author's instance because every new install would inherit it. Every preset is
/// pinned against the option table here.
#[tokio::test]
async fn every_shipped_preset_only_carries_options_its_protocol_can_send() {
    let presets = nekowriter::services::providers::presets(true);
    assert!(!presets.is_empty());
    let options_of = |preset: &dto::ProviderPresetDto| {
        let value = serde_json::to_value(&preset.options).expect("options");
        serde_json::from_value::<serde_json::Map<String, Value>>(value).expect("object")
    };
    let protocol_of = |preset: &dto::ProviderPresetDto| match preset.protocol.as_str() {
        "openai" => nekowriter_llm::Protocol::Openai,
        "anthropic" => nekowriter_llm::Protocol::Anthropic,
        other => {
            assert_eq!(other, "mock");
            nekowriter_llm::Protocol::Mock
        }
    };
    for preset in &presets {
        let options = options_of(preset);
        let protocol = protocol_of(preset);
        // Only a *set* field can strand; the preset DTO omits unset ones.
        for key in options.keys() {
            let known = nekowriter_llm::ModelOptionKey::ALL
                .into_iter()
                .find(|candidate| candidate.as_str() == key);
            let Some(key) = known else {
                panic!("preset {:?} carries an unknown option {key}", preset.key);
            };
            assert!(
                key.reachable_by(protocol),
                "preset {:?} is {} but carries {}, which that protocol cannot send",
                preset.key,
                preset.protocol,
                key.as_str()
            );
        }
    }

    // Spot-check the two presets that carry options, so a silently emptied preset fails too.
    let anthropic = presets
        .iter()
        .find(|preset| preset.key == nekowriter::services::providers::PresetKey::Anthropic)
        .expect("anthropic");
    assert_eq!(anthropic.options.effort.as_deref(), Some("high"));
    let openai = presets
        .iter()
        .find(|preset| preset.key == nekowriter::services::providers::PresetKey::Openai)
        .expect("openai");
    assert_eq!(
        openai.options.max_tokens_field.as_deref(),
        Some("max_completion_tokens")
    );
    let deepseek = presets
        .iter()
        .find(|preset| preset.key == nekowriter::services::providers::PresetKey::Deepseek)
        .expect("deepseek");
    assert_eq!(deepseek.options.reasoning_replay.as_deref(), Some("always"));
}
