//! Canon service tests (PLAN M5.1, spec/06 §1–§3, §9; spec/02 §4–§5).
//!
//! These exercise the rules the milestone is judged by:
//!
//! - the change log: every real write records one row in the same transaction with the full typed
//!   snapshot before/after, and a no-op update records nothing (spec/06 §3, D-032);
//! - revert: create/update/delete all work, only the newest non-reverted change of a target may
//!   be reverted (`change.not_latest`), and a reverted delete recreates with a new id without
//!   restoring relations (spec/02 §5.9–§5.10);
//! - visibility (D-014): an isolated track creates private rows and cannot modify a shared one;
//! - appearance bounds (D-059): recomputed on appearance changes, merges and chapter deletes, and
//!   an isolated chapter never moves a shared entity's bounds;
//! - merge (spec/06 §9): both entities in one chapter, a relation between them, private into
//!   shared, and shared into private refused.

use axum::http::StatusCode;
use nekowriter::db::entities::{appearances, change_log, entities, relations};
use nekowriter::db::json::{AppearanceRole, ChangeOp, ChangeTargetType, EntityKind, EntityStatus};
use nekowriter::services::canon::{self, CanonError, CanonWrite, Visibility, appearance, entity};
use nekowriter::testing::{TestApp, assert_error, assert_error_detail};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::json;

mod canon_fixture;

use canon_fixture::{book, isolated_track};

// ───────────────────────────── fixtures ─────────────────────────────

