//! The Chat Completions client (`docs/product-spec.md` §8.1).
//!
//! Requests stream. Text and tool-call deltas are passed to the caller as they arrive (for
//! live previews); tool calls are accumulated per index and handed over only when the
//! response is complete.

use std::{collections::BTreeMap, time::Duration};

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use nekowriter_core::{
    model::{EffectiveModel, ModelService},
    run::{ProviderFailure, TimeoutPhase},
    run_message::{ModelToolCall, TranscriptMessage},
    usage::TokenUsage,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::time::timeout;

use crate::{context::PlannedRequest, model_text::model_text, round::AssistantTurn};

/// The largest answer accepted: its text and tool-call names and arguments. The stream's
/// framing and the reasoning text, which are discarded as they arrive, do not count.
pub const MAX_ANSWER_BYTES: u32 = 2 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Until the response headers arrive. Reasoning models may think for a while first.
const FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(300);
/// Between two streamed events.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// How much of an error response body is kept for the log.
const ERROR_BODY_BYTES: usize = 4_096;

/// A message in the Chat Completions format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum ChatMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ChatToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChatToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: &'static str,
    pub function: ChatFunctionCall,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChatFunctionCall {
    pub name: String,
    pub arguments: String,
}

impl From<&TranscriptMessage> for ChatMessage {
    fn from(message: &TranscriptMessage) -> Self {
        match message {
            TranscriptMessage::Assistant {
                content,
                tool_calls,
            } => Self::Assistant {
                content: content.clone(),
                tool_calls: tool_calls
                    .iter()
                    .map(|call| ChatToolCall {
                        id: call.provider_id.clone(),
                        call_type: "function",
                        function: ChatFunctionCall {
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        },
                    })
                    .collect(),
            },
            TranscriptMessage::Tool {
                provider_call_id,
                content,
            } => Self::Tool {
                tool_call_id: provider_call_id.clone(),
                content: content.clone(),
            },
            TranscriptMessage::User { content } => Self::User {
                content: content.clone(),
            },
        }
    }
}

/// The JSON body of a request, exactly as it is sent. The API key travels only in the
/// `Authorization` header, so the body can be stored as the request snapshot.
#[must_use]
pub fn request_body(model: &EffectiveModel, request: &PlannedRequest) -> Value {
    let mut body = json!({
        "model": model.profile.model_id.as_str(),
        "messages": request.messages,
        "tools": request.tools,
        "parallel_tool_calls": false,
        "stream": true,
    });
    add_common_options(&mut body, model);
    body
}

/// The body of the connection test: a tiny request with the model's own options, so the
/// test proves what real requests will need.
#[must_use]
pub fn connection_test_body(model: &EffectiveModel) -> Value {
    let text = &model_text().connection_test;
    let mut body = json!({
        "model": model.profile.model_id.as_str(),
        "messages": [
            ChatMessage::System { content: text.system.render(&[]) },
            ChatMessage::User { content: text.user.render(&[]) },
        ],
        "stream": true,
    });
    add_common_options(&mut body, model);
    body
}

fn add_common_options(body: &mut Value, model: &EffectiveModel) {
    if model.profile.reports_stream_usage {
        body["stream_options"] = json!({ "include_usage": true });
    }
    if let Some(effort) = model.thinking.reasoning_effort() {
        body["reasoning_effort"] = json!(effort);
    }
}

/// A piece of the answer while it streams.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delta<'a> {
    Text(&'a str),
    ToolCall {
        index: u32,
        name: Option<&'a str>,
        arguments: &'a str,
    },
}

/// A complete answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub turn: AssistantTurn,
    pub finish_reason: Option<String>,
    pub usage: TokenUsage,
    /// The usage object as the service sent it, if it sent one.
    pub raw_usage: Option<Value>,
    pub provider_request_id: Option<String>,
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("the model service could not be reached: {0}")]
    Connect(String),
    #[error("the model service timed out ({0:?})")]
    Timeout(TimeoutPhase),
    #[error("the model service answered HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("the model service sent an invalid response: {0}")]
    InvalidResponse(String),
    #[error("the answer exceeded {max_bytes} bytes")]
    ResponseTooLarge { max_bytes: u32 },
}

