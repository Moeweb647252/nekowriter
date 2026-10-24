//! M5 review regressions: tiered-memory window rules (spec/06 §4.1–§4.3).
//!
//! Three rules the snapshot tests did not exercise:
//!
//! - `absent_important`'s "latest appearance in P" is the **highest chapter**, not the appearance
//!   row written last (rows are re-inserted whenever a chapter's appearance set changes);
//! - a side story anchored `before_start` gets **no** main-story chapters in P: the fill rule adds
//!   main-track chapters "up to the anchor", and nothing precedes `before_start`;
//! - `GET /books/:id/memory?chapter_id=` names a position, so the track comes from the chapter
//!   when `track_id` is omitted; using the main track for a side chapter mislabels it.

use axum::http::StatusCode;
use nekowriter::services::memory::{self, MemorySnapshot};
use nekowriter::testing::TestApp;
use sea_orm::{ConnectionTrait, EntityTrait};
use serde_json::json;

/// One integer from a fixture query (the raw JSON mapping does not cover SQLite integers).
async fn scalar_i64(app: &TestApp, sql: &str) -> i64 {
    let row = app
        .db()
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .expect("fixture query")
        .expect("fixture row");
    row.try_get::<i64>("", "value").expect("fixture value")
}

/// A book with 12 official chapters (summaries set, text written) and a 13th, unwritten target.
struct Fixture {
    app: TestApp,
    book_id: i32,
    track_id: i32,
    chapters: Vec<i32>,
    target: i32,
}