/// Creates an entity through the service (as the author) and returns its DTO.
async fn create_entity(app: &TestApp, book_id: i32, kind: &str, name: &str) -> serde_json::Value {
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({
                "kind": kind,
                "name": name,
                "summary": format!("{name} summary"),
                "description": format!("{name} description"),
                "state": "somewhere",
                "attributes": { "age": "17" },
                "importance": 5,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create entity {name}: {body}");
    body
}

fn id_of(value: &serde_json::Value) -> i32 {
    value["id"].as_i64().expect("id") as i32
}

// ───────────────────────────── the change log ─────────────────────────────

/// Every canon write records one row in the same transaction, with the full typed snapshot
/// before/after (spec/06 §3, spec/02 §4).
#[tokio::test]
async fn every_entity_write_records_a_typed_change() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let entity = create_entity(&app, book_id, "character", "Lin Fan").await;
    let entity_id = id_of(&entity);

    // create: no before, full after.
    let created = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    assert_eq!(created.len(), 1, "one create change");
    assert!(created[0].before.is_none());
    let after = snapshot_after(&created[0]);
    assert_eq!(
        after,
        json!({
            "type": "entity",
            "kind": "character",
            "name": "Lin Fan",
            "aliases": [],
            "summary": "Lin Fan summary",
            "description": "Lin Fan description",
            "state": "somewhere",
            "attributes": { "age": "17" },
            "importance": 5,
            "status": "active",
            "is_protagonist": false,
            "track_id": null,
        }),
        "the after snapshot carries every domain field"
    );

    // update: before and after, with the author's note.
    let (status, body) = app
        .patch(
            &format!("/api/entities/{entity_id}"),
            json!({ "state": "in the sword tomb", "importance": 4, "note": "moved on" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let history = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    assert_eq!(history.len(), 2, "the update appended one change");
    let update = &history[0];
    assert_eq!(update.op, ChangeOp::Update);
    assert_eq!(update.note, "moved on");
    assert_eq!(
        snapshot_before(update)["state"],
        "somewhere",
        "the before snapshot is the state before the change"
    );
    assert_eq!(snapshot_after(update)["state"], "in the sword tomb");

    // A patch that stores the same value writes no row (spec/06 §3 "real change").
    let (status, _) = app
        .patch(
            &format!("/api/entities/{entity_id}"),
            json!({ "state": "in the sword tomb" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let unchanged = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    assert_eq!(unchanged.len(), 2, "a no-op save writes no change row");

    // delete: full before, no after (spec/02 §5.10).
    let (status, _) = app.delete(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let deleted = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    assert_eq!(deleted.len(), 3);
    assert_eq!(deleted[0].op, ChangeOp::Delete);
    assert_eq!(snapshot_before(&deleted[0])["name"], "Lin Fan");
    assert!(deleted[0].after.is_none());
}

/// Every canon target records its change rows with the matching target type (spec/06 §3).
#[tokio::test]
async fn every_canon_target_records_its_change() {
    let app = TestApp::new().await;
    let (book_id, track_id) = book(&app).await;
    let chapter = app.create_chapter(track_id, "One").await;
    let chapter_id = id_of(&chapter);
    let first = create_entity(&app, book_id, "character", "Lin Fan").await;
    let second = create_entity(&app, book_id, "character", "Zhang San").await;

    // relation
    let (status, relation) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({
                "source_id": id_of(&first),
                "target_id": id_of(&second),
                "kind": "senior brother",
                "inverse_kind": "junior brother",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{relation}");
    let relation_id = id_of(&relation);
    let relation_changes = change_log_of(&app, ChangeTargetType::Relation, relation_id).await;
    assert_eq!(relation_changes.len(), 1);
    assert_eq!(
        snapshot_after(&relation_changes[0])["source_id"],
        id_of(&first)
    );

    // appearance set
    let (status, body) = app
        .put(
            &format!("/api/chapters/{chapter_id}/appearances"),
            json!([{ "entity_id": id_of(&first), "role": "pov", "note": "" }]),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let appearance_changes = change_log_of(&app, ChangeTargetType::AppearanceSet, chapter_id).await;
    assert_eq!(appearance_changes.len(), 1);
    assert_eq!(
        snapshot_after(&appearance_changes[0])["appearances"][0]["role"],
        "pov"
    );

    // timeline event
    let (status, event) = app
        .post(
            &format!("/api/books/{book_id}/timeline"),
            json!({ "title": "The gate", "entity_ids": [id_of(&first)] }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{event}");
    let event_id = id_of(&event);
    let event_changes = change_log_of(&app, ChangeTargetType::TimelineEvent, event_id).await;
    assert_eq!(event_changes.len(), 1);
    assert_eq!(
        snapshot_after(&event_changes[0])["entity_ids"],
        json!([id_of(&first)])
    );

    // memory note
    let (status, note) = app
        .post(
            &format!("/api/books/{book_id}/memory-notes"),
            json!({ "kind": "fact", "content": "The sword was a gift.", "importance": 4 }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{note}");
    let note_id = id_of(&note);
    let note_changes = change_log_of(&app, ChangeTargetType::MemoryNote, note_id).await;
    assert_eq!(note_changes.len(), 1);
    assert_eq!(
        snapshot_after(&note_changes[0])["content"],
        "The sword was a gift."
    );

    // foreshadowing
    let (status, foreshadow) = app
        .post(
            &format!("/api/books/{book_id}/foreshadowing"),
            json!({ "title": "The broken hilt", "description": "A hilt with no blade." }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{foreshadow}");
    let foreshadow_id = id_of(&foreshadow);
    let foreshadow_changes =
        change_log_of(&app, ChangeTargetType::Foreshadowing, foreshadow_id).await;
    assert_eq!(foreshadow_changes.len(), 1);
    assert_eq!(
        snapshot_after(&foreshadow_changes[0])["title"],
        "The broken hilt"
    );
}

/// `change_log` rows are immutable except `reverted_by_id` (spec/02 §5.9): the only UPDATE the
/// service ever performs on them is that one column.
#[tokio::test]
async fn a_change_row_is_never_rewritten_except_reverted_by() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let entity = create_entity(&app, book_id, "character", "Lin Fan").await;
    let entity_id = id_of(&entity);
    let created = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    let change_id = created[0].id;

    // Revert the create: the original is marked, nothing else about it changes.
    let (status, revert_change) = app
        .post(&format!("/api/changes/{change_id}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{revert_change}");
    assert_eq!(revert_change["reverts_id"], change_id);

    use sea_orm::ConnectionTrait;
    let row = app
        .db()
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            format!(
                "SELECT op, note, before IS NULL AS before_null, after IS NOT NULL AS after_present \
                 FROM change_log WHERE id = {change_id}"
            ),
        ))
        .await
        .expect("query")
        .expect("row");
    assert_eq!(row.try_get::<String>("", "op").expect("op"), "create");
    assert_eq!(row.try_get::<String>("", "note").expect("note"), "");
    assert!(row.try_get::<bool>("", "after_present").expect("after"));
}

// ───────────────────────────── revert ─────────────────────────────

/// Reverting an `update` restores the `before` snapshot (spec/06 §3).
#[tokio::test]
async fn revert_of_an_update_restores_the_before_snapshot() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let entity = create_entity(&app, book_id, "character", "Lin Fan").await;
    let entity_id = id_of(&entity);
    let (status, _) = app
        .patch(
            &format!("/api/entities/{entity_id}"),
            json!({ "state": "wounded", "note": "the duel" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let history = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    let update_id = history[0].id;

    let (status, body) = app
        .post(&format!("/api/changes/{update_id}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["op"], "update");
    assert_eq!(body["reverts_id"], update_id);

    let (status, entity) = app.get(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(entity["state"], "somewhere", "the before state is restored");
}

/// Reverting a `create` deletes the target (spec/06 §3).
#[tokio::test]
async fn revert_of_a_create_deletes_the_target() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let entity = create_entity(&app, book_id, "character", "Lin Fan").await;
    let entity_id = id_of(&entity);
    let created = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    let create_id = created[0].id;

    let (status, body) = app
        .post(&format!("/api/changes/{create_id}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["op"], "delete");

    let (status, _) = app.get(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the created entity is gone");
}

/// Reverting a `delete` recreates the entity with a **new id**, and does not restore its
/// relations or appearances (spec/02 §5.10).
#[tokio::test]
async fn revert_of_a_delete_recreates_with_a_new_id_and_no_relations() {
    let app = TestApp::new().await;
    let (book_id, track_id) = book(&app).await;
    let chapter = app.create_chapter(track_id, "One").await;
    let chapter_id = id_of(&chapter);
    let first = create_entity(&app, book_id, "character", "Lin Fan").await;
    let first_id = id_of(&first);
    let second = create_entity(&app, book_id, "character", "Zhang San").await;
    let second_id = id_of(&second);
    let (status, relation) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({
                "source_id": first_id,
                "target_id": second_id,
                "kind": "senior brother",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{relation}");
    let (status, _) = app
        .put(
            &format!("/api/chapters/{chapter_id}/appearances"),
            json!([{ "entity_id": first_id, "role": "pov", "note": "" }]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app.delete(&format!("/api/entities/{first_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let deleted = change_log_of(&app, ChangeTargetType::Entity, first_id).await;
    let delete_id = deleted[0].id;

    let (status, revert_change) = app
        .post(&format!("/api/changes/{delete_id}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{revert_change}");
    assert_eq!(revert_change["op"], "create");
    let new_id = revert_change["target_id"].as_i64().expect("new id") as i32;
    assert_ne!(new_id, first_id, "the recreated entity has a NEW id");

    let (status, recreated) = app.get(&format!("/api/entities/{new_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(recreated["name"], "Lin Fan");
    assert_eq!(
        recreated["relations"].as_array().expect("relations").len(),
        0,
        "the relations of a deleted entity are not restored (spec/02 §5.10)"
    );
    assert_eq!(
        recreated["appearances"]
            .as_array()
            .expect("appearances")
            .len(),
        0,
        "its appearances cascade with the delete and are not restored"
    );
}

/// Only the newest non-reverted change of a target may be reverted (`change.not_latest`,
/// spec/06 §3.1).
#[tokio::test]
async fn only_the_newest_change_of_a_target_can_be_reverted() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let entity = create_entity(&app, book_id, "character", "Lin Fan").await;
    let entity_id = id_of(&entity);
    for state in ["wounded", "healed"] {
        let (status, _) = app
            .patch(
                &format!("/api/entities/{entity_id}"),
                json!({ "state": state }),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
    }
    let history = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    // Newest first: [healed, wounded, create].
    let older_update = &history[1];
    let newer_update = &history[0];
    let (status, body) = app
        .post(
            &format!("/api/changes/{}/revert", older_update.id),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&body, "change.not_latest");
    assert_error_detail(&body, "change.not_latest", "later_change_ids");
    let later: Vec<i64> = body["error"]["details"]["later_change_ids"]
        .as_array()
        .expect("later ids")
        .iter()
        .map(|value| value.as_i64().expect("id"))
        .collect();
    assert!(
        later.contains(&(newer_update.id as i64)),
        "the later change ids are linked: {later:?}"
    );

    // Reverting the newest one succeeds, and then the older one is blocked by it.
    let (status, _) = app
        .post(
            &format!("/api/changes/{}/revert", newer_update.id),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

/// A change can be reverted only once (spec/06 §3.1 "newest non-reverted").
#[tokio::test]
async fn a_reverted_change_is_no_longer_revertible() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let entity = create_entity(&app, book_id, "character", "Lin Fan").await;
    let entity_id = id_of(&entity);
    let created = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    let change_id = created[0].id;
    let (status, _) = app
        .post(&format!("/api/changes/{change_id}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = app
        .post(&format!("/api/changes/{change_id}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&body, "change.not_latest");
}

/// A revert is itself a change pointing at the original, and the original is marked
/// (`reverts_id` / `reverted_by_id`, spec/06 §3.3).
#[tokio::test]
async fn a_revert_links_the_original_change() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let entity = create_entity(&app, book_id, "character", "Lin Fan").await;
    let entity_id = id_of(&entity);
    let (status, _) = app
        .patch(
            &format!("/api/entities/{entity_id}"),
            json!({ "state": "changed" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let history = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    let original_id = history[0].id;
    let (status, revert_change) = app
        .post(&format!("/api/changes/{original_id}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let revert_id = revert_change["id"].as_i64().expect("revert id") as i32;

    let after = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    let original = after
        .iter()
        .find(|row| row.id == original_id)
        .expect("original");
    assert_eq!(
        original.reverted_by_id,
        Some(revert_id),
        "the original names the revert that superseded it"
    );
    assert_eq!(
        after.len(),
        history.len() + 1,
        "a revert writes exactly one row: the inverse operation's row IS the revert, so the \
         history gains one entry rather than the operation plus its revert: {}",
        after.len()
    );
}

/// A revert of a create writes one row, not the delete plus a revert row (D-079).
#[tokio::test]
async fn a_revert_of_a_create_writes_no_second_change_row() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let entity = create_entity(&app, book_id, "item", "Old Letter").await;
    let entity_id = id_of(&entity);
    let history = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    assert_eq!(history.len(), 1, "the create is the only change");
    let (status, body) = app
        .post(&format!("/api/changes/{}/revert", history[0].id), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let after = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    assert_eq!(
        after.len(),
        2,
        "the delete that the revert performed is the revert row itself: {:?}",
        after.iter().map(|row| (row.id, row.op)).collect::<Vec<_>>()
    );
}

/// A revert of a delete writes one row too, and it names the new id (D-079, spec/02 §5.10).
#[tokio::test]
async fn a_revert_of_a_delete_writes_no_second_change_row() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    // A second row keeps the id allocator from reusing the deleted one, so "new id" is testable.
    create_entity(&app, book_id, "character", "Zhang San").await;
    let entity = create_entity(&app, book_id, "character", "Lin Fan").await;
    let entity_id = id_of(&entity);
    let (status, _) = app.delete(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let history = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    let delete_id = history[0].id;
    let (status, body) = app
        .post(&format!("/api/changes/{delete_id}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let new_id = body["target_id"].as_i64().expect("new id") as i32;
    assert_ne!(new_id, entity_id, "the recreated entity has a new id");

    let recreated = change_log_of(&app, ChangeTargetType::Entity, new_id).await;
    assert_eq!(
        recreated.len(),
        1,
        "the recreate is one change row: {:?}",
        recreated.iter().map(|row| row.id).collect::<Vec<_>>()
    );
    assert_eq!(recreated[0].op, ChangeOp::Create);
    assert_eq!(recreated[0].reverts_id, Some(delete_id));
}

/// Reverted changes do not reach mid-term memory (spec/06 §3): the memory snapshot drops them.
#[tokio::test]
async fn reverted_changes_do_not_reach_mid_term_memory() {
    let app = TestApp::new().await;
    let (book_id, track_id) = book(&app).await;
    let chapter = app.create_chapter(track_id, "One").await;
    let chapter_id = id_of(&chapter);
    app.put_text(chapter_id, 0, "Text.").await;
    app.set_chapter_status(chapter_id, "official").await;
    let second = app.create_chapter(track_id, "Two").await;
    let second_id = id_of(&second);
    app.put_text(second_id, 0, "More text.").await;
    app.set_chapter_status(second_id, "official").await;

    let entity = create_entity(&app, book_id, "character", "Lin Fan").await;
    let entity_id = id_of(&entity);
    // An agent-style change tied to the second chapter with a note.
    let (status, _) = app
        .patch(
            &format!("/api/entities/{entity_id}"),
            json!({ "state": "wounded", "note": "the duel" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    // Tie the change to the official chapter through the service (the API edit has no chapter).
    app.execute(&format!(
        "UPDATE change_log SET chapter_id = {chapter_id} WHERE target_type = 'entity' AND \
         target_id = {entity_id} AND op = 'update'"
    ))
    .await;

    // The change belongs to the first chapter, so the Memory page for the *second* chapter has
    // it in its mid-term window W (spec/06 §4.1, §4.3).
    let (status, memory) = app
        .get(&format!(
            "/api/books/{book_id}/memory?chapter_id={second_id}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{memory}");
    assert!(
        memory["rendered"]
            .as_str()
            .expect("rendered")
            .contains("the duel"),
        "the change note is in mid-term memory:\n{memory}"
    );

    let history = change_log_of(&app, ChangeTargetType::Entity, entity_id).await;
    let update_id = history[0].id;
    let (status, _) = app
        .post(&format!("/api/changes/{update_id}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, memory) = app
        .get(&format!(
            "/api/books/{book_id}/memory?chapter_id={second_id}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !memory["rendered"]
            .as_str()
            .expect("rendered")
            .contains("the duel"),
        "a reverted change is not in mid-term memory:\n{memory}"
    );
}

// ───────────────────────────── visibility (D-014) ─────────────────────────────

/// An isolated track can read shared canon but cannot modify a shared row: the service answers
/// `error.isolated_readonly` (spec/06 §2, D-014, D-036).
#[tokio::test]
async fn an_isolated_track_cannot_modify_a_shared_entity() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let isolated = isolated_track(&app, book_id, "Side story").await;
    let shared = create_entity(&app, book_id, "character", "Lin Fan").await;
    let shared_id = id_of(&shared);

    let visibility = Visibility::isolated(book_id, isolated);
    let write = CanonWrite::author().with_visibility(visibility);
    let error = canon::write(app.db(), move |tx| {
        Box::pin(async move {
            let patch = nekowriter::api::dto::canon::EntityPatch {
                state: Some("modified".to_owned()),
                ..Default::default()
            };
            entity::update_inner(tx, &write, shared_id, patch).await
        })
    })
    .await
    .expect_err("a shared row is read-only for an isolated track");
    match error {
        CanonError::IsolatedReadonly { target } => {
            assert!(
                target.contains("Lin Fan"),
                "the error names the row: {target}"
            );
        }
        other => panic!("expected isolated_readonly, got {other:?}"),
    }

    // The shared row still reads for the isolated visibility.
    let visible = Visibility::isolated(book_id, isolated);
    let rows = canon::visible_entities(app.db().conn(), visible)
        .await
        .expect("visible entities");
    assert!(
        rows.iter().any(|row| row.id == shared_id),
        "shared canon is readable from an isolated track"
    );

    // But the API still shows the shared row untouched.
    let (status, body) = app.get(&format!("/api/entities/{shared_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "somewhere");
}

/// An isolated track creates private rows (`track_id = T`), and its own rows are writable
/// (spec/06 §2).
#[tokio::test]
async fn an_isolated_track_creates_and_updates_its_own_rows() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let isolated = isolated_track(&app, book_id, "Side story").await;

    let write = CanonWrite::author().with_visibility(Visibility::isolated(book_id, isolated));
    let (created, _) = canon::write(app.db(), move |tx| {
        Box::pin(async move {
            let input = nekowriter::api::dto::canon::EntityInput {
                kind: EntityKind::Character,
                name: "Side Character".to_owned(),
                summary: "only here".to_owned(),
                ..Default::default()
            };
            entity::create_inner(tx, &write, book_id, input).await
        })
    })
    .await
    .expect("create a private entity");
    let private_id = created.id;
    assert_eq!(created.track_id, Some(isolated), "the row is private");

    // The private row is writable from its own track …
    let visibility = Visibility::isolated(book_id, isolated);
    let (updated, _) = canon::write(app.db(), move |tx| {
        Box::pin(async move {
            let write = CanonWrite::author().with_visibility(visibility);
            let patch = nekowriter::api::dto::canon::EntityPatch {
                state: Some("here".to_owned()),
                ..Default::default()
            };
            entity::update_inner(tx, &write, private_id, patch).await
        })
    })
    .await
    .expect("its own row is writable");
    assert_eq!(updated.state, "here");

    // … and not part of shared canon's list (the author's own API can still read it by id; the
    // *canon pages* filter to shared rows by default, spec/06 §2).
    let (status, _) = app.get(&format!("/api/entities/{private_id}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the author may read every row of the book"
    );
    let list = app.get(&format!("/api/books/{book_id}/entities")).await.1;
    assert_eq!(
        list.as_array().expect("entities").len(),
        0,
        "shared canon does not list a private row"
    );
    let list = app
        .get(&format!(
            "/api/books/{book_id}/entities?track_id={isolated}"
        ))
        .await
        .1;
    assert_eq!(
        list.as_array().expect("entities").len(),
        1,
        "the track filter adds the private rows"
    );
}

/// The visibility helper builds `track_id IS NULL OR track_id = T` (spec/06 §2).
#[tokio::test]
async fn visible_condition_covers_shared_and_own_rows() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let isolated = isolated_track(&app, book_id, "Side story").await;
    let shared = create_entity(&app, book_id, "character", "Shared").await;
    let write = CanonWrite::author().with_visibility(Visibility::isolated(book_id, isolated));
    let private = canon::write(app.db(), move |tx| {
        Box::pin(async move {
            let input = nekowriter::api::dto::canon::EntityInput {
                kind: EntityKind::Item,
                name: "Private".to_owned(),
                ..Default::default()
            };
            entity::create_inner(tx, &write, book_id, input).await
        })
    })
    .await
    .expect("private entity")
    .0;

    let isolated_visibility = Visibility::isolated(book_id, isolated);
    let rows = canon::visible_entities(app.db().conn(), isolated_visibility)
        .await
        .expect("rows");
    let ids: Vec<i32> = rows.iter().map(|row| row.id).collect();
    assert!(ids.contains(&id_of(&shared)), "shared rows are visible");
    assert!(ids.contains(&private.id), "own rows are visible");

    let other = isolated_track(&app, book_id, "Another side story").await;
    let other_visibility = Visibility::isolated(book_id, other);
    let rows = canon::visible_entities(app.db().conn(), other_visibility)
        .await
        .expect("rows");
    let ids: Vec<i32> = rows.iter().map(|row| row.id).collect();
    assert!(ids.contains(&id_of(&shared)));
    assert!(
        !ids.contains(&private.id),
        "another isolated track's rows are invisible"
    );
}

// ───────────────────────────── appearance bounds (D-059) ─────────────────────────────

/// `first_chapter_id` / `last_chapter_id` follow the appearance set, ordered by (track ord,
/// chapter ord) (spec/06 §1.1, D-059).
#[tokio::test]
async fn appearance_bounds_follow_the_chapter_order() {
    let app = TestApp::new().await;
    let (book_id, track_id) = book(&app).await;
    let first = app.create_chapter(track_id, "One").await;
    let second = app.create_chapter(track_id, "Two").await;
    let entity = create_entity(&app, book_id, "character", "Lin Fan").await;
    let entity_id = id_of(&entity);

    let (status, _) = app
        .put(
            &format!("/api/chapters/{}/appearances", id_of(&second)),
            json!([{ "entity_id": entity_id, "role": "major", "note": "" }]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = app.get(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(body["first_chapter_id"], id_of(&second));
    assert_eq!(body["last_chapter_id"], id_of(&second));

    let (status, _) = app
        .put(
            &format!("/api/chapters/{}/appearances", id_of(&first)),
            json!([{ "entity_id": entity_id, "role": "pov", "note": "" }]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = app.get(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(body["first_chapter_id"], id_of(&first));
    assert_eq!(body["last_chapter_id"], id_of(&second));

    // Clearing the earlier chapter's set moves `first` forward again.
    let (status, _) = app
        .put(
            &format!("/api/chapters/{}/appearances", id_of(&first)),
            json!([]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = app.get(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(
        body["first_chapter_id"],
        id_of(&second),
        "a chapter that lost the appearance no longer bounds the entity"
    );

    // An entity that appeared nowhere has no bounds.
    let (status, _) = app
        .put(
            &format!("/api/chapters/{}/appearances", id_of(&second)),
            json!([]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = app.get(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(body["first_chapter_id"], serde_json::Value::Null);
    assert_eq!(body["last_chapter_id"], serde_json::Value::Null);
}

/// An isolated chapter never changes a shared entity's bounds (D-036, D-059).
#[tokio::test]
async fn an_isolated_chapter_does_not_move_a_shared_entitys_bounds() {
    let app = TestApp::new().await;
    let (book_id, track_id) = book(&app).await;
    let chapter = app.create_chapter(track_id, "One").await;
    let isolated = isolated_track(&app, book_id, "Side story").await;
    let isolated_chapter = app.create_chapter(isolated, "Side one").await;
    let entity = create_entity(&app, book_id, "character", "Lin Fan").await;
    let entity_id = id_of(&entity);
    let visibility = Visibility::isolated(book_id, isolated);
    let write = CanonWrite::author().with_visibility(visibility);

    // The isolated track records the shared entity appearing in its own chapter.
    canon::write(app.db(), move |tx| {
        Box::pin(async move {
            let inputs = vec![nekowriter::api::dto::canon::AppearanceInput {
                entity_id,
                role: AppearanceRole::Major,
                note: String::new(),
            }];
            appearance::record_set_inner(tx, &write, id_of(&isolated_chapter), &inputs).await
        })
    })
    .await
    .expect("record the isolated appearance");

    let (_, body) = app.get(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(
        body["first_chapter_id"],
        serde_json::Value::Null,
        "an isolated chapter never bounds a shared entity"
    );
    assert_eq!(body["last_chapter_id"], serde_json::Value::Null);

    // The shared chapter's appearance does move them.
    let (status, _) = app
        .put(
            &format!("/api/chapters/{}/appearances", id_of(&chapter)),
            json!([{ "entity_id": entity_id, "role": "pov", "note": "" }]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = app.get(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(body["first_chapter_id"], id_of(&chapter));
}

/// Deleting a chapter recomputes the bounds of the entities that appeared in it (spec/08 §1.1,
/// D-059).
#[tokio::test]
async fn deleting_a_chapter_recomputes_the_bounds() {
    let app = TestApp::new().await;
    let (book_id, track_id) = book(&app).await;
    let first = app.create_chapter(track_id, "One").await;
    let second = app.create_chapter(track_id, "Two").await;
    let entity = create_entity(&app, book_id, "character", "Lin Fan").await;
    let entity_id = id_of(&entity);
    for chapter in [&first, &second] {
        let (status, _) = app
            .put(
                &format!("/api/chapters/{}/appearances", id_of(chapter)),
                json!([{ "entity_id": entity_id, "role": "major", "note": "" }]),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
    }
    let (_, body) = app.get(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(body["first_chapter_id"], id_of(&first));
    assert_eq!(body["last_chapter_id"], id_of(&second));

    let (status, _) = app
        .delete(&format!("/api/chapters/{}", id_of(&first)))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, body) = app.get(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(
        body["first_chapter_id"],
        id_of(&second),
        "the deleted chapter no longer bounds the entity"
    );
    assert_eq!(body["last_chapter_id"], id_of(&second));
}

/// A reverted appearance-set change also recomputes the bounds (D-059).
#[tokio::test]
async fn a_reverted_appearance_set_recomputes_the_bounds() {
    let app = TestApp::new().await;
    let (book_id, track_id) = book(&app).await;
    let first = app.create_chapter(track_id, "One").await;
    let second = app.create_chapter(track_id, "Two").await;
    let entity = create_entity(&app, book_id, "character", "Lin Fan").await;
    let entity_id = id_of(&entity);
    let (status, _) = app
        .put(
            &format!("/api/chapters/{}/appearances", id_of(&second)),
            json!([{ "entity_id": entity_id, "role": "major", "note": "" }]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app
        .put(
            &format!("/api/chapters/{}/appearances", id_of(&first)),
            json!([{ "entity_id": entity_id, "role": "pov", "note": "" }]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = app.get(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(body["first_chapter_id"], id_of(&first));

    // Revert `first`'s appearance-set change: the entity appeared there only as a result of it.
    let changes = change_log_of(&app, ChangeTargetType::AppearanceSet, id_of(&first)).await;
    let change_id = changes[0].id;
    let (status, body) = app
        .post(&format!("/api/changes/{change_id}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, body) = app.get(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(
        body["first_chapter_id"],
        id_of(&second),
        "the reverted appearance no longer bounds the entity"
    );
}

// ───────────────────────────── merge (spec/06 §9) ─────────────────────────────

/// A merge moves B's appearances (A keeps the more prominent role where both appear), unions the
/// aliases, and records the two changes with the language-neutral note `#B → #A`.
#[tokio::test]
async fn merging_two_entities_in_one_chapter_keeps_the_more_prominent_role() {
    let app = TestApp::new().await;
    let (book_id, track_id) = book(&app).await;
    let chapter = app.create_chapter(track_id, "One").await;
    let target = create_entity(&app, book_id, "character", "Lin Fan").await;
    let source = create_entity(&app, book_id, "character", "Lin Fann").await;
    let target_id = id_of(&target);
    let source_id = id_of(&source);
    let (status, _) = app
        .put(
            &format!("/api/chapters/{}/appearances", id_of(&chapter)),
            json!([
                { "entity_id": target_id, "role": "minor", "note": "" },
                { "entity_id": source_id, "role": "pov", "note": "" },
            ]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, merged) = app
        .post(
            &format!("/api/entities/{target_id}/merge"),
            json!({ "from_id": source_id }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{merged}");
    let role: Vec<String> = merged["appearances"]
        .as_array()
        .expect("appearances")
        .iter()
        .map(|row| row["role"].as_str().expect("role").to_owned())
        .collect();
    assert_eq!(role, vec!["pov"], "A keeps the more prominent role");
    assert!(
        merged["aliases"]
            .as_array()
            .expect("aliases")
            .iter()
            .any(|alias| alias == "Lin Fann"),
        "B's name becomes an alias of A: {merged}"
    );

    // The two changes share the neutral merge note.
    let changes = change_log_of(&app, ChangeTargetType::Entity, target_id).await;
    assert!(
        changes.iter().any(|row| row.note == "#Lin Fann → #Lin Fan"),
        "the update change carries the merge note: {changes:?}"
    );
    let deleted = change_log_of(&app, ChangeTargetType::Entity, source_id).await;
    assert_eq!(deleted[0].op, ChangeOp::Delete);
    assert_eq!(deleted[0].note, "#Lin Fann → #Lin Fan");

    // B is gone, and A's bounds still point at the chapter both appeared in.
    let (status, _) = app.get(&format!("/api/entities/{source_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(merged["first_chapter_id"], id_of(&chapter));
}

/// Relations that would become self-relations are dropped; the rest move to A (spec/06 §9).
#[tokio::test]
async fn merging_drops_the_relation_between_the_two_entities() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let target = create_entity(&app, book_id, "character", "Lin Fan").await;
    let target_id = id_of(&target);
    let source = create_entity(&app, book_id, "character", "Lin Fann").await;
    let source_id = id_of(&source);
    let third = create_entity(&app, book_id, "character", "Zhang San").await;
    let third_id = id_of(&third);
    let (status, between) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({
                "source_id": target_id,
                "target_id": source_id,
                "kind": "senior brother",
                "inverse_kind": "junior brother",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let between_id = id_of(&between);
    let (status, keep) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({
                "source_id": source_id,
                "target_id": third_id,
                "kind": "ally",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let keep_id = id_of(&keep);

    let (status, merged) = app
        .post(
            &format!("/api/entities/{target_id}/merge"),
            json!({ "from_id": source_id }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{merged}");

    let (_, _) = app.get(&format!("/api/relations/{between_id}")).await;
    let (status, _) = app.get(&format!("/api/relations/{between_id}")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the relation between A and B is dropped"
    );
    let (status, moved) = app.get(&format!("/api/relations/{keep_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        moved["source_id"], target_id,
        "B's other relation moved to A"
    );
    let (_, relations) = app
        .get(&format!(
            "/api/books/{book_id}/relations?entity_id={target_id}"
        ))
        .await;
    assert_eq!(
        relations.as_array().expect("relations").len(),
        1,
        "only the moved relation remains"
    );
}

/// A shared entity cannot be merged into an isolated track's private entity, but a private one
/// can be merged into a shared one, keeping its private rows (spec/06 §9).
#[tokio::test]
async fn merging_a_shared_entity_into_a_private_one_is_refused() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let isolated = isolated_track(&app, book_id, "Side story").await;
    let shared = create_entity(&app, book_id, "character", "Lin Fan").await;
    let shared_id = id_of(&shared);
    let private = canon::write(app.db(), {
        let write = CanonWrite::author().with_visibility(Visibility::isolated(book_id, isolated));
        move |tx| {
            Box::pin(async move {
                let input = nekowriter::api::dto::canon::EntityInput {
                    kind: EntityKind::Character,
                    name: "Side Lin".to_owned(),
                    ..Default::default()
                };
                entity::create_inner(tx, &write, book_id, input).await
            })
        }
    })
    .await
    .expect("private entity")
    .0;

    // Shared into private: refused with 400 validation.failed (the API has no track context, so
    // the service rule is exercised directly).
    let result = canon::write(app.db(), move |tx| {
        Box::pin(async move {
            let (dto, events) = entity::merge_inner(
                tx,
                private.id,
                nekowriter::api::dto::canon::EntityMergeInput { from_id: shared_id },
            )
            .await?;
            Ok((dto, events))
        })
    })
    .await;
    match result {
        Err(CanonError::App(error)) => {
            assert_eq!(
                error.code,
                nekowriter::error::ErrorCode::ValidationFailed,
                "400 validation.failed (spec/06 §9)"
            );
        }
        other => panic!("expected validation.failed, got {other:?}"),
    }

    // Private into shared: allowed, and A stays shared.
    let (status, merged) = app
        .post(
            &format!("/api/entities/{shared_id}/merge"),
            json!({ "from_id": private.id }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{merged}");
    assert_eq!(merged["track_id"], serde_json::Value::Null);
}

/// The merge moves the protagonist flag (D-046).
#[tokio::test]
async fn merging_moves_the_protagonist_flag() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let target = create_entity(&app, book_id, "character", "Lin Fan").await;
    let target_id = id_of(&target);
    let source = create_entity(&app, book_id, "character", "Lin Fann").await;
    let source_id = id_of(&source);
    let (status, _) = app
        .post(&format!("/api/entities/{source_id}/protagonist"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, merged) = app
        .post(
            &format!("/api/entities/{target_id}/merge"),
            json!({ "from_id": source_id }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{merged}");
    assert_eq!(
        merged["is_protagonist"], true,
        "B's protagonist flag moves to A"
    );
}

/// The set-protagonist endpoint moves the flag rather than duplicating it (spec/06 §9).
#[tokio::test]
async fn setting_the_protagonist_moves_the_flag() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let first = create_entity(&app, book_id, "character", "Lin Fan").await;
    let second = create_entity(&app, book_id, "character", "Lin Fann").await;
    let (status, _) = app
        .post(
            &format!("/api/entities/{}/protagonist", id_of(&first)),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = app
        .post(
            &format!("/api/entities/{}/protagonist", id_of(&second)),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["is_protagonist"], true);
    let (_, first) = app.get(&format!("/api/entities/{}", id_of(&first))).await;
    assert_eq!(
        first["is_protagonist"], false,
        "the previous protagonist lost the flag"
    );
    let (_, list) = app.get(&format!("/api/books/{book_id}/entities")).await;
    let protagonists = list
        .as_array()
        .expect("entities")
        .iter()
        .filter(|row| row["is_protagonist"] == true)
        .count();
    assert_eq!(protagonists, 1, "at most one protagonist per book");
}

// ───────────────────────────── helpers ─────────────────────────────

/// The typed `before` snapshot of a change row, as JSON (spec/02 §4).
fn snapshot_before(row: &change_log::Model) -> serde_json::Value {
    parse_snapshot(row.before.as_deref())
}

/// The typed `after` snapshot of a change row, as JSON.
fn snapshot_after(row: &change_log::Model) -> serde_json::Value {
    parse_snapshot(row.after.as_deref())
}

fn parse_snapshot(raw: Option<&str>) -> serde_json::Value {
    serde_json::from_str(raw.expect("a snapshot")).expect("valid snapshot JSON")
}

/// The change rows of one target, newest first.
async fn change_log_of(
    app: &TestApp,
    target_type: ChangeTargetType,
    target_id: i32,
) -> Vec<change_log::Model> {
    change_log::Entity::find()
        .filter(change_log::Column::TargetType.eq(target_type.as_str()))
        .filter(change_log::Column::TargetId.eq(target_id))
        .order_by_desc(change_log::Column::Id)
        .all(app.db().conn())
        .await
        .expect("change log rows")
}

/// Unused-import guard for the row types the fixtures read directly.
#[allow(dead_code)]
fn _entities() {
    let _ = std::any::type_name::<entities::Entity>();
    let _ = std::any::type_name::<relations::Entity>();
    let _ = std::any::type_name::<appearances::Entity>();
    let _ = EntityStatus::Active;
}

/// Renders a relation from one entity's perspective, mirroring the service (used by the tool
/// tests' expectations).
#[allow(dead_code)]
fn relation_line(kind: &str, inverse: &str, source_is_query: bool) -> String {
    if source_is_query || inverse.is_empty() {
        kind.to_owned()
    } else {
        inverse.to_owned()
    }
}

/// Convenience used by the fixture module: the id of a created entity.
#[allow(dead_code)]
fn entity_id(value: &serde_json::Value) -> i32 {
    value["id"].as_i64().expect("id") as i32
}
