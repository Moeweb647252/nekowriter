//! Shared SSE decoding helpers for the streaming adapters.
//!
//! Both protocols use `text/event-stream` framing, but Anthropic names events with an `event:`
//! line while OpenAI-compatible providers only send `data:` lines. This module keeps the
//! byte-level accumulation in one place so the adapters only interpret payloads.

use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Response;

use crate::retry::LlmError;

/// One decoded server-sent event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` name when the provider sends one.
    pub event: Option<String>,
    pub data: String,
}

/// Turns an HTTP response into a stream of SSE events.
///
/// Dropping the returned stream cancels the underlying request (spec/04 §1).
pub fn event_stream(response: Response) -> impl futures::Stream<Item = Result<SseEvent, LlmError>> {
    response
        .bytes_stream()
        .eventsource()
        .map(|item| match item {
            Ok(event) => Ok(SseEvent {
                event: if event.event.is_empty() || event.event == "message" {
                    None
                } else {
                    Some(event.event)
                },
                data: event.data,
            }),
            Err(err) => Err(LlmError::Network(err.to_string())),
        })
}

/// Reads the error text of a non-success response body (best effort).
pub async fn error_body(response: Response) -> String {
    match response.text().await {
        Ok(text) if text.len() > 4096 => text.chars().take(4096).collect(),
        Ok(text) => text,
        Err(err) => format!("<unreadable body: {err}>"),
    }
}

/// Extracts the retry hint from a response's headers.
pub fn retry_after(response: &Response) -> Option<std::time::Duration> {
    let value = response.headers().get(reqwest::header::RETRY_AFTER)?;
    crate::retry::parse_retry_after(value.to_str().ok()?, chrono::Utc::now())
}

/// The provider's request id, from `request-id` or `x-request-id`.
pub fn request_id(response: &Response) -> Option<String> {
    for name in ["request-id", "x-request-id"] {
        if let Some(value) = response.headers().get(name)
            && let Ok(text) = value.to_str()
        {
            return Some(text.to_owned());
        }
    }
    None
}
