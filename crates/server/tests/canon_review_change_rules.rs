//! M5 review regressions for the change log and revert rules (spec/06 §3, spec/02 §5.9–§5.10,
//! D-079, D-085).
//!
//! - A revert is a system change and its note is **empty**: `note` only ever holds written text
//!   (spec/06 §3), and the author-facing endpoint has no note to send.
//! - A revert of a revert is refused (the way back is to change the row again).
//! - The actor is `user` for author writes and `agent` for tool writes.
//! - `fresh_id` keeps two reverts of deleted rows apart: the serialized write lock is what makes
//!   the "one past the high-water mark" allocation safe within one process.

mod canon_fixture;

use axum::http::StatusCode;
use nekowriter::api::dto::canon::EntityPatch;
use nekowriter::db::entities::change_log;
use nekowriter::db::json::{ChangeActor, ChangeOp, ChangeTargetType, Snapshot};
use nekowriter::services::canon::{self, CanonWrite, Visibility, entity};
use nekowriter::testing::TestApp;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};

use canon_fixture::book;

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

/// The change rows of one target, newest first.
async fn changes_of(
    app: &TestApp,
    target: ChangeTargetType,
    target_id: i32,
) -> Vec<change_log::Model> {
    change_log::Entity::find()
        .filter(change_log::Column::TargetType.eq(target.as_str()))
        .filter(change_log::Column::TargetId.eq(target_id))
        .order_by_desc(change_log::Column::Id)
        .all(app.db().conn())
        .await
        .expect("change rows")
}