impl ProviderError {
    /// The failure as it is stored and shown.
    #[must_use]
    pub fn failure(&self) -> ProviderFailure {
        match self {
            Self::Connect(_) => ProviderFailure::Connect,
            Self::Timeout(phase) => ProviderFailure::Timeout { phase: *phase },
            Self::HttpStatus { status, .. } => ProviderFailure::HttpStatus { status: *status },
            Self::InvalidResponse(_) => ProviderFailure::InvalidResponse,
            Self::ResponseTooLarge { max_bytes } => ProviderFailure::ResponseTooLarge {
                max_bytes: *max_bytes,
            },
        }
    }
}

/// A client for one or more model services. Cloning is cheap.
#[derive(Clone, Debug)]
pub struct ChatClient {
    http: reqwest::Client,
}

impl Default for ChatClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatClient {
    #[must_use]
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .expect("the HTTP client configuration is valid");
        Self { http }
    }

    /// Posts `body` to an endpoint of the service and returns the successful response,
    /// whose body is still to be read.
    pub(crate) async fn post(
        &self,
        service: &ModelService,
        endpoint: &str,
        body: &Value,
    ) -> Result<reqwest::Response, ProviderError> {
        let mut request = self
            .http
            .post(service.base_url.endpoint(endpoint))
            .json(body);
        if let Some(key) = &service.api_key {
            request = request.bearer_auth(key.expose());
        }
        let response = timeout(FIRST_BYTE_TIMEOUT, request.send())
            .await
            .map_err(|_| ProviderError::Timeout(TimeoutPhase::FirstByte))?
            .map_err(|error| {
                if error.is_timeout() {
                    ProviderError::Timeout(TimeoutPhase::Connect)
                } else {
                    ProviderError::Connect(error.to_string())
                }
            })?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let body = body.chars().take(ERROR_BODY_BYTES).collect();
            return Err(ProviderError::HttpStatus {
                status: status.as_u16(),
                body,
            });
        }
        Ok(response)
    }

    /// Sends a streaming chat completion request and collects the answer.
    pub async fn complete(
        &self,
        service: &ModelService,
        body: &Value,
        mut on_delta: impl FnMut(Delta<'_>),
    ) -> Result<Completion, ProviderError> {
        let response = self.post(service, "chat/completions", body).await?;
        let provider_request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        let mut events = response.bytes_stream().eventsource();
        let mut answer = Answer::default();
        loop {
            let Some(event) = timeout(IDLE_TIMEOUT, events.next())
                .await
                .map_err(|_| ProviderError::Timeout(TimeoutPhase::Idle))?
            else {
                break;
            };
            let event = event.map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
            if event.data.trim() == "[DONE]" {
                break;
            }
            let chunk: StreamChunk = serde_json::from_str(&event.data)
                .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
            if let Some(error) = chunk.error {
                return Err(ProviderError::InvalidResponse(error.to_string()));
            }
            answer.add(chunk, &mut on_delta);
            if answer.bytes > MAX_ANSWER_BYTES as usize {
                return Err(ProviderError::ResponseTooLarge {
                    max_bytes: MAX_ANSWER_BYTES,
                });
            }
        }
        Ok(answer.finish(provider_request_id))
    }
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<Value>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: ChunkDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ChunkDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChunkToolCall>,
}

#[derive(Debug, Deserialize)]
struct ChunkToolCall {
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ChunkFunction>,
}

#[derive(Debug, Deserialize)]
struct ChunkFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// An answer being assembled from stream chunks.
#[derive(Default)]
struct Answer {
    content: String,
    tool_calls: BTreeMap<u32, PartialCall>,
    /// The bytes of text, tool-call names and arguments so far.
    bytes: usize,
    finish_reason: Option<String>,
    usage: Option<Value>,
}

