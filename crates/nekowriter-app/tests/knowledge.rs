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

async fn create_entry(server: &TestServer, space: &Value, title: &str, content: &str) -> String {
    let entry: Value = server
        .post(
            "/api/knowledge/entries",
            &json!({
                "commandId": command_id(),
                "spaceId": space["id"],
                "title": title,
                "keywords": [],
                "content": content
            }),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    entry["id"].as_str().unwrap().to_owned()
}

async fn search(server: &TestServer, query: &str) -> reqwest::Response {
    server
        .post(
            "/api/knowledge/search",
            &json!({ "query": query, "spaces": [], "topK": 2 }),
        )
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn search_ranks_chunks_by_meaning_once_indexed() {
    let server = TestServer::start().await;
    let unavailable = search(&server, "lamp").await;
    assert_eq!(unavailable.status(), 409);
    let body: Value = unavailable.json().await.unwrap();
    assert_eq!(body["code"], "SEARCH_UNAVAILABLE");
    assert_eq!(
        body["params"]["reason"],
        json!({ "kind": "not_configured" })
    );

    let model = common::model::FakeModel::default();
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
    let oil = create_entry(
        &server,
        &space,
        "Lamp oil",
        "The lamp burns oil. Lamp oil is heavy.",
    )
    .await;
    let storms = create_entry(&server, &space, "Storms", "A storm rolls in from the sea.").await;
    let calm = create_entry(&server, &space, "Calm", "Nothing happens.").await;
    for entry in [&oil, &storms, &calm] {
        wait_for_index(&server, entry, "ready").await;
    }

    let found: Value = search(&server, "storm at sea").await.json().await.unwrap();
    let hits = found["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0]["title"], "Storms");
    assert_eq!(hits[0]["spaceName"], "Lighthouses");
    assert!(hits[0]["relevance"].as_f64().unwrap() > hits[1]["relevance"].as_f64().unwrap());

    let dashboard = get(&server, "/api/dashboard?boundaries=0,9999999999999").await;
    // Three entries were indexed and one query embedded.
    assert_eq!(dashboard["usage"]["embeddings"]["requests"], 4);
}

#[tokio::test]
async fn the_preparation_writer_queries_the_knowledge_base() {
    use common::workspace::{command, configure, first_round, step, wait_for};

    let model = common::model::FakeModel::default();
    let server = TestServer::start().await;
    configure(&server, model.serve().await).await;
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
    let oil = create_entry(&server, &space, "Lamp oil", "The lamp burns oil.").await;
    wait_for_index(&server, &oil, "ready").await;

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
    model.call(
        "query_knowledge",
        json!({ "query": "lamp oil", "spaces": ["Lighthouses"] }),
    );
    model.call("submit_preparation", first_round());
    command(
        &server,
        &format!("/api/versions/{version}/preparation/start"),
        &version,
        json!({}),
    )
    .await;
    wait_for(&server, &version, |view| {
        step(view) == ("preparation", "reviewing")
    })
    .await;

    let requests = model.requests();
    let first = requests[0].body["tools"].to_string();
    assert!(
        first.contains("query_knowledge") && first.contains("Lighthouses"),
        "{first}"
    );
    let result = requests[1].body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "tool")
        .unwrap()["content"]
        .as_str()
        .unwrap()
        .to_owned();
    let result: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["status"], "found");
    assert_eq!(result["hits"][0]["title"], "Lamp oil");
    assert_eq!(result["hits"][0]["text"], "The lamp burns oil.");
}

#[tokio::test]
async fn no_engines_are_registered_and_the_reason_is_given() {
    let server = TestServer::start().await;
    let engines = get(&server, "/api/engines").await;
    for kind in ["research", "accumulation"] {
        assert_eq!(
            engines[kind],
            json!({ "engines": [], "unavailable": "none_registered" }),
            "{kind}"
        );
    }
}