async fn fixture() -> Fixture {
    let app = TestApp::new().await;
    let book = app.create_book("Memory Review Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let mut chapters = Vec::new();
    for index in 1..=12 {
        let chapter = app
            .create_chapter(track_id, &format!("Chapter {index}"))
            .await;
        let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
        app.put_text(
            chapter_id,
            0,
            &format!("Paragraph of chapter {index}.\n\nEnding of chapter {index}."),
        )
        .await;
        app.set_chapter_status(chapter_id, "official").await;
        app.execute(&format!(
            "UPDATE chapters SET summary = 'Summary of chapter {index}.' WHERE id = {chapter_id}"
        ))
        .await;
        chapters.push(chapter_id);
    }
    let target = app.create_chapter(track_id, "Chapter 13").await;
    let target = target["id"].as_i64().expect("target id") as i32;
    Fixture {
        app,
        book_id,
        track_id,
        chapters,
        target,
    }
}

impl Fixture {
    async fn entity(&self, name: &str, kind: &str, importance: i32) -> i32 {
        let (status, body) = self
            .app
            .post(
                &format!("/api/books/{}/entities", self.book_id),
                json!({ "kind": kind, "name": name, "summary": "s", "importance": importance }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["id"].as_i64().expect("id") as i32
    }

    async fn appears(&self, chapter_id: i32, entity_id: i32) {
        let (status, body) = self
            .app
            .put(
                &format!("/api/chapters/{chapter_id}/appearances"),
                json!([{ "entity_id": entity_id, "role": "major", "note": "" }]),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    async fn snapshot(&self) -> MemorySnapshot {
        self.snapshot_for(self.track_id).await
    }

    async fn snapshot_for(&self, track_id: i32) -> MemorySnapshot {
        let state = self.app.state();
        let prompts = state
            .prompts()
            .for_book(state.db(), Some(self.book_id))
            .await
            .expect("prompts");
        let conn = state.db().conn();
        let track = nekowriter::db::entities::tracks::Entity::find_by_id(track_id)
            .one(conn)
            .await
            .expect("query")
            .expect("track");
        let chapter = nekowriter::db::entities::chapters::Entity::find_by_id(self.target)
            .one(conn)
            .await
            .expect("query")
            .expect("chapter");
        memory::snapshot(conn, &prompts, &track, &chapter)
            .await
            .expect("snapshot")
    }
}

/// `absent_important` names the chapter of the **latest** appearance in P, ordered by chapter, not
/// by the order the appearance rows were written (spec/06 §4.3).
#[tokio::test]
async fn absent_important_uses_the_highest_chapter_as_the_latest_appearance() {
    let fixture = fixture().await;
    let character = fixture.entity("Absent Late", "character", 4).await;
    // Both appearances are outside W (chapters 3..=12); write chapter 2 first, then chapter 1,
    // so the row order and the chapter order disagree.
    fixture.appears(fixture.chapters[1], character).await;
    fixture.appears(fixture.chapters[0], character).await;

    let snapshot = fixture.snapshot().await;
    let item = snapshot
        .absent_important
        .iter()
        .find(|item| item.id == character)
        .unwrap_or_else(|| panic!("the important absent character is listed: {snapshot:?}"));
    assert_eq!(
        item.last_label, "第2章",
        "the latest appearance is chapter 2, not the row written last"
    );
}

/// A side story anchored `before_start` has no prior chapter to draw from, so the fill rule adds
/// nothing: P stays empty and the first chapter is told there is no earlier text (spec/06 §4.1,
/// §4.5; spec/08 §6).
#[tokio::test]
async fn a_before_start_side_story_gets_no_main_story_fill() {
    let fixture = fixture().await;
    // The tracks API cannot create side tracks until M10; write the row the services read.
    fixture
        .app
        .execute(&format!(
            "INSERT INTO tracks(book_id, kind, title, synopsis, canon_mode, anchor_kind, ord, \
             created_at, updated_at) VALUES ({}, 'side', 'Past', '', 'isolated', 'before_start', \
             1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            fixture.book_id
        ))
        .await;
    let side_track = scalar_i64(
        &fixture.app,
        &format!(
            "SELECT max(id) AS value FROM tracks WHERE book_id = {}",
            fixture.book_id
        ),
    )
    .await as i32;
    let side_chapter = fixture.app.create_chapter(side_track, "Side one").await["id"]
        .as_i64()
        .expect("side chapter") as i32;

    let snapshot = {
        let state = fixture.app.state();
        let prompts = state
            .prompts()
            .for_book(state.db(), Some(fixture.book_id))
            .await
            .expect("prompts");
        let conn = state.db().conn();
        let track = nekowriter::db::entities::tracks::Entity::find_by_id(side_track)
            .one(conn)
            .await
            .expect("query")
            .expect("track");
        let chapter = nekowriter::db::entities::chapters::Entity::find_by_id(side_chapter)
            .one(conn)
            .await
            .expect("query")
            .expect("chapter");
        memory::snapshot(conn, &prompts, &track, &chapter)
            .await
            .expect("snapshot")
    };
    assert!(
        snapshot.is_first,
        "nothing precedes a before_start side story, so this is its first chapter: {snapshot:?}"
    );
    assert!(
        snapshot.recent_summaries.is_empty(),
        "the main story's chapters are not in a before_start side story's P: {snapshot:?}"
    );
    assert!(snapshot.previous_tail.is_none(), "{snapshot:?}");
}

/// `GET /books/:id/memory?chapter_id=` derives the target track from the chapter when `track_id`
/// is omitted; otherwise the side chapter is labelled and windowed as a main-track chapter
/// (spec/10 §2.4).
#[tokio::test]
async fn the_memory_api_derives_the_track_from_the_chapter() {
    let fixture = fixture().await;
    fixture
        .app
        .execute(&format!(
            "INSERT INTO tracks(book_id, kind, title, synopsis, canon_mode, anchor_kind, ord, \
             created_at, updated_at) VALUES ({}, 'side', 'Side Story', '', 'isolated', 'none', \
             1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            fixture.book_id
        ))
        .await;
    let side_track = scalar_i64(
        &fixture.app,
        &format!(
            "SELECT max(id) AS value FROM tracks WHERE book_id = {}",
            fixture.book_id
        ),
    )
    .await as i32;
    let side_chapter = fixture.app.create_chapter(side_track, "Side one").await["id"]
        .as_i64()
        .expect("side chapter") as i32;

    let (status, with_track) = fixture
        .app
        .get(&format!(
            "/api/books/{}/memory?track_id={side_track}&chapter_id={side_chapter}",
            fixture.book_id
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{with_track}");
    let (status, derived) = fixture
        .app
        .get(&format!(
            "/api/books/{}/memory?chapter_id={side_chapter}",
            fixture.book_id
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{derived}");
    assert_eq!(
        with_track["snapshot"]["chapter"]["label"], derived["snapshot"]["chapter"]["label"],
        "the chapter names its own position, so both requests render the side-track label"
    );
}
