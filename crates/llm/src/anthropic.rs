//! Anthropic Messages adapter (spec/04 §3).

use serde_json::{Map, Value, json};

use crate::retry::LlmError;
use crate::sse;
use crate::types::{
    AssistantTurn, ChatRequest, Effort, Message, ModelOptions, ProviderConfig, ProviderPayload,
    StopReason, StreamEvent, ThinkingDisplay, ToolCallOut, Usage,
};

/// The Anthropic API version this client targets (spec/04 §3; DECISIONS D-005).
pub const ANTHROPIC_VERSION: &str = "2026-10-21";

/// The beta header for server-side refusal fallback (spec/04 §3).
pub const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

#[derive(Debug, Clone)]
pub struct AnthropicClient {
    pub config: ProviderConfig,
    pub http: reqwest::Client,
    /// Idle gap between stream events before the turn is abandoned (spec/04 §4).
    pub idle_timeout: std::time::Duration,
}

impl AnthropicClient {
    pub fn new(config: ProviderConfig) -> Result<Self, LlmError> {
        Ok(Self {
            config,
            http: crate::http_client()?,
            idle_timeout: crate::openai::IDLE_TIMEOUT,
        })
    }

    /// Overrides the idle timeout; tests use a short one to exercise the error path.
    pub fn with_idle_timeout(mut self, idle_timeout: std::time::Duration) -> Self {
        self.idle_timeout = idle_timeout;
        self
    }

