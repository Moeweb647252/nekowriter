//! A preparation task is not a setting material (M15 15.4 review, spec/07 §8.1).
//!
//! The production path never leaves a preparation task in `draft`, so each test inserts a
//! synthetic `kind = 'preparation', origin = 'conception', status = 'draft'` row. Removing any of
//! the four `kind = 'topic'` filters makes one of these assertions fail.

#[path = "conception_harness/mod.rs"]
mod harness;

use harness::*;
use nekowriter::db::entities::agent_sessions;
use nekowriter::db::json::ViewSnapshot;
use sea_orm::EntityTrait;
use serde_json::json;

async fn synthetic_preparation(app: &nekowriter::testing::TestApp, book_id: i32) {
    app.execute(&format!(
        "INSERT INTO research_tasks(book_id, kind, origin, scope_name, topic, requirements, language, status, created_at, updated_at) \
         VALUES ({book_id}, 'preparation', 'conception', 'Synthetic scope', 'Synthetic preparation', '', 'zh-CN', 'draft', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
}

async fn seed_finalize_materials(app: &nekowriter::testing::TestApp, book_id: i32) {
    for (kind, content) in [
        ("outline", "# 大纲\n\n第一幕：入城。"),
        ("world", "# 世界观\n\n剑气折损心脉。"),
        ("protagonist", "# 主角\n\n沈砚。"),
    ] {
        let (status, body) = app
            .put(
                &format!("/api/books/{book_id}/setting-docs/{kind}"),
                json!({ "content": content, "base_revision": 0, "note": "" }),
            )
            .await;
        assert_eq!(status, 200, "seed {kind}: {body}");
    }
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "沈砚", "role_label": "主角", "summary": "游侠", "profile": "# 沈砚", "note": "" }),
        )
        .await;
    assert_eq!(status, 201, "seed character: {body}");
}

#[tokio::test]
async fn snapshot_and_current_conception_state_ignore_a_synthetic_preparation_draft() {
    let runtime = runtime(vec![text_step("waiting")]).await;
    let book = create_conceiving_book(&runtime, "Snapshot filter").await;
    let book_id = book["book"]["id"].as_i64().unwrap() as i32;
    let session_id = book["session_id"].as_i64().unwrap() as i32;
    synthetic_preparation(&runtime.app, book_id).await;

    let session = agent_sessions::Entity::find_by_id(session_id)
        .one(runtime.app.db().conn())
        .await
        .unwrap()
        .unwrap();
    let snapshot =
        nekowriter::agent::session::capture_view_snapshot(runtime.app.db().conn(), &session)
            .await
            .unwrap()
            .unwrap();
    let ViewSnapshot::Conception { topics, .. } = snapshot else {
        panic!("a conception session must have a conception snapshot");
    };
    assert!(
        topics
            .iter()
            .all(|topic| topic.topic != "Synthetic preparation"),
        "the view snapshot must not include a preparation task: {topics:?}"
    );

    let current =
        nekowriter::agent::report::current_conception_state(runtime.app.db().conn(), book_id)
            .await
            .unwrap();
    assert!(
        current
            .topics
            .iter()
            .all(|topic| topic.topic != "Synthetic preparation"),
        "current_conception_state must not include a preparation task: {:?}",
        current.topics
    );
}

#[tokio::test]
async fn set_research_topics_does_not_delete_a_synthetic_preparation_draft() {
    let runtime = runtime(vec![
        nekowriter_llm::mock::ScriptStep::new()
            .delay(std::time::Duration::from_secs(1))
            .tool(
                "set_research_topics",
                json!({ "topics": [{ "topic": "New topic", "requirements": "detail", "scope": "New scope" }] }),
            ),
        text_step("done"),
    ])
    .await;
    let book = create_conceiving_book(&runtime, "Topics filter").await;
    let book_id = book["book"]["id"].as_i64().unwrap() as i32;
    let session_id = book["session_id"].as_i64().unwrap() as i32;
    synthetic_preparation(&runtime.app, book_id).await;
    runtime.wait_any_status(session_id).await;

    let rows = research_tasks(&runtime, book_id).await;
    assert!(
        rows.iter().any(|row| row.0 == "Synthetic preparation"),
        "set_research_topics must leave a preparation draft alone: {rows:?}"
    );
    assert!(
        rows.iter().any(|row| row.0 == "New topic"),
        "the tool must still create its own topics: {rows:?}"
    );
}

#[tokio::test]
async fn finalize_does_not_ready_a_synthetic_preparation_draft() {
    let runtime = runtime(vec![text_step("waiting")]).await;
    let book = runtime.app.create_book("Finalize filter").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    runtime
        .app
        .execute(&format!(
            "UPDATE books SET status = 'conceiving' WHERE id = {book_id}"
        ))
        .await;
    synthetic_preparation(&runtime.app, book_id).await;
    seed_finalize_materials(&runtime.app, book_id).await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "finalize: {body}");
    let rows = research_tasks(&runtime, book_id).await;
    let synthetic = rows
        .iter()
        .find(|row| row.0 == "Synthetic preparation")
        .expect("the synthetic row survives");
    assert_eq!(
        synthetic.2, "draft",
        "finalize must not ready a preparation task"
    );
}
