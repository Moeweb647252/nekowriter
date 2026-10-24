//! Anthropic adapter tests (spec/04 §9).

use nekowriter_llm::anthropic::{AnthropicClient, build_body};
use nekowriter_llm::retry::LlmError;
use nekowriter_llm::types::{
    CacheHints, CacheTtl, ChatRequest, Effort, Message, ModelOptions, ProviderConfig,
    ProviderPayload, StopReason, StreamEvent, TextBlock, ThinkingDisplay, ToolDef, ToolResult,
};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("cannot read {path}: {err}"))
}

fn config(base_url: &str) -> ProviderConfig {
    ProviderConfig {
        protocol: nekowriter_llm::types::Protocol::Anthropic,
        base_url: base_url.to_owned(),
        api_key: Some("sk-ant-test".to_owned()),
        extra_headers: Default::default(),
    }
}

fn request() -> ChatRequest {
    ChatRequest {
        model: "claude-test".to_owned(),
        system: "You are a writer.".to_owned(),
        messages: vec![Message::User {
            blocks: vec![TextBlock {
                text: "Write.".to_owned(),
                cache: true,
            }],
        }],
        tools: vec![ToolDef {
            name: "insert_text".to_owned(),
            description: "Write text.".to_owned(),
            input_schema: serde_json::json!({"type": "object"}),
        }],
        max_output_tokens: 2048,
        parallel_tool_calls: false,
        cache: CacheHints {
            ttl: CacheTtl::OneHour,
        },
        options: ModelOptions::default(),
        demo_hints: None,
    }
}

async fn serve(name: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-test"))
        .and(header("anthropic-version", "2026-10-21"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .insert_header("request-id", "msg_req_1")
                .set_body_string(fixture(name)),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn thinking_signature_text_and_tool_use() {
    let server = serve("anthropic_thinking_tool.sse").await;
    let client = AnthropicClient::new(config(&server.uri())).expect("client");
    let mut events = Vec::new();
    let turn = client
        .run(&request(), &mut |event| events.push(event))
        .await
        .expect("turn");

    assert_eq!(turn.text, "Here is the scene.");
    assert_eq!(turn.reasoning, "The author wants a scene.");
    assert_eq!(turn.tool_calls.len(), 1);
    assert_eq!(turn.tool_calls[0].id, "toolu_1");
    assert_eq!(turn.tool_calls[0].name, "insert_text");
    assert_eq!(
        turn.tool_calls[0].input_text,
        r#"{"text":"The wind rose.","paragraph_end":true}"#
    );
    assert_eq!(turn.stop_reason, StopReason::ToolUse);
    assert_eq!(turn.usage.input_tokens, 120);
    assert_eq!(turn.usage.cached_input_tokens, 80);
    assert_eq!(turn.usage.cache_write_tokens, 10);
    assert_eq!(turn.usage.output_tokens, 45);
    assert_eq!(turn.request_id.as_deref(), Some("msg_req_1"));

    let reasoning: String = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ReasoningDelta { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(reasoning, "The author wants a scene.");

    // The replay payload keeps every block exactly, including thinking with its signature.
    let ProviderPayload::Anthropic { content } = &turn.payload else {
        panic!("expected anthropic payload");
    };
    assert_eq!(content.len(), 3);
    assert_eq!(content[0]["type"], "thinking");
    assert_eq!(content[0]["signature"], "sig-abc");
    assert_eq!(content[0]["thinking"], "The author wants a scene.");
    assert_eq!(content[1]["type"], "text");
    assert_eq!(content[1]["text"], "Here is the scene.");
    assert_eq!(content[2]["type"], "tool_use");
    assert_eq!(content[2]["id"], "toolu_1");
    assert_eq!(content[2]["input"]["paragraph_end"], true);

    // Tool-call ordinals ignore thinking and text blocks.
    let starts: Vec<u32> = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ToolCallStart { call_index, .. } => Some(*call_index),
            _ => None,
        })
        .collect();
    assert_eq!(starts, vec![0]);
}

#[tokio::test]
async fn redacted_thinking_and_unknown_blocks_are_preserved() {
    let server = serve("anthropic_redacted_and_unknown.sse").await;
    let client = AnthropicClient::new(config(&server.uri())).expect("client");
    let turn = client
        .run(&request(), &mut |_event| {})
        .await
        .expect("turn");
    let ProviderPayload::Anthropic { content } = &turn.payload else {
        panic!("expected anthropic payload");
    };
    assert_eq!(content.len(), 3);
    assert_eq!(content[0]["type"], "redacted_thinking");
    assert_eq!(content[0]["data"], "opaque");
    assert_eq!(content[1]["type"], "fallback");
    assert_eq!(content[1]["reason"], "model_unavailable");
    assert_eq!(content[2]["type"], "text");
}

#[tokio::test]
async fn refusal_carries_stop_details() {
    let server = serve("anthropic_refusal.sse").await;
    let client = AnthropicClient::new(config(&server.uri())).expect("client");
    let turn = client
        .run(&request(), &mut |_event| {})
        .await
        .expect("turn");
    match turn.stop_reason {
        StopReason::Refusal {
            category,
            explanation,
        } => {
            assert_eq!(category.as_deref(), Some("cyber"));
            assert_eq!(explanation.as_deref(), Some("unsafe request"));
        }
        other => panic!("unexpected stop reason {other:?}"),
    }
}

#[tokio::test]
async fn overloaded_error_event() {
    let server = serve("anthropic_overloaded.sse").await;
    let client = AnthropicClient::new(config(&server.uri())).expect("client");
    let error = client
        .run(&request(), &mut |_event| {})
        .await
        .expect_err("overloaded");
    assert!(matches!(error, LlmError::Overloaded), "got {error:?}");
    assert!(error.retryable());
}

#[tokio::test]
async fn list_models_maps_anthropic_shape() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                {"id": "claude-opus-5-5", "display_name": "Claude Opus 5.5", "max_input_tokens": 1000000, "max_tokens": 128000}
            ]
        })))
        .mount(&server)
        .await;
    let client = AnthropicClient::new(config(&server.uri())).expect("client");
    let models = client.list_models().await.expect("models");
    assert_eq!(models[0].id, "claude-opus-5-5");
    assert_eq!(models[0].display_name.as_deref(), Some("Claude Opus 5.5"));
    assert_eq!(models[0].context_window, Some(1_000_000));
    assert_eq!(models[0].max_output_tokens, Some(128_000));
}

