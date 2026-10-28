//! Side-story tracks (PLAN M10.1, spec/08 §6, spec/06 §2/§4.1, spec/03 §10).
//!
//! The tests cover the tracks API (defaults, `ord`, anchor and focus validation, the canon-mode
//! lock), the chapter rules of a side track (the first chapter's "previous chapter" is the
//! anchor, an anchor chapter cannot be deleted, a side track can), `block.anchor` in every
//! writer/reviewer/archivist task of a side-track chapter, and the §4.1 memory window of
//! spec/06 for a side track.

mod agent_write_harness;

use std::time::Duration;

use agent_write_harness::base::runtime;
use axum::http::StatusCode;
use nekowriter::db::json::{Granularity, SessionKind, SessionStatus};
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter::testing::{TestApp, assert_error, assert_error_detail};
use nekowriter_llm::mock::{ScriptStep, ScriptedProvider};
use serde_json::{Value, json};

/// Creates a side track through the API and returns its DTO.
async fn side_track(app: &TestApp, book_id: i32, body: Value) -> (StatusCode, Value) {
    app.post(&format!("/api/books/{book_id}/tracks"), body)
        .await
}

/// A shared character entity, marked official through the API.
async fn shared_character(app: &TestApp, book_id: i32, name: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": name, "summary": "a shared row" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_i64().expect("entity id") as i32
}

/// An official main chapter with synced canon (the write-precondition fixture of §6).
async fn official_main_chapter(runtime: &agent_write_harness::base::Runtime, title: &str) -> i32 {
    let chapter = runtime.app.create_chapter(runtime.track_id, title).await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    runtime
        .app
        .put_text(chapter_id, 0, "第一段正文。\n\n第二段正文。")
        .await;
    runtime.app.set_chapter_status(chapter_id, "official").await;
    runtime.app.mark_canon_synced(chapter_id).await;
    chapter_id
}

/// A side track anchored after an official main chapter, with one planned chapter.
async fn side_story_after_anchor(
    runtime: &agent_write_harness::base::Runtime,
    anchor_chapter_id: i32,
) -> (i32, i32) {
    let (status, track) = side_track(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "剑冢旧事", "synopsis": "数十年前的剑冢。", "anchor_chapter_id": anchor_chapter_id }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{track}");
    let track_id = track["id"].as_i64().expect("track id") as i32;
    let chapter = runtime.app.create_chapter(track_id, "番外一").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    (track_id, chapter_id)
}

// ───────────────────────────── creation (spec/08 §6) ─────────────────────────────

/// The defaults of spec/08 §6: `isolated`, and `after_chapter` on the latest official main
/// chapter — `before_start` when no main chapter is official yet.
#[tokio::test]
async fn creating_a_side_track_applies_the_spec_defaults() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let main_id = tracks[0]["id"].as_i64().expect("main id") as i32;

    // No official chapter yet → before_start.
    let (status, track) = side_track(&app, book_id, json!({ "title": "Side" })).await;
    assert_eq!(status, StatusCode::CREATED, "{track}");
    assert_eq!(track["kind"], "side");
    assert_eq!(track["canon_mode"], "isolated");
    assert_eq!(track["anchor_kind"], "before_start");
    assert_eq!(track["anchor_chapter_id"], Value::Null);
    assert_eq!(track["ord"], 1, "side tracks are 1, 2, … in creation order");

    // Two official main chapters → the anchor is the latest official one.
    let first = app.create_chapter(main_id, "One").await;
    let first_id = first["id"].as_i64().expect("id") as i32;
    let second = app.create_chapter(main_id, "Two").await;
    let second_id = second["id"].as_i64().expect("id") as i32;
    for id in [first_id, second_id] {
        app.set_chapter_status(id, "official").await;
    }
    let (status, track_two) = side_track(&app, book_id, json!({ "title": "Second" })).await;
    assert_eq!(status, StatusCode::CREATED, "{track_two}");
    assert_eq!(track_two["anchor_kind"], "after_chapter");
    assert_eq!(track_two["anchor_chapter_id"], second_id);
    assert_eq!(track_two["ord"], 2);

    // The list is ordered: main first, then the side tracks by ord.
    let (_, list) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let ords: Vec<i64> = list
        .as_array()
        .expect("tracks")
        .iter()
        .map(|row| row["ord"].as_i64().expect("ord"))
        .collect();
    assert_eq!(ords, vec![0, 1, 2]);
}

