//! Driving a version's workspace through the HTTP API.

use std::{net::SocketAddr, time::Duration};

use serde_json::{Value, json};

use super::{TestServer, command_id, model::FakeModel};

pub fn profile() -> Value {
    json!({
        "modelId": "writer",
        "contextWindow": 128_000,
        "maxOutputTokens": 8_000,
        "supportsThinking": false,
        "reportsStreamUsage": true
    })
}

/// Points the system settings at `model`.
pub async fn configure(server: &TestServer, model: SocketAddr) {
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

pub async fn workspace(server: &TestServer, version: &str) -> Value {
    let response = server
        .get(&format!("/api/versions/{version}/workspace"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.json().await.unwrap()
}

/// Waits until the workspace satisfies `done`, which the dispatcher reaches on its own.
pub async fn wait_for(server: &TestServer, version: &str, done: impl Fn(&Value) -> bool) -> Value {
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

pub fn step(view: &Value) -> (&str, &str) {
    let stage = &view["version"]["stage"];
    (
        stage["phase"].as_str().unwrap(),
        // A completed version has no step.
        stage["step"].as_str().unwrap_or(""),
    )
}

/// Posts a workspace command quoting the workspace's current revision.
pub async fn command(server: &TestServer, path: &str, version: &str, mut body: Value) -> Value {
    body["commandId"] = json!(command_id());
    body["expectedRevision"] = workspace(server, version).await["revision"].clone();
    let response = server.post(path, &body).send().await.unwrap();
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{path}: {body}");
    body
}

pub async fn decide(server: &TestServer, version: &str, artifact: &str, decision: Value) {
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

pub fn first_round() -> Value {
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

/// Saves the brief, starts preparation and waits for the first round's reviews.
pub async fn first_round_in_review(server: &TestServer, model: &FakeModel) -> String {
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
    model.call("submit_preparation", first_round());
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

/// A version whose preparation the user approved at once, at its first chapter's guidance.
pub async fn prepared(server: &TestServer, model: &FakeModel) -> String {
    let version = first_round_in_review(server, model).await;
    for artifact in ["background", "research", "characters", "outline"] {
        decide(server, &version, artifact, json!({ "kind": "approve" })).await;
    }
    wait_for(server, &version, |view| step(view) == ("chapter", "ready")).await;
    version
}
