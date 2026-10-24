//! OpenAI-compatible adapter tests (spec/04 §9).

use std::time::Duration;

use nekowriter_llm::openai::{OpenAiClient, build_body};
use nekowriter_llm::retry::LlmError;
use nekowriter_llm::types::{
    AssistantTurn, CacheHints, CacheTtl, ChatRequest, Message, ModelOptions, ProviderConfig,
    ProviderPayload, ReasoningReplay, StopReason, StreamEvent, TextBlock, ToolDef, ToolResult,
    Usage,
};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("cannot read {path}: {err}"))
}

fn config(base_url: &str) -> ProviderConfig {
    ProviderConfig {
        protocol: nekowriter_llm::types::Protocol::Openai,
        base_url: base_url.to_owned(),
        api_key: Some("sk-test".to_owned()),
        extra_headers: Default::default(),
    }
}

fn request() -> ChatRequest {
    ChatRequest {
        model: "test-model".to_owned(),
        system: "You are a writer.".to_owned(),
        messages: vec![Message::User {
            blocks: vec![TextBlock {
                text: "Write.".to_owned(),
                cache: false,
            }],
        }],
        tools: vec![ToolDef {
            name: "insert_text".to_owned(),
            description: "Write text.".to_owned(),
            input_schema: serde_json::json!({"type": "object"}),
        }],
        max_output_tokens: 1024,
        parallel_tool_calls: false,
        cache: CacheHints {
            ttl: CacheTtl::FiveMinutes,
        },
        options: ModelOptions::default(),
        demo_hints: None,
    }
}

async fn serve(name: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .insert_header("request-id", "req-123")
                .set_body_string(fixture(name)),
        )
        .mount(&server)
        .await;
    server
}

/// Collects events while running a request against a fixture.
async fn run_fixture(
    name: &str,
    options: ModelOptions,
) -> (AssistantTurn, Vec<StreamEvent>, MockServer) {
    let server = serve(name).await;
    let client = OpenAiClient::new(config(&server.uri())).expect("client");
    let mut req = request();
    req.options = options;
    let mut events = Vec::new();
    let turn = client
        .run(&req, &mut |event| events.push(event))
        .await
        .expect("turn");
    (turn, events, server)
}

#[tokio::test]
async fn plain_text_events_and_turn() {
    let (turn, events, _server) = run_fixture("openai_text.sse", ModelOptions::default()).await;

    let text_deltas: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::TextDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text_deltas, vec!["Hello", ", world"]);

    assert_eq!(turn.text, "Hello, world");
    assert_eq!(turn.stop_reason, StopReason::EndTurn);
    assert!(turn.tool_calls.is_empty());
    assert_eq!(turn.request_id.as_deref(), Some("req-123"));
    // Usage: cached 40 of 100 prompt tokens.
    assert_eq!(turn.usage.input_tokens, 60);
    assert_eq!(turn.usage.cached_input_tokens, 40);
    assert_eq!(turn.usage.output_tokens, 7);
    assert!(!turn.usage.estimated);
    match &turn.payload {
        ProviderPayload::Openai { content, .. } => {
            assert_eq!(content.as_deref(), Some("Hello, world"));
        }
        other => panic!("unexpected payload {other:?}"),
    }
}

