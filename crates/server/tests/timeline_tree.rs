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

/// Creates an isolated side track through the API and returns its id.
async fn isolated_track(app: &TestApp, book_id: i32) -> i32 {
    let (status, track) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "title": "Side", "canon_mode": "isolated" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{track}");
    track["id"].as_i64().expect("track id") as i32
}

/// Inserts one private timeline row directly (the API writes shared canon only; a private row is
/// normally created by an isolated-track session).
async fn insert_private_event(
    app: &TestApp,
    book_id: i32,
    track_id: i32,
    title: &str,
    parent_id: i32,
    status: &str,
) -> i32 {
    let sql = format!(
        "INSERT INTO timeline_events(book_id, track_id, position, story_time, title, description, \
         chapter_id, parent_id, kind, status, importance, created_at, updated_at) VALUES \
         ({book_id}, {track_id}, 9000.0, '', '{title}', '', NULL, {parent_id}, 'entry', \
         '{status}', 2, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    );
    app.execute(&sql).await;
    scalar(app, "SELECT max(id) AS value FROM timeline_events").await as i32
}

/// A create with children cannot be reverted: the inverse delete is refused with
/// `409 timeline.has_children`, exactly like deleting the parent (D-170 T9, spec/10 §4).
#[tokio::test]
async fn reverting_a_create_with_children_is_refused() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let parent = create_event(&app, book_id, json!({ "title": "Parent" })).await;
    let parent_id = parent["id"].as_i64().expect("id") as i32;
    let _child = create_event(
        &app,
        book_id,
        json!({ "title": "Child", "parent_id": parent_id }),
    )
    .await;
    let create_change = scalar(
        &app,
        &format!(
            "SELECT id AS value FROM change_log WHERE target_type = 'timeline_event' \
             AND target_id = {parent_id} AND op = 'create' ORDER BY id LIMIT 1"
        ),
    )
    .await as i32;
    let (status, response) = app
        .post(&format!("/api/changes/{create_change}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{response}");
    assert_error(&response, "timeline.has_children");
    assert_eq!(response["error"]["details"]["children_count"], json!(1));
    assert_eq!(
        scalar(&app, "SELECT count(*) AS value FROM timeline_events").await,
        3,
        "the refused revert writes nothing (root + two entries)"
    );
}

/// T5/T7 count descendants only in the entry's own track scope (M22-06): a private planned child
/// under a shared occurred entry is legal, and a shared parent's descendant checks ignore private
/// rows of an isolated track.
#[tokio::test]
async fn descendant_checks_only_see_the_entrys_own_scope() {
    use nekowriter::services::canon::timeline_tree::{
        check_planned_ancestors, occurred_descendants, planned_descendants,
    };

    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let side = isolated_track(&app, book_id).await;

    // A shared occurred entry accepts a planned child from an isolated track's own scope.
    let shared_occurred = create_event(&app, book_id, json!({ "title": "Shared happened" })).await;
    let shared_occurred_id = shared_occurred["id"].as_i64().expect("id") as i32;
    check_planned_ancestors(app.db().conn(), shared_occurred_id, Some(side))
        .await
        .expect("a private planned child under a shared occurred entry is allowed");
    check_planned_ancestors(app.db().conn(), shared_occurred_id, None)
        .await
        .expect_err("a shared planned child would break T5");

    // A shared planned parent has one shared planned child and one private one; abandoning the
    // parent touches only its own scope.
    let parent = create_event(
        &app,
        book_id,
        json!({ "title": "Shared plan", "status": "planned" }),
    )
    .await;
    let parent_id = parent["id"].as_i64().expect("id") as i32;
    let shared_child = create_event(
        &app,
        book_id,
        json!({ "title": "Shared scene", "status": "planned", "parent_id": parent_id }),
    )
    .await;
    let shared_child_id = shared_child["id"].as_i64().expect("id") as i32;
    let private_child =
        insert_private_event(&app, book_id, side, "Private scene", parent_id, "planned").await;
    let planned = planned_descendants(app.db().conn(), parent_id)
        .await
        .expect("planned descendants");
    assert_eq!(
        planned.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![shared_child_id],
        "only the shared planned child is in the shared entry's scope"
    );
    assert!(planned.iter().all(|row| row.id != private_child));

    // The same scoping applies to T7's occurred check.
    let parent2 = create_event(
        &app,
        book_id,
        json!({ "title": "Shared plan 2", "status": "planned" }),
    )
    .await;
    let parent2_id = parent2["id"].as_i64().expect("id") as i32;
    let shared_occurred_child = create_event(
        &app,
        book_id,
        json!({ "title": "Shared happened child", "parent_id": parent2_id }),
    )
    .await;
    let shared_occurred_child_id = shared_occurred_child["id"].as_i64().expect("id") as i32;
    let private_occurred = insert_private_event(
        &app,
        book_id,
        side,
        "Private happened",
        parent2_id,
        "occurred",
    )
    .await;
    let occurred = occurred_descendants(app.db().conn(), parent2_id)
        .await
        .expect("occurred descendants");
    assert_eq!(
        occurred.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![shared_occurred_child_id]
    );
    assert!(occurred.iter().all(|row| row.id != private_occurred));
}

/// A shared foreshadowing cannot anchor a private timeline entry (M22-05): the private row is
/// outside shared canon, so the author's shared write is a validation error and stores nothing.
#[tokio::test]
async fn a_shared_foreshadowing_cannot_anchor_a_private_entry() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let side = isolated_track(&app, book_id).await;
    let root_id = scalar(
        &app,
        &format!(
            "SELECT id AS value FROM timeline_events WHERE book_id = {book_id} AND kind = 'root'"
        ),
    )
    .await as i32;
    let private =
        insert_private_event(&app, book_id, side, "Private payoff", root_id, "occurred").await;

    let before = scalar(&app, "SELECT count(*) AS value FROM foreshadowing").await;
    let (status, response) = app
        .post(
            &format!("/api/books/{book_id}/foreshadowing"),
            json!({
                "title": "Shared hook",
                "description": "A shared hook.",
                "payoff_event_id": private,
                "payoff_position": "after",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    assert_error(&response, "validation.failed");
    assert_eq!(
        scalar(&app, "SELECT count(*) AS value FROM foreshadowing").await,
        before,
        "the refused anchor stores nothing"
    );
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
    let planned = create_event(
        &app,
        book_id,
        json!({ "title": "Planned", "status": "planned" }),
    )
    .await;
    let planned_id = planned["id"].as_i64().expect("id") as i32;
    let (status, response) = patch_event(
        &app,
        planned_id,
        json!({ "status": "planned", "note": "test" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    assert_error(&response, "validation.failed");
    let (status, response) = patch_event(
        &app,
        planned_id,
        json!({ "status": "planned", "note": "test" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    assert_error(&response, "validation.failed");
    for body in [json!({ "status": "abandoned", "note": "test" })] {
        let (status, response) = patch_event(&app, occurred_id, body.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}: {response}");
        assert_error(&response, "timeline.status_conflict");
        assert_eq!(response["error"]["details"]["event_id"], occurred_id);
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
        json!({ "status": "occurred", "chapter_id": chapter_id, "note": "promote" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{response}");
    assert_error(&response, "timeline.status_conflict");
    assert_eq!(
        response["error"]["details"]["reason"],
        json!("planned_descendant")
    );
    let conflict = response["error"]["details"]["conflict"]
        .as_str()
        .unwrap_or_default();
    assert!(conflict.contains("Scene"), "names the conflict: {conflict}");

    // The descendant may happen first; then the phase can be promoted.
    let (status, response) = patch_event(
        &app,
        scene_id,
        json!({ "status": "occurred", "chapter_id": chapter_id, "note": "promote" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let (status, response) = patch_event(
        &app,
        plan_id,
        json!({ "status": "occurred", "chapter_id": chapter_id, "note": "promote" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["status"], "occurred");
}

#[tokio::test]
async fn abandoning_a_planned_parent_abandons_its_planned_subtree() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let parent = create_event(
        &app,
        book_id,
        json!({ "title": "Phase", "status": "planned" }),
    )
    .await;
    let parent_id = parent["id"].as_i64().expect("id") as i32;
    let child = create_event(
        &app,
        book_id,
        json!({ "title": "Scene", "status": "planned", "parent_id": parent_id }),
    )
    .await;
    let child_id = child["id"].as_i64().expect("id") as i32;
    let (status, response) = patch_event(
        &app,
        parent_id,
        json!({ "status": "abandoned", "note": "give up the phase" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let rows = timeline(&app, book_id).await;
    let status_of = |id: i32| {
        rows.iter()
            .find(|row| row["id"] == id)
            .and_then(|row| row["status"].as_str())
            .unwrap_or_default()
    };
    assert_eq!(status_of(parent_id), "abandoned");
    assert_eq!(status_of(child_id), "abandoned");
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
    let (status, response) = app.get(&format!("/api/timeline-events/{root_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "root get: {response}");
    assert_error(&response, "not_found");
    let (status, response) = patch_event(&app, root_id, json!({ "title": "Changed" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "root patch: {response}");
    assert_error(&response, "not_found");
    let (status, response) = app.delete(&format!("/api/timeline-events/{root_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "root delete: {response}");
    assert_error(&response, "not_found");

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
    assert_eq!(status, StatusCode::CONFLICT, "parent delete: {response}");
    assert_error(&response, "timeline.has_children");
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
    let (status, response) = app
        .patch(
            &format!("/api/foreshadowing/{hook_id}"),
            json!({ "payoff_event_id": null, "payoff_position": "after" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "half clear: {response}");
    assert_error(&response, "validation.failed");
    assert_eq!(hook["payoff_event_id"], planned_id);
    assert_eq!(hook["payoff_position"], "after");
    assert_eq!(hook["payoff_event_status"], "planned");

    // Clearing the event alone clears both anchor fields.
    let (status, cleared) = app
        .patch(
            &format!("/api/foreshadowing/{hook_id}"),
            json!({ "payoff_event_id": null }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["payoff_event_id"], serde_json::Value::Null);
    assert_eq!(cleared["payoff_position"], serde_json::Value::Null);

    // Deleting an anchored leaf clears its anchors and succeeds.
    let leaf = create_event(&app, book_id, json!({ "title": "Anchored leaf" })).await;
    let leaf_id = leaf["id"].as_i64().expect("id") as i32;
    let (status, anchored) = app
        .post(
            &format!("/api/books/{book_id}/foreshadowing"),
            json!({
                "title": "Anchor to delete",
                "payoff_event_id": leaf_id,
                "payoff_position": "at",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{anchored}");
    let anchored_id = anchored["id"].as_i64().expect("id") as i32;
    let (status, response) = app.delete(&format!("/api/timeline-events/{leaf_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{response}");
    let (_, after) = app.get(&format!("/api/foreshadowing/{anchored_id}")).await;
    assert_eq!(after["payoff_event_id"], serde_json::Value::Null);
    assert_eq!(after["payoff_position"], serde_json::Value::Null);

    let (_, hook2) = app
        .post(
            &format!("/api/books/{book_id}/foreshadowing"),
            json!({
                "title": "Still planned",
                "payoff_event_id": planned_id,
                "payoff_position": "after",
            }),
        )
        .await;
    let hook2_id = hook2["id"].as_i64().expect("id") as i32;
    let (status, resolved) = app
        .patch(
            &format!("/api/foreshadowing/{hook2_id}"),
            json!({ "status": "resolved", "resolution": "The plan paid off." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{resolved}");
    assert_eq!(
        resolved["payoff_event_status"], "planned",
        "resolving against a planned anchor keeps the anchor and its status"
    );
}
