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

#[tokio::test]
async fn the_embedding_test_checks_the_vectors() {
    let model = common::model::FakeModel::default();
    let address = model.serve().await;
    let server = TestServer::start().await;
    let settings = get(&server, "/api/settings/embedding").await;
    let saved = server
        .post(
            "/api/settings/embedding",
            &json!({
                "commandId": command_id(),
                "expectedRevision": settings["revision"],
                "apiBaseUrl": format!("http://{address}/v1"),
                "apiKey": "sk-embed",
                "model": { "modelId": "embedder", "dimensions": 4 }
            }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(saved.status(), 200);
    let saved: Value = saved.json().await.unwrap();
    assert_eq!(saved["apiKeySet"], true);
    assert!(saved.get("apiKey").is_none());

    let test = |model: Value| {
        server
            .post("/api/settings/test-embedding", &json!({ "model": model }))
            .send()
    };
    let passed = test(json!({ "modelId": "embedder", "dimensions": 4 }))
        .await
        .unwrap();
    assert_eq!(passed.status(), 200);
    let passed: Value = passed.json().await.unwrap();
    assert_eq!(passed["dimensions"], 4);

    model.embed_with_size(3);
    let failed = test(json!({ "modelId": "embedder", "dimensions": 4 }))
        .await
        .unwrap();
    assert_eq!(failed.status(), 502);
    let body: Value = failed.json().await.unwrap();
    assert_eq!(body["code"], "EMBEDDING_FAILED");
    assert_eq!(
        body["params"]["failure"],
        json!({ "kind": "wrong_dimensions", "expected": 4, "actual": 3 })
    );
}

/// Saves embedding settings that point at the fake model service.
async fn embed_with(server: &TestServer, model: &common::model::FakeModel) {
    let address = model.serve().await;
    let settings = get(server, "/api/settings/embedding").await;
    let saved = server
        .post(
            "/api/settings/embedding",
            &json!({
                "commandId": command_id(),
                "expectedRevision": settings["revision"],
                "apiBaseUrl": format!("http://{address}/v1"),
                "model": { "modelId": "embedder", "dimensions": 4 }
            }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(saved.status(), 200);
}

/// Polls an entry until its index reaches `state`.
async fn wait_for_index(server: &TestServer, entry: &str, state: &str) -> Value {
    for _ in 0..200 {
        let view = get(server, &format!("/api/knowledge/entries/{entry}")).await;
        if view["index"]["state"] == state {
            return view;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the entry's index never became {state}");
}

#[tokio::test]
async fn the_indexer_embeds_saved_entries() {
    let model = common::model::FakeModel::default();
    let server = TestServer::start().await;
    embed_with(&server, &model).await;
    let space: Value = server
        .post(
            "/api/knowledge/spaces",
            &json!({ "commandId": command_id(), "name": "Lighthouses" }),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let entry: Value = server
        .post(
            "/api/knowledge/entries",
            &json!({
                "commandId": command_id(),
                "spaceId": space["id"],
                "title": "Lamp oil",
                "keywords": [],
                "content": "Lamp oil was carried up the stairs before the storm."
            }),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = entry["id"].as_str().unwrap();

    wait_for_index(&server, id, "ready").await;
    assert_eq!(
        model.embedded(),
        ["Lamp oil was carried up the stairs before the storm."]
    );
}
