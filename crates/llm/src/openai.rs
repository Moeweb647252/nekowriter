//! OpenAI-compatible Chat Completions adapter (spec/04 §2).

use std::time::Duration;

use futures::StreamExt;
use serde_json::{Map, Value, json};

use crate::retry::LlmError;
use crate::sse;
use crate::types::{
    AssistantTurn, CacheTtl, ChatRequest, Message, ModelOptions, ProviderConfig, ProviderPayload,
    ReasoningReplay, StopReason, StreamEvent, ToolCallOut, Usage,
};

/// Client for an OpenAI-compatible endpoint.
#[derive(Debug, Clone)]
pub struct OpenAiClient {
    pub config: ProviderConfig,
    pub http: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(config: ProviderConfig) -> Result<Self, LlmError> {
        Ok(Self {
            config,
            http: crate::http_client()?,
        })
    }

    fn endpoint(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.config.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    /// Streams one assistant turn, invoking `on_event` for every incremental event.
    pub async fn run(
        &self,
        req: &ChatRequest,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<AssistantTurn, LlmError> {
        let body = build_body(req, &self.config, &req.options);
        let mut builder = self
            .http
            .post(self.endpoint("chat/completions"))
            .header("content-type", "application/json")
            .json(&body);
        if let Some(key) = &self.config.api_key {
            builder = builder.bearer_auth(key);
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
            return Err(LlmError::from_status(status.as_u16(), &body, retry_after));
        }

        let mut stream = std::pin::pin!(sse::event_stream(response));
        let mut state = StreamState::default();
        let options = &req.options;

        while let Some(item) = stream.next().await {
            let event = item?;
            let data = event.data.trim();
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                break;
            }
            let value: Value = serde_json::from_str(data)
                .map_err(|err| LlmError::Protocol(format!("invalid JSON chunk: {err}")))?;
            if let Some(error) = value.get("error") {
                return Err(LlmError::Provider(error.to_string()));
            }
            state.consume_chunk(&value, options, on_event)?;
        }

        Ok(state.finish(request_id))
    }

    /// Lists the provider's models (spec/04 §8).
    pub async fn list_models(&self) -> Result<Vec<crate::types::RemoteModel>, LlmError> {
        let mut builder = self
            .http
            .get(self.endpoint("models"))
            .header("content-type", "application/json");
        if let Some(key) = &self.config.api_key {
            builder = builder.bearer_auth(key);
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
                                .get("name")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                            context_window: item
                                .get("context_length")
                                .or_else(|| item.get("context_window"))
                                .and_then(Value::as_i64),
                            max_output_tokens: item
                                .get("max_output_tokens")
                                .and_then(Value::as_i64),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(models)
    }
}

fn map_reqwest_error(err: reqwest::Error) -> LlmError {
    if err.is_timeout() {
        return LlmError::Timeout("request");
    }
    if err.is_connect() {
        return LlmError::Timeout("connect");
    }
    if err.is_request() {
        return LlmError::Network(err.to_string());
    }
    LlmError::Network(err.to_string())
}

/// Builds the request body (spec/04 §2). Public for the body-builder tests and for
/// byte-identical request comparisons in the runtime tests (spec/04 §7.1).
pub fn build_body(req: &ChatRequest, config: &ProviderConfig, options: &ModelOptions) -> Value {
    let mut messages: Vec<Value> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(json!({ "role": "system", "content": req.system }));
    for message in &req.messages {
        match message {
            Message::User { blocks } => {
                let text = blocks
                    .iter()
                    .map(|block| block.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                messages.push(json!({ "role": "user", "content": text }));
            }
            Message::Assistant { payload } => {
                let ProviderPayload::Openai {
                    content,
                    reasoning_content,
                    tool_calls,
                } = payload
                else {
                    // A payload from another protocol cannot be replayed here; send the text we
                    // have rather than dropping the turn.
                    messages.push(json!({ "role": "assistant", "content": null }));
                    continue;
                };
                let mut object = Map::new();
                object.insert("role".to_owned(), json!("assistant"));
                object.insert("content".to_owned(), json!(content));
                if options.reasoning_replay == ReasoningReplay::Always
                    && let Some(reasoning) = reasoning_content
                    && !reasoning.is_empty()
                {
                    object.insert("reasoning_content".to_owned(), json!(reasoning));
                }
                if !tool_calls.is_empty() {
                    object.insert("tool_calls".to_owned(), json!(tool_calls));
                }
                messages.push(Value::Object(object));
            }
            Message::ToolResults { results } => {
                for result in results {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": result.call_id,
                        "content": result.content,
                    }));
                }
            }
        }
    }

