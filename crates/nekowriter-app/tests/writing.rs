//! Writing chapters end to end over a real port, against a scripted model service
//! (`docs/product-spec.md` §5.3, §7, §8.5, §11).

mod common;

use common::{
    TestServer,
    model::FakeModel,
    workspace::{command, configure, prepared, step, wait_for, workspace},
};
use serde_json::{Value, json};

/// The ID of the chapter the version is at.
fn current_chapter(view: &Value) -> String {
    view["version"]["stage"]["chapterId"]
        .as_str()
        .expect("the version is at a chapter")
        .to_owned()
}

fn chapter<'a>(view: &'a Value, id: &str) -> &'a Value {
    view["chapters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|chapter| chapter["id"] == id)
        .expect("the chapter is listed")
}

/// Decides the current chapter's latest review.
async fn decide_chapter(server: &TestServer, version: &str, decision: Value) {
    let view = workspace(server, version).await;
    let review = chapter(&view, &current_chapter(&view))["review"]["id"]
        .as_str()
        .expect("the chapter has a review")
        .to_owned();
    command(
        server,
        &format!("/api/reviews/{review}/decide"),
        version,
        json!({ "decision": decision }),
    )
    .await;
}

/// Writes the current chapter: its outline, two edits and its submission, each approved.
async fn write_chapter(server: &TestServer, model: &FakeModel, version: &str, number: u32) {
    let chapter_id = current_chapter(&workspace(server, version).await);
    model.call(
        "submit_chapter_outline",
        json!({ "chapter_id": chapter_id, "content": format!("Outline of chapter {number}.") }),
    );
    command(
        server,
        &format!("/api/versions/{version}/chapters/start"),
        version,
        json!({ "guidance": "Keep it quiet." }),
    )
    .await;
    wait_for(server, version, |view| {
        step(view) == ("chapter", "outline_reviewing")
    })
    .await;

    model.call(
        "append_content",
        json!({ "content": format!("Night {number}. The keeper woke.") }),
    );
    model.call(
        "replace_content",
        json!({ "old_str": "woke", "new_str": "rose" }),
    );
    model.call(
        "submit_chapter",
        json!({ "chapter_id": chapter_id, "summary": "The keeper rises." }),
    );
    decide_chapter(server, version, json!({ "kind": "approve" })).await;
    wait_for(server, version, |view| {
        step(view) == ("chapter", "reviewing")
    })
    .await;
    decide_chapter(server, version, json!({ "kind": "approve" })).await;
    wait_for(server, version, |view| step(view) == ("chapter", "done")).await;
}

#[tokio::test]
async fn the_model_writes_a_two_chapter_novel_that_the_user_completes() {
    let model = FakeModel::default();
    let server = TestServer::start().await;
    configure(&server, model.serve().await).await;
    let version = prepared(&server, &model).await;

    write_chapter(&server, &model, &version, 1).await;
    command(
        &server,
        &format!("/api/versions/{version}/chapters/next"),
        &version,
        json!({}),
    )
    .await;
    write_chapter(&server, &model, &version, 2).await;
    command(
        &server,
        &format!("/api/versions/{version}/complete"),
        &version,
        json!({}),
    )
    .await;

    let view = workspace(&server, &version).await;
    assert_eq!(view["version"]["stage"], json!({ "phase": "completed" }));
    assert!(view["completedAt"].is_number());
    let chapters = view["chapters"].as_array().unwrap();
    assert_eq!(chapters.len(), 2);
    for chapter in chapters {
        assert_eq!(chapter["status"], "approved");
        assert_eq!(chapter["body"]["wordCount"], 5);
    }
    let body = chapters[1]["body"]["id"].as_str().unwrap();
    let revision: Value = server
        .get(&format!("/api/revisions/{body}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(revision["content"], "Night 2. The keeper rose.");

    // The second chapter's body requests carry every completed stage before them, once, as
    // one assistant message after the system prompt.
    let requests = model.requests();
    let last = &requests.last().unwrap().body;
    let folds = last["messages"][1]["content"].as_str().unwrap();
    assert_eq!(last["messages"][1]["role"], "assistant");
    for text in [
        "A lighthouse on a cold coast.",
        "Outline of chapter 1.",
        "Night 1. The keeper rose.",
        "Outline of chapter 2.",
    ] {
        assert_eq!(folds.matches(text).count(), 1, "{text} in {folds}");
    }
    let tools: Vec<&str> = last["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        tools,
        ["append_content", "replace_content", "submit_chapter"]
    );
}

#[tokio::test]
async fn a_completed_novel_takes_extra_chapters() {
    let model = FakeModel::default();
    let server = TestServer::start().await;
    configure(&server, model.serve().await).await;
    let version = prepared(&server, &model).await;
    for number in 1..=2 {
        write_chapter(&server, &model, &version, number).await;
        let next = if number == 1 {
            "chapters/next"
        } else {
            "complete"
        };
        command(
            &server,
            &format!("/api/versions/{version}/{next}"),
            &version,
            json!({}),
        )
        .await;
    }

    command(
        &server,
        &format!("/api/versions/{version}/extra-chapters"),
        &version,
        json!({ "title": "Coda" }),
    )
    .await;
    let view = workspace(&server, &version).await;
    let extra = chapter(&view, &current_chapter(&view));
    assert_eq!(
        (extra["kind"].as_str(), extra["number"].as_i64()),
        (Some("extra"), Some(1))
    );
    write_chapter(&server, &model, &version, 3).await;
    command(
        &server,
        &format!("/api/versions/{version}/complete"),
        &version,
        json!({}),
    )
    .await;
    assert_eq!(
        workspace(&server, &version).await["version"]["stage"],
        json!({ "phase": "completed" })
    );
}

#[tokio::test]
async fn the_writer_takes_feedback_and_a_pause_while_it_writes() {
    let model = FakeModel::default();
    let server = TestServer::start().await;
    configure(&server, model.serve().await).await;
    let version = prepared(&server, &model).await;
    let chapter_id = current_chapter(&workspace(&server, &version).await);
    model.call(
        "submit_chapter_outline",
        json!({ "chapter_id": chapter_id, "content": "Outline." }),
    );
    command(
        &server,
        &format!("/api/versions/{version}/chapters/start"),
        &version,
        json!({ "guidance": "" }),
    )
    .await;
    wait_for(&server, &version, |view| {
        step(view) == ("chapter", "outline_reviewing")
    })
    .await;
    model.hang();
    decide_chapter(&server, &version, json!({ "kind": "approve" })).await;
    let view = wait_for(&server, &version, |view| view["run"]["status"] == "running").await;
    let run = view["run"]["id"].as_str().unwrap().to_owned();

    for (path, body) in [
        (
            "feedback",
            json!({ "commandId": common::command_id(), "text": "Colder." }),
        ),
        ("pause", json!({ "commandId": common::command_id() })),
    ] {
        let response = server
            .post(&format!("/api/runs/{run}/{path}"), &body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{path}");
    }

    let view = workspace(&server, &version).await;
    assert_eq!(view["pauseRequested"], true);
    assert_eq!(view["feedback"][0]["text"], "Colder.");
    assert_eq!(view["feedback"][0]["status"], "queued");
}
