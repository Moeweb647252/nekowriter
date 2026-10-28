//! M5 review regressions: isolated-track visibility on every read path (spec/06 §2, D-014).
//!
//! An isolated side track's private canon is `track_id = T`; every reader — the author's canon
//! pages and the agent's retrieval tools — sees `track_id IS NULL OR track_id = T` and nothing
//! else (D-036). These tests build the rows the tracks API cannot create until M10 and prove the
//! rule on the paths that read canon outside the canon-page track filter:
//!
//! - `get_entity` / `get_relations`: a relation whose source is shared but whose target is
//!   another track's private entity must not leak the target's name;
//! - the author's list endpoints: shared rows by default, the track filter adds one track's
//!   private rows (spec/06 §2);
//! - the author's entity detail: a shared entity shows shared relations only, while an isolated
//!   entity shows its own track's rows.

mod agent_write_harness;
mod canon_fixture;

use agent_write_harness::base::runtime;
use axum::http::StatusCode;
use nekowriter::api::dto::canon::{
    EntityInput, ForeshadowingInput, MemoryNoteInput, RelationInput, TimelineEventInput,
};
use nekowriter::db::json::{EntityKind, NoteKind};
use nekowriter::services::canon::{
    self, CanonWrite, Visibility, entity, foreshadow, note, relation, timeline,
};
use nekowriter::testing::TestApp;
use nekowriter_llm::mock::ScriptStep;
use serde_json::json;

use canon_fixture::{book, isolated_track};

/// Creates a shared entity through the API and returns its id.
async fn shared_entity(app: &TestApp, book_id: i32, name: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": name, "summary": "shared row" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_i64().expect("entity id") as i32
}

/// Creates a private entity of `track_id` through the service and returns its id.
async fn private_entity(app: &TestApp, book_id: i32, track_id: i32, name: &str) -> i32 {
    let write = CanonWrite::author().with_visibility(Visibility::isolated(book_id, track_id));
    let name = name.to_owned();
    let (dto, _) = canon::write(app.db(), move |tx| {
        Box::pin(async move {
            let input = EntityInput {
                kind: EntityKind::Character,
                name: name.clone(),
                summary: "private row".to_owned(),
                ..EntityInput::default()
            };
            entity::create_inner(tx, &write, book_id, input).await
        })
    })
    .await
    .expect("create the private entity");
    dto.id
}

/// Creates a private relation of `track_id` between two visible entities.
async fn private_relation(
    app: &TestApp,
    book_id: i32,
    track_id: i32,
    source_id: i32,
    target_id: i32,
    kind: &str,
) {
    let write = CanonWrite::author().with_visibility(Visibility::isolated(book_id, track_id));
    let kind = kind.to_owned();
    let input = RelationInput {
        source_id,
        target_id,
        kind: kind.clone(),
        inverse_kind: String::new(),
        description: String::new(),
        since_chapter_id: None,
        until_chapter_id: None,
        note: None,
    };
    canon::write(app.db(), move |tx| {
        Box::pin(async move { relation::create_inner(tx, &write, book_id, input).await })
    })
    .await
    .expect("create the private relation");
}

/// Creates a private timeline event of `track_id`.
async fn private_event(app: &TestApp, book_id: i32, track_id: i32, title: &str) {
    let write = CanonWrite::author().with_visibility(Visibility::isolated(book_id, track_id));
    let title = title.to_owned();
    let input = TimelineEventInput {
        title: title.clone(),
        importance: 2,
        ..TimelineEventInput::default()
    };
    canon::write(app.db(), move |tx| {
        Box::pin(async move { timeline::create_inner(tx, &write, book_id, input).await })
    })
    .await
    .expect("create the private event");
}

/// Creates a private memory note of `track_id`.
async fn private_note(app: &TestApp, book_id: i32, track_id: i32, content: &str) {
    let write = CanonWrite::author().with_visibility(Visibility::isolated(book_id, track_id));
    let content = content.to_owned();
    let input = MemoryNoteInput {
        kind: NoteKind::Fact,
        content: content.clone(),
        chapter_id: None,
        importance: 3,
        entity_ids: Vec::new(),
        note: None,
    };
    canon::write(app.db(), move |tx| {
        Box::pin(async move { note::create_inner(tx, &write, book_id, input).await })
    })
    .await
    .expect("create the private note");
}

