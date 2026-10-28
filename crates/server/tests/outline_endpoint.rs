//! `POST /chapters/:id/outline` precondition matrix (F12, spec/08 §1.1, D-146). The behaviour
//! tests call `sessions::create` directly; these drive the HTTP endpoint itself.

#![allow(dead_code)]

#[path = "agent_harness/mod.rs"]
mod base;

pub use base::*;

use axum::http::StatusCode;
use nekowriter_llm::mock::ScriptStep;
use serde_json::json;

fn submit_step() -> ScriptStep {
    ScriptStep::new().tool(
        "submit_outline",
        json!({ "outline": "细纲", "reference_notes": "- 要点" }),
    )
}

#[tokio::test]
async fn an_official_chapter_cannot_be_reoutlined() {
    let runtime = runtime(vec![submit_step()]).await;
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET status = 'official' WHERE id = {}",
            runtime.chapter_id
        ))
        .await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/outline", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "chapter.invalid_state");
}

#[tokio::test]
async fn previous_not_ready_and_force_and_guidance_storage() {
    let runtime = runtime(vec![submit_step()]).await;
    let second = runtime.app.create_chapter(runtime.track_id, "Second").await;
    let second_id = second["id"].as_i64().expect("id") as i32;
    let path = format!("/api/chapters/{second_id}/outline");
    let (status, body) = runtime.app.post(&path, json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "chapter.previous_not_ready");
    assert_eq!(body["error"]["details"]["chapter_id"], runtime.chapter_id);

    let (status, body) = runtime
        .app
        .post(
            &path,
            json!({ "force": true, "guidance": "作者对本章的要求", "pause_after_outline": true }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().expect("session id") as i32;
    let (_, chapter) = runtime.app.get(&format!("/api/chapters/{second_id}")).await;
    assert_eq!(chapter["guidance"], "作者对本章的要求");
    let (_, session) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(session["session"]["pause_after_outline"], true);
}

#[tokio::test]
async fn a_missing_planner_and_writer_model_is_model_missing_role() {
    let runtime = runtime(vec![submit_step()]).await;
    runtime
        .app
        .execute("DELETE FROM app_settings WHERE key = 'roles.defaults'")
        .await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/outline", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "model.missing_role");
}