/// `after_chapter` must name a main-track chapter of the same book, and a focus entity must be a
/// visible character (spec/08 §6); anything else is `validation.failed`.
#[tokio::test]
async fn anchor_and_focus_validation() {
    let runtime = runtime(Vec::new()).await;
    let app = &runtime.app;
    let book_id = runtime.book_id;
    let other = app.create_book("Other").await;
    let other_id = other["id"].as_i64().expect("book id") as i32;
    let (_, other_tracks) = app.get(&format!("/api/books/{other_id}/tracks")).await;
    let other_chapter = app
        .create_chapter(
            other_tracks[0]["id"].as_i64().expect("main") as i32,
            "Other",
        )
        .await;
    let other_chapter_id = other_chapter["id"].as_i64().expect("id") as i32;

    // Another book's chapter.
    let (status, body) = side_track(
        app,
        book_id,
        json!({ "title": "S", "anchor_kind": "after_chapter", "anchor_chapter_id": other_chapter_id }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_error(&body, "validation.failed");

    // Another side track's chapter.
    let (_, first_side) = side_track(app, book_id, json!({ "title": "Side A" })).await;
    let side_chapter = app
        .create_chapter(first_side["id"].as_i64().expect("id") as i32, "S1")
        .await;
    let side_chapter_id = side_chapter["id"].as_i64().expect("id") as i32;
    let (status, body) = side_track(
        app,
        book_id,
        json!({ "title": "S", "anchor_kind": "after_chapter", "anchor_chapter_id": side_chapter_id }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_error(&body, "validation.failed");

    // `after_chapter` without a chapter.
    let (status, body) = side_track(
        app,
        book_id,
        json!({ "title": "S", "anchor_kind": "after_chapter" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_error_detail(&body, "validation.failed", "fields");

    // A chapter named under a kind that names none.
    let (status, body) = side_track(
        app,
        book_id,
        json!({ "title": "S", "anchor_kind": "after_end", "anchor_chapter_id": runtime.chapter_id }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_error_detail(&body, "validation.failed", "fields");

    // A focus entity that is not a character.
    let (status, place) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "location", "name": "剑冢", "summary": "a place" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{place}");
    let place_id = place["id"].as_i64().expect("id") as i32;
    let (status, body) = side_track(
        app,
        book_id,
        json!({ "title": "S", "focus_entity_id": place_id }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_error_detail(&body, "validation.failed", "fields");

    // A focus entity of another book.
    let other_character = shared_character(app, other_id, "别的书的人物").await;
    let (status, body) = side_track(
        app,
        book_id,
        json!({ "title": "S", "focus_entity_id": other_character }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_error_detail(&body, "validation.failed", "fields");

    // A visible shared character is accepted and returned.
    let character = shared_character(app, book_id, "林凡").await;
    let (status, track) = side_track(
        app,
        book_id,
        json!({ "title": "番外", "focus_entity_id": character }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{track}");
    assert_eq!(track["focus_entity_id"], character);
}

/// An empty title is a field error (a side story is named by its title, D-054).
#[tokio::test]
async fn a_side_track_needs_a_title() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (status, body) = side_track(&app, book_id, json!({ "title": "  " })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_error_detail(&body, "validation.failed", "fields");
}

// ───────────────────────────── editing (spec/08 §6) ─────────────────────────────

/// `canon_mode` is locked once the track has an official chapter; before that it may change, and
/// the anchor may change at any time (affecting only new sessions).
#[tokio::test]
async fn the_canon_mode_is_locked_by_an_official_chapter_and_the_anchor_is_not() {
    let runtime = runtime(Vec::new()).await;
    let app = &runtime.app;
    let book_id = runtime.book_id;

    let (status, track) = side_track(
        app,
        book_id,
        json!({ "title": "番外", "canon_mode": "canon", "anchor_kind": "before_start" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{track}");
    let track_id = track["id"].as_i64().expect("track id") as i32;

    // Before any official chapter the mode may change.
    let (status, body) = app
        .patch(
            &format!("/api/tracks/{track_id}"),
            json!({ "canon_mode": "isolated" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["canon_mode"], "isolated");

    // The anchor may move to a main chapter at any time.
    let anchor = official_main_chapter(&runtime, "第一章").await;
    let (status, body) = app
        .patch(
            &format!("/api/tracks/{track_id}"),
            json!({ "anchor_kind": "after_chapter", "anchor_chapter_id": anchor }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["anchor_kind"], "after_chapter");
    assert_eq!(body["anchor_chapter_id"], anchor);

    // With an official chapter of its own, the mode is locked.
    let chapter = app.create_chapter(track_id, "番外一").await;
    let chapter_id = chapter["id"].as_i64().expect("id") as i32;
    app.set_chapter_status(chapter_id, "official").await;
    let (status, body) = app
        .patch(
            &format!("/api/tracks/{track_id}"),
            json!({ "canon_mode": "canon" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "track.mode_locked");

    // The same mode is a no-op, and the anchor still moves.
    let (status, body) = app
        .patch(
            &format!("/api/tracks/{track_id}"),
            json!({ "canon_mode": "isolated" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let second = official_main_chapter(&runtime, "第二章").await;
    let (status, body) = app
        .patch(
            &format!("/api/tracks/{track_id}"),
            json!({ "anchor_chapter_id": second }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["anchor_chapter_id"], second);
}

/// The focus character can be set and cleared, and the main track keeps refusing side-track
/// fields (D-037).
#[tokio::test]
async fn the_focus_character_comes_and_goes() {
    let runtime = runtime(Vec::new()).await;
    let app = &runtime.app;
    let character = shared_character(app, runtime.book_id, "林凡").await;

    let (status, track) = side_track(app, runtime.book_id, json!({ "title": "番外" })).await;
    assert_eq!(status, StatusCode::CREATED, "{track}");
    let track_id = track["id"].as_i64().expect("track id") as i32;

    let (status, body) = app
        .patch(
            &format!("/api/tracks/{track_id}"),
            json!({ "focus_entity_id": character }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["focus_entity_id"], character);

    let (status, body) = app
        .patch(
            &format!("/api/tracks/{track_id}"),
            json!({ "clear_focus_entity": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["focus_entity_id"], Value::Null);

    // The main track has none of these fields (D-037).
    let (_, tracks) = app
        .get(&format!("/api/books/{}/tracks", runtime.book_id))
        .await;
    let main_id = tracks[0]["id"].as_i64().expect("main id") as i32;
    for patch in [
        json!({ "canon_mode": "isolated" }),
        json!({ "anchor_kind": "after_end" }),
        json!({ "focus_entity_id": character }),
        json!({ "clear_focus_entity": true }),
    ] {
        let (status, body) = app.patch(&format!("/api/tracks/{main_id}"), patch).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_error(&body, "validation.failed");
    }
}

// ───────────────────────────── deletion (spec/08 §1.1, §1.3) ─────────────────────────────

/// An anchor chapter cannot be deleted (`chapter.is_anchor` with `details.track_ids`); a side
/// track can be deleted and takes its chapters with it.
#[tokio::test]
async fn an_anchor_chapter_is_pinned_and_a_side_track_is_deletable() {
    let runtime = runtime(Vec::new()).await;
    let app = &runtime.app;
    let book_id = runtime.book_id;
    let anchor = official_main_chapter(&runtime, "第一章").await;
    let (track_id, chapter_id) = side_story_after_anchor(&runtime, anchor).await;

    // The anchor chapter is pinned by the side track.
    let (status, body) = app.delete(&format!("/api/chapters/{anchor}")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "chapter.is_anchor");
    assert_eq!(
        body["error"]["details"]["track_ids"],
        json!([track_id]),
        "the error names the tracks to re-anchor: {body}"
    );

    // The side track goes, chapters and all.
    let (status, body) = app.delete(&format!("/api/tracks/{track_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    assert_eq!(
        tracks
            .as_array()
            .expect("tracks")
            .iter()
            .filter(|row| row["id"].as_i64() == Some(i64::from(track_id)))
            .count(),
        0,
        "the side track is gone"
    );

    // With the side track gone, the anchor chapter is deletable again.
    let (status, body) = app.delete(&format!("/api/chapters/{anchor}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

/// Deleting a side track abandons its open sessions before the cascade (spec/08 §1.3), so no
/// runner can write into deleted rows.
#[tokio::test]
async fn deleting_a_side_track_abandons_its_open_sessions_first() {
    let runtime = runtime(Vec::new()).await;
    let app = &runtime.app;
    let anchor = official_main_chapter(&runtime, "第一章").await;
    let (track_id, chapter_id) = side_story_after_anchor(&runtime, anchor).await;
    app.patch(
        &format!("/api/chapters/{chapter_id}"),
        json!({ "outline": "番外细纲" }),
    )
    .await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().expect("session") as i32;
    runtime
        .wait_status_within(session_id, SessionStatus::Running, Duration::from_secs(10))
        .await;

    let (status, body) = app.delete(&format!("/api/tracks/{track_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    // The runner was cancelled and awaited before the cascade; the session row itself cascades
    // with its chapter (`agent_sessions.chapter_id ON DELETE CASCADE`, spec/02 §2), so the API
    // answers 404 and no runner is left writing into deleted rows.
    let (status, session) = app.get(&format!("/api/sessions/{session_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{session}");
    assert!(
        !app.state().manager().is_running(session_id),
        "the runner is gone before the cascade"
    );
}

// ───────────────────────────── the first side chapter's write rule (§6) ─────────────────────────────

/// The first chapter of a side story takes its "previous chapter" from the anchor
/// (`after_chapter`) or from nothing (`before_start`, `after_end`) (spec/08 §6, §1.1).
#[tokio::test]
async fn the_first_side_chapter_reads_the_anchor_as_its_previous_chapter() {
    let runtime = runtime(Vec::new()).await;
    let app = &runtime.app;
    let book_id = runtime.book_id;

    // Anchor chapter not official yet → the write is refused with the anchor named.
    let anchor = app.create_chapter(runtime.track_id, "第一章").await;
    let anchor_id = anchor["id"].as_i64().expect("id") as i32;
    app.put_text(anchor_id, 0, "正文。").await;
    let (_track_id, chapter_id) = side_story_after_anchor(&runtime, anchor_id).await;
    app.patch(
        &format!("/api/chapters/{chapter_id}"),
        json!({ "outline": "番外细纲" }),
    )
    .await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "chapter.previous_not_ready");
    assert_eq!(body["error"]["details"]["chapter_id"], anchor_id);
    assert_eq!(body["error"]["details"]["reason"], "not_official");
    assert_eq!(body["error"]["details"]["anchor"], true);

    // Official but not synced → canon_not_synced.
    app.set_chapter_status(anchor_id, "official").await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["details"]["reason"], "canon_not_synced");

    // Synced → the first side chapter is writable.
    app.mark_canon_synced(anchor_id).await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session = body["session_id"].as_i64().expect("session") as i32;
    app.post(&format!("/api/sessions/{session}/stop"), json!({}))
        .await;
    runtime
        .wait_status_within(session, SessionStatus::Stopped, Duration::from_secs(10))
        .await;

    // `before_start` and `after_end` have nothing before them: always writable.
    for kind in ["before_start", "after_end"] {
        let (status, track) = side_track(
            app,
            book_id,
            json!({ "title": format!("T {kind}"), "anchor_kind": kind }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{track}");
        let chapter = app
            .create_chapter(track["id"].as_i64().expect("id") as i32, "一")
            .await;
        let id = chapter["id"].as_i64().expect("id") as i32;
        app.patch(
            &format!("/api/chapters/{id}"),
            json!({ "outline": "番外细纲" }),
        )
        .await;
        let (status, body) = app
            .post(&format!("/api/chapters/{id}/write"), json!({}))
            .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "a {kind} side story's first chapter is writable: {body}"
        );
        let session = body["session_id"].as_i64().expect("session") as i32;
        app.post(&format!("/api/sessions/{session}/stop"), json!({}))
            .await;
        runtime
            .wait_status_within(session, SessionStatus::Stopped, Duration::from_secs(10))
            .await;
    }
}

// ───────────────────────────── `block.anchor` (spec/03 §10, spec/08 §6) ─────────────────────────────

/// The rendered kickoff text of a session created through the production path.
async fn kickoff_text(
    runtime: &agent_write_harness::base::Runtime,
    input: CreateSession,
) -> String {
    let (session_id, _warnings) = sessions::create(runtime.app.state(), input)
        .await
        .expect("create session");
    let messages = runtime.messages(session_id).await;
    let kickoff = messages.first().expect("a kickoff message");
    assert_eq!(kickoff.kind, "kickoff");
    kickoff.text()
}

/// `block.anchor` is rendered inside every writer, reviewer and archivist task of a side-track
/// chapter, carrying the premise, the canon mode and the anchor notice (spec/03 §10, spec/08 §6).
#[tokio::test]
async fn block_anchor_renders_for_every_side_track_task() {
    let runtime = runtime(Vec::new()).await;
    let anchor = official_main_chapter(&runtime, "第一章").await;
    let character = shared_character(&runtime.app, runtime.book_id, "林凡").await;
    let (status, track) = side_track(
        &runtime.app,
        runtime.book_id,
        json!({
            "title": "剑冢旧事",
            "synopsis": "数十年前的剑冢。",
            "anchor_kind": "after_chapter",
            "anchor_chapter_id": anchor,
            "focus_entity_id": character,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{track}");
    let track_id = track["id"].as_i64().expect("track id") as i32;
    let chapter = runtime.app.create_chapter(track_id, "番外一").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    runtime
        .app
        .put_text(chapter_id, 0, "番外正文第一段。")
        .await;

    let chapter_of = |kind| CreateSession {
        kind,
        book_id: Some(runtime.book_id),
        chapter_id: Some(chapter_id),
        granularity: Some(Granularity::Paragraph),
        ..CreateSession::default()
    };
    let writer = kickoff_text(&runtime, chapter_of(SessionKind::ChapterWrite)).await;
    assert!(
        writer.contains("【番外说明】"),
        "the writer task carries block.anchor:\n{writer}"
    );
    assert!(
        writer.contains("番外《剑冢旧事》"),
        "the block names the side story:\n{writer}"
    );
    assert!(
        writer.contains("独立番外"),
        "the canon mode reads isolated:\n{writer}"
    );
    assert!(
        writer.contains("番外梗概：数十年前的剑冢。"),
        "the premise is carried:\n{writer}"
    );
    assert!(
        writer.contains("锚点之后"),
        "the after_chapter anchor notice is carried:\n{writer}"
    );

    // A canon-mode side story reads differently, and so do the other anchors.
    let (status, canon_track) = side_track(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "正史番外", "canon_mode": "canon", "anchor_kind": "after_end" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{canon_track}");
    let canon_chapter = runtime
        .app
        .create_chapter(canon_track["id"].as_i64().expect("id") as i32, "一")
        .await;
    let canon_chapter_id = canon_chapter["id"].as_i64().expect("id") as i32;
    let text = kickoff_text(
        &runtime,
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(canon_chapter_id),
            granularity: Some(Granularity::Paragraph),
            ..CreateSession::default()
        },
    )
    .await;
    assert!(
        text.contains("正史番外：其中发生的事件属于正史"),
        "a canon-mode side story states its mode:\n{text}"
    );
    assert!(
        text.contains("发生在正文完结之后"),
        "the after_end anchor is carried:\n{text}"
    );

    // Reviewer and archivist tasks of the isolated chapter carry the block too.
    runtime
        .app
        .set_chapter_status(chapter_id, "in_review")
        .await;
    let reviewer = kickoff_text(&runtime, chapter_of(SessionKind::ChapterReview)).await;
    assert!(
        reviewer.contains("【番外说明】"),
        "the reviewer task carries block.anchor:\n{reviewer}"
    );
    let archivist = kickoff_text(&runtime, chapter_of(SessionKind::ChapterMaintenance)).await;
    assert!(
        archivist.contains("【番外说明】"),
        "the archivist task carries block.anchor:\n{archivist}"
    );
    assert!(
        archivist.contains("独立番外"),
        "the isolated archivist task explains the mode:\n{archivist}"
    );

    // A main-track task never renders it (M4's regression).
    let main = kickoff_text(
        &runtime,
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            granularity: Some(Granularity::Paragraph),
            ..CreateSession::default()
        },
    )
    .await;
    assert!(
        !main.contains("【番外说明】"),
        "a main-track task never carries block.anchor:\n{main}"
    );
}

/// A `before_start` anchor states that nothing of the main story has happened yet.
#[tokio::test]
async fn a_before_start_anchor_says_the_story_has_not_begun() {
    let runtime = runtime(Vec::new()).await;
    let (status, track) = side_track(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "前传", "anchor_kind": "before_start" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{track}");
    let chapter = runtime
        .app
        .create_chapter(track["id"].as_i64().expect("id") as i32, "序")
        .await;
    let chapter_id = chapter["id"].as_i64().expect("id") as i32;
    let text = kickoff_text(
        &runtime,
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(chapter_id),
            granularity: Some(Granularity::Paragraph),
            ..CreateSession::default()
        },
    )
    .await;
    assert!(
        text.contains("发生在正文开始之前"),
        "the before_start anchor is carried:\n{text}"
    );
}

// ───────────────────────────── side-track memory (spec/06 §4.1) ─────────────────────────────

/// The persisted tool calls of a finished session, as `(name, result_text, is_error)`.
async fn tool_calls(
    runtime: &agent_write_harness::base::Runtime,
    session_id: i32,
) -> Vec<(String, String, bool)> {
    runtime
        .calls(session_id)
        .await
        .into_iter()
        .map(|call| {
            (
                call.name,
                call.result_text.unwrap_or_default(),
                call.is_error,
            )
        })
        .collect()
}

/// A side track's memory is topped up from the main story to max(N, M) (spec/06 §4.1, D-038):
/// the first chapter of a side story anchored after chapter 2 of a 3-chapter main story sees
/// chapters 1 and 2 as its prior chapters (labels included), and the anchor chapter's tail as
/// its `previous_tail`.
#[tokio::test]
async fn side_track_memory_tops_up_from_the_main_story() {
    let mut runtime = runtime(Vec::new()).await;
    let book_id = runtime.book_id;
    // Three official main chapters; the anchor is chapter 2.
    let _first = official_main_chapter(&runtime, "第一章").await;
    let second = official_main_chapter(&runtime, "第二章").await;
    let third = official_main_chapter(&runtime, "第三章").await;
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET summary = '第二章摘要。' WHERE id = {second}"
        ))
        .await;
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET summary = '第三章摘要。' WHERE id = {third}"
        ))
        .await;
    let (_track_id, chapter_id) = side_story_after_anchor(&runtime, second).await;
    // Turn the recent-chapter block off so its full chapter text cannot mask a missing
    // `previous_tail` in `block.memory`: this test is about the memory window's tail itself.
    let (_, book) = runtime.app.get(&format!("/api/books/{book_id}")).await;
    let mut settings = book["settings"].clone();
    settings["recent_chapter_count"] = json!(0);
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "settings": settings }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "patch settings: {body}");
    runtime.app.set_provider(ScriptedProvider::new(Vec::new()));
    let app = &runtime.app;
    let (session_id, _) = sessions::create(
        app.state(),
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(book_id),
            chapter_id: Some(chapter_id),
            granularity: Some(Granularity::Paragraph),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create side-track writer session");
    runtime.wait_any_status(session_id).await;
    let messages = runtime.messages(session_id).await;
    let kickoff = messages.first().expect("kickoff");
    let text = kickoff.text();
    // The window lists the main story's chapters up to the anchor with their summaries — never
    // chapter 3, which is after the anchor (spec/06 §4.1 "up to the anchor").
    assert!(
        text.contains("第二章摘要。"),
        "the anchor chapter's summary is in the window:\n{text}"
    );
    assert!(
        text.contains("（尚无摘要）") || text.contains("第一章摘要。"),
        "the fill covers the chapters before the anchor:\n{text}"
    );
    assert!(
        !text.contains("第三章摘要。"),
        "a chapter after the anchor never enters the window:\n{text}"
    );
    // The previous tail is the anchor chapter's ending (spec/06 §4.2). Assert the heading, not
    // only the text: the text also appears in `block.recent_chapters` when that block is on.
    assert!(
        text.contains("### 上一章结尾（第3章）"),
        "the anchor chapter is named as the previous tail:\n{text}"
    );
    assert!(
        text.contains("第二段正文。"),
        "the anchor chapter's tail is the previous tail:\n{text}"
    );
}

/// The focus character is the "protagonist" of a side track's short-term memory, and its state is
/// flagged when its newest change came from a main chapter after the anchor (spec/06 §4.2, D-038).
#[tokio::test]
async fn the_focus_character_is_the_side_protagonist_and_post_anchor_states_are_flagged() {
    let mut runtime = runtime(Vec::new()).await;
    let book_id = runtime.book_id;
    let anchor = official_main_chapter(&runtime, "第一章").await;
    // A later main chapter (official) whose maintenance changed the protagonist's state.
    let later = official_main_chapter(&runtime, "第二章").await;
    let hero = shared_character(&runtime.app, book_id, "林凡").await;
    runtime
        .app
        .execute(&format!(
            "UPDATE entities SET state = '身处剑冢第三层' WHERE id = {hero}"
        ))
        .await;
    // The newest change of the state comes from the chapter after the anchor.
    runtime
        .app
        .execute(&format!(
            "INSERT INTO change_log(book_id, target_type, target_id, op, actor, note, before, after, \
             chapter_id, track_id, created_at) VALUES ({book_id}, 'entity', {hero}, 'update', 'agent', \
             '状态更新', NULL, NULL, {later}, NULL, '2026-01-01T00:00:00Z')"
        ))
        .await;

    let (status, track) = side_track(
        &runtime.app,
        book_id,
        json!({
            "title": "剑冢旧事",
            "anchor_kind": "after_chapter",
            "anchor_chapter_id": anchor,
            "focus_entity_id": hero
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{track}");
    let track_id = track["id"].as_i64().expect("track id") as i32;
    let chapter = runtime.app.create_chapter(track_id, "番外一").await;
    let chapter_id = chapter["id"].as_i64().expect("id") as i32;

    runtime
        .app
        .set_provider(ScriptedProvider::new(vec![ScriptStep::new()]));
    let (session_id, _) = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(book_id),
            chapter_id: Some(chapter_id),
            granularity: Some(Granularity::Paragraph),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create side-track writer session");
    runtime.wait_any_status(session_id).await;
    let messages = runtime.messages(session_id).await;
    let text = messages.first().expect("kickoff").text();
    assert!(
        text.contains("林凡"),
        "the focus character is the side story's protagonist:\n{text}"
    );
    assert!(
        text.contains("身处剑冢第三层"),
        "the focus character's state is short-term memory:\n{text}"
    );
    assert!(
        text.contains("锚点之后"),
        "a state changed after the anchor is flagged (D-038):\n{text}"
    );
    let _ = tool_calls(&runtime, session_id).await;
}