    fn endpoint(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.config.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    pub async fn run(
        &self,
        req: &ChatRequest,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<AssistantTurn, LlmError> {
        let body = build_body(req, &req.options);
        let mut builder = self
            .http
            .post(self.endpoint("v1/messages"))
            .header("content-type", "application/json")
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body);
        if let Some(key) = &self.config.api_key {
            builder = builder.header("x-api-key", key);
        }
        if req.options.refusal_fallback.unwrap_or(false) {
            builder = builder.header("anthropic-beta", FALLBACK_BETA);
        }
        for (name, value) in &self.config.extra_headers {
            builder = builder.header(name, value);
        }

        let response = builder.send().await.map_err(map_reqwest_error)?;
        let status = response.status();
        let request_id = sse::request_id(&response);
        if !status.is_success() {
            let retry_after = sse::retry_after(&response);
            let body = sse::error_body(response).await;
            return Err(LlmError::from_status(status.as_u16(), &body, retry_after)
                .with_request_id(request_id.as_deref()));
        }

        let mut stream = std::pin::pin!(sse::event_stream(response));
        let mut state = StreamState::default();
        loop {
            // Anthropic sends `ping` events while a turn is quiet; beyond the idle gap the
            // turn is abandoned as a timeout (spec/04 §4).
            let next = match tokio::time::timeout(self.idle_timeout, stream.next_event()).await {
                Ok(next) => next?,
                Err(_) => return Err(LlmError::Timeout("stream idle")),
            };
            let Some(item) = next else {
                break;
            };
            // An in-stream error object still belongs to this HTTP request, so it carries the
            // response's request id like every other failure of the turn (spec/04 §4).
            state
                .consume_event(&item, &req.options, on_event)
                .map_err(|err| err.with_request_id(request_id.as_deref()))?;
            if state.done {
                break;
            }
        }
        Ok(state.finish(req, request_id))
    }

    pub async fn list_models(&self) -> Result<Vec<crate::types::RemoteModel>, LlmError> {
        let mut builder = self
            .http
            .get(self.endpoint("v1/models"))
            .header("anthropic-version", ANTHROPIC_VERSION);
        if let Some(key) = &self.config.api_key {
            builder = builder.header("x-api-key", key);
        }
        for (name, value) in &self.config.extra_headers {
            builder = builder.header(name, value);
        }
        let response = builder.send().await.map_err(map_reqwest_error)?;
        let status = response.status();
        if !status.is_success() {
            let retry_after = sse::retry_after(&response);
            let body = sse::error_body(response).await;
            return Err(LlmError::from_status(status.as_u16(), &body, retry_after));
        }
        let value: Value = response
            .json()
            .await
            .map_err(|err| LlmError::Protocol(format!("invalid model list: {err}")))?;
        let models = value
            .get("data")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let id = item.get("id")?.as_str()?.to_owned();
                        Some(crate::types::RemoteModel {
                            id,
                            display_name: item
                                .get("display_name")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                            context_window: item.get("max_input_tokens").and_then(Value::as_i64),
                            max_output_tokens: item.get("max_tokens").and_then(Value::as_i64),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(models)
    }
}

/// Small extension so `StreamState` can pull events with `?` while keeping the state machine
/// readable.
trait NextEvent {
    async fn next_event(&mut self) -> Result<Option<sse::SseEvent>, LlmError>;
}

impl<S> NextEvent for std::pin::Pin<&mut S>
where
    S: futures::Stream<Item = Result<sse::SseEvent, LlmError>> + Unpin,
{
    async fn next_event(&mut self) -> Result<Option<sse::SseEvent>, LlmError> {
        use futures::StreamExt;
        self.next().await.transpose()
    }
}

fn map_reqwest_error(err: reqwest::Error) -> LlmError {
    if err.is_timeout() {
        return LlmError::Timeout("request");
    }
    LlmError::Network(err.to_string())
}

/// Builds the Messages API body (spec/04 §3).
///
/// Rules that matter: never `temperature`/`top_p`/`top_k`, never a forced tool choice, one TTL
/// for every cache breakpoint, and exact replay of the stored content blocks.
pub fn build_body(req: &ChatRequest, options: &ModelOptions) -> Value {
    let ttl = req.cache.ttl;
    let mut messages: Vec<Value> = Vec::with_capacity(req.messages.len() + 1);

    let mut index = 0usize;
    while index < req.messages.len() {
        match &req.messages[index] {
            Message::User { blocks } => {
                let content: Vec<Value> = blocks
                    .iter()
                    .map(|block| {
                        let mut object = Map::new();
                        object.insert("type".to_owned(), json!("text"));
                        object.insert("text".to_owned(), json!(block.text));
                        if block.cache {
                            object.insert("cache_control".to_owned(), cache_control(ttl));
                        }
                        Value::Object(object)
                    })
                    .collect();
                messages.push(json!({ "role": "user", "content": content }));
                index += 1;
            }
            Message::Assistant { payload } => {
                let content = match payload {
                    ProviderPayload::Anthropic { content } => content.clone(),
                    _ => Vec::new(),
                };
                messages.push(json!({ "role": "assistant", "content": content }));
                index += 1;

                // Tool results come from the next neutral message.
                if let Some(Message::ToolResults { results }) = req.messages.get(index) {
                    let mut content: Vec<Value> = results
                        .iter()
                        .map(|result| {
                            json!({
                                "type": "tool_result",
                                "tool_use_id": result.call_id,
                                "content": [ { "type": "text", "text": result.content } ],
                                "is_error": result.is_error,
                            })
                        })
                        .collect();
                    index += 1;
                    // Adjacent author text joins the same user message so roles alternate
                    // (spec/04 §3).
                    while let Some(Message::User { blocks }) = req.messages.get(index) {
                        for block in blocks {
                            let mut object = Map::new();
                            object.insert("type".to_owned(), json!("text"));
                            object.insert("text".to_owned(), json!(block.text));
                            if block.cache {
                                object.insert("cache_control".to_owned(), cache_control(ttl));
                            }
                            content.push(Value::Object(object));
                        }
                        index += 1;
                    }
                    messages.push(json!({ "role": "user", "content": content }));
                }
            }
            Message::ToolResults { results } => {
                // A tool-result message without a preceding assistant message cannot happen in
                // a valid history; keep the API happy by emitting a user message.
                let content: Vec<Value> = results
                    .iter()
                    .map(|result| {
                        json!({
                            "type": "tool_result",
                            "tool_use_id": result.call_id,
                            "content": [ { "type": "text", "text": result.content } ],
                            "is_error": result.is_error,
                        })
                    })
                    .collect();
                messages.push(json!({ "role": "user", "content": content }));
                index += 1;
            }
        }
    }

    let mut body = Map::new();
    body.insert("model".to_owned(), json!(req.model));
    body.insert("max_tokens".to_owned(), json!(req.max_output_tokens));
    body.insert("stream".to_owned(), json!(true));
    // The Messages API rejects empty text blocks, and the connection test has no system prompt;
    // an absent system prompt is sent as an absent field (spec/04 §3, §8).
    if !req.system.trim().is_empty() {
        body.insert(
            "system".to_owned(),
            json!([{
                "type": "text",
                "text": req.system,
                "cache_control": cache_control(ttl),
            }]),
        );
    }
    if !req.tools.is_empty() {
        let eager = options.eager_input_streaming();
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|tool| {
                let mut object = Map::new();
                object.insert("name".to_owned(), json!(tool.name));
                object.insert("description".to_owned(), json!(tool.description));
                object.insert("input_schema".to_owned(), tool.input_schema.clone());
                object.insert("eager_input_streaming".to_owned(), json!(eager));
                Value::Object(object)
            })
            .collect();
        body.insert("tools".to_owned(), Value::Array(tools));
        body.insert(
            "tool_choice".to_owned(),
            json!({
                "type": "auto",
                "disable_parallel_tool_use": !req.parallel_tool_calls,
            }),
        );
    }
    body.insert(
        "thinking".to_owned(),
        json!({
            "type": "adaptive",
            "display": options
                .thinking_display
                .unwrap_or(ThinkingDisplay::Summarized)
                .as_str(),
        }),
    );
    body.insert(
        "output_config".to_owned(),
        json!({
            "effort": options.effort.unwrap_or(Effort::High).as_str(),
        }),
    );
    body.insert("cache_control".to_owned(), cache_control(ttl));
    body.insert("messages".to_owned(), Value::Array(messages));
    if options.refusal_fallback.unwrap_or(false) {
        body.insert("fallbacks".to_owned(), json!("default"));
    }
    Value::Object(body)
}

/// `cache_control` with the one TTL the request uses (spec/04 §3). The API rejects an explicit
/// marker whose TTL differs from the top-level one, so every breakpoint shares it; the 5m
/// default is expressed by omitting `ttl`.
fn cache_control(ttl: crate::types::CacheTtl) -> Value {
    match ttl {
        crate::types::CacheTtl::FiveMinutes => json!({ "type": "ephemeral" }),
        crate::types::CacheTtl::OneHour => json!({ "type": "ephemeral", "ttl": "1h" }),
    }
}

#[derive(Default)]
struct StreamState {
    text: String,
    reasoning: String,
    /// Raw content blocks in arrival order, replayed exactly.
    blocks: Vec<Value>,
    /// `block index` → tool-call ordinal.
    block_to_call: std::collections::HashMap<u64, usize>,
    calls: Vec<PartialCall>,
    stop_reason: Option<StopReason>,
    usage: Usage,
    saw_usage: bool,
    done: bool,
}

struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

impl StreamState {
    fn consume_event(
        &mut self,
        event: &sse::SseEvent,
        _options: &ModelOptions,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), LlmError> {
        let kind = event.event.as_deref().unwrap_or("message");
        let data: Value = if event.data.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&event.data)
                .map_err(|err| LlmError::Protocol(format!("invalid event JSON: {err}")))?
        };
        match kind {
            "message_start" => {
                if let Some(usage) = data.get("message").and_then(|message| message.get("usage")) {
                    let parsed = self.read_usage(usage, None);
                    self.usage.merge(parsed);
                    self.saw_usage = true;
                    on_event(StreamEvent::Usage(parsed));
                }
            }
            "content_block_start" => {
                let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
                let block = data.get("content_block").cloned().unwrap_or(Value::Null);
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        let text = block
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        if !text.is_empty() {
                            self.text.push_str(&text);
                            on_event(StreamEvent::TextDelta {
                                index: index as u32,
                                text,
                            });
                        }
                    }
                    Some("thinking") => {
                        // Thinking text arrives through `thinking_delta`; keep the block for
                        // replay with its signature filled in later.
                    }
                    Some("redacted_thinking") => {
                        // Complete in the start event.
                    }
                    Some("tool_use") => {
                        let call_index = self.calls.len();
                        let id = block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        let name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        self.block_to_call.insert(index, call_index);
                        self.calls.push(PartialCall {
                            id: id.clone(),
                            name: name.clone(),
                            arguments: String::new(),
                        });
                        on_event(StreamEvent::ToolCallStart {
                            call_index: call_index as u32,
                            id,
                            name,
                        });
                    }
                    _ => {}
                }
                self.blocks.push(block);
            }
            "content_block_delta" => {
                let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
                let delta = data.get("delta").cloned().unwrap_or(Value::Null);
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        let text = delta
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        self.text.push_str(&text);
                        if let Some(slot) = self.blocks.get_mut(index as usize) {
                            append_text(slot, &text);
                        }
                        on_event(StreamEvent::TextDelta {
                            index: index as u32,
                            text,
                        });
                    }
                    Some("thinking_delta") => {
                        let thinking = delta
                            .get("thinking")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        self.reasoning.push_str(&thinking);
                        if let Some(slot) = self.blocks.get_mut(index as usize) {
                            append_field(slot, "thinking", &thinking);
                        }
                        on_event(StreamEvent::ReasoningDelta {
                            index: index as u32,
                            text: thinking,
                        });
                    }
                    Some("signature_delta") => {
                        let signature = delta
                            .get("signature")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        if let Some(slot) = self.blocks.get_mut(index as usize) {
                            append_field(slot, "signature", &signature);
                        }
                    }
                    Some("input_json_delta") => {
                        let fragment = delta
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        if let Some(ordinal) = self.block_to_call.get(&index).copied() {
                            if let Some(call) = self.calls.get_mut(ordinal) {
                                call.arguments.push_str(&fragment);
                            }
                            on_event(StreamEvent::ToolCallArgsDelta {
                                call_index: ordinal as u32,
                                fragment,
                            });
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
                if let Some(ordinal) = self.block_to_call.get(&index).copied() {
                    let raw = self
                        .calls
                        .get(ordinal)
                        .map(|call| call.arguments.clone())
                        .unwrap_or_default();
                    let parsed: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
                    if let Some(slot) = self.blocks.get_mut(index as usize) {
                        set_json_field(slot, "input", parsed);
                    }
                }
            }
            "message_delta" => {
                if let Some(reason) = data
                    .get("delta")
                    .and_then(|delta| delta.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    let details = data
                        .get("delta")
                        .and_then(|delta| delta.get("stop_details"));
                    self.stop_reason = Some(map_stop_reason(reason, details));
                }
                if let Some(usage) = data.get("usage") {
                    let parsed = self.read_usage(usage, Some(&self.usage));
                    self.usage.merge(parsed);
                    self.saw_usage = true;
                    on_event(StreamEvent::Usage(self.usage));
                }
            }
            "message_stop" => {
                self.done = true;
            }
            "ping" => {}
            "error" => {
                let error = data.get("error").cloned().unwrap_or(Value::Null);
                let kind = error
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("api_error");
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("provider error")
                    .to_owned();
                return Err(match kind {
                    "overloaded_error" => LlmError::Overloaded,
                    "rate_limit_error" => LlmError::RateLimited { retry_after: None },
                    "authentication_error" | "permission_error" => LlmError::Auth(message),
                    "invalid_request_error" => {
                        if crate::retry::is_context_overflow(&message) {
                            LlmError::ContextOverflow(message)
                        } else {
                            LlmError::BadRequest(message)
                        }
                    }
                    "not_found_error" => LlmError::NotFound(message),
                    _ => LlmError::Provider(format!("{kind}: {message}")),
                });
            }
            _ => {}
        }
        Ok(())
    }

    /// Anthropic splits usage across two events: `message_start` carries the input side and
    /// `message_delta` the output side. Every field is read independently so a provider that
    /// reports both sides at once still merges correctly.
    fn read_usage(&self, usage: &Value, base: Option<&Usage>) -> Usage {
        let mut result = base.copied().unwrap_or_default();
        if let Some(value) = usage.get("input_tokens").and_then(Value::as_i64) {
            result.input_tokens = value;
        }
        if let Some(value) = usage.get("output_tokens").and_then(Value::as_i64) {
            result.output_tokens = value;
        }
        if let Some(value) = usage.get("cache_read_input_tokens").and_then(Value::as_i64) {
            result.cached_input_tokens = value;
        }
        if let Some(value) = usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_i64)
        {
            result.cache_write_tokens = value;
        }
        result.estimated = false;
        result
    }

    fn finish(self, request: &ChatRequest, request_id: Option<String>) -> AssistantTurn {
        let tool_calls: Vec<ToolCallOut> = self
            .calls
            .iter()
            .map(|call| ToolCallOut {
                id: call.id.clone(),
                name: call.name.clone(),
                input_text: call.arguments.clone(),
            })
            .collect();
        let mut usage = self.usage;
        if !self.saw_usage {
            usage =
                crate::openai::estimate_usage(request, &self.text, &self.reasoning, &tool_calls);
        }
        AssistantTurn {
            text: self.text,
            reasoning: self.reasoning,
            tool_calls,
            stop_reason: self.stop_reason.unwrap_or(StopReason::EndTurn),
            usage,
            payload: ProviderPayload::Anthropic {
                content: self.blocks,
            },
            request_id,
        }
    }
}

fn append_text(block: &mut Value, text: &str) {
    append_field(block, "text", text);
}

fn append_field(block: &mut Value, field: &str, value: &str) {
    if let Some(object) = block.as_object_mut() {
        let current = object
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        object.insert(field.to_owned(), json!(format!("{current}{value}")));
    }
}

fn set_json_field(block: &mut Value, field: &str, value: Value) {
    if let Some(object) = block.as_object_mut() {
        object.insert(field.to_owned(), value);
    }
}

fn map_stop_reason(reason: &str, details: Option<&Value>) -> StopReason {
    match reason {
        "end_turn" | "stop_sequence" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        "refusal" => StopReason::Refusal {
            category: details
                .and_then(|d| d.get("category"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            explanation: details
                .and_then(|d| d.get("explanation"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        "pause_turn" => StopReason::PauseTurn,
        other => StopReason::Other(other.to_owned()),
    }
}
