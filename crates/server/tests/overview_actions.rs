//! The book overview: next action, attention list and usage (PLAN 11.1, spec/08 §7).
//!
//! Each test drives the real router and seeds only the state a later workflow produces (an
//! official chapter, an open session, a pending proposal), so the assertions cover the exact
//! first-match order spec/08 §7 defines.

use axum::http::StatusCode;
use nekowriter::testing::TestApp;
use serde_json::{Value, json};

/// A migrated app with an active book, its main track and one chapter.
async fn app_with_book() -> (TestApp, i32, i32, i32) {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    (app, book_id, track_id, chapter_id)
}

async fn overview(app: &TestApp, book_id: i32) -> Value {
    let (status, body) = app.get(&format!("/api/books/{book_id}/overview")).await;
    assert_eq!(status, StatusCode::OK, "overview: {body}");
    body
}

/// Makes the chapter official with the given canon status.
async fn official(app: &TestApp, chapter_id: i32, canon: &str) {
    app.execute(&format!(
        "UPDATE chapters SET status = 'official', canon_status = '{canon}' WHERE id = {chapter_id}"
    ))
    .await;
}

#[tokio::test]
async fn a_conceiving_book_continues_its_conception() {
    let (app, book_id, _, _) = app_with_book().await;
    app.execute(&format!(
        "UPDATE books SET status = 'conceiving' WHERE id = {book_id}"
    ))
    .await;
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "continue_conception");
}

#[tokio::test]
async fn a_book_with_designs_but_no_canon_seeds_it() {
    let (app, book_id, _, _) = app_with_book().await;
    let (status, character) = app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "林凡", "role_label": "主角", "summary": "the protagonist" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "character: {character}");
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "seed_canon");
}

#[tokio::test]
async fn a_canon_seed_that_completed_is_not_offered_again() {
    let (app, book_id, _, _) = app_with_book().await;
    let (status, _) = app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "林凡" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    // A completed seed session means the canonical next step is no longer seeding.
    app.execute(&format!(
        "INSERT INTO agent_sessions(book_id, role, kind, model_snapshot, system_prompt, tools, \
         status, created_at, updated_at) VALUES ({book_id}, 'archivist', 'canon_seed', '{{}}', '', \
         '[]', 'completed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    let body = overview(&app, book_id).await;
    assert_ne!(
        body["next_action"]["kind"], "seed_canon",
        "a completed seed is not repeated: {body}"
    );
}

#[tokio::test]
async fn a_chapter_in_review_is_the_next_action() {
    let (app, book_id, track_id, chapter_id) = app_with_book().await;
    app.execute(&format!(
        "UPDATE chapters SET status = 'in_review' WHERE id = {chapter_id}"
    ))
    .await;
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "review_chapter");
    assert_eq!(body["next_action"]["chapter"]["id"], chapter_id);
    assert_eq!(body["next_action"]["track_id"], track_id);
    assert_eq!(
        body["next_action"]["chapter"]["number"], 1,
        "the UI composes the label from the number (D-054)"
    );
}

#[tokio::test]
async fn a_drafting_chapter_continues_with_its_open_writer_session() {
    let (app, book_id, _, chapter_id) = app_with_book().await;
    app.set_chapter_status(chapter_id, "drafting").await;
    app.insert_writer_session(chapter_id, "held").await;
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "continue_chapter");
    assert_eq!(body["next_action"]["chapter"]["id"], chapter_id);
    let session_id = body["next_action"]["session_id"]
        .as_i64()
        .expect("the open writer session is named");
    // The session row the fixture created is the one the action points at.
    let (status, session) = app.get(&format!("/api/sessions/{session_id}")).await;
    assert_eq!(status, StatusCode::OK, "session: {session}");
}

#[tokio::test]
async fn an_unsynced_official_chapter_is_the_next_action() {
    let (app, book_id, track_id, chapter_id) = app_with_book().await;
    official(&app, chapter_id, "pending").await;
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "sync_canon");
    assert_eq!(body["next_action"]["chapter"]["id"], chapter_id);
    assert_eq!(body["next_action"]["track_id"], track_id);
}