    let mut body = Map::new();
    body.insert("model".to_owned(), json!(req.model));
    body.insert("stream".to_owned(), json!(true));
    if options.stream_usage() {
        body.insert(
            "stream_options".to_owned(),
            json!({ "include_usage": true }),
        );
    }
    body.insert("messages".to_owned(), Value::Array(messages));
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                    }
                })
            })
            .collect();
        body.insert("tools".to_owned(), Value::Array(tools));
        body.insert("tool_choice".to_owned(), json!("auto"));
    }
    if options.send_parallel_tool_calls() {
        body.insert(
            "parallel_tool_calls".to_owned(),
            json!(req.parallel_tool_calls),
        );
    }
    body.insert(
        options.max_tokens_field.field_name().to_owned(),
        json!(req.max_output_tokens),
    );
    if let Some(temperature) = options.temperature {
        body.insert("temperature".to_owned(), json!(temperature));
    }
    if let Some(effort) = &options.reasoning_effort {
        body.insert("reasoning_effort".to_owned(), json!(effort));
    }
    if let Some(extra) = &options.extra_body {
        for (key, value) in extra {
            body.insert(key.clone(), value.clone());
        }
    }
    let _ = config;
    let _ = CacheTtl::FiveMinutes;
    Value::Object(body)
}

/// Accumulates stream chunks into an `AssistantTurn`.
#[derive(Default)]
struct StreamState {
    text: String,
    reasoning: String,
    /// Tool calls in ordinal order, indexed by the provider's `index`.
    tool_calls: Vec<PartialToolCall>,
    stop_reason: Option<StopReason>,
    usage: Usage,
    saw_usage: bool,
}

struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
    /// The `ToolCallStart` event has been emitted for this call.
    announced: bool,
}

impl StreamState {
    fn consume_chunk(
        &mut self,
        value: &Value,
        options: &ModelOptions,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), LlmError> {
        // A usage-only chunk has empty choices.
        if let Some(usage) = self.read_usage(value, options) {
            self.usage.merge(usage);
            self.saw_usage = true;
            on_event(StreamEvent::Usage(usage));
        }

        let Some(choices) = value.get("choices").and_then(Value::as_array) else {
            return Ok(());
        };
        let Some(choice) = choices.first() else {
            return Ok(());
        };
        let Some(delta) = choice.get("delta") else {
            // Some providers put the finish reason on the choice only.
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.stop_reason = Some(map_finish_reason(reason));
            }
            return Ok(());
        };

        if let Some(content) = delta.get("content").and_then(Value::as_str)
            && !content.is_empty()
        {
            self.text.push_str(content);
            on_event(StreamEvent::TextDelta {
                index: 0,
                text: content.to_owned(),
            });
        }
        let reasoning = delta
            .get("reasoning_content")
            .and_then(Value::as_str)
            .or_else(|| delta.get("reasoning").and_then(Value::as_str));
        if let Some(reasoning) = reasoning
            && !reasoning.is_empty()
        {
            self.reasoning.push_str(reasoning);
            on_event(StreamEvent::ReasoningDelta {
                index: 0,
                text: reasoning.to_owned(),
            });
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call
                    .get("index")
                    .and_then(Value::as_u64)
                    .unwrap_or(self.tool_calls.len() as u64) as usize;
                while self.tool_calls.len() <= index {
                    self.tool_calls.push(PartialToolCall {
                        id: String::new(),
                        name: String::new(),
                        arguments: String::new(),
                        announced: false,
                    });
                }
                let slot = &mut self.tool_calls[index];
                let mut started = false;
                if slot.id.is_empty()
                    && let Some(id) = call.get("id").and_then(Value::as_str)
                {
                    slot.id = id.to_owned();
                    started = true;
                }
                if let Some(function) = call.get("function") {
                    if let Some(name) = function.get("name").and_then(Value::as_str) {
                        if slot.name.is_empty() {
                            slot.name = name.to_owned();
                            started = true;
                        } else if slot.name != name {
                            slot.name = name.to_owned();
                        }
                    }
                    if started && !slot.announced {
                        slot.announced = true;
                        let id = if slot.id.is_empty() {
                            format!("call_{index}")
                        } else {
                            slot.id.clone()
                        };
                        on_event(StreamEvent::ToolCallStart {
                            call_index: index as u32,
                            id,
                            name: slot.name.clone(),
                        });
                    }
                    if let Some(arguments) = function.get("arguments").and_then(Value::as_str)
                        && !arguments.is_empty()
                    {
                        slot.arguments.push_str(arguments);
                        on_event(StreamEvent::ToolCallArgsDelta {
                            call_index: index as u32,
                            fragment: arguments.to_owned(),
                        });
                    }
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.stop_reason = Some(map_finish_reason(reason));
        }
        Ok(())
    }

