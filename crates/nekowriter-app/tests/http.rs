//! The HTTP API over a real port.

mod common;

use common::{TestServer, command_id};
use serde_json::{Value, json};

#[tokio::test]
async fn health_reports_the_running_service() {
    let server = TestServer::start().await;

    let health: Value = server
        .get("/api/health")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(health["appVersion"], env!("CARGO_PKG_VERSION"));
    assert!(
        health["dataDir"]
            .as_str()
            .is_some_and(|dir| !dir.is_empty())
    );
}

#[tokio::test]
async fn projects_can_be_created_renamed_deleted_and_restored() {
    let server = TestServer::start().await;
    let project = server.create_project("Rain").await;
    let id = project["id"].as_str().unwrap();
    assert_eq!(project["writingLanguage"], "en");
    assert_eq!(project["selectedVersion"]["novelTitle"], "Rain Letters");

    let renamed: Value = server
        .post(
            &format!("/api/projects/{id}/rename"),
            &json!({ "commandId": command_id(), "expectedRevision": 1, "name": "Snow" }),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(renamed["name"], "Snow");

    let deleted = server
        .post(
            &format!("/api/projects/{id}/delete"),
            &json!({ "commandId": command_id(), "expectedRevision": 2 }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 200);
    let active: Vec<Value> = server
        .get("/api/projects")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(active, Vec::<Value>::new());
    let trash: Vec<Value> = server
        .get("/api/projects?filter=deleted")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(trash.len(), 1);

    let restored = server
        .post(
            &format!("/api/projects/{id}/restore"),
            &json!({ "commandId": command_id(), "expectedRevision": 3 }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(restored.status(), 200);
    let active: Vec<Value> = server
        .get("/api/projects")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
}

#[tokio::test]
async fn a_retried_command_gets_the_original_response() {
    let server = TestServer::start().await;
    let request = json!({
        "commandId": command_id(),
        "name": "Rain",
        "novelTitle": "Rain Letters",
        "writingLanguage": "zh-CN"
    });

    let first: Value = server
        .post("/api/projects", &request)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let retry: Value = server
        .post("/api/projects", &request)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(first, retry);
    let active: Vec<Value> = server
        .get("/api/projects")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
}

#[tokio::test]
async fn errors_are_codes_with_typed_parameters() {
    let server = TestServer::start().await;

    let invalid = server
        .post(
            "/api/projects",
            &json!({
                "commandId": command_id(),
                "name": "   ",
                "novelTitle": "Rain Letters",
                "writingLanguage": "en"
            }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), 422);
    let body: Value = invalid.json().await.unwrap();
    assert_eq!(body["code"], "FIELD_INVALID");
    assert_eq!(
        body["params"],
        json!({ "field": "project_name", "problem": { "kind": "empty" } })
    );
    assert!(body["requestId"].as_str().is_some());

    let project = server.create_project("Rain").await;
    let stale = server
        .post(
            &format!("/api/projects/{}/rename", project["id"].as_str().unwrap()),
            &json!({ "commandId": command_id(), "expectedRevision": 7, "name": "Snow" }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 409);
    let body: Value = stale.json().await.unwrap();
    assert_eq!(body["code"], "REVISION_CONFLICT");
    assert_eq!(body["params"], json!({ "resource": "project" }));

    let unknown = server.get("/api/nothing-here").send().await.unwrap();
    assert_eq!(unknown.status(), 404);
    let body: Value = unknown.json().await.unwrap();
    assert_eq!(body["code"], "ROUTE_NOT_FOUND");

    let malformed = server
        .client
        .post(format!("{}/api/projects", server.base))
        .header("content-type", "application/json")
        .body("{")
        .send()
        .await
        .unwrap();
    assert_eq!(malformed.status(), 400);
    let body: Value = malformed.json().await.unwrap();
    assert_eq!(body["code"], "MALFORMED_REQUEST");
}

#[tokio::test]
async fn requests_from_other_sites_are_refused() {
    let server = TestServer::start().await;

    let foreign_origin = server
        .post(
            "/api/projects",
            &json!({
                "commandId": command_id(),
                "name": "Rain",
                "novelTitle": "Rain Letters",
                "writingLanguage": "en"
            }),
        )
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(foreign_origin.status(), 403);
    let body: Value = foreign_origin.json().await.unwrap();
    assert_eq!(body["code"], "UNTRUSTED_ORIGIN");

    let rebound_host = server
        .get("/api/projects")
        .header("host", "evil.example:47821")
        .send()
        .await
        .unwrap();
    assert_eq!(rebound_host.status(), 403);
    let body: Value = rebound_host.json().await.unwrap();
    assert_eq!(body["code"], "UNTRUSTED_HOST");

    let own_origin = server
        .post(
            "/api/projects",
            &json!({
                "commandId": command_id(),
                "name": "Rain",
                "novelTitle": "Rain Letters",
                "writingLanguage": "en"
            }),
        )
        .header("origin", server.base.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(own_origin.status(), 201);
}

#[tokio::test]
async fn the_shutdown_request_stops_the_server() {
    let server = TestServer::start().await;

    let response = server
        .post("/api/system/shutdown", &json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["status"], "shutting_down");

    tokio::time::timeout(std::time::Duration::from_secs(10), server.server.wait())
        .await
        .expect("the server stops promptly")
        .expect("the server stops cleanly");
}
