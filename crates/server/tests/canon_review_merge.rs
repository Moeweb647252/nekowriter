//! M5 review regressions: entity merge and the one-protagonist invariant (spec/06 §9,
//! spec/02 §5 (7), D-046).
//!
//! The merge is one transaction (spec/06 §9) that must never surface a database constraint as
//! `internal`: every edge the schema could reject is filtered first (the active-relation unique
//! indexes, the name index, the one-protagonist index). These tests reproduce the edges that do
//! hit a constraint today.

mod canon_fixture;

use axum::http::StatusCode;
use nekowriter::api::dto::canon::EntityInput;
use nekowriter::db::json::EntityKind;
use nekowriter::error::ErrorCode;
use nekowriter::services::canon::{self, CanonError, CanonWrite, Visibility, entity};
use nekowriter::testing::TestApp;
use serde_json::json;

use canon_fixture::{book, isolated_track};

/// Creates a shared entity through the API and returns its id.
async fn create(app: &TestApp, book_id: i32, name: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": name }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_i64().expect("entity id") as i32
}

/// Creates a private entity of an isolated track through the service and returns its id.
async fn create_private(app: &TestApp, book_id: i32, track_id: i32, name: &str) -> i32 {
    let write = CanonWrite::author().with_visibility(Visibility::isolated(book_id, track_id));
    let name = name.to_owned();
    let (dto, _) = canon::write(app.db(), move |tx| {
        Box::pin(async move {
            let input = EntityInput {
                kind: EntityKind::Character,
                name,
                ..EntityInput::default()
            };
            entity::create_inner(tx, &write, book_id, input).await
        })
    })
    .await
    .expect("private entity");
    dto.id
}

/// Creates a shared active relation through the API and returns its id.
async fn relate(app: &TestApp, book_id: i32, source: i32, target: i32, kind: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": source, "target_id": target, "kind": kind }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_i64().expect("relation id") as i32
}

/// Merging B into A moves B's relations to A; a moved relation that duplicates an active relation
/// **of A**, in either direction, is dropped rather than violating the unique index (spec/06 §9).
///
/// The second direction is the one the service misses: a relation `X → B` becomes `X → A`, and A
/// already has an active `X → A`. Only relations where A is the *source* are collected as taken,
/// so the moved row is kept and the unique index `relations_active_shared` aborts the merge with
/// an internal error.
#[tokio::test]
async fn merging_drops_a_moved_relation_that_duplicates_an_active_incoming_relation() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let x = create(&app, book_id, "Zhang San").await;
    let a = create(&app, book_id, "Lin Fan").await;
    let b = create(&app, book_id, "Lin Fann").await;
    let kept = relate(&app, book_id, x, a, "mentor").await;
    relate(&app, book_id, x, b, "mentor").await;

    let (status, body) = app
        .post(&format!("/api/entities/{a}/merge"), json!({ "from_id": b }))
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the duplicate moved relation is dropped instead of failing the merge: {body}"
    );

    let (status, relations) = app
        .get(&format!("/api/books/{book_id}/relations?entity_id={a}"))
        .await;
    assert_eq!(status, StatusCode::OK);
    let rows = relations.as_array().expect("relations");
    let same_kind: Vec<&serde_json::Value> = rows
        .iter()
        .filter(|row| row["source_id"] == json!(x) && row["kind"] == json!("mentor"))
        .collect();
    assert_eq!(
        same_kind.len(),
        1,
        "exactly one active X → A relation remains: {relations}"
    );
    assert_eq!(
        same_kind[0]["id"],
        json!(kept),
        "the relation A already had is the survivor: {relations}"
    );
}

/// The mirror case (A is the source) keeps working: `B → X` becoming `A → X` is dropped when
/// `A → X` exists (spec/06 §9). This is the direction the service already handles, kept here so
/// the two directions stay covered together.
#[tokio::test]
async fn merging_drops_a_moved_relation_that_duplicates_an_active_outgoing_relation() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let a = create(&app, book_id, "Lin Fan").await;
    let b = create(&app, book_id, "Lin Fann").await;
    let x = create(&app, book_id, "Zhang San").await;
    relate(&app, book_id, a, x, "mentor").await;
    relate(&app, book_id, b, x, "mentor").await;

    let (status, body) = app
        .post(&format!("/api/entities/{a}/merge"), json!({ "from_id": b }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, relations) = app
        .get(&format!("/api/books/{book_id}/relations?entity_id={a}"))
        .await;
    assert_eq!(status, StatusCode::OK);
    let rows = relations.as_array().expect("relations");
    assert_eq!(
        rows.iter()
            .filter(|row| row["kind"] == json!("mentor"))
            .count(),
        1,
        "one A → X relation remains: {relations}"
    );
}