/// Creates a private foreshadowing item of `track_id`.
async fn private_foreshadowing(app: &TestApp, book_id: i32, track_id: i32, title: &str) {
    let write = CanonWrite::author().with_visibility(Visibility::isolated(book_id, track_id));
    let title = title.to_owned();
    let input = ForeshadowingInput {
        title: title.clone(),
        description: String::new(),
        planned_payoff: String::new(),
        planted_chapter_id: None,
        importance: 2,
        entity_ids: Vec::new(),
        note: None,
    };
    canon::write(app.db(), move |tx| {
        Box::pin(async move { foreshadow::create_inner(tx, &write, book_id, input).await })
    })
    .await
    .expect("create the private foreshadowing");
}

/// The persisted tool calls of a finished session, as `(name, result_text, ui, is_error)`.
async fn tool_calls(
    runtime: &agent_write_harness::Runtime,
    session_id: i32,
) -> Vec<(String, String, serde_json::Value, bool)> {
    let session = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await
        .1;
    session["tool_calls"]
        .as_array()
        .expect("tool calls")
        .iter()
        .map(|call| {
            (
                call["name"].as_str().expect("name").to_owned(),
                call["result_text"].as_str().unwrap_or_default().to_owned(),
                call["ui"].clone(),
                call["is_error"].as_bool().expect("is_error"),
            )
        })
        .collect()
}

/// A relation whose source is shared canon but whose target is another isolated track's private
/// entity must not reach a main-track session (spec/06 §2).
#[tokio::test]
async fn retrieval_tools_hide_another_tracks_private_relations() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("get_entity", json!({ "entity": "Shared One" })),
        ScriptStep::new().tool("get_relations", json!({ "entity": "Shared One" })),
    ])
    .await;
    let book_id = runtime.book_id;
    let shared = shared_entity(&runtime.app, book_id, "Shared One").await;
    let isolated = isolated_track(&runtime.app, book_id, "Side story").await;
    let private = private_entity(&runtime.app, book_id, isolated, "Secret Sidekick").await;
    private_relation(&runtime.app, book_id, isolated, shared, private, "ally").await;

    let session_id = agent_write_harness::start_planner(&runtime).await;
    runtime.wait_any_status(session_id).await;
    let calls = tool_calls(&runtime, session_id).await;

    let (_, entity_text, entity_ui, entity_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "get_entity")
        .expect("the get_entity call");
    assert!(!entity_error, "{entity_text}");
    assert_eq!(entity_ui["entity_id"], json!(shared));
    assert!(
        !entity_text.contains("Secret Sidekick"),
        "the entity card names another track's private entity:\n{entity_text}"
    );

    let (_, relations_text, relations_ui, relations_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "get_relations")
        .expect("the get_relations call");
    assert!(!relations_error, "{relations_text}");
    assert!(
        !relations_text.contains("Secret Sidekick") && !relations_text.contains("ally"),
        "the relation list shows another track's private relation:\n{relations_text}"
    );
    assert_eq!(
        relations_ui["count"],
        json!(0),
        "the private relation is not counted"
    );
}