#[derive(Default)]
struct PartialCall {
    id: Option<String>,
    name: String,
    arguments: String,
}

impl Answer {
    fn add(&mut self, chunk: StreamChunk, on_delta: &mut impl FnMut(Delta<'_>)) {
        if let Some(usage) = chunk.usage.filter(|usage| !usage.is_null()) {
            self.usage = Some(usage);
        }
        for choice in chunk.choices {
            if let Some(text) = choice.delta.content.filter(|text| !text.is_empty()) {
                on_delta(Delta::Text(&text));
                self.bytes += text.len();
                self.content.push_str(&text);
            }
            for (position, call) in choice.delta.tool_calls.into_iter().enumerate() {
                let index = call
                    .index
                    .unwrap_or_else(|| u32::try_from(position).unwrap_or(u32::MAX));
                let partial = self.tool_calls.entry(index).or_default();
                if let Some(id) = call.id.filter(|id| !id.is_empty()) {
                    partial.id = Some(id);
                }
                let (name, arguments) = call
                    .function
                    .map(|function| (function.name, function.arguments.unwrap_or_default()))
                    .unwrap_or_default();
                if let Some(name) = &name {
                    self.bytes += name.len();
                    partial.name.push_str(name);
                }
                self.bytes += arguments.len();
                partial.arguments.push_str(&arguments);
                on_delta(Delta::ToolCall {
                    index,
                    name: name.as_deref(),
                    arguments: &arguments,
                });
            }
            if choice.finish_reason.is_some() {
                self.finish_reason = choice.finish_reason;
            }
        }
    }

    fn finish(self, provider_request_id: Option<String>) -> Completion {
        let tool_calls = self
            .tool_calls
            .into_iter()
            .map(|(index, call)| ModelToolCall {
                // Some services omit call IDs; the conversation still needs one to pair
                // the call with its result.
                provider_id: call.id.unwrap_or_else(|| format!("call_{index}")),
                name: call.name,
                arguments: call.arguments,
            })
            .collect();
        let content = (!self.content.is_empty()).then_some(self.content);
        Completion {
            turn: AssistantTurn {
                content,
                tool_calls,
            },
            finish_reason: self.finish_reason,
            usage: token_usage(self.usage.as_ref()),
            raw_usage: self.usage,
            provider_request_id,
        }
    }
}

/// Reads the counts of an OpenAI-style usage object; anything missing stays unknown.
fn token_usage(raw: Option<&Value>) -> TokenUsage {
    let Some(raw) = raw else {
        return TokenUsage::default();
    };
    let count = |value: &Value| value.as_i64();
    TokenUsage {
        input_tokens: raw.get("prompt_tokens").and_then(count),
        output_tokens: raw.get("completion_tokens").and_then(count),
        cached_input_tokens: raw
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(count),
        reasoning_tokens: raw
            .pointer("/completion_tokens_details/reasoning_tokens")
            .and_then(count),
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::{Router, http::HeaderMap, response::IntoResponse, routing::post};
    use nekowriter_core::model::{ApiBaseUrl, ApiKey};

    use super::*;

    /// Serves one canned streamed answer and reports the Authorization header it saw.
    async fn serve(
        body: &'static str,
        status: u16,
    ) -> (SocketAddr, tokio::sync::oneshot::Receiver<Option<String>>) {
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
        let seen_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(seen_tx)));
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move |headers: HeaderMap| {
                let seen_tx = seen_tx.clone();
                async move {
                    let authorization = headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    if let Some(sender) = seen_tx.lock().unwrap().take() {
                        let _ = sender.send(authorization);
                    }
                    (
                        axum::http::StatusCode::from_u16(status).unwrap(),
                        [("content-type", "text/event-stream")],
                        body,
                    )
                        .into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (address, seen_rx)
    }

    fn service(address: SocketAddr) -> ModelService {
        ModelService {
            base_url: ApiBaseUrl::parse(&format!("http://{address}/v1")).unwrap(),
            api_key: Some(ApiKey::parse("sk-test").unwrap()),
        }
    }

    const STREAM: &str = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Thinking \"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"function\":{\"name\":\"submit_preparation\",\"arguments\":\"{\\\"artifacts\\\":\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"[]}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":120,\"completion_tokens\":30,\"prompt_tokens_details\":{\"cached_tokens\":100}}}\n\n",
        "data: [DONE]\n\n",
    );

