//! Tiered memory snapshot tests (PLAN M5.3, spec/06 §4).
//!
//! The fixture is the 12-chapter book of the task description: P has 12 official chapters, the
//! target position is the 13th. The tests pin the windows (§4.1), each part's content, the caps
//! and ordering of §4.3–§4.4, the empty states of §4.5, and the rule that `block.memory` leaves
//! the chapter outline out when the kickoff already shows it (§4.2).

use axum::http::StatusCode;
use nekowriter::db::json::{AppearanceRole, NoteKind};
use nekowriter::services::canon::{self, CanonWrite};
use nekowriter::services::memory::{self, MemorySnapshot};
use nekowriter::testing::TestApp;
use sea_orm::EntityTrait;
use serde_json::json;

/// The fixture: one book, 12 official chapters with summaries, a target chapter 13, and canon.
struct Fixture {
    app: TestApp,
    book_id: i32,
    track_id: i32,
    /// The 12 official chapters, oldest first.
    chapters: Vec<i32>,
    /// The target chapter (13), not official.
    target: i32,
}

/// Builds the 12-chapter fixture of PLAN M5.3.
async fn fixture() -> Fixture {
    let app = TestApp::new().await;
    let book = app.create_book("Memory Book").await;
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
                json!({
                    "kind": kind,
                    "name": name,
                    "summary": format!("{name} summary"),
                    "state": format!("{name} state"),
                    "attributes": { "level": "2" },
                    "importance": importance,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["id"].as_i64().expect("entity id") as i32
    }

    async fn appears(&self, chapter_id: i32, appearances: serde_json::Value) {
        let (status, body) = self
            .app
            .put(
                &format!("/api/chapters/{chapter_id}/appearances"),
                appearances,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    /// The snapshot of the target chapter through the service.
    async fn snapshot(&self) -> MemorySnapshot {
        let state = self.app.state();
        let prompts = state
            .prompts()
            .for_book(state.db(), Some(self.book_id))
            .await
            .expect("prompts");
        let conn = state.db().conn();
        let track = nekowriter::db::entities::tracks::Entity::find_by_id(self.track_id)
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

    async fn rendered(&self) -> String {
        let (status, body) = self
            .app
            .get(&format!(
                "/api/books/{}/memory?chapter_id={}",
                self.book_id, self.target
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["rendered"].as_str().expect("rendered").to_owned()
    }
}

// ───────────────────────────── §4.1 windows ─────────────────────────────

/// The short-term window is the last N chapters of P and the mid-term window the last M.
#[tokio::test]
async fn the_windows_are_the_last_n_and_m_chapters_of_p() {
    let fixture = fixture().await;
    let snapshot = fixture.snapshot().await;
    assert_eq!(snapshot.window_mid, 10, "the default M is 10");
    assert_eq!(snapshot.recent_summaries.len(), 5, "the default N is 5");
    let labels: Vec<&str> = snapshot
        .recent_summaries
        .iter()
        .map(|item| item.label.as_str())
        .collect();
    assert_eq!(
        labels,
        vec!["第8章", "第9章", "第10章", "第11章", "第12章"],
        "the short-term window is the last N chapters, oldest first"
    );
    assert_eq!(
        snapshot.recent_summaries[0].title, "Chapter 8",
        "each summary carries its title"
    );
    assert_eq!(
        snapshot.recent_summaries[0].summary,
        "Summary of chapter 8."
    );
    assert!(
        !snapshot.is_first,
        "twelve earlier chapters are official, so this is not a first chapter"
    );
}

/// A book setting narrows the windows (§4.1: N and M are book settings).
#[tokio::test]
async fn the_book_settings_change_both_windows() {
    let fixture = fixture().await;
    let (status, body) = fixture
        .app
        .patch(
            &format!("/api/books/{}", fixture.book_id),
            json!({ "settings": {
                "granularity": "paragraph", "auto_pause": false, "pause_after_outline": true,
                "target_words_min": 3000, "target_words_max": 4000,
                "recent_summary_count": 2, "midterm_window": 4, "auto_maintenance": true,
            }}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let snapshot = fixture.snapshot().await;
    assert_eq!(snapshot.window_mid, 4);
    assert_eq!(snapshot.recent_summaries.len(), 2);
    assert_eq!(snapshot.recent_summaries[1].label, "第12章");
}

// ───────────────────────────── §4.2 short-term ─────────────────────────────

/// The previous tail is the last paragraphs of the latest chapter in P (at most 800 characters).
#[tokio::test]
async fn the_previous_tail_comes_from_the_latest_prior_chapter() {
    let fixture = fixture().await;
    let snapshot = fixture.snapshot().await;
    let tail = snapshot.previous_tail.expect("a previous chapter exists");
    assert_eq!(tail.label, "第12章");
    assert!(
        tail.text.contains("Ending of chapter 12."),
        "the tail keeps the end of the chapter: {tail:?}"
    );
    assert!(
        tail.text.chars().count() <= memory::PREVIOUS_TAIL_LIMIT,
        "the tail is capped at 800 characters"
    );
}

/// The protagonist carries its state, status and every attribute (§4.2).
#[tokio::test]
async fn the_protagonist_carries_its_state_and_attributes() {
    let fixture = fixture().await;
    let protagonist = fixture.entity("Shen Yan", "character", 5).await;
    let (status, body) = fixture
        .app
        .post(
            &format!("/api/entities/{protagonist}/protagonist"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let snapshot = fixture.snapshot().await;
    let item = snapshot.protagonist.expect("a protagonist");
    assert_eq!(item.id, protagonist);
    assert_eq!(item.name, "Shen Yan");
    assert_eq!(item.state, "Shen Yan state");
    assert_eq!(item.status, "active");
    assert_eq!(item.status_label, "活跃");
    assert_eq!(
        item.attributes,
        vec![("level".to_owned(), "2".to_owned())],
        "all attributes are key facts"
    );
}

/// The target chapter's brief carries its guidance, target length and outline; `block.memory`
/// renders only the label and the title (§4.2).
#[tokio::test]
async fn the_chapter_brief_carries_the_guidance_and_the_target() {
    let fixture = fixture().await;
    let (status, _) = fixture
        .app
        .patch(
            &format!("/api/chapters/{}", fixture.target),
            json!({ "guidance": "Make it rain.", "outline": "Beat one\nBeat two" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let snapshot = fixture.snapshot().await;
    assert_eq!(snapshot.chapter.guidance, "Make it rain.");
    assert_eq!(snapshot.chapter.outline, "Beat one\nBeat two");
    assert!(snapshot.chapter.has_outline);
    assert_eq!(snapshot.chapter.target_min, 3000);
    assert_eq!(snapshot.chapter.target_max, 4000);

    let rendered = fixture.rendered().await;
    assert!(rendered.contains("第13章"), "the label:\n{rendered}");
    assert!(
        !rendered.contains("Make it rain."),
        "block.memory does not restate the guidance:\n{rendered}"
    );
    assert!(
        !rendered.contains("目标"),
        "block.memory does not restate the target length:\n{rendered}"
    );
}

// ───────────────────────────── §4.3 mid-term ─────────────────────────────

/// `recent_changes` carries entity and relation notes inside W, ordered by chapter then id, with
/// the rendered target and the chapter label (§4.3).
#[tokio::test]
async fn recent_changes_are_the_noted_entity_and_relation_changes_in_the_window() {
    let fixture = fixture().await;
    let entity = fixture.entity("Lin Fan", "character", 4).await;
    // Notes on chapters inside W, and one on a chapter outside W (chapter 1) and one outside P.
    for (index, note) in [(5, "first note"), (12, "newest note")] {
        let chapter_id = fixture.chapters[index - 1];
        let (status, body) = fixture
            .app
            .patch(
                &format!("/api/entities/{entity}"),
                json!({ "state": note, "note": note }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        fixture
            .app
            .execute(&format!(
                "UPDATE change_log SET chapter_id = {chapter_id} WHERE target_type = 'entity' \
                 AND target_id = {entity} AND note = '{note}'"
            ))
            .await;
    }
    // A note on chapter 1 is outside W (the last 10 chapters are 3..=12).
    let first_chapter = fixture.chapters[0];
    let (status, body) = fixture
        .app
        .patch(
            &format!("/api/entities/{entity}"),
            json!({ "state": "old", "note": "outside the window" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    fixture
        .app
        .execute(&format!(
            "UPDATE change_log SET chapter_id = {first_chapter} WHERE target_type = 'entity' \
             AND target_id = {entity} AND note = 'outside the window'"
        ))
        .await;

    let snapshot = fixture.snapshot().await;
    let notes: Vec<(&str, &str)> = snapshot
        .recent_changes
        .iter()
        .map(|item| (item.note.as_str(), item.chapter_label.as_str()))
        .collect();
    assert!(
        notes.contains(&("first note", "第5章")),
        "a change inside W is rendered with its chapter: {notes:?}"
    );
    assert!(notes.contains(&("newest note", "第12章")), "{notes:?}");
    assert!(
        !notes.iter().any(|(note, _)| *note == "outside the window"),
        "a change outside W is not in mid-term memory"
    );
    // "Ordered by chapter, then id" (spec/06 §4.3).
    let labels: Vec<&str> = snapshot
        .recent_changes
        .iter()
        .map(|item| item.chapter_label.as_str())
        .collect();
    let mut sorted = labels.clone();
    sorted.sort_by_key(|label| chapter_number(label));
    assert_eq!(labels, sorted, "the changes are ordered by chapter");
    assert!(
        snapshot
            .recent_changes
            .iter()
            .any(|item| item.target == format!("Lin Fan(#{entity})")),
        "the target is rendered as 名字(#id): {:?}",
        snapshot
            .recent_changes
            .iter()
            .map(|item| item.target.clone())
            .collect::<Vec<_>>()
    );
}

/// `recent_changes` is capped at 40, keeping the newest (§4.3).
#[tokio::test]
async fn recent_changes_are_capped_at_forty() {
    let fixture = fixture().await;
    let entity = fixture.entity("Lin Fan", "character", 4).await;
    for index in 0..45 {
        let note = format!("note {index:02}");
        let (status, _) = fixture
            .app
            .patch(
                &format!("/api/entities/{entity}"),
                json!({ "state": note, "note": note }),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        fixture
            .app
            .execute(&format!(
                "UPDATE change_log SET chapter_id = {} WHERE target_type = 'entity' AND \
                 target_id = {entity} AND note = '{note}'",
                fixture.chapters[11]
            ))
            .await;
    }
    let snapshot = fixture.snapshot().await;
    assert_eq!(
        snapshot.recent_changes.len(),
        memory::RECENT_CHANGES_LIMIT,
        "the mid-term list keeps at most 40 changes"
    );
    assert!(
        snapshot
            .recent_changes
            .iter()
            .any(|item| item.note == "note 44"),
        "the newest changes are kept"
    );
    assert!(
        !snapshot
            .recent_changes
            .iter()
            .any(|item| item.note == "note 00"),
        "the oldest changes fall out of the cap"
    );
}

/// `appearances` counts characters in W, sorted by count desc, at most 30 (§4.3).
#[tokio::test]
async fn appearances_count_the_characters_in_the_window() {
    let fixture = fixture().await;
    let frequent = fixture.entity("Frequent", "character", 3).await;
    let rare = fixture.entity("Rare", "character", 3).await;
    let old = fixture.entity("Old", "character", 3).await;
    // `frequent` appears in chapters 10..=12 (inside W, chapter 10 as the POV), `rare` only in 12.
    for index in [9usize, 10, 11] {
        let chapter_id = fixture.chapters[index];
        let role = if index == 9 { "pov" } else { "major" };
        let mut set = vec![json!({ "entity_id": frequent, "role": role, "note": "" })];
        if index == 11 {
            set.push(json!({ "entity_id": rare, "role": "minor", "note": "" }));
        }
        fixture.appears(chapter_id, json!(set)).await;
    }
    // `old` appears only in chapter 1, outside W.
    fixture
        .appears(
            fixture.chapters[0],
            json!([{ "entity_id": old, "role": "pov", "note": "" }]),
        )
        .await;

    let snapshot = fixture.snapshot().await;
    let names: Vec<(&str, i32)> = snapshot
        .appearances
        .iter()
        .map(|item| (item.name.as_str(), item.count))
        .collect();
    assert!(
        names.contains(&("Frequent", 3)),
        "the count covers every window chapter: {names:?}"
    );
    assert!(names.contains(&("Rare", 1)), "{names:?}");
    assert!(
        !names.iter().any(|(name, _)| *name == "Old"),
        "an appearance outside W is not counted"
    );
    assert_eq!(names[0].0, "Frequent", "sorted by count desc: {names:?}");
    let first = &snapshot.appearances[0];
    assert_eq!(first.chapters.len(), 3);
    assert_eq!(first.chapters[0].label, "第10章");
    assert_eq!(
        first.chapters[0].role_label, "视角",
        "each chapter carries its rendered role"
    );
}

/// `appearances` is capped at 30 (§4.3).
#[tokio::test]
async fn appearances_are_capped_at_thirty() {
    let fixture = fixture().await;
    let mut ids = Vec::new();
    for index in 0..35 {
        ids.push(
            fixture
                .entity(&format!("Char {index:02}"), "character", 2)
                .await,
        );
    }
    let set: Vec<serde_json::Value> = ids
        .iter()
        .map(|id| json!({ "entity_id": id, "role": "minor", "note": "" }))
        .collect();
    fixture.appears(fixture.chapters[11], json!(set)).await;
    let snapshot = fixture.snapshot().await;
    assert_eq!(
        snapshot.appearances.len(),
        memory::APPEARANCES_LIMIT,
        "at most 30 characters are listed"
    );
}

/// `absent_important` lists active important characters that appear in P but not in W (§4.3).
#[tokio::test]
async fn absent_important_lists_characters_missing_from_the_window() {
    let fixture = fixture().await;
    let absent = fixture.entity("Absent", "character", 4).await;
    let present = fixture.entity("Present", "character", 4).await;
    let minor = fixture.entity("Minor Absent", "character", 2).await;
    // Appear early (outside W) and in the window respectively.
    fixture
        .appears(
            fixture.chapters[0],
            json!([
                { "entity_id": absent, "role": "pov", "note": "" },
                { "entity_id": minor, "role": "pov", "note": "" },
            ]),
        )
        .await;
    fixture
        .appears(
            fixture.chapters[11],
            json!([{ "entity_id": present, "role": "pov", "note": "" }]),
        )
        .await;

    let snapshot = fixture.snapshot().await;
    let names: Vec<(&str, &str)> = snapshot
        .absent_important
        .iter()
        .map(|item| (item.name.as_str(), item.last_label.as_str()))
        .collect();
    assert!(
        names.contains(&("Absent", "第1章")),
        "the absent important character is listed with its latest appearance: {names:?}"
    );
    assert!(
        !names.iter().any(|(name, _)| *name == "Present"),
        "a character in the window is not absent"
    );
    assert!(
        !names.iter().any(|(name, _)| *name == "Minor Absent"),
        "importance below 4 is not 'important'"
    );
    assert!(snapshot.absent_important.len() <= memory::ABSENT_LIMIT);
}

// ───────────────────────────── §4.4 long-term index ─────────────────────────────

/// `key_notes` holds active facts and world rules with importance ≥ 4, at most 15 (§4.4).
#[tokio::test]
async fn key_notes_are_the_important_active_facts_and_rules() {
    let fixture = fixture().await;
    for index in 0..20 {
        let (status, body) = fixture
            .app
            .post(
                &format!("/api/books/{}/memory-notes", fixture.book_id),
                json!({
                    "kind": if index % 2 == 0 { "fact" } else { "world_rule" },
                    "content": format!("important note {index:02}"),
                    "importance": 4,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    // One note below the importance threshold and one retired note are both out.
    fixture
        .app
        .post(
            &format!("/api/books/{}/memory-notes", fixture.book_id),
            json!({ "kind": "fact", "content": "minor note", "importance": 2 }),
        )
        .await;
    let (status, retired) = fixture
        .app
        .post(
            &format!("/api/books/{}/memory-notes", fixture.book_id),
            json!({ "kind": "fact", "content": "retired note", "importance": 5 }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = fixture
        .app
        .patch(
            &format!("/api/memory-notes/{}", retired["id"]),
            json!({ "status": "retired", "note": "overturned" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let snapshot = fixture.snapshot().await;
    assert_eq!(
        snapshot.key_notes.len(),
        memory::KEY_NOTES_LIMIT,
        "at most 15 key notes"
    );
    let contents: Vec<&str> = snapshot
        .key_notes
        .iter()
        .map(|item| item.content.as_str())
        .collect();
    assert!(!contents.contains(&"minor note"));
    assert!(!contents.contains(&"retired note"));
    assert!(
        snapshot
            .key_notes
            .iter()
            .all(|item| item.kind == NoteKind::Fact || item.kind == NoteKind::WorldRule),
        "only facts and world rules are key notes"
    );
    assert_eq!(snapshot.key_notes[0].kind_label, "事实");
}

/// `open_foreshadowing` is sorted by importance desc, then planted chapter asc, capped at 20
/// (§4.4).
#[tokio::test]
async fn open_foreshadowing_is_sorted_and_capped() {
    let fixture = fixture().await;
    let mut ids = Vec::new();
    for index in 0..25 {
        let (status, body) = fixture
            .app
            .post(
                &format!("/api/books/{}/foreshadowing", fixture.book_id),
                json!({
                    "title": format!("Hook {index:02}"),
                    "description": "a hook",
                    "importance": if index < 3 { 5 } else { 3 },
                    "planted_chapter_id": fixture.chapters[index % 12],
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        ids.push(body["id"].as_i64().expect("id") as i32);
    }
    // Resolve one so it leaves the open list.
    let (status, _) = fixture
        .app
        .patch(
            &format!("/api/foreshadowing/{}", ids[0]),
            json!({ "status": "resolved", "resolution": "paid off" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let snapshot = fixture.snapshot().await;
    assert_eq!(
        snapshot.open_foreshadowing.len(),
        memory::OPEN_FORESHADOWING_LIMIT,
        "at most 20 open items"
    );
    assert!(
        snapshot
            .open_foreshadowing
            .iter()
            .all(|item| item.id != ids[0]),
        "a resolved item is not open"
    );
    let importances: Vec<i32> = snapshot
        .open_foreshadowing
        .iter()
        .map(|item| item.importance)
        .collect();
    let mut sorted = importances.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(importances, sorted, "sorted by importance desc");
    assert_eq!(
        snapshot.open_foreshadowing[0].importance, 5,
        "the most important hooks come first"
    );
}

/// `cast` is sorted by importance desc then last appearance desc, capped at 40 (§4.4).
#[tokio::test]
async fn the_cast_is_sorted_and_capped() {
    let fixture = fixture().await;
    for index in 0..45 {
        fixture
            .entity(&format!("Entity {index:02}"), "character", (index % 5) + 1)
            .await;
    }
    let snapshot = fixture.snapshot().await;
    assert_eq!(snapshot.cast.len(), memory::CAST_LIMIT, "at most 40");
    let importances: Vec<i32> = snapshot.cast.iter().map(|item| item.importance).collect();
    let mut sorted = importances.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(importances, sorted);
    assert_eq!(snapshot.cast[0].importance, 5);
    assert_eq!(
        snapshot.cast[0].kind_label, "人物",
        "the cast renders the kind label"
    );
}

/// `counts` is the archive size line: entities per kind, events, active notes, open hooks
/// (§4.4).
#[tokio::test]
async fn counts_report_the_archive_size() {
    let fixture = fixture().await;
    fixture.entity("A character", "character", 2).await;
    fixture.entity("An organization", "organization", 2).await;
    fixture.entity("A location", "location", 2).await;
    fixture.entity("An item", "item", 2).await;
    fixture.entity("Something else", "other", 2).await;
    let (status, _) = fixture
        .app
        .post(
            &format!("/api/books/{}/timeline", fixture.book_id),
            json!({ "title": "An event" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    fixture
        .app
        .post(
            &format!("/api/books/{}/memory-notes", fixture.book_id),
            json!({ "kind": "fact", "content": "a fact", "importance": 2 }),
        )
        .await;
    fixture
        .app
        .post(
            &format!("/api/books/{}/memory-notes", fixture.book_id),
            json!({ "kind": "world_rule", "content": "a rule", "importance": 2 }),
        )
        .await;
    fixture
        .app
        .post(
            &format!("/api/books/{}/memory-notes", fixture.book_id),
            json!({ "kind": "other", "content": "a joke", "importance": 2 }),
        )
        .await;
    fixture
        .app
        .post(
            &format!("/api/books/{}/foreshadowing", fixture.book_id),
            json!({ "title": "A hook" }),
        )
        .await;

    let snapshot = fixture.snapshot().await;
    assert_eq!(snapshot.counts.character, 1);
    assert_eq!(snapshot.counts.organization, 1);
    assert_eq!(snapshot.counts.location, 1);
    assert_eq!(snapshot.counts.item, 1);
    assert_eq!(snapshot.counts.other, 1);
    assert_eq!(snapshot.counts.event, 1);
    assert_eq!(snapshot.counts.fact, 1);
    assert_eq!(snapshot.counts.world_rule, 1);
    assert_eq!(snapshot.counts.note_other, 1);
    assert_eq!(snapshot.counts.foreshadowing_open, 1);
}

// ───────────────────────────── §4.5 empty states ─────────────────────────────

/// The first chapter of a book is told there is no earlier text; every empty part renders
/// explicitly (§4.5).
#[tokio::test]
async fn a_first_chapter_renders_every_empty_state() {
    let app = TestApp::new().await;
    let book = app.create_book("Empty Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter One").await;
    let chapter_id = chapter["id"].as_i64().expect("id") as i32;

    let (status, body) = app
        .get(&format!(
            "/api/books/{book_id}/memory?chapter_id={chapter_id}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rendered = body["rendered"].as_str().expect("rendered");
    assert!(
        rendered.contains("这是本书（本轨道）的第一章，暂无前文。"),
        "the first chapter is told there is no earlier text:\n{rendered}"
    );
    assert!(body["snapshot"]["is_first"].as_bool().expect("is_first"));
    assert!(
        body["snapshot"]["protagonist"].is_null(),
        "no protagonist, no state"
    );
    assert_eq!(
        body["snapshot"]["recent_summaries"]
            .as_array()
            .expect("summaries")
            .len(),
        0
    );
    assert!(
        rendered.contains("近期没有记录到状态或关系变化。"),
        "the mid-term empty state:\n{rendered}"
    );
    assert!(
        rendered.contains("档案规模：人物 0"),
        "the counts line renders zeroes:\n{rendered}"
    );
}

/// A later chapter written before its predecessors are official is told that none of them is
/// official yet, instead of being called the first chapter (D-059, §4.5).
#[tokio::test]
async fn a_later_chapter_before_its_predecessors_says_none_is_official() {
    let app = TestApp::new().await;
    let book = app.create_book("Draft Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    app.create_chapter(track_id, "One").await;
    let second = app.create_chapter(track_id, "Two").await;
    let second_id = second["id"].as_i64().expect("id") as i32;

    let (status, body) = app
        .get(&format!(
            "/api/books/{book_id}/memory?chapter_id={second_id}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rendered = body["rendered"].as_str().expect("rendered");
    assert!(
        rendered.contains("前面的章节都还没有定稿"),
        "the empty-but-not-first state:\n{rendered}"
    );
    assert!(
        !rendered.contains("这是本书（本轨道）的第一章"),
        "it is not the first chapter:\n{rendered}"
    );
}

// ───────────────────────────── §4.2 outline rule ─────────────────────────────

/// `block.memory` leaves the chapter outline out when the kickoff already shows it (§4.2): a
/// review kickoff renders `block.chapter_draft`, and the memory block does not repeat it.
#[tokio::test]
async fn block_memory_omits_the_outline_when_the_kickoff_shows_it() {
    use nekowriter::services::sessions::{self, CreateSession};

    let fixture = fixture().await;
    configure_models(&fixture.app).await;
    let (status, _) = fixture
        .app
        .patch(
            &format!("/api/chapters/{}", fixture.target),
            json!({ "outline": "OUTLINE-MARKER", "guidance": "GUIDANCE-MARKER" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // A review kickoff shows the outline through `block.chapter_draft`.
    let (session_id, _) = sessions::create(
        fixture.app.state(),
        CreateSession {
            kind: nekowriter::db::json::SessionKind::ChapterReview,
            book_id: Some(fixture.book_id),
            chapter_id: Some(fixture.target),
            ..CreateSession::default()
        },
    )
    .await
    .expect("review session");
    let kickoff = kickoff_text(fixture.app.state(), session_id).await;
    assert!(
        kickoff.contains("OUTLINE-MARKER"),
        "the draft block shows it"
    );
    let memory_block_starts = kickoff.find("# 记忆").expect("the memory block");
    let memory_block = &kickoff[memory_block_starts..];
    let draft_block = memory_block
        .find("# 本章正文")
        .unwrap_or(memory_block.len());
    assert!(
        !memory_block[..draft_block].contains("### 本章章纲"),
        "block.memory does not repeat the outline when block.chapter_draft renders it:\n{kickoff}"
    );
}

/// Every write kickoff carries `block.chapter_draft` (even with an empty draft), so `block.memory`
/// never repeats the outline (spec/06 §4.2, D-146).
#[tokio::test]
async fn block_memory_does_not_repeat_the_outline_of_a_write_kickoff() {
    use nekowriter::services::sessions::{self, CreateSession};

    let app = TestApp::new().await;
    configure_models(&app).await;
    let book = app.create_book("Outline Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "One").await;
    let chapter_id = chapter["id"].as_i64().expect("id") as i32;
    let (status, _) = app
        .patch(
            &format!("/api/chapters/{chapter_id}"),
            json!({ "outline": "OUTLINE-MARKER" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (session_id, _) = sessions::create(
        app.state(),
        CreateSession {
            kind: nekowriter::db::json::SessionKind::ChapterWrite,
            book_id: Some(book_id),
            chapter_id: Some(chapter_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("writer session");
    let kickoff = kickoff_text(app.state(), session_id).await;
    assert!(
        !kickoff.contains("### 本章章纲"),
        "block.memory omits the outline the draft block renders:\n{kickoff}"
    );
    assert_eq!(
        kickoff.matches("OUTLINE-MARKER").count(),
        1,
        "the outline appears exactly once, in block.chapter_draft:\n{kickoff}"
    );
}

// ───────────────────────────── helpers ─────────────────────────────

/// One Demo model profile wired to every agent role, so a session can be created.
async fn configure_models(app: &TestApp) {
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
    assert_eq!(status, StatusCode::CREATED, "create provider: {provider}");
    let provider_id = provider["id"].as_i64().expect("provider id") as i32;
    let (status, profile) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Demo model",
                "model": "demo-model",
                "context_window": 200000,
                "max_output_tokens": 8192,
                "options": {},
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create profile: {profile}");
    let profile_id = profile["id"].as_i64().expect("profile id") as i32;
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": {
                "conception": profile_id, "researcher": profile_id, "writer": profile_id,
                "reviewer": profile_id, "archivist": profile_id,
            }}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "roles: {body}");
}

/// The joined text of a session's kickoff message.
async fn kickoff_text(state: &nekowriter::state::AppState, session_id: i32) -> String {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = state
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT content FROM agent_messages WHERE session_id = ? AND seq = 0",
            vec![session_id.into()],
        ))
        .await
        .expect("query kickoff")
        .expect("kickoff row");
    let raw: String = row.try_get("", "content").expect("content");
    let blocks: Vec<serde_json::Value> = serde_json::from_str(&raw).expect("blocks");
    blocks
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The chapter number of a rendered label (`第12章` → 12).
fn chapter_number(label: &str) -> i32 {
    label
        .trim_start_matches('第')
        .trim_end_matches('章')
        .parse()
        .unwrap_or(i32::MAX)
}

/// Unused-import guard for the appearance role the fixture writes as JSON.
#[allow(dead_code)]
fn _roles() {
    let _ = AppearanceRole::Pov;
    let _ = canon::DETAIL_CHANGE_LIMIT;
    let _ = CanonWrite::author();
}
