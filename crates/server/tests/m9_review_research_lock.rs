//! M9 review finding: the draft research topics are setting materials and are locked while the
//! book's conception session runs (spec/08 §1.4).

#[path = "conception_harness/mod.rs"]
mod harness;

use std::time::Duration;

use harness::*;
use nekowriter::db::json::SessionStatus;
use serde_json::json;

/// Setting materials include the draft research topics (spec/08 §1.4): while the conception
/// session runs, creating, editing and deleting a draft topic is refused with
/// `423 settings.locked` naming the session, exactly like the documents and the cast.
#[tokio::test]
async fn draft_research_topics_are_locked_while_the_conception_session_runs() {
    // A slow first turn keeps the session `running` while the author's writes arrive.
    let slow = nekowriter_llm::mock::ScriptStep {
        delay: Duration::from_millis(1500),
        ..tool_step("echo", json!({ "text": "thinking" }))
    };
    let runtime = runtime(vec![slow]).await;
    // The book starts without a conception model so its draft topics can be created first.
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["conception"] = serde_json::Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "drop the conception role: {body}");
    let book = create_conceiving_book(&runtime, "Locked Topics").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let (status, topic) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "topic": "边城城防",
                "requirements": "城门与轮值",
                "scope_name": "背景设定",
            }),
        )
        .await;
    assert_eq!(status, 201, "draft topic: {topic}");
    let topic_id = topic["id"].as_i64().expect("topic id") as i32;
    assert_eq!(topic["status"], "draft");

    // Configure the model and start the conception session; its first turn is slow.
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["conception"] = roles["writer"].clone();
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "restore the conception role: {body}");
    let (status, body) = runtime
        .app
        .post(&format!("/api/books/{book_id}/conception/start"), json!({}))
        .await;
    assert_eq!(status, 200, "start conception: {body}");
    let session_id = body["session_id"].as_i64().expect("session id") as i32;
    assert_eq!(
        runtime.session_row(session_id).await.status,
        "running",
        "the slow first turn is running"
    );

    // Creating a new draft topic is refused while the agent works on the materials.
    let (status, body) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "topic": "夜禁规矩",
                "requirements": "细节",
                "scope_name": "背景设定",
            }),
        )
        .await;
    assert_eq!(status, 423, "create while running: {body}");
    assert_eq!(body["error"]["code"], "settings.locked");
    assert_eq!(body["error"]["details"]["session_id"], json!(session_id));

    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/research-tasks/{topic_id}"),
            json!({ "topic": "改过的主题" }),
        )
        .await;
    assert_eq!(status, 423, "edit while running: {body}");
    assert_eq!(body["error"]["code"], "settings.locked");
    assert_eq!(body["error"]["details"]["session_id"], json!(session_id));

    let (status, body) = runtime
        .app
        .delete(&format!("/api/research-tasks/{topic_id}"))
        .await;
    assert_eq!(status, 423, "delete while running: {body}");
    assert_eq!(body["error"]["code"], "settings.locked");
    assert_eq!(body["error"]["details"]["session_id"], json!(session_id));

    // Once the turn settles (idle), the author may edit again (spec/08 §1.4).
    let _ = runtime.wait_status(session_id, SessionStatus::Idle).await;
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/research-tasks/{topic_id}"),
            json!({ "topic": "改过的主题" }),
        )
        .await;
    assert_eq!(status, 200, "editable once idle: {body}");
}