/// An isolated track records a **private** relation over shared entities: the relation row is
/// the write (`track_id = T`, spec/06 §2), the shared entities are only read, and the per-track
/// unique index of spec/06 §1.2 exists exactly so a track can add its own relation that shared
/// canon may not have. Rejecting the shared endpoint as "not writable" made that impossible.
#[tokio::test]
async fn an_isolated_track_records_a_private_relation_over_shared_entities() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let isolated = isolated_track(&app, book_id, "Side story").await;
    let first = shared_entity(&app, book_id, "Shared One").await;
    let second = shared_entity(&app, book_id, "Shared Two").await;

    let write = CanonWrite::author().with_visibility(Visibility::isolated(book_id, isolated));
    let input = RelationInput {
        source_id: first,
        target_id: second,
        kind: "ally".to_owned(),
        inverse_kind: "ally".to_owned(),
        description: "in this side story only".to_owned(),
        since_chapter_id: None,
        until_chapter_id: None,
        note: None,
    };
    let (created, _) = canon::write(app.db(), move |tx| {
        Box::pin(async move { relation::create_inner(tx, &write, book_id, input).await })
    })
    .await
    .expect("a private relation over shared entities is a private row, not a shared write");
    assert_eq!(
        created.track_id,
        Some(isolated),
        "the relation row is private"
    );

    // Shared canon does not show it (spec/06 §2).
    let (_, relations) = app.get(&format!("/api/books/{book_id}/relations")).await;
    assert!(
        relations.as_array().expect("relations").is_empty(),
        "the private relation stays out of shared canon: {relations}"
    );
}

/// The author's canon pages show shared rows by default; only the track filter adds one isolated
/// track's private rows (spec/06 §2). The entity list already works this way; the timeline, the
/// relations, the notes and the foreshadowing must too.
#[tokio::test]
async fn the_canon_pages_default_to_shared_rows() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let isolated = isolated_track(&app, book_id, "Side story").await;
    let shared = shared_entity(&app, book_id, "Shared One").await;
    let private = private_entity(&app, book_id, isolated, "Private One").await;
    private_relation(&app, book_id, isolated, shared, private, "ally").await;
    private_event(&app, book_id, isolated, "Private event").await;
    private_note(&app, book_id, isolated, "Private note").await;
    private_foreshadowing(&app, book_id, isolated, "Private hook").await;

    let (status, relations) = app.get(&format!("/api/books/{book_id}/relations")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        relations
            .as_array()
            .expect("relations")
            .iter()
            .all(|row| row["track_id"].is_null()),
        "the relations list defaults to shared rows: {relations}"
    );

    let (status, events) = app.get(&format!("/api/books/{book_id}/timeline")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        events
            .as_array()
            .expect("events")
            .iter()
            .all(|row| row["track_id"].is_null()),
        "the timeline defaults to shared rows: {events}"
    );
    // The track filter adds the isolated track's private events (it will drive the M10 badge).
    let (status, events) = app
        .get(&format!(
            "/api/books/{book_id}/timeline?track_id={isolated}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        events
            .as_array()
            .expect("events")
            .iter()
            .any(|row| !row["track_id"].is_null()),
        "the track filter adds the private event: {events}"
    );

    let (status, notes) = app.get(&format!("/api/books/{book_id}/memory-notes")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        notes
            .as_array()
            .expect("notes")
            .iter()
            .all(|row| row["track_id"].is_null()),
        "the notes list defaults to shared rows: {notes}"
    );

    let (status, hooks) = app
        .get(&format!("/api/books/{book_id}/foreshadowing"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        hooks
            .as_array()
            .expect("foreshadowing")
            .iter()
            .all(|row| row["track_id"].is_null()),
        "the foreshadowing list defaults to shared rows: {hooks}"
    );
}

/// A shared entity's detail shows shared relations only; an isolated entity's detail shows its
/// own track's rows (spec/06 §2, PROJECT §8.8).
#[tokio::test]
async fn the_entity_detail_keeps_relations_to_their_own_canon() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let isolated = isolated_track(&app, book_id, "Side story").await;
    let shared = shared_entity(&app, book_id, "Shared One").await;
    let private = private_entity(&app, book_id, isolated, "Private One").await;
    private_relation(&app, book_id, isolated, shared, private, "ally").await;

    let (status, detail) = app.get(&format!("/api/entities/{shared}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        detail["relations"]
            .as_array()
            .expect("relations")
            .is_empty(),
        "the shared entity's detail shows no private relation: {detail}"
    );

    let (status, detail) = app.get(&format!("/api/entities/{private}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        detail["relations"].as_array().expect("relations").len(),
        1,
        "the isolated entity's detail shows its own relation: {detail}"
    );
}