    fn read_usage(&self, value: &Value, _options: &ModelOptions) -> Option<Usage> {
        let usage = value.get("usage")?;
        if usage.is_null() {
            return None;
        }
        let prompt = usage
            .get("prompt_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let cached = usage
            .get("prompt_tokens_details")
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_i64)
            .or_else(|| usage.get("prompt_cache_hit_tokens").and_then(Value::as_i64))
            .unwrap_or(0);
        let completion = usage
            .get("completion_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let reasoning = usage
            .get("completion_tokens_details")
            .and_then(|details| details.get("reasoning_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        Some(Usage {
            input_tokens: (prompt - cached).max(0),
            cached_input_tokens: cached,
            cache_write_tokens: 0,
            output_tokens: completion,
            reasoning_tokens: reasoning,
            estimated: false,
        })
    }

    fn finish(self, request_id: Option<String>) -> AssistantTurn {
        let tool_calls: Vec<ToolCallOut> = self
            .tool_calls
            .iter()
            .enumerate()
            .map(|(index, call)| ToolCallOut {
                id: if call.id.is_empty() {
                    format!("call_{index}")
                } else {
                    call.id.clone()
                },
                name: call.name.clone(),
                input_text: call.arguments.clone(),
            })
            .collect();

        let replay_calls: Vec<Value> = tool_calls
            .iter()
            .enumerate()
            .map(|(index, call)| {
                json!({
                    "index": index,
                    "id": call.id,
                    "type": "function",
                    "function": {
                        "name": call.name,
                        "arguments": call.input_text,
                    }
                })
            })
            .collect();

        let stop_reason = match self.stop_reason {
            Some(reason) => reason,
            None if !tool_calls.is_empty() => StopReason::ToolUse,
            None => StopReason::EndTurn,
        };

        let mut usage = self.usage;
        if !self.saw_usage {
            usage = estimate_usage(&self.text, &self.reasoning, &tool_calls);
        }

        AssistantTurn {
            text: self.text.clone(),
            reasoning: self.reasoning.clone(),
            tool_calls,
            stop_reason,
            usage,
            payload: ProviderPayload::Openai {
                content: if self.text.is_empty() {
                    None
                } else {
                    Some(self.text)
                },
                reasoning_content: if self.reasoning.is_empty() {
                    None
                } else {
                    Some(self.reasoning)
                },
                tool_calls: replay_calls,
            },
            request_id,
        }
    }
}

/// Estimates usage when the provider reported none (spec/03 §10 heuristic, spec/04 §2).
pub fn estimate_usage(text: &str, reasoning: &str, tool_calls: &[ToolCallOut]) -> Usage {
    let mut arguments = 0usize;
    for call in tool_calls {
        arguments += call.input_text.chars().count();
    }
    let output_chars = text.chars().count() + reasoning.chars().count() + arguments;
    Usage {
        input_tokens: 0,
        cached_input_tokens: 0,
        cache_write_tokens: 0,
        output_tokens: (output_chars as i64 + 3) / 4,
        reasoning_tokens: 0,
        estimated: true,
    }
}

fn map_finish_reason(reason: &str) -> StopReason {
    match reason {
        "stop" => StopReason::EndTurn,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "length" => StopReason::MaxTokens,
        "content_filter" => StopReason::Refusal {
            category: None,
            explanation: None,
        },
        other => StopReason::Other(other.to_owned()),
    }
}

/// The idle timeout between stream events (spec/04 §4).
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
