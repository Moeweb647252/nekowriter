//! M9 review findings (spec/08 §5, spec/05 §7, spec/10 §3).
//!
//! Each test here pins one defect the review verified: the failing assertion is the reproduction
//! of the reported behaviour, so it must fail before its fix and pass after it.

#[path = "conception_harness/mod.rs"]
mod harness;

use std::time::Duration;

use harness::*;
use nekowriter::api::dto::ResearchTaskDto;
use nekowriter::db::json::ResearchTaskStatus;
use nekowriter::events::ServerEvent;
use serde_json::json;

/// A conceiving book with every material finalize needs, created without a conception model so no
/// session exists and the scripted queue stays deterministic.
async fn finalized_materials(runtime: &Runtime) -> i32 {
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["conception"] = serde_json::Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "drop the conception role: {body}");

    let book = create_conceiving_book(runtime, "Review Findings").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    for (kind, content) in [
        ("outline", "# 大纲\n\n第一幕：入城。"),
        ("world", "# 世界观\n\n剑气折损心脉。"),
        ("protagonist", "# 主角\n\n沈砚。"),
    ] {
        let (status, body) = runtime
            .app
            .put(
                &format!("/api/books/{book_id}/setting-docs/{kind}"),
                json!({ "content": content, "base_revision": 0, "note": "" }),
            )
            .await;
        assert_eq!(status, 200, "seed {kind}: {body}");
    }
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({
                "name": "沈砚",
                "role_label": "主角",
                "summary": "游侠",
                "profile": "# 沈砚",
                "note": "",
            }),
        )
        .await;
    assert_eq!(status, 201, "seed character: {body}");
    book_id
}

/// The `research.updated` event finalize publishes must carry the task as it is **after** the
/// update (spec/10 §3: the payload is the current `ResearchTaskDto`, and the client upserts it), so
/// the task reads `ready` and names its scope, not the `draft` row the transaction replaced.
#[tokio::test]
async fn finalize_emits_the_ready_task_in_its_event() {
    let runtime = runtime(vec![]).await;
    let book_id = finalized_materials(&runtime).await;
    let (status, task) = runtime
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
    assert_eq!(status, 201, "create draft topic: {task}");
    assert_eq!(task["status"], "draft");

    let mut receiver = runtime.app.events().subscribe();
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "finalize: {body}");

    let mut published: Vec<ResearchTaskDto> = Vec::new();
    while let Ok(Ok(envelope)) =
        tokio::time::timeout(Duration::from_millis(200), receiver.recv()).await
    {
        if let ServerEvent::ResearchUpdated { task } = envelope.event {
            published.push(task);
        }
    }
    let task = published
        .last()
        .unwrap_or_else(|| panic!("finalize publishes research.updated: {published:?}"));
    assert_eq!(
        task.status,
        ResearchTaskStatus::Ready,
        "the event carries the updated task, not the draft row: {task:?}"
    );
    assert!(
        task.scope_id.is_some(),
        "the event names the scope finalize attached: {task:?}"
    );
}

/// Two concurrent `conception/start` requests: the loser of the creation race must return the open
/// session (spec/08 §5: "it returns the open conception session if there is one"), not a 409 that
/// surfaces as an error toast for a double click.
#[tokio::test]
async fn concurrent_conception_start_returns_the_open_session() {
    let runtime = runtime(vec![]).await;
    // A conceiving book created while no conception model exists has no session yet.
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["conception"] = serde_json::Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "drop the conception role: {body}");
    let book = create_conceiving_book(&runtime, "Double Start").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    assert!(book["session_id"].is_null(), "no session yet: {book}");

    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["conception"] = roles["writer"].clone();
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "restore the conception role: {body}");

    let path = format!("/api/books/{book_id}/conception/start");
    let (first, second) = tokio::join!(
        runtime.app.post(&path, json!({})),
        runtime.app.post(&path, json!({}))
    );
    assert_eq!(first.0, 200, "first start: {}", first.1);
    assert_eq!(
        second.0, 200,
        "the creation-race loser returns the open session: {}",
        second.1
    );
    assert_eq!(
        first.1["session_id"], second.1["session_id"],
        "both calls name the same session"
    );
}

/// Two concurrent `canon/seed` requests behave the same way (spec/08 §5, D-047): the loser of the
/// creation race returns the open session instead of a 409.
#[tokio::test]
async fn concurrent_canon_seed_returns_the_open_session() {
    let runtime = runtime(vec![]).await;
    let book_id = finalized_materials(&runtime).await;
    // Finalize without an archivist, so no seed session exists yet (spec/08 §5 step 5).
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["archivist"] = serde_json::Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "drop the archivist: {body}");
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "finalize: {body}");
    assert!(body["seed_session_id"].is_null(), "no seed yet: {body}");
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["archivist"] = roles["writer"].clone();
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "restore the archivist: {body}");

    let path = format!("/api/books/{book_id}/canon/seed");
    let (first, second) = tokio::join!(
        runtime.app.post(&path, json!({})),
        runtime.app.post(&path, json!({}))
    );
    assert_eq!(first.0, 200, "first seed: {}", first.1);
    assert_eq!(
        second.0, 200,
        "the creation-race loser returns the open session: {}",
        second.1
    );
    assert_eq!(
        first.1["session_id"], second.1["session_id"],
        "both calls name the same session"
    );
}
