//! A scripted model service: it answers each request with the next scripted answer, or
//! with a short text when nothing is scripted, and records what it received.

use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{Json, Router, extract::State, http::HeaderMap, response::IntoResponse, routing::post};
use serde_json::{Value, json};

#[derive(Clone, Default)]
pub struct FakeModel {
    script: Arc<Mutex<VecDeque<Answer>>>,
    requests: Arc<Mutex<Vec<Received>>>,
}

enum Answer {
    Call {
        tool: String,
        arguments: Value,
    },
    /// Accepts the request and never answers it.
    Hang,
}

#[derive(Clone, Debug)]
pub struct Received {
    pub authorization: Option<String>,
    pub body: Value,
}

impl FakeModel {
    pub async fn serve(&self) -> SocketAddr {
        let app = Router::new()
            .route("/v1/chat/completions", post(complete))
            .with_state(self.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        address
    }

    /// Scripts the model's next answer: a call of `tool` with `arguments`.
    pub fn call(&self, tool: &str, arguments: Value) {
        self.script.lock().unwrap().push_back(Answer::Call {
            tool: tool.to_owned(),
            arguments,
        });
    }

    /// Scripts the next request to stay unanswered.
    pub fn hang(&self) {
        self.script.lock().unwrap().push_back(Answer::Hang);
    }

    pub fn requests(&self) -> Vec<Received> {
        self.requests.lock().unwrap().clone()
    }
}

async fn complete(
    State(model): State<FakeModel>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let number = {
        let mut requests = model.requests.lock().unwrap();
        requests.push(Received {
            authorization,
            body,
        });
        requests.len()
    };
    let scripted = model.script.lock().unwrap().pop_front();
    let stream = match scripted {
        Some(Answer::Call { tool, arguments }) => {
            tool_call_stream(&format!("call_{number}"), &tool, &arguments.to_string())
        }
        Some(Answer::Hang) => std::future::pending().await,
        None => text_stream("pong"),
    };
    ([("content-type", "text/event-stream")], stream)
}

fn event(chunk: &Value) -> String {
    format!("data: {chunk}\n\n")
}

const USAGE: &str = "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":900,\"completion_tokens\":300}}\n\ndata: [DONE]\n\n";

/// A tool call whose arguments arrive in two pieces, split inside a string.
fn tool_call_stream(id: &str, tool: &str, arguments: &str) -> String {
    let middle = arguments
        .char_indices()
        .nth(arguments.chars().count() / 2)
        .map_or(0, |(index, _)| index);
    let (head, tail) = arguments.split_at(middle);
    [
        event(&json!({ "choices": [{ "delta": { "tool_calls": [{
            "index": 0, "id": id, "type": "function",
            "function": { "name": tool, "arguments": head }
        }] } }] })),
        event(&json!({ "choices": [{
            "delta": { "tool_calls": [{ "index": 0, "function": { "arguments": tail } }] },
            "finish_reason": "tool_calls"
        }] })),
        USAGE.to_owned(),
    ]
    .concat()
}

fn text_stream(text: &str) -> String {
    [
        event(&json!({ "choices": [{ "delta": { "content": text }, "finish_reason": "stop" }] })),
        USAGE.to_owned(),
    ]
    .concat()
}
