//! Timeline tree rules (PLAN M22 22.3, spec/06 §1.4–§1.5, D-170).
//!
//! Exercises the API-facing tree rules: unlimited depth with position order, parent/status
//! validation that writes nothing, promotion consistency, root/child delete protection and the
//! foreshadowing anchor.

use axum::http::StatusCode;
use nekowriter::testing::{TestApp, assert_error};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serde_json::{Value, json};

mod canon_fixture;

use canon_fixture::book;

async fn create_event(app: &TestApp, book_id: i32, body: Value) -> Value {
    let (status, event) = app
        .post(&format!("/api/books/{book_id}/timeline"), body)
        .await;
    assert_eq!(status, StatusCode::CREATED, "create event: {event}");
    event
}

async fn patch_event(app: &TestApp, event_id: i32, body: Value) -> (StatusCode, Value) {
    app.patch(&format!("/api/timeline-events/{event_id}"), body)
        .await
}

async fn scalar(app: &TestApp, sql: &str) -> i64 {
    let row = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i64>("", "value").expect("value")
}

async fn timeline(app: &TestApp, book_id: i32) -> Vec<Value> {
    let (status, rows) = app.get(&format!("/api/books/{book_id}/timeline")).await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    rows.as_array().expect("array").clone()
}

#[tokio::test]
async fn three_levels_keep_position_order_and_parent_ids() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let a = create_event(&app, book_id, json!({ "title": "A" })).await;
    let a_id = a["id"].as_i64().expect("id") as i32;
    let b = create_event(&app, book_id, json!({ "title": "B", "parent_id": a_id })).await;
    let b_id = b["id"].as_i64().expect("id") as i32;
    let c = create_event(&app, book_id, json!({ "title": "C", "parent_id": b_id })).await;
    let c_id = c["id"].as_i64().expect("id") as i32;
    let d = create_event(&app, book_id, json!({ "title": "D", "parent_id": a_id })).await;
    let d_id = d["id"].as_i64().expect("id") as i32;

    let rows = timeline(&app, book_id).await;
    let ids: Vec<i32> = rows
        .iter()
        .map(|row| row["id"].as_i64().expect("id") as i32)
        .collect();
    assert_eq!(
        ids,
        vec![a_id, b_id, c_id, d_id],
        "story order is position order"
    );
    let parent = |id: i32| {
        rows.iter()
            .find(|row| row["id"] == id)
            .and_then(|row| row["parent_id"].as_i64())
            .map(|value| value as i32)
    };
    assert_eq!(parent(a_id), None, "the root is shown as top level");
    assert_eq!(parent(b_id), Some(a_id));
    assert_eq!(parent(c_id), Some(b_id));
    assert_eq!(parent(d_id), Some(a_id), "D goes after A's subtree");
    assert_eq!(
        rows.iter().filter(|row| row["kind"] == "entry").count(),
        4,
        "the root is never listed"
    );
}

#[tokio::test]
async fn validation_matrix_writes_nothing() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let (other_book, _) = book(&app).await;
    let foreign = create_event(&app, other_book, json!({ "title": "Foreign" })).await;
    let foreign_id = foreign["id"].as_i64().expect("id") as i32;
    let chapter = app.create_chapter(1, "One").await;
    let chapter_id = chapter["id"].as_i64().expect("id") as i32;
    let before = scalar(&app, "SELECT count(*) AS value FROM timeline_events").await;

    let cases = [
        (
            json!({ "title": "Unknown parent", "parent_id": 999 }),
            "parent",
        ),
        (
            json!({ "title": "Foreign parent", "parent_id": foreign_id }),
            "parent",
        ),
        (
            json!({ "title": "Planned with chapter", "status": "planned", "chapter_id": chapter_id }),
            "chapter_id",
        ),
    ];
    for (body, field) in cases {
        let (status, response) = app
            .post(&format!("/api/books/{book_id}/timeline"), body)
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {response}");
        assert_error(&response, "validation.failed");
        assert!(
            response["error"]["details"]["fields"][field].is_string(),
            "{field}: {response}"
        );
    }
    assert_eq!(
        scalar(&app, "SELECT count(*) AS value FROM timeline_events").await,
        before,
        "a rejected write stores nothing"
    );

    let occurred = create_event(&app, book_id, json!({ "title": "Occurred" })).await;
    let occurred_id = occurred["id"].as_i64().expect("id") as i32;
    for body in [
        json!({ "status": "planned" }),
        json!({ "status": "abandoned" }),
    ] {
        let (status, response) = patch_event(&app, occurred_id, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {response}");
        assert_error(&response, "validation.failed");
        assert!(
            response["error"]["details"]["fields"]["status"].is_string(),
            "{response}"
        );
    }
}

