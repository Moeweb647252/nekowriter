//! `ScriptedProvider` behavior tests (spec/04 §7.1).

use nekowriter_llm::mock::{ScriptStep, ScriptedProvider};
use nekowriter_llm::retry::LlmError;
use nekowriter_llm::types::{
    CacheHints, CacheTtl, ChatRequest, Message, ModelOptions, StopReason, StreamEvent, TextBlock,
    Usage,
};

fn request() -> ChatRequest {
    ChatRequest {
        model: "m".to_owned(),
        system: "s".to_owned(),
        messages: vec![Message::User {
            blocks: vec![TextBlock {
                text: "go".to_owned(),
                cache: false,
            }],
        }],
        tools: Vec::new(),
        max_output_tokens: 64,
        parallel_tool_calls: false,
        cache: CacheHints {
            ttl: CacheTtl::FiveMinutes,
        },
        options: ModelOptions::default(),
        demo_hints: None,
    }
}

#[tokio::test]
async fn records_requests_and_bodies_in_order() {
    let provider = ScriptedProvider::new(vec![ScriptStep::new().text("one")]);
    provider
        .run(&request(), &mut |_event| {})
        .await
        .expect("turn");
    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model, "m");
    let bodies = provider.bodies();
    assert_eq!(bodies.len(), 1);
    // The recorded body is the OpenAI adapter's serialization, byte-exact for comparisons
    // across steps (spec/04 §7.1).
    assert_eq!(bodies[0]["model"], "m");
    assert_eq!(bodies[0]["messages"][1]["content"], "go");
}

#[tokio::test]
async fn streams_text_reasoning_and_tool_calls() {
    let provider = ScriptedProvider::new(vec![
        ScriptStep::new()
            .text("hello")
            .reasoning("thinking")
            .tool("insert_text", serde_json::json!({ "text": "hi" }))
            .stop(StopReason::ToolUse)
            .usage(Usage {
                input_tokens: 5,
                output_tokens: 6,
                ..Usage::default()
            }),
    ]);
    let mut events = Vec::new();
    let turn = provider
        .run(&request(), &mut |event| events.push(event))
        .await
        .expect("turn");
    assert_eq!(turn.text, "hello");
    assert_eq!(turn.reasoning, "thinking");
    assert_eq!(turn.stop_reason, StopReason::ToolUse);
    assert_eq!(turn.tool_calls.len(), 1);
    assert_eq!(turn.tool_calls[0].name, "insert_text");
    assert_eq!(turn.tool_calls[0].input_text, r#"{"text":"hi"}"#);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::TextDelta { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::ReasoningDelta { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::ToolCallStart { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::ToolCallArgsDelta { .. }))
    );
}

#[tokio::test]
async fn injects_failures_before_steps() {
    let provider = ScriptedProvider::new(vec![ScriptStep::new().text("after")])
        .failing(LlmError::Overloaded, 2);
    assert!(matches!(
        provider.run(&request(), &mut |_e| {}).await,
        Err(LlmError::Overloaded)
    ));
    assert!(matches!(
        provider.run(&request(), &mut |_e| {}).await,
        Err(LlmError::Overloaded)
    ));
    let turn = provider.run(&request(), &mut |_e| {}).await.expect("turn");
    assert_eq!(turn.text, "after");
}

#[tokio::test]
async fn can_fail_once_the_queue_is_exhausted() {
    let provider =
        ScriptedProvider::new(vec![]).fail_when_exhausted(LlmError::Auth("no key".into()));
    assert!(matches!(
        provider.run(&request(), &mut |_e| {}).await,
        Err(LlmError::Auth(_))
    ));
}

#[tokio::test]
async fn lists_its_model() {
    let provider = ScriptedProvider::new(vec![]);
    let models = provider.list_models().await.expect("models");
    assert_eq!(models[0].id, "scripted-model");
}