#[tokio::test]
async fn tool_call_split_across_fragments() {
    let (turn, events, _server) =
        run_fixture("openai_tool_split.sse", ModelOptions::default()).await;

    let starts: Vec<&StreamEvent> = events
        .iter()
        .filter(|event| matches!(event, StreamEvent::ToolCallStart { .. }))
        .collect();
    assert_eq!(starts.len(), 1, "the start event is emitted once");
    match starts[0] {
        StreamEvent::ToolCallStart {
            call_index,
            id,
            name,
        } => {
            assert_eq!(*call_index, 0);
            assert_eq!(id, "call_abc");
            assert_eq!(name, "insert_text");
        }
        _ => unreachable!(),
    }
    let fragments: String = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ToolCallArgsDelta { fragment, .. } => Some(fragment.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(fragments, r#"{"text":"Hello"}"#);

    assert_eq!(turn.tool_calls.len(), 1);
    assert_eq!(turn.tool_calls[0].id, "call_abc");
    assert_eq!(turn.tool_calls[0].name, "insert_text");
    assert_eq!(turn.tool_calls[0].input_text, r#"{"text":"Hello"}"#);
    assert_eq!(turn.stop_reason, StopReason::ToolUse);
    // No usage chunk: estimated.
    assert!(turn.usage.estimated);
}

#[tokio::test]
async fn two_parallel_tool_calls() {
    let (turn, _events, _server) =
        run_fixture("openai_parallel_tools.sse", ModelOptions::default()).await;
    assert_eq!(turn.tool_calls.len(), 2);
    assert_eq!(turn.tool_calls[0].name, "get_entity");
    assert_eq!(turn.tool_calls[0].input_text, r#"{"name":"Shen Yan"}"#);
    assert_eq!(turn.tool_calls[1].name, "query_timeline");
    assert_eq!(turn.tool_calls[1].input_text, "{}");
}

#[tokio::test]
async fn deepseek_reasoning_and_cache_hit_usage() {
    let (turn, events, _server) =
        run_fixture("openai_deepseek_reasoning.sse", ModelOptions::default()).await;

    let reasoning: String = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ReasoningDelta { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(reasoning, "Let me think. The author wants a plan.");
    assert_eq!(turn.reasoning, reasoning);
    // prompt_cache_hit_tokens=300 of prompt_tokens=500.
    assert_eq!(turn.usage.cached_input_tokens, 300);
    assert_eq!(turn.usage.input_tokens, 200);
    assert_eq!(turn.usage.reasoning_tokens, 20);
    assert_eq!(turn.usage.output_tokens, 60);

    // Replay carries reasoning_content when the option is `always`.
    let options = ModelOptions {
        reasoning_replay: ReasoningReplay::Always,
        ..ModelOptions::default()
    };
    let body = build_body(
        &request_with_payload(&turn.payload),
        &config("http://x"),
        &options,
    );
    let assistant = &body["messages"][2];
    assert_eq!(
        assistant["reasoning_content"],
        "Let me think. The author wants a plan."
    );

    // ... and omits it by default.
    let body = build_body(
        &request_with_payload(&turn.payload),
        &config("http://x"),
        &ModelOptions::default(),
    );
    assert!(body["messages"][2].get("reasoning_content").is_none());
}

fn request_with_payload(payload: &ProviderPayload) -> ChatRequest {
    let mut req = request();
    req.messages.push(Message::Assistant {
        payload: payload.clone(),
    });
    req.messages.push(Message::ToolResults {
        results: vec![ToolResult {
            call_id: "call_1".to_owned(),
            content: "ok".to_owned(),
            is_error: false,
        }],
    });
    req
}

#[tokio::test]
async fn in_stream_error_object_is_a_provider_error() {
    let server = serve("openai_stream_error.sse").await;
    let client = OpenAiClient::new(config(&server.uri())).expect("client");
    let error = client
        .run(&request(), &mut |_event| {})
        .await
        .expect_err("error");
    assert!(matches!(error, LlmError::Provider(_)), "got {error:?}");
}

#[tokio::test]
async fn length_finish_reason_is_max_tokens() {
    let (turn, _events, _server) = run_fixture("openai_length.sse", ModelOptions::default()).await;
    assert_eq!(turn.stop_reason, StopReason::MaxTokens);
}

#[tokio::test]
async fn http_error_classification() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "7")
                .set_body_string("slow down"),
        )
        .mount(&server)
        .await;
    let client = OpenAiClient::new(config(&server.uri())).expect("client");
    let error = client
        .run(&request(), &mut |_event| {})
        .await
        .expect_err("rate limited");
    match error {
        LlmError::RateLimited { retry_after } => {
            assert_eq!(retry_after, Some(Duration::from_secs(7)));
        }
        other => panic!("unexpected error {other:?}"),
    }
}

#[tokio::test]
async fn list_models_maps_openai_shape() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                {"id": "gpt-x", "name": "GPT X", "context_length": 128000},
                {"id": "gpt-y"}
            ]
        })))
        .mount(&server)
        .await;
    let client = OpenAiClient::new(config(&server.uri())).expect("client");
    let models = client.list_models().await.expect("models");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "gpt-x");
    assert_eq!(models[0].display_name.as_deref(), Some("GPT X"));
    assert_eq!(models[0].context_window, Some(128_000));
    assert_eq!(models[1].context_window, None);
}

