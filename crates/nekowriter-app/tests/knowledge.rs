//! The knowledge base over HTTP (`docs/product-spec.md` §12.2).

mod common;

use common::{TestServer, command_id};
use serde_json::{Value, json};

async fn get(server: &TestServer, path: &str) -> Value {
    server.get(path).send().await.unwrap().json().await.unwrap()
}

#[tokio::test]
async fn entries_are_saved_in_spaces_and_browsed_by_keyword() {
    let server = TestServer::start().await;
    let created = server
        .post(
            "/api/knowledge/spaces",
            &json!({ "commandId": command_id(), "name": "Lighthouses" }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let space: Value = created.json().await.unwrap();

    let entry: Value = server
        .post(
            "/api/knowledge/entries",
            &json!({
                "commandId": command_id(),
                "spaceId": space["id"],
                "title": "Lamp oil",
                "keywords": ["fuel", " ", "fuel"],
                "content": "Lamp oil was carried up the stairs."
            }),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(entry["keywords"], json!(["fuel"]));

    let found = get(&server, "/api/knowledge/entries?filter=FUEL").await;
    assert_eq!(found[0]["title"], "Lamp oil");
    assert!(found[0].get("content").is_none());
    let full = get(
        &server,
        &format!("/api/knowledge/entries/{}", entry["id"].as_str().unwrap()),
    )
    .await;
    assert_eq!(full["content"], "Lamp oil was carried up the stairs.");

    let refused = server
        .post(
            &format!(
                "/api/knowledge/spaces/{}/delete",
                space["id"].as_str().unwrap()
            ),
            &json!({ "commandId": command_id(), "expectedRevision": space["revision"] }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 409);
    let body: Value = refused.json().await.unwrap();
    assert_eq!(body["params"], "space_not_empty");
}