/// A merge moves side tracks' `focus_entity_id` from B to A (spec/06 §9, D-046).
#[tokio::test]
async fn merging_moves_a_side_tracks_focus_entity() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let target = create(&app, book_id, "Lin Fan").await;
    let source = create(&app, book_id, "Lin Fann").await;
    let isolated = isolated_track(&app, book_id, "Side story").await;
    app.execute(&format!(
        "UPDATE tracks SET focus_entity_id = {source} WHERE id = {isolated}"
    ))
    .await;

    let (status, body) = app
        .post(
            &format!("/api/entities/{target}/merge"),
            json!({ "from_id": source }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    assert_eq!(status, StatusCode::OK);
    let side = tracks
        .as_array()
        .expect("tracks")
        .iter()
        .find(|track| track["id"] == json!(isolated))
        .expect("the side track");
    assert_eq!(
        side["focus_entity_id"],
        json!(target),
        "the focus entity moves with the merge: {side}"
    );
}

/// Private entities of two different isolated tracks belong to different canons and cannot be
/// merged: the moved rows would point at a row the other track cannot see (spec/06 §2, §9).
#[tokio::test]
async fn merging_two_isolated_tracks_private_entities_is_refused() {
    use nekowriter::services::canon::CanonError;

    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let first_track = isolated_track(&app, book_id, "Side one").await;
    let second_track = isolated_track(&app, book_id, "Side two").await;
    let target = create_private(&app, book_id, second_track, "Side Two Hero").await;
    let source = create_private(&app, book_id, first_track, "Side One Hero").await;

    let result = canon::write(app.db(), move |tx| {
        Box::pin(async move {
            entity::merge_inner(
                tx,
                target,
                nekowriter::api::dto::canon::EntityMergeInput { from_id: source },
            )
            .await
        })
    })
    .await;
    match result {
        Err(CanonError::App(error)) => assert_eq!(
            error.code,
            ErrorCode::ValidationFailed,
            "400 validation.failed: {error:?}"
        ),
        other => panic!("expected validation.failed, got {other:?}"),
    }

    // Both rows are still there: the refused merge changed nothing.
    for entity_id in [target, source] {
        let (status, _) = app.get(&format!("/api/entities/{entity_id}")).await;
        assert_eq!(status, StatusCode::OK, "entity #{entity_id} still exists");
    }
}

/// At most one entity per book has `is_protagonist = 1` (spec/02 §5 (7)); a second one is
/// `conflict.exists`, not a unique-index failure surfacing as `internal`. The service's check is
/// per track today, which lets a private entity pass it while the book already has a shared
/// protagonist.
#[tokio::test]
async fn a_second_protagonist_is_a_conflict_not_a_database_error() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let isolated = isolated_track(&app, book_id, "Side story").await;
    let shared = create(&app, book_id, "Lin Fan").await;
    let (status, body) = app
        .post(&format!("/api/entities/{shared}/protagonist"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let write = CanonWrite::author().with_visibility(Visibility::isolated(book_id, isolated));
    let result = canon::write(app.db(), move |tx| {
        Box::pin(async move {
            let input = EntityInput {
                kind: EntityKind::Character,
                name: "Side Protagonist".to_owned(),
                is_protagonist: true,
                ..EntityInput::default()
            };
            entity::create_inner(tx, &write, book_id, input).await
        })
    })
    .await;
    match result {
        Err(CanonError::App(error)) => assert_eq!(
            error.code,
            ErrorCode::ConflictExists,
            "a second protagonist is conflict.exists: {error:?}"
        ),
        other => panic!("expected conflict.exists, got {other:?}"),
    }

    // Nothing was inserted by the failed create (the track-filtered list adds the shared rows).
    let (_, list) = app
        .get(&format!(
            "/api/books/{book_id}/entities?track_id={isolated}"
        ))
        .await;
    assert!(
        !list
            .as_array()
            .expect("entities")
            .iter()
            .any(|row| row["name"] == json!("Side Protagonist")),
        "the rejected create inserted nothing: {list}"
    );
}