/// A revert records an empty note, even when the caller sends one: `RevertInput` has no note any
/// more, and the row is a system change (spec/06 §3, D-085).
#[tokio::test]
async fn a_revert_records_no_note() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let entity_id = create(&app, book_id, "Lin Fan").await;
    let (status, _) = app
        .patch(
            &format!("/api/entities/{entity_id}"),
            json!({ "state": "wounded", "note": "the duel" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let history = changes_of(&app, ChangeTargetType::Entity, entity_id).await;
    let update_id = history[0].id;

    // The endpoint takes no note; an extra field is ignored rather than stored.
    let (status, revert) = app
        .post(
            &format!("/api/changes/{update_id}/revert"),
            json!({ "note": "undo it" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{revert}");
    assert_eq!(revert["note"], json!(""), "a revert's note is empty");
    assert_eq!(revert["actor"], json!("system"));
    assert_eq!(revert["reverts_id"], json!(update_id));

    let revert_id = revert["id"].as_i64().expect("revert id") as i32;
    let stored = change_log::Entity::find_by_id(revert_id)
        .one(app.db().conn())
        .await
        .expect("query")
        .expect("revert row");
    assert_eq!(stored.note, "", "the stored note is empty");
}

/// A revert of a revert is refused: it is a system change, so the way back is to change the row
/// again (spec/06 §3.1, D-079).
#[tokio::test]
async fn a_revert_of_a_revert_is_refused() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let entity_id = create(&app, book_id, "Lin Fan").await;
    let (status, _) = app
        .patch(
            &format!("/api/entities/{entity_id}"),
            json!({ "state": "wounded" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let history = changes_of(&app, ChangeTargetType::Entity, entity_id).await;
    let (status, revert) = app
        .post(&format!("/api/changes/{}/revert", history[0].id), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{revert}");
    let revert_id = revert["id"].as_i64().expect("revert id") as i32;

    let (status, body) = app
        .post(&format!("/api/changes/{revert_id}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], json!("validation.failed"));
}

/// Author writes record `user`; an agent's write records `agent` (spec/06 §3).
#[tokio::test]
async fn the_change_actor_names_who_wrote() {
    let app = TestApp::new().await;
    let (book_id, track_id) = book(&app).await;
    let chapter_id = app.create_chapter(track_id, "One").await["id"]
        .as_i64()
        .expect("chapter id") as i32;
    let entity_id = create(&app, book_id, "Lin Fan").await;

    // Author write.
    let (status, _) = app
        .patch(
            &format!("/api/entities/{entity_id}"),
            json!({ "state": "by the author" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let history = changes_of(&app, ChangeTargetType::Entity, entity_id).await;
    assert_eq!(history[0].actor, ChangeActor::User);

    // Agent write through the same service (the archivist tools call it this way).
    let write = CanonWrite {
        visibility: Some(Visibility::shared(book_id)),
        actor: ChangeActor::Agent,
        session_id: None,
        chapter_id: Some(chapter_id),
        revert: None,
    };
    let (dto, events) = canon::write(app.db(), move |tx| {
        Box::pin(async move {
            let patch = EntityPatch {
                state: Some("by the archivist".to_owned()),
                ..EntityPatch::default()
            };
            entity::update_inner(tx, &write, entity_id, patch).await
        })
    })
    .await
    .expect("agent update");
    assert_eq!(dto.state, "by the archivist");
    assert_eq!(events.len(), 1);
    let history = changes_of(&app, ChangeTargetType::Entity, entity_id).await;
    assert_eq!(
        history[0].actor,
        ChangeActor::Agent,
        "a tool write is an agent change"
    );
    assert!(
        matches!(
            history[0].after.as_ref().map(|value| &value.0),
            Some(Snapshot::Entity { .. })
        ),
        "the row still carries the typed after snapshot"
    );
}

/// Two reverts of deleted rows in one process get distinct new ids: `fresh_id` allocates one past
/// the larger of the live table's max id and the change log's highest target id (spec/02 §5.10,
/// D-079).
#[tokio::test]
async fn concurrent_reverts_of_deleted_rows_get_distinct_fresh_ids() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let first = create(&app, book_id, "First").await;
    let second = create(&app, book_id, "Second").await;
    for entity_id in [first, second] {
        let (status, _) = app.delete(&format!("/api/entities/{entity_id}")).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    let first_delete = changes_of(&app, ChangeTargetType::Entity, first).await[0].id;
    let second_delete = changes_of(&app, ChangeTargetType::Entity, second).await[0].id;

    let first_path = format!("/api/changes/{first_delete}/revert");
    let second_path = format!("/api/changes/{second_delete}/revert");
    let (first_result, second_result) = tokio::join!(
        app.post(&first_path, json!({})),
        app.post(&second_path, json!({})),
    );
    assert_eq!(first_result.0, StatusCode::OK, "{}", first_result.1);
    assert_eq!(second_result.0, StatusCode::OK, "{}", second_result.1);
    let first_new = first_result.1["target_id"].as_i64().expect("new id");
    let second_new = second_result.1["target_id"].as_i64().expect("new id");
    assert_ne!(first_new, second_new, "each recreated row has its own id");
    assert!(first_new > i64::from(first.max(second)));
    assert!(second_new > i64::from(first.max(second)));

    for new_id in [first_new, second_new] {
        let (status, _) = app.get(&format!("/api/entities/{new_id}")).await;
        assert_eq!(status, StatusCode::OK, "entity #{new_id} exists");
        let rows = changes_of(&app, ChangeTargetType::Entity, new_id as i32).await;
        assert_eq!(rows.len(), 1, "one recreate change for #{new_id}");
        assert_eq!(rows[0].op, ChangeOp::Create);
    }
}

/// Reverting a delete restores the join rows whose entities still exist (spec/02 §5.10): an
/// event's `entity_ids` come back for the survivors only.
#[tokio::test]
async fn revert_of_a_deleted_event_restores_the_surviving_entity_links() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let survivor = create(&app, book_id, "Lin Fan").await;
    let removed = create(&app, book_id, "Zhang San").await;
    let (status, event) = app
        .post(
            &format!("/api/books/{book_id}/timeline"),
            json!({
                "title": "The gate",
                "entity_ids": [survivor, removed],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{event}");
    let event_id = event["id"].as_i64().expect("event id") as i32;
    let (status, _) = app
        .delete(&format!("/api/timeline-events/{event_id}"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // One endpoint disappears before the revert: its join row cannot be restored.
    let (status, _) = app.delete(&format!("/api/entities/{removed}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let delete_change = changes_of(&app, ChangeTargetType::TimelineEvent, event_id).await[0].id;
    let (status, revert) = app
        .post(&format!("/api/changes/{delete_change}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{revert}");
    let new_id = revert["target_id"].as_i64().expect("new id") as i32;
    assert_ne!(new_id, event_id, "a reverted delete gets a new id");
    let (status, recreated) = app.get(&format!("/api/timeline-events/{new_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        recreated["entity_ids"],
        json!([survivor]),
        "the link to the surviving entity is restored and the dead one is skipped: {recreated}"
    );
}

/// Every recreated row takes a new id, not the deleted row's reused rowid (spec/02 §5.10): the
/// relation, note and foreshadowing paths of `fresh_id` in addition to the entity and event ones.
#[tokio::test]
async fn reverting_a_delete_recreates_every_canon_target_with_a_new_id() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let first = create(&app, book_id, "Lin Fan").await;
    let second = create(&app, book_id, "Zhang San").await;

    // Relation.
    let (status, relation) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": first, "target_id": second, "kind": "ally" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{relation}");
    let relation_id = relation["id"].as_i64().expect("relation id") as i32;
    // Memory note.
    let (status, note) = app
        .post(
            &format!("/api/books/{book_id}/memory-notes"),
            json!({ "kind": "fact", "content": "The sword was a gift." }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{note}");
    let note_id = note["id"].as_i64().expect("note id") as i32;
    // Foreshadowing.
    let (status, hook) = app
        .post(
            &format!("/api/books/{book_id}/foreshadowing"),
            json!({ "title": "The broken hilt" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");
    let hook_id = hook["id"].as_i64().expect("hook id") as i32;

    let targets: Vec<(ChangeTargetType, &str, i32)> = vec![
        (ChangeTargetType::Relation, "relations", relation_id),
        (ChangeTargetType::MemoryNote, "memory-notes", note_id),
        (ChangeTargetType::Foreshadowing, "foreshadowing", hook_id),
    ];
    for (target, _path, target_id) in targets {
        let (status, _) = app
            .delete(&format!(
                "/api/{}/{}",
                match target {
                    ChangeTargetType::Relation => "relations",
                    ChangeTargetType::MemoryNote => "memory-notes",
                    ChangeTargetType::Foreshadowing => "foreshadowing",
                    _ => unreachable!(),
                },
                target_id
            ))
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "delete {target:?}");
        let delete_change = changes_of(&app, target, target_id).await[0].id;
        let (status, revert) = app
            .post(&format!("/api/changes/{delete_change}/revert"), json!({}))
            .await;
        assert_eq!(status, StatusCode::OK, "revert {target:?}: {revert}");
        let new_id = revert["target_id"].as_i64().expect("new id");
        assert_ne!(
            new_id as i32, target_id,
            "{target:?} is recreated with a new id"
        );
        assert_eq!(revert["op"], json!("create"), "{target:?}");
    }
}

/// The revert response's `before`/`after` describe the inverse operation like any other change
/// (D-079), so the Changes page can diff it.
#[tokio::test]
async fn a_revert_change_carries_the_inverse_operation_snapshots() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let entity_id = create(&app, book_id, "Lin Fan").await;
    let (status, _) = app
        .patch(
            &format!("/api/entities/{entity_id}"),
            json!({ "state": "wounded" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let history = changes_of(&app, ChangeTargetType::Entity, entity_id).await;
    let (status, revert) = app
        .post(&format!("/api/changes/{}/revert", history[0].id), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let revert: Value = revert;
    assert_eq!(revert["op"], json!("update"));
    assert_eq!(
        revert["before"]["state"],
        json!("wounded"),
        "before is the state the revert replaces"
    );
    assert_eq!(
        revert["after"]["state"],
        json!(""),
        "after is the restored state"
    );
}
