//! Protected author research topics (M15 15.5, spec/05 §7, D-158).

#[path = "conception_harness/mod.rs"]
mod harness;

use harness::*;
use nekowriter::prompts::PromptRegistry;
use sea_orm::EntityTrait;
use serde_json::json;

async fn rows(
    app: &nekowriter::testing::TestApp,
    book_id: i32,
) -> Vec<(i32, String, String, String, String)> {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    app.db()
        .conn()
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT id, kind, origin, status, topic FROM research_tasks WHERE book_id = ? ORDER BY id",
            vec![book_id.into()],
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get("", "id").unwrap(),
                row.try_get("", "kind").unwrap(),
                row.try_get("", "origin").unwrap(),
                row.try_get("", "status").unwrap(),
                row.try_get("", "topic").unwrap(),
            )
        })
        .collect()
}

async fn create_author_task(app: &nekowriter::testing::TestApp, book_id: i32, topic: &str) -> i32 {
    let (status, body) = app
        .post(
            "/api/research-tasks",
            json!({ "book_id": book_id, "scope_name": "Background", "topic": topic, "requirements": "author detail" }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_i64().unwrap() as i32
}

async fn kickoff_text(runtime: &Runtime, session_id: i32) -> String {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT content FROM agent_messages WHERE session_id = ? AND seq = 0",
            vec![session_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    let blocks: Vec<serde_json::Value> =
        serde_json::from_str(&row.try_get::<String>("", "content").unwrap()).unwrap();
    blocks
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

async fn insert_agent_draft(app: &nekowriter::testing::TestApp, book_id: i32, topic: &str) -> i32 {
    app.execute(&format!(
        "INSERT INTO research_tasks(book_id, kind, origin, scope_name, topic, requirements, language, status, created_at, updated_at) \
         VALUES ({book_id}, 'topic', 'conception', 'Background', '{topic}', '', 'en', 'draft', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    rows(app, book_id).await.last().unwrap().0
}

#[tokio::test]
async fn author_drafts_survive_set_research_topics_and_are_in_the_kickoff() {
    let runtime = runtime(vec![
        tools_step(&[("set_research_topics", json!({
            "topics": [{ "topic": "Agent replacement", "requirements": "agent detail", "scope": "Background" }]
        }))]),
        text_step("done"),
        tools_step(&[("set_research_topics", json!({
            "topics": [{ "topic": "Agent second", "requirements": "", "scope": "Background" }]
        }))]),
        text_step("done"),
    ])
    .await;
    let book = runtime.app.create_book("Ownership").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    runtime
        .app
        .execute(&format!(
            "UPDATE books SET status = 'conceiving' WHERE id = {book_id}"
        ))
        .await;
    let author_before = create_author_task(&runtime.app, book_id, "Author before").await;
    let _agent_draft = insert_agent_draft(&runtime.app, book_id, "Agent old").await;

    let (status, started) = runtime
        .app
        .post(&format!("/api/books/{book_id}/conception/start"), json!({}))
        .await;
    assert_eq!(status, 200, "{started}");
    let session = started["session_id"].as_i64().unwrap() as i32;
    runtime.wait_any_status(session).await;
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session}/abandon"), json!({}))
        .await;
    assert_eq!(status, 200, "{body}");
    let after = rows(&runtime.app, book_id).await;
    assert!(
        after
            .iter()
            .any(|row| row.0 == author_before && row.4 == "Author before")
    );
    assert!(!after.iter().any(|row| row.4 == "Agent old"));
    assert!(after.iter().any(|row| row.4 == "Agent replacement"));

    // One more author task while the conception session is idle; a later kickoff lists both.
    let author_idle = create_author_task(&runtime.app, book_id, "Author while idle").await;
    println!(
        "before second kickoff rows: {:?}",
        rows(&runtime.app, book_id).await
    );
    let (status, body) = runtime
        .app
        .post(&format!("/api/books/{book_id}/conception/start"), json!({}))
        .await;
    assert_eq!(status, 200, "{body}");
    let second = body["session_id"].as_i64().unwrap() as i32;
    runtime.wait_any_status(second).await;
    let kickoff = kickoff_text(&runtime, second).await;
    let after = rows(&runtime.app, book_id).await;
    assert!(
        after
            .iter()
            .any(|row| row.0 == author_idle && row.4 == "Author while idle")
    );
    assert!(kickoff.contains("Author before"), "{kickoff}");
    assert!(kickoff.contains("Author while idle"), "{kickoff}");
    assert!(kickoff.contains("不要重复"), "{kickoff}");
}

#[tokio::test]
async fn adopted_agent_draft_survives_and_is_reported_as_an_edit() {
    let runtime = runtime(vec![text_step("idle")]).await;
    let book = runtime.app.create_book("Adoption").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    runtime
        .app
        .execute(&format!(
            "UPDATE books SET status = 'conceiving' WHERE id = {book_id}"
        ))
        .await;
    let agent = insert_agent_draft(&runtime.app, book_id, "Agent draft").await;
    // Capture the agent's view before adoption.
    let (status, started) = runtime
        .app
        .post(&format!("/api/books/{book_id}/conception/start"), json!({}))
        .await;
    assert_eq!(status, 200, "{started}");
    let session = started["session_id"].as_i64().unwrap() as i32;
    runtime.wait_any_status(session).await;
    let (_, session_body) = runtime.app.get(&format!("/api/sessions/{session}")).await;
    let _ = session_body;
    let session_row = nekowriter::db::entities::agent_sessions::Entity::find_by_id(session)
        .one(runtime.app.db().conn())
        .await
        .unwrap()
        .unwrap();
    let snapshot =
        nekowriter::agent::session::capture_view_snapshot(runtime.app.db().conn(), &session_row)
            .await
            .unwrap()
            .unwrap();
    // Adoption is a PATCH of the agent's draft.
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/research-tasks/{agent}"),
            json!({ "requirements": "adopted by author" }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let current =
        nekowriter::agent::report::current_conception_state(runtime.app.db().conn(), book_id)
            .await
            .unwrap();
    let prompts = PromptRegistry::embedded().expect("pack").defaults_only();
    let report = nekowriter::agent::report::conception_report(&snapshot, &current, &prompts);
    assert!(
        report.items.iter().any(|item| item["kind"] == "topics"),
        "an adoption must be a setting edit: {report:?}"
    );
    let adopted = rows(&runtime.app, book_id).await;
    assert!(
        adopted
            .iter()
            .any(|row| row.0 == agent && row.2 == "author")
    );
}

#[tokio::test]
async fn finalize_turns_drafts_of_both_origins_ready_but_not_preparation() {
    let runtime = runtime(Vec::new()).await;
    let book = runtime.app.create_book("Finalize ownership").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    runtime
        .app
        .execute(&format!(
            "UPDATE books SET status = 'conceiving' WHERE id = {book_id}"
        ))
        .await;
    let author = create_author_task(&runtime.app, book_id, "Author final").await;
    let agent = insert_agent_draft(&runtime.app, book_id, "Agent final").await;
    // A synthetic preparation draft proves finalize ignores the fourth kind even though it is
    // otherwise a draft.
    runtime
        .app
        .execute(&format!(
            "INSERT INTO research_tasks(book_id, kind, origin, scope_name, topic, requirements, language, status, created_at, updated_at) \
             VALUES ({book_id}, 'preparation', 'conception', 'Synthetic', 'Synthetic preparation', '', 'en', 'draft', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ))
        .await;
    for (kind, content) in [
        ("outline", "# 大纲"),
        ("world", "# 世界观"),
        ("protagonist", "# 主角"),
    ] {
        let (status, body) = runtime
            .app
            .put(
                &format!("/api/books/{book_id}/setting-docs/{kind}"),
                json!({ "content": content, "base_revision": 0, "note": "" }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
    }
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "沈砚", "role_label": "主角", "summary": "游侠", "profile": "# 沈砚", "note": "" }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let after = rows(&runtime.app, book_id).await;
    assert_eq!(after.iter().find(|row| row.0 == author).unwrap().3, "ready");
    assert_eq!(after.iter().find(|row| row.0 == agent).unwrap().3, "ready");
    assert!(
        after
            .iter()
            .any(|row| row.4 == "Synthetic preparation" && row.3 == "draft")
    );
}
