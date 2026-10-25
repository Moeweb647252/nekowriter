//! M7 review: the knowledge scopes reach the model through `block.settings` (spec/07 §1).
//!
//! Spec/07 §1 says the settings block lists the book's attached scopes (name + description) and
//! that the description is shown to agents. Without it the model has to guess which scope names
//! `search_knowledge`'s `scope` parameter accepts.

mod agent_write_harness;

use agent_write_harness::base::runtime;
use nekowriter::db::json::Granularity;
use nekowriter::testing::TestApp;
use serde_json::json;

async fn scope(app: &TestApp, name: &str, description: &str) -> i32 {
    let (status, body) = app
        .post(
            "/api/knowledge/scopes",
            json!({ "name": name, "description": description }),
        )
        .await;
    assert_eq!(status, 201, "scope: {body}");
    body["id"].as_i64().expect("scope id") as i32
}

/// The kickoff text of a session, joined across its blocks.
async fn kickoff_text(runtime: &agent_write_harness::Runtime, session_id: i32) -> String {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = runtime
        .app
        .state()
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT content FROM agent_messages WHERE session_id = ? AND seq = 0",
            vec![session_id.into()],
        ))
        .await
        .expect("query kickoff")
        .expect("kickoff row");
    let raw: String = row.try_get("", "content").expect("content");
    let blocks: Vec<serde_json::Value> = serde_json::from_str(&raw).expect("blocks");
    blocks
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A writer session's kickoff lists the attached scopes with their descriptions, and only those.
#[tokio::test]
async fn the_kickoff_settings_block_lists_the_attached_scopes() {
    let runtime = runtime(Vec::new()).await;
    let attached = scope(&runtime.app, "Song life", "Song dynasty daily life").await;
    scope(&runtime.app, "Hidden lore", "Not attached to this book").await;
    let (status, body) = runtime
        .app
        .put(
            &format!("/api/books/{}/scopes", runtime.book_id),
            json!({ "scope_ids": [attached] }),
        )
        .await;
    assert_eq!(status, 200, "attach: {body}");

    let session_id =
        agent_write_harness::start_writer(&runtime, Granularity::Paragraph, false, false).await;
    let text = kickoff_text(&runtime, session_id).await;
    assert!(
        text.contains("Song life"),
        "the attached scope's name must reach the model (spec/07 §1): {text}"
    );
    assert!(
        text.contains("Song dynasty daily life"),
        "the scope description is shown to agents (spec/07 §1): {text}"
    );
    assert!(
        !text.contains("Hidden lore"),
        "an unattached scope must not be listed: {text}"
    );
}