#[tokio::test]
async fn promotion_refuses_a_planned_descendant_and_names_it() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let chapter = app.create_chapter(1, "One").await;
    let chapter_id = chapter["id"].as_i64().expect("id") as i32;
    let plan = create_event(
        &app,
        book_id,
        json!({ "title": "Phase", "status": "planned" }),
    )
    .await;
    let plan_id = plan["id"].as_i64().expect("id") as i32;
    let scene = create_event(
        &app,
        book_id,
        json!({ "title": "Scene", "status": "planned", "parent_id": plan_id }),
    )
    .await;
    let scene_id = scene["id"].as_i64().expect("id") as i32;

    let (status, response) = patch_event(
        &app,
        plan_id,
        json!({ "status": "occurred", "chapter_id": chapter_id }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    assert_error(&response, "validation.failed");
    let message = response["error"]["details"]["fields"]["status"]
        .as_str()
        .unwrap_or_default();
    assert!(message.contains("Scene"), "names the conflict: {message}");

    // The descendant may happen first; then the phase can be promoted.
    let (status, response) = patch_event(
        &app,
        scene_id,
        json!({ "status": "occurred", "chapter_id": chapter_id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let (status, response) = patch_event(
        &app,
        plan_id,
        json!({ "status": "occurred", "chapter_id": chapter_id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["status"], "occurred");
}

#[tokio::test]
async fn root_and_parent_delete_protection() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let root_id = scalar(
        &app,
        &format!(
            "SELECT id AS value FROM timeline_events WHERE book_id = {book_id} AND kind = 'root'"
        ),
    )
    .await as i32;
    let (status, response) = app.delete(&format!("/api/timeline-events/{root_id}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "root delete: {response}");
    assert_error(&response, "validation.failed");

    let parent = create_event(&app, book_id, json!({ "title": "Parent" })).await;
    let parent_id = parent["id"].as_i64().expect("id") as i32;
    let child = create_event(
        &app,
        book_id,
        json!({ "title": "Child", "parent_id": parent_id }),
    )
    .await;
    let child_id = child["id"].as_i64().expect("id") as i32;
    let (status, response) = app
        .delete(&format!("/api/timeline-events/{parent_id}"))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "parent delete: {response}");
    assert_error(&response, "validation.failed");
    let (status, _) = app
        .delete(&format!("/api/timeline-events/{child_id}"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn foreshadowing_anchor_validation_and_planned_status() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let (other_book, _) = book(&app).await;
    let foreign = create_event(&app, other_book, json!({ "title": "Foreign" })).await;
    let foreign_id = foreign["id"].as_i64().expect("id") as i32;
    let planned = create_event(
        &app,
        book_id,
        json!({ "title": "Planned payoff", "status": "planned" }),
    )
    .await;
    let planned_id = planned["id"].as_i64().expect("id") as i32;

    // Both anchor fields or neither; a foreign/abandoned entry is refused.
    for body in [
        json!({ "title": "Only position", "payoff_position": "after" }),
        json!({ "title": "Only event", "payoff_event_id": planned_id }),
        json!({ "title": "Foreign", "payoff_event_id": foreign_id, "payoff_position": "at" }),
    ] {
        let (status, response) = app
            .post(&format!("/api/books/{book_id}/foreshadowing"), body)
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
        assert_error(&response, "validation.failed");
    }

    let (status, hook) = app
        .post(
            &format!("/api/books/{book_id}/foreshadowing"),
            json!({
                "title": "Anchored hook",
                "payoff_event_id": planned_id,
                "payoff_position": "after",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");
    let hook_id = hook["id"].as_i64().expect("id") as i32;
    assert_eq!(hook["payoff_event_id"], planned_id);
    assert_eq!(hook["payoff_position"], "after");
    assert_eq!(hook["payoff_event_status"], "planned");

    let (status, resolved) = app
        .patch(
            &format!("/api/foreshadowing/{hook_id}"),
            json!({ "status": "resolved", "resolution": "The plan paid off." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{resolved}");
    assert_eq!(
        resolved["payoff_event_status"], "planned",
        "resolving against a planned anchor keeps the anchor and its status"
    );
}