// ───────────────────────────── body builder ─────────────────────────────

#[test]
fn body_has_no_temperature_unless_configured() {
    let body = build_body(&request(), &config("http://x"), &ModelOptions::default());
    assert!(body.get("temperature").is_none());
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["parallel_tool_calls"], false);
    assert_eq!(body["max_tokens"], 1024);
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["content"], "Write.");
}

#[test]
fn body_respects_max_tokens_field_and_parallel_flag() {
    let mut options = ModelOptions {
        max_tokens_field: nekowriter_llm::types::MaxTokensField::MaxCompletionTokens,
        send_parallel_tool_calls: Some(false),
        stream_usage: Some(false),
        temperature: Some(0.7),
        ..ModelOptions::default()
    };
    options.send_parallel_tool_calls = Some(false);
    let mut req = request();
    req.parallel_tool_calls = true;
    let body = build_body(&req, &config("http://x"), &options);
    assert_eq!(body["max_completion_tokens"], 1024);
    assert!(body.get("max_tokens").is_none());
    // Sending is disabled, so the flag is absent even though the request asks for parallel.
    assert!(body.get("parallel_tool_calls").is_none());
    assert!(body.get("stream_options").is_none());
    assert_eq!(body["temperature"], 0.7);

    let options = ModelOptions {
        max_tokens_field: nekowriter_llm::types::MaxTokensField::MaxTokens,
        ..ModelOptions::default()
    };
    let body = build_body(&req, &config("http://x"), &options);
    assert_eq!(body["parallel_tool_calls"], true);
}

#[test]
fn body_merges_extra_body_last() {
    let mut extra = std::collections::BTreeMap::new();
    extra.insert("top_k".to_owned(), serde_json::json!(10));
    extra.insert("max_tokens".to_owned(), serde_json::json!(99));
    let options = ModelOptions {
        extra_body: Some(extra),
        ..ModelOptions::default()
    };
    let body = build_body(&request(), &config("http://x"), &options);
    assert_eq!(body["top_k"], 10);
    assert_eq!(body["max_tokens"], 99, "extra_body wins");
}

#[test]
fn tool_results_become_tool_messages() {
    let payload = ProviderPayload::Openai {
        content: None,
        reasoning_content: None,
        tool_calls: vec![serde_json::json!({
            "index": 0, "id": "call_1", "type": "function",
            "function": {"name": "read_draft", "arguments": "{}"}
        })],
    };
    let req = request_with_payload(&payload);
    let body = build_body(&req, &config("http://x"), &ModelOptions::default());
    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[2]["role"], "assistant");
    assert_eq!(messages[2]["tool_calls"][0]["id"], "call_1");
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["tool_call_id"], "call_1");
    assert_eq!(messages[3]["content"], "ok");
}

#[test]
fn user_blocks_join_with_blank_lines() {
    let mut req = request();
    req.messages = vec![Message::User {
        blocks: vec![
            TextBlock {
                text: "first".to_owned(),
                cache: true,
            },
            TextBlock {
                text: "second".to_owned(),
                cache: false,
            },
        ],
    }];
    let body = build_body(&req, &config("http://x"), &ModelOptions::default());
    assert_eq!(body["messages"][1]["content"], "first\n\nsecond");
}

#[test]
fn no_tools_means_no_tools_field() {
    let mut req = request();
    req.tools.clear();
    let body = build_body(&req, &config("http://x"), &ModelOptions::default());
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
}

#[test]
fn usage_helper_merges() {
    let mut usage = Usage::default();
    usage.merge(Usage {
        input_tokens: 10,
        ..Usage::default()
    });
    usage.merge(Usage {
        output_tokens: 5,
        ..Usage::default()
    });
    assert_eq!(usage.input_tokens, 10);
    assert_eq!(usage.output_tokens, 5);
}