// ───────────────────────────── body builder ─────────────────────────────

#[test]
fn body_never_contains_sampling_parameters() {
    let options = ModelOptions {
        temperature: Some(0.9),
        ..ModelOptions::default()
    };
    let body = build_body(&request(), &options);
    assert!(body.get("temperature").is_none());
    assert!(body.get("top_p").is_none());
    assert!(body.get("top_k").is_none());
    // Never a forced tool choice.
    assert_eq!(body["tool_choice"]["type"], "auto");
    assert_eq!(body["tool_choice"]["disable_parallel_tool_use"], true);
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert_eq!(body["output_config"]["effort"], "high");
    assert_eq!(body["tools"][0]["eager_input_streaming"], true);
}

#[test]
fn cache_control_uses_one_ttl() {
    let body = build_body(&request(), &ModelOptions::default());
    // System breakpoint.
    assert_eq!(body["system"][0]["cache_control"]["ttl"], "1h");
    // Explicit user-block breakpoint.
    assert_eq!(
        body["messages"][0]["content"][0]["cache_control"]["ttl"],
        "1h"
    );
    // Top-level automatic breakpoint.
    assert_eq!(body["cache_control"]["ttl"], "1h");

    let mut req = request();
    req.cache = CacheHints {
        ttl: CacheTtl::FiveMinutes,
    };
    let body = build_body(&req, &ModelOptions::default());
    assert!(body["system"][0]["cache_control"].get("ttl").is_none());
    assert!(
        body["messages"][0]["content"][0]["cache_control"]
            .get("ttl")
            .is_none()
    );
    assert!(body["cache_control"].get("ttl").is_none());
}

#[test]
fn thinking_display_and_effort_options() {
    let options = ModelOptions {
        effort: Some(Effort::Low),
        thinking_display: Some(ThinkingDisplay::Omitted),
        ..ModelOptions::default()
    };
    let body = build_body(&request(), &options);
    assert_eq!(body["thinking"]["display"], "omitted");
    assert_eq!(body["output_config"]["effort"], "low");
}

#[test]
fn refusal_fallback_adds_fallbacks_field() {
    let options = ModelOptions {
        refusal_fallback: Some(true),
        ..ModelOptions::default()
    };
    let body = build_body(&request(), &options);
    assert_eq!(body["fallbacks"], "default");
    let body = build_body(&request(), &ModelOptions::default());
    assert!(body.get("fallbacks").is_none());
}

#[test]
fn tool_results_merge_with_following_user_text() {
    let payload = ProviderPayload::Anthropic {
        content: vec![serde_json::json!({
            "type": "tool_use", "id": "toolu_1", "name": "read_draft", "input": {}
        })],
    };
    let mut req = request();
    req.messages = vec![
        Message::User {
            blocks: vec![TextBlock {
                text: "kickoff".to_owned(),
                cache: true,
            }],
        },
        Message::Assistant {
            payload: payload.clone(),
        },
        Message::ToolResults {
            results: vec![ToolResult {
                call_id: "toolu_1".to_owned(),
                content: "draft text".to_owned(),
                is_error: false,
            }],
        },
        Message::User {
            blocks: vec![TextBlock {
                text: "author note".to_owned(),
                cache: false,
            }],
        },
    ];
    let body = build_body(&req, &ModelOptions::default());
    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[2]["role"], "user");
    let content = messages[2]["content"].as_array().expect("content");
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["type"], "tool_result");
    assert_eq!(content[0]["tool_use_id"], "toolu_1");
    assert_eq!(content[0]["content"][0]["text"], "draft text");
    assert_eq!(content[1]["type"], "text");
    assert_eq!(content[1]["text"], "author note");
}

#[test]
fn replay_payload_is_byte_identical() {
    let blocks = vec![
        serde_json::json!({"type":"thinking","thinking":"hmm","signature":"sig"}),
        serde_json::json!({"type":"text","text":"hello"}),
        serde_json::json!({"type":"tool_use","id":"t1","name":"x","input":{"a":1}}),
    ];
    let mut req = request();
    req.messages.push(Message::Assistant {
        payload: ProviderPayload::Anthropic {
            content: blocks.clone(),
        },
    });
    let body = build_body(&req, &ModelOptions::default());
    let replayed = &body["messages"][1]["content"];
    assert_eq!(
        serde_json::to_string(&blocks).expect("blocks"),
        serde_json::to_string(replayed).expect("replayed"),
        "the replay payload must be byte-identical"
    );
}
