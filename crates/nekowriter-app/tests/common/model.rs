//! A scripted model service: it answers each request with the next scripted answer, or
//! with a short text when nothing is scripted, and records what it received. It also
//! embeds texts deterministically: each dimension counts one of [`TERMS`].

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
    embedded: Arc<Mutex<Vec<String>>>,
    /// Vectors of this size instead of the requested one, to test the checks.
    embedding_size: Arc<Mutex<Option<usize>>>,
}

/// The terms the fake embeddings count, one per dimension.
pub const TERMS: [&str; 4] = ["lamp", "storm", "sea", "oil"];

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
            .route("/v1/embeddings", post(embed))
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

    /// Every text embedded so far, in order.
    pub fn embedded(&self) -> Vec<String> {
        self.embedded.lock().unwrap().clone()
    }

    /// Makes later embeddings come back with `size` dimensions.
    pub fn embed_with_size(&self, size: usize) {
        *self.embedding_size.lock().unwrap() = Some(size);
    }
}

/// How often each term occurs in `text`, plus a small constant so no vector is zero.
fn vector(text: &str, size: usize) -> Vec<f64> {
    let text = text.to_lowercase();
    (0..size)
        .map(|index| {
            TERMS.get(index).map_or(0.0, |term| {
                f64::from(u32::try_from(text.matches(term).count()).unwrap())
            }) + 0.01
        })
        .collect()
}

async fn embed(State(model): State<FakeModel>, Json(body): Json<Value>) -> Json<Value> {
    let texts: Vec<String> = body["input"]
        .as_array()
        .unwrap()
        .iter()
        .map(|text| text.as_str().unwrap().to_owned())
        .collect();
    model.embedded.lock().unwrap().extend(texts.iter().cloned());
    let size = model
        .embedding_size
        .lock()
        .unwrap()
        .unwrap_or_else(|| usize::try_from(body["dimensions"].as_u64().unwrap()).unwrap());
    let data: Vec<Value> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| json!({ "index": index, "embedding": vector(text, size) }))
        .collect();
    let tokens: usize = texts.iter().map(|text| text.chars().count()).sum();
    Json(json!({ "data": data, "usage": { "prompt_tokens": tokens } }))
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