#[tokio::test]
async fn a_finished_chapter_points_at_writing_the_next_one() {
    let (app, book_id, track_id, chapter_id) = app_with_book().await;
    official(&app, chapter_id, "synced").await;
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "write_next");
    assert_eq!(body["next_action"]["track_id"], track_id);
    assert_eq!(
        body["next_action"].get("chapter"),
        None,
        "write_next names the track, not a chapter: {body}"
    );
}

#[tokio::test]
async fn attention_lists_sessions_reviews_canon_proposals_and_research() {
    let (app, book_id, _, chapter_id) = app_with_book().await;
    // A held writer session and a failed one both need the author.
    app.insert_writer_session(chapter_id, "held").await;
    app.insert_writer_session(chapter_id, "failed").await;
    // A second chapter in review and an official chapter whose canon is stale.
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let (_, second) = app
        .post(
            &format!("/api/tracks/{track_id}/chapters"),
            json!({ "title": "Chapter Two" }),
        )
        .await;
    let second_id = second["id"].as_i64().expect("chapter id") as i32;
    app.execute(&format!(
        "UPDATE chapters SET status = 'in_review' WHERE id = {second_id}"
    ))
    .await;
    official(&app, chapter_id, "stale").await;
    // One pending proposal and one ready research task.
    app.execute(&format!(
        "INSERT INTO setting_proposals(book_id, target_type, target_kind, target_id, edits, \
         rationale, status, chapter_id, created_at) VALUES ({book_id}, 'setting_doc', 'world', 1, \
         '[]', 'why', 'pending', {chapter_id}, '2026-01-01T00:00:00Z')"
    ))
    .await;
    let (status, task) = app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "scope_name": "背景设定",
                "topic": "边城城防",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "research task: {task}");

    let body = overview(&app, book_id).await;
    let attention = body["attention"].as_array().expect("attention list");
    let kinds: Vec<&str> = attention
        .iter()
        .map(|item| item["kind"].as_str().expect("kind"))
        .collect();
    assert!(
        kinds.contains(&"open_session") && kinds.iter().filter(|k| **k == "open_session").count() == 2,
        "both open sessions are listed: {body}"
    );
    assert!(kinds.contains(&"chapter_in_review"), "{body}");
    assert!(kinds.contains(&"canon_unsynced"), "{body}");
    let proposals = attention
        .iter()
        .find(|item| item["kind"] == "pending_proposals")
        .expect("the pending proposal is counted");
    assert_eq!(proposals["count"], 1);
    let research = attention
        .iter()
        .find(|item| item["kind"] == "ready_research")
        .expect("the ready research task is counted");
    assert_eq!(research["count"], 1);
    // The library card's badge is the same number (spec/10 §2.2).
    let (_, books) = app.get("/api/books").await;
    let card = books
        .as_array()
        .expect("book list")
        .iter()
        .find(|book| book["id"] == json!(book_id))
        .expect("the book's card");
    assert_eq!(card["attention_count"], attention.len() as i64);
}

#[tokio::test]
async fn usage_sums_the_token_totals_of_the_books_sessions() {
    let (app, book_id, _, chapter_id) = app_with_book().await;
    let usage = |input: i64, output: i64| {
        format!(
            "{{\"requests\":2,\"input_tokens\":{input},\"cached_input_tokens\":0,\
             \"cache_write_tokens\":0,\"output_tokens\":{output},\"reasoning_tokens\":0,\
             \"last_prompt_tokens\":0,\"estimated\":false}}"
        )
    };
    for (status, values) in [("completed", usage(100, 10)), ("held", usage(200, 20))] {
        app.execute(&format!(
            "INSERT INTO agent_sessions(book_id, role, kind, chapter_id, track_id, model_snapshot, \
             system_prompt, tools, status, usage, created_at, updated_at) VALUES ({book_id}, \
             'writer', 'chapter_write', {chapter_id}, 1, '{{}}', '', '[]', '{status}', '{values}', \
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ))
        .await;
    }
    let body = overview(&app, book_id).await;
    assert_eq!(body["usage"]["requests"], 4, "{body}");
    assert_eq!(body["usage"]["input_tokens"], 300);
    assert_eq!(body["usage"]["output_tokens"], 30);
    assert_eq!(body["usage"]["last_prompt_tokens"], 300);
}
