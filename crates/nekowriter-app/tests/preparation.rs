//! Preparation end to end over a real port, against a scripted model service
//! (`docs/product-spec.md` §6, §8, §13.3).

mod common;

use axum::{Router, routing::post};
use common::{
    TestServer, command_id,
    model::FakeModel,
    workspace::{
        command, configure, decide, first_round, first_round_in_review, profile, step, wait_for,
        workspace,
    },
};
use serde_json::{Value, json};

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

#[tokio::test]
async fn preparation_runs_from_the_brief_through_a_revision_to_the_first_chapter() {
    let model = FakeModel::default();
    let server = TestServer::start().await;
    configure(&server, model.serve().await).await;
    let version = first_round_in_review(&server, &model).await;

    for artifact in ["background", "research", "characters"] {
        decide(&server, &version, artifact, json!({ "kind": "approve" })).await;
    }
    model.call("submit_preparation", revised_outline());
    decide(
        &server,
        &version,
        "outline",
        json!({ "kind": "reject", "feedback": "Give the keeper a name.", "citations": [] }),
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
    let tools: Vec<&str> = first["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(tools, ["ask_user", "submit_preparation"]);
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
                "decision": { "kind": "reject", "feedback": "  ", "citations": [] }
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

#[tokio::test]
async fn stopping_a_run_abandons_its_request_and_continuing_starts_a_new_one() {
    let model = FakeModel::default();
    let server = TestServer::start().await;
    configure(&server, model.serve().await).await;
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
    model.hang();
    command(
        &server,
        &format!("/api/versions/{version}/preparation/start"),
        &version,
        json!({}),
    )
    .await;
    let view = wait_for(&server, &version, |view| view["run"]["status"] == "running").await;
    let run = view["run"]["id"].as_str().unwrap().to_owned();

    let response = server
        .post(
            &format!("/api/runs/{run}/stop"),
            &json!({ "commandId": command_id() }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let view = workspace(&server, &version).await;
    assert_eq!(view["run"]["status"], "stopped");
    assert_eq!(view["run"]["halt"], json!({ "kind": "stopped_by_user" }));

    model.call("submit_preparation", first_round());
    command(
        &server,
        &format!("/api/runs/{run}/continue"),
        &version,
        json!({}),
    )
    .await;
    wait_for(&server, &version, |view| {
        step(view) == ("preparation", "reviewing")
    })
    .await;
    assert_eq!(model.requests().len(), 2);
}

#[tokio::test]
async fn a_delegated_review_is_decided_by_the_review_agent() {
    let model = FakeModel::default();
    let server = TestServer::start().await;
    configure(&server, model.serve().await).await;
    let version = first_round_in_review(&server, &model).await;
    let view = workspace(&server, &version).await;
    let review = view["artifacts"][0]["review"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    model.call(
        "submit_review",
        json!({ "passed": true, "confidence": 0.75, "issues": [] }),
    );
    command(
        &server,
        &format!("/api/reviews/{review}/delegate"),
        &version,
        json!({ "notes": "Check the coast.", "citations": [] }),
    )
    .await;

    let view = wait_for(&server, &version, |view| {
        view["artifacts"][0]["review"]["status"] == "approved"
    })
    .await;
    assert_eq!(view["artifacts"][0]["review"]["decidedBy"], "delegate");
    assert_eq!(view["artifacts"][0]["review"]["confidencePermille"], 750);
    let requests = model.requests();
    let request = &requests.last().unwrap().body;
    let tools: Vec<&str> = request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(tools, ["submit_review"]);
    let user = request["messages"][1]["content"].as_str().unwrap();
    assert!(user.contains("A lighthouse on a cold coast.") && user.contains("Check the coast."));
}