    #[tokio::test]
    async fn a_streamed_answer_is_assembled_with_previews_and_usage() {
        let (address, seen) = serve(STREAM, 200).await;
        let mut previews = Vec::new();

        let completion = ChatClient::new()
            .complete(&service(address), &json!({ "model": "m" }), |delta| {
                previews.push(format!("{delta:?}"));
            })
            .await
            .unwrap();

        assert_eq!(seen.await.unwrap().as_deref(), Some("Bearer sk-test"));
        assert_eq!(completion.turn.content.as_deref(), Some("Thinking "));
        assert_eq!(
            completion.turn.tool_calls,
            vec![ModelToolCall {
                provider_id: "call_a".to_owned(),
                name: "submit_preparation".to_owned(),
                arguments: "{\"artifacts\":[]}".to_owned(),
            }]
        );
        assert_eq!(completion.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(
            completion.usage,
            TokenUsage {
                input_tokens: Some(120),
                output_tokens: Some(30),
                cached_input_tokens: Some(100),
                reasoning_tokens: None,
            }
        );
        assert_eq!(previews.len(), 3);
    }

    /// A stream of `count` events, each with `text` as content and `reasoning` as reasoning
    /// text, which the adapter discards.
    fn repeated_stream(count: usize, text: &str, reasoning: &str) -> &'static str {
        let event = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\",\"reasoning_content\":\"{reasoning}\"}}}}]}}\n\n"
        );
        let mut stream = event.repeat(count);
        stream.push_str("data: [DONE]\n\n");
        stream.leak()
    }

    #[tokio::test]
    async fn framing_and_reasoning_do_not_count_toward_the_answer_limit() {
        // About 3 MiB of stream around 4 KiB of answer.
        let stream = repeated_stream(1024, "word", &"r".repeat(3 * 1024));
        let (address, _seen) = serve(stream, 200).await;

        let completion = ChatClient::new()
            .complete(&service(address), &json!({}), |_| {})
            .await
            .unwrap();

        assert_eq!(
            completion.turn.content.map(|text| text.len()),
            Some(4 * 1024)
        );
    }

    #[tokio::test]
    async fn an_answer_over_the_limit_fails() {
        let piece = "a".repeat(1024);
        let count = MAX_ANSWER_BYTES as usize / piece.len() + 1;
        let (address, _seen) = serve(repeated_stream(count, &piece, ""), 200).await;

        let error = ChatClient::new()
            .complete(&service(address), &json!({}), |_| {})
            .await
            .unwrap_err();

        assert_eq!(
            error.failure(),
            ProviderFailure::ResponseTooLarge {
                max_bytes: MAX_ANSWER_BYTES
            }
        );
    }

    #[tokio::test]
    async fn an_error_status_is_reported_with_its_code() {
        let (address, _seen) = serve("{\"error\":\"no such model\"}", 404).await;

        let error = ChatClient::new()
            .complete(&service(address), &json!({}), |_| {})
            .await
            .unwrap_err();

        assert_eq!(error.failure(), ProviderFailure::HttpStatus { status: 404 });
    }

    #[tokio::test]
    async fn an_unreachable_service_is_a_connection_failure() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);

        let error = ChatClient::new()
            .complete(&service(address), &json!({}), |_| {})
            .await
            .unwrap_err();

        assert_eq!(error.failure(), ProviderFailure::Connect);
    }
}
