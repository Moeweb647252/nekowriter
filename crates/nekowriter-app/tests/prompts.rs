//! Prompt overrides and template rendering over HTTP (`docs/product-spec.md` §13).

mod common;

use common::{
    TestServer, command_id,
    model::FakeModel,
    workspace::{command, configure, first_round, step, wait_for},
};
use serde_json::{Value, json};

async fn prompts(server: &TestServer, project: &str) -> Value {
    server
        .get(&format!("/api/prompts?project={project}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

fn generate_user(view: &Value) -> &Value {
    view["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|prompt| prompt["key"] == "preparation.generate" && prompt["role"] == "user")
        .unwrap()
}

async fn save(
    server: &TestServer,
    path: &str,
    revision: &Value,
    content: Value,
) -> reqwest::Response {
    server
        .post(
            path,
            &json!({
                "commandId": command_id(),
                "expectedRevision": revision,
                "key": "preparation.generate",
                "role": "user",
                "content": content
            }),
        )
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn the_projects_override_wins_and_reaches_the_model() {
    let model = FakeModel::default();
    let server = TestServer::start().await;
    configure(&server, model.serve().await).await;
    let project = server.create_project("Rain").await;
    let project_id = project["id"].as_str().unwrap().to_owned();
    let version = project["selectedVersion"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let project_prompts = format!("/api/projects/{project_id}/prompts");

    let system: Value = server
        .get("/api/prompts")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let view = prompts(&server, &project_id).await;
    assert_eq!(view["prompts"].as_array().unwrap().len(), 20);
    assert!(generate_user(&view)["system"].is_null());
    assert_ne!(generate_user(&view)["builtin"], "");

    let saved = save(
        &server,
        "/api/settings/prompts",
        &system["revision"],
        json!("SYSTEM {{novel_brief}} {{required_artifacts}}"),
    )
    .await;
    assert_eq!(saved.status(), 200);
    // A project override must keep the required variables.
    let refused = save(
        &server,
        &project_prompts,
        &view["revision"],
        json!("PROJECT"),
    )
    .await;
    assert_eq!(refused.status(), 422);
    let body: Value = refused.json().await.unwrap();
    assert_eq!(body["code"], "TEMPLATE_INVALID");
    assert_eq!(body["params"]["error"]["kind"], "missing_required_variable");
    let saved = save(
        &server,
        &project_prompts,
        &view["revision"],
        json!("PROJECT {{novel_brief}} / {{required_artifacts}}"),
    )
    .await;
    assert_eq!(saved.status(), 200);

    command(
        &server,
        &format!("/api/versions/{version}/brief"),
        &version,
        json!({ "brief": "A ghost story in a lighthouse." }),
    )
    .await;
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
    let sent = model.requests()[0].body["messages"].to_string();
    assert!(
        sent.contains("PROJECT A ghost story in a lighthouse. /"),
        "{sent}"
    );
    assert!(!sent.contains("SYSTEM"), "{sent}");

    // Resetting the project's override leaves the system's in force.
    let view = prompts(&server, &project_id).await;
    let reset = save(&server, &project_prompts, &view["revision"], Value::Null).await;
    assert_eq!(reset.status(), 200);
    let view = prompts(&server, &project_id).await;
    assert!(generate_user(&view)["project"].is_null());
    assert_eq!(
        generate_user(&view)["system"],
        "SYSTEM {{novel_brief}} {{required_artifacts}}"
    );
}

#[tokio::test]
async fn templates_render_with_every_value_or_not_at_all() {
    let server = TestServer::start().await;
    let render = |values: Value| {
        server
            .post(
                "/api/templates/render",
                &json!({ "template": "Dear {{name}}, \\{{name}}.", "values": values }),
            )
            .send()
    };

    let rendered: Value = render(json!({ "name": "Rin" }))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(rendered["text"], "Dear Rin, {{name}}.");
    let missing = render(json!({})).await.unwrap();
    assert_eq!(missing.status(), 422);
    let body: Value = missing.json().await.unwrap();
    assert_eq!(
        body["params"]["error"],
        json!({ "kind": "missing_value", "name": "name" })
    );
}
