//! Outline-session concurrency and locking rules (PLAN M12.5, spec/03 §14, spec/08 §1.4, D-146).
//!
//! R6: at most one open session across the planner and writer roles per chapter, so outline,
//! write and revise sessions exclude each other; chapters never block each other. The creation
//! check is re-run inside the transaction (D-121), so two concurrent `POST /outline` calls start
//! exactly one session.

use std::time::Duration;

use axum::http::StatusCode;
use nekowriter::testing::TestApp;
use nekowriter_llm::mock::{ScriptStep, ScriptedProvider};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde_json::json;

/// A migrated app with an active book, its main track and one chapter.
async fn app_with_book() -> (TestApp, i32, i32) {
    let app = TestApp::new().await;
    let (book_id, _track_id, chapter_id) = create_book(&app).await;
    (app, book_id, chapter_id)
}

async fn create_book(app: &TestApp) -> (i32, i32, i32) {
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    (book_id, track_id, chapter_id)
}

/// Assigns one Demo model to every role so the endpoints can resolve a model.
async fn assign_models(app: &TestApp) {
    let (status, provider) = app
        .post(
            "/api/providers",
            json!({
                "name": "Demo",
                "protocol": "mock",
                "base_url": "mock://demo",
                "extra_headers": {},
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let provider_id = provider["id"].as_i64().expect("provider id") as i32;
    let (status, profile) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Demo model",
                "model": "demo-model",
                "context_window": 200_000,
                "max_output_tokens": 8_192,
                "options": {},
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{profile}");
    let profile_id = profile["id"].as_i64().expect("profile id") as i32;
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": {
                "conception": profile_id,
                "researcher": profile_id,
                "writer": profile_id,
                "reviewer": profile_id,
                "archivist": profile_id,
            }}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Inserts an open planner session for a chapter, without a runner (a state fixture).
async fn insert_outline_session(app: &TestApp, chapter_id: i32, status: &str) -> i32 {
    app.execute(&format!(
        "INSERT INTO agent_sessions(book_id, role, kind, chapter_id, track_id, model_snapshot, \
         system_prompt, tools, status, phase, counters, usage, created_at, updated_at) \
         SELECT book_id, 'planner', 'chapter_outline', id, track_id, '{{}}', '', '[]', \
         '{status}', 'none', '{{}}', '{{}}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z' \
         FROM chapters WHERE id = {chapter_id}"
    ))
    .await;
    let row = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT max(id) AS id FROM agent_sessions".to_owned(),
        ))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i32>("", "id").expect("id")
}

/// One concurrent `POST /outline` wins; the loser answers `chapter.session_exists` with its
/// `session_id` (D-121 style).
#[tokio::test]
async fn concurrent_outline_creations_start_one_session() {
    let mut app = TestApp::new().await;
    // Hold the winner's model request open long enough for both requests to overlap.
    let _scripted = app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new()
            .tool(
                "submit_outline",
                json!({ "outline": "细纲", "reference_notes": "- 要点" }),
            )
            .delay(Duration::from_secs(2)),
    ]));
    assign_models(&app).await;
    let (_book_id, _track_id, chapter_id) = create_book(&app).await;

    let path = format!("/api/chapters/{chapter_id}/outline");
    let (a, b) = tokio::join!(app.post(&path, json!({})), app.post(&path, json!({})));
    let mut created = 0;
    let mut refused = None;
    for (status, body) in [&a, &b] {
        match *status {
            StatusCode::CREATED => created += 1,
            StatusCode::CONFLICT => refused = Some(body.clone()),
            other => panic!("unexpected status {other}: {body}"),
        }
    }
    assert_eq!(created, 1, "exactly one outline may start: {a:?} {b:?}");
    let refused = refused.expect("the loser answers 409");
    assert_eq!(
        refused["error"]["code"], "chapter.session_exists",
        "{refused}"
    );
    assert!(
        refused["error"]["details"]["session_id"].is_number(),
        "{refused}"
    );
}

/// An open outline session blocks starting a write session on the same chapter.
#[tokio::test]
async fn an_open_outline_session_blocks_a_write_session() {
    let (app, _book_id, chapter_id) = app_with_book().await;
    assign_models(&app).await;
    app.execute(&format!(
        "UPDATE chapters SET outline = '细纲' WHERE id = {chapter_id}"
    ))
    .await;
    insert_outline_session(&app, chapter_id, "held").await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "chapter.session_exists");
}

/// An open owner session blocks starting an outline session.
#[tokio::test]
async fn an_open_writer_session_blocks_an_outline_session() {
    let (app, _book_id, chapter_id) = app_with_book().await;
    assign_models(&app).await;
    app.insert_writer_session(chapter_id, "held").await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/outline"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "chapter.session_exists");
}

/// Author edits are `423 chapter.locked` while the outline session runs.
#[tokio::test]
async fn author_edits_are_locked_while_an_outline_session_runs() {
    let (app, _book_id, chapter_id) = app_with_book().await;
    insert_outline_session(&app, chapter_id, "running").await;
    let (status, body) = app
        .patch(
            &format!("/api/chapters/{chapter_id}"),
            json!({ "outline": "作者想改的细纲" }),
        )
        .await;
    assert_eq!(status, StatusCode::LOCKED, "{body}");
    assert_eq!(body["error"]["code"], "chapter.locked");
    let (status, body) = app
        .patch(
            &format!("/api/chapters/{chapter_id}/content"),
            json!({ "base_version": 0, "ops": [] }),
        )
        .await;
    assert_eq!(status, StatusCode::LOCKED, "{body}");
    assert_eq!(body["error"]["code"], "chapter.locked");
}

/// PATCH per kind: an outline session accepts `pause_after_outline` only.
#[tokio::test]
async fn patch_of_an_outline_session_accepts_only_pause_after_outline() {
    let (app, _book_id, chapter_id) = app_with_book().await;
    let session_id = insert_outline_session(&app, chapter_id, "paused").await;

    let (status, body) = app
        .patch(
            &format!("/api/sessions/{session_id}"),
            json!({ "auto_pause": true }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "session.invalid_state");

    let (status, body) = app
        .patch(
            &format!("/api/sessions/{session_id}"),
            json!({ "pause_after_outline": false }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["pause_after_outline"], false);
}
