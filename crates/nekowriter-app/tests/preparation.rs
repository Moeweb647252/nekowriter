//! Preparation end to end over a real port, against a scripted model service
//! (`docs/product-spec.md` §6, §8, §13.3).

mod common;

use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{Json, Router, extract::State, http::HeaderMap, response::IntoResponse, routing::post};
use common::{TestServer, command_id};
use serde_json::{Value, json};

/// A model service that answers each request with the next scripted submission (or a short
/// text when nothing is scripted) and records what it received.
#[derive(Clone, Default)]
struct FakeModel {
    script: Arc<Mutex<VecDeque<Value>>>,
    requests: Arc<Mutex<Vec<Received>>>,
}

#[derive(Clone, Debug)]
struct Received {
    authorization: Option<String>,
    body: Value,
}

impl FakeModel {
    async fn serve(&self) -> SocketAddr {
        let app = Router::new()
            .route("/v1/chat/completions", post(complete))
            .with_state(self.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        address
    }

    /// Scripts the arguments of the next `submit_preparation` call.
    fn submit(&self, arguments: Value) {
        self.script.lock().unwrap().push_back(arguments);
    }

    fn requests(&self) -> Vec<Received> {
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
        Some(arguments) => tool_call_stream(&format!("call_{number}"), &arguments.to_string()),
        None => text_stream("pong"),
    };
    ([("content-type", "text/event-stream")], stream)
}

fn event(chunk: &Value) -> String {
    format!("data: {chunk}\n\n")
}

const USAGE: &str = "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":900,\"completion_tokens\":300}}\n\ndata: [DONE]\n\n";

/// A tool call whose arguments arrive in two pieces, split inside a string.
fn tool_call_stream(id: &str, arguments: &str) -> String {
    let middle = arguments
        .char_indices()
        .nth(arguments.chars().count() / 2)
        .map_or(0, |(index, _)| index);
    let (head, tail) = arguments.split_at(middle);
    [
        event(&json!({ "choices": [{ "delta": { "tool_calls": [{
            "index": 0, "id": id, "type": "function",
            "function": { "name": "submit_preparation", "arguments": head }
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

fn profile() -> Value {
    json!({
        "modelId": "writer",
        "contextWindow": 128_000,
        "maxOutputTokens": 8_000,
        "supportsThinking": false,
        "reportsStreamUsage": true
    })
}

/// Points the system settings at `model`.
async fn configure(server: &TestServer, model: SocketAddr) {
    let response = server
        .post(
            "/api/settings",
            &json!({
                "commandId": command_id(),
                "expectedRevision": 0,
                "apiBaseUrl": format!("http://{model}/v1"),
                "apiKey": "sk-test",
                "defaultModel": profile()
            }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let view: Value = response.json().await.unwrap();
    assert_eq!(view["apiKeySet"], true);
    assert!(!view.to_string().contains("sk-test"));
}

async fn workspace(server: &TestServer, version: &str) -> Value {
    let response = server
        .get(&format!("/api/versions/{version}/workspace"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.json().await.unwrap()
}

/// Waits until the workspace satisfies `done`, which the dispatcher reaches on its own.
async fn wait_for(server: &TestServer, version: &str, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let view = workspace(server, version).await;
        if done(&view) {
            return view;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the workspace did not get there in time: {view}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn step(view: &Value) -> (&str, &str) {
    let stage = &view["version"]["stage"];
    (
        stage["phase"].as_str().unwrap(),
        stage["step"].as_str().unwrap(),
    )
}

/// Posts a workspace command quoting the workspace's current revision.
async fn command(server: &TestServer, path: &str, version: &str, mut body: Value) -> Value {
    body["commandId"] = json!(command_id());
    body["expectedRevision"] = workspace(server, version).await["revision"].clone();
    let response = server.post(path, &body).send().await.unwrap();
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{path}: {body}");
    body
}

async fn decide(server: &TestServer, version: &str, artifact: &str, decision: Value) {
    let view = workspace(server, version).await;
    let review = view["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == artifact)
        .unwrap()["review"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    command(
        server,
        &format!("/api/reviews/{review}/decide"),
        version,
        json!({ "decision": decision }),
    )
    .await;
}

fn first_round() -> Value {
    json!({
        "artifacts": [
            { "kind": "background", "content": "A lighthouse on a cold coast." },
            { "kind": "research", "content": "Gap: no sources on lamp oil were provided." },
            { "kind": "characters", "content": "The keeper, alone for years." },
            { "kind": "outline", "content": "A letter arrives; a storm comes." }
        ],
        "chapter_plan": [
            { "title": "Arrival", "goal": "The keeper finds the letter." },
            { "title": "Storm", "goal": "The lamp fails." }
        ]
    })
}

fn revised_outline() -> Value {
    json!({
        "artifacts": [{ "kind": "outline", "content": "Mara finds a letter; a storm comes." }],
        "chapter_plan": [
            { "title": "Arrival", "goal": "Mara finds the letter." },
            { "title": "Storm", "goal": "The lamp fails." },
            { "title": "Dawn", "goal": "The ship is saved." }
        ]
    })
}

/// Saves the brief, starts preparation and waits for the first round's reviews.
async fn first_round_in_review(server: &TestServer, model: &FakeModel) -> String {
    let project = server.create_project("Rain").await;
    let version = project["selectedVersion"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    command(
        server,
        &format!("/api/versions/{version}/brief"),
        &version,
        json!({ "brief": "A ghost story in a lighthouse." }),
    )
    .await;
    model.submit(first_round());
    command(
        server,
        &format!("/api/versions/{version}/preparation/start"),
        &version,
        json!({}),
    )
    .await;
    wait_for(server, &version, |view| {
        step(view) == ("preparation", "reviewing")
    })
    .await;
    version
}

#[tokio::test]
async fn preparation_runs_from_the_brief_through_a_revision_to_the_first_chapter() {
    let model = FakeModel::default();
    let server = TestServer::start().await;
    configure(&server, model.serve().await).await;
    let version = first_round_in_review(&server, &model).await;

    for artifact in ["background", "research", "characters"] {
        decide(&server, &version, artifact, json!({ "kind": "approve" })).await;
    }
    model.submit(revised_outline());
    decide(
        &server,
        &version,
        "outline",
        json!({ "kind": "reject", "feedback": "Give the keeper a name." }),
    )
    .await;
    wait_for(&server, &version, |view| {
        step(view) == ("preparation", "reviewing")
            && view["artifacts"][3]["revision"]["number"] == 2
    })
    .await;
    decide(&server, &version, "outline", json!({ "kind": "approve" })).await;

    let view = wait_for(&server, &version, |view| step(view).0 == "chapter").await;
    assert_eq!(step(&view), ("chapter", "ready"));
    assert_eq!(view["run"], Value::Null);
    let titles: Vec<&str> = view["chapters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|chapter| chapter["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Arrival", "Storm", "Dawn"]);

    let requests = model.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].authorization.as_deref(), Some("Bearer sk-test"));
    let first = &requests[0].body;
    assert_eq!(first["model"], "writer");
    assert_eq!(first["parallel_tool_calls"], false);
    assert_eq!(first["tools"][0]["function"]["name"], "submit_preparation");
    assert!(
        first["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("A ghost story in a lighthouse.")
    );
    // The revision request continues the conversation: the submission, its accepted
    // result, then the review outcome with the reviewer's feedback.
    let messages = requests[1].body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 5);
    assert_eq!(messages[2]["tool_calls"][0]["id"], "call_1");
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["tool_call_id"], "call_1");
    assert_eq!(messages[4]["role"], "user");
    assert!(
        messages[4]["content"]
            .as_str()
            .unwrap()
            .contains("Give the keeper a name.")
    );
}

#[tokio::test]
async fn a_rejection_needs_feedback() {
    let model = FakeModel::default();
    let server = TestServer::start().await;
    configure(&server, model.serve().await).await;
    let version = first_round_in_review(&server, &model).await;
    let view = workspace(&server, &version).await;

    let response = server
        .post(
            &format!(
                "/api/reviews/{}/decide",
                view["artifacts"][0]["review"]["id"].as_str().unwrap()
            ),
            &json!({
                "commandId": command_id(),
                "expectedRevision": view["revision"],
                "decision": { "kind": "reject", "feedback": "  " }
            }),
        )
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 422);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "FIELD_INVALID");
    assert_eq!(body["params"]["field"], "review_feedback");
}

#[tokio::test]
async fn preparation_does_not_start_without_a_model() {
    let server = TestServer::start().await;
    let project = server.create_project("Rain").await;
    let version = project["selectedVersion"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    command(
        &server,
        &format!("/api/versions/{version}/brief"),
        &version,
        json!({ "brief": "A ghost story in a lighthouse." }),
    )
    .await;
    let view = workspace(&server, &version).await;

    let response = server
        .post(
            &format!("/api/versions/{version}/preparation/start"),
            &json!({ "commandId": command_id(), "expectedRevision": view["revision"] }),
        )
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 409);
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body["params"],
        json!({ "problem": { "kind": "service_not_configured" } })
    );
    assert_eq!(body["code"], "MODEL_NOT_READY");
}

#[tokio::test]
async fn the_connection_test_sends_a_real_request_and_reports_failures_by_kind() {
    let model = FakeModel::default();
    let address = model.serve().await;
    let server = TestServer::start().await;

    let response = server
        .post(
            "/api/settings/test-model",
            &json!({
                "apiBaseUrl": format!("http://{address}/v1"),
                "apiKey": "sk-test",
                "model": profile()
            }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let view: Value = response.json().await.unwrap();
    assert_eq!(view["usageReported"], true);
    assert_eq!(model.requests()[0].body["model"], "writer");

    let response = server
        .post(
            "/api/settings/test-model",
            &json!({
                "apiBaseUrl": format!("http://{address}/missing"),
                "model": profile()
            }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 502);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "MODEL_SERVICE_FAILED");
    assert_eq!(
        body["params"]["failure"],
        json!({ "kind": "http_status", "status": 404 })
    );
}

#[tokio::test]
async fn runs_in_flight_at_shutdown_are_interrupted_and_wait_for_the_user() {
    // A service that accepts the request and never answers keeps the run in flight.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let silent = Router::new().route(
        "/v1/chat/completions",
        post(std::future::pending::<&'static str>),
    );
    tokio::spawn(async move { axum::serve(listener, silent).await.unwrap() });
    let data_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start_in(data_dir.path()).await;
    configure(&server, address).await;
    let project = server.create_project("Rain").await;
    let version = project["selectedVersion"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    command(
        &server,
        &format!("/api/versions/{version}/brief"),
        &version,
        json!({ "brief": "A ghost story in a lighthouse." }),
    )
    .await;
    command(
        &server,
        &format!("/api/versions/{version}/preparation/start"),
        &version,
        json!({}),
    )
    .await;
    wait_for(&server, &version, |view| view["run"]["status"] == "running").await;

    server.stop().await;
    let server = TestServer::start_in(data_dir.path()).await;

    let view = workspace(&server, &version).await;
    assert_eq!(view["run"]["status"], "interrupted");
    assert_eq!(view["run"]["halt"], json!({ "kind": "process_stopped" }));
    assert_eq!(view["run"]["requestInFlight"], Value::Null);
}
