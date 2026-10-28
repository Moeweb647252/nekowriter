//! Isolated side-story writes end to end (PLAN M10.2, spec/06 §2, D-014/D-036/D-059) and the
//! anchor markers of the retrieval tools over tracks created by the real API (spec/05 §3).
//!
//! The runtime tests drive a real `chapter_maintenance` session in an isolated track through the
//! Scripted provider and assert the persisted canon: private rows carry the track id, shared rows
//! are untouched, and appearances in the isolated chapter do not move a shared entity's
//! first/last chapter.

mod agent_write_harness;

use std::time::Duration;

use agent_write_harness::base::runtime;
use nekowriter::db::json::{SessionKind, SessionStatus};
use nekowriter_llm::mock::ScriptStep;
use serde_json::json;

/// The calls of the finished session, as `(name, result_text, is_error)`.
async fn calls(
    runtime: &agent_write_harness::Runtime,
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

/// An official, canon-synced main chapter with text.
async fn official_main_chapter(runtime: &agent_write_harness::Runtime, title: &str) -> i32 {
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

/// Creates an isolated side track anchored after `anchor_chapter_id` with one official chapter.
async fn isolated_side_story(
    runtime: &agent_write_harness::Runtime,
    anchor_chapter_id: i32,
) -> (i32, i32) {
    let (status, track) = runtime
        .app
        .post(
            &format!("/api/books/{}/tracks", runtime.book_id),
            json!({ "title": "剑冢旧事", "anchor_chapter_id": anchor_chapter_id }),
        )
        .await;
    assert_eq!(status, 201, "create side track: {track}");
    assert_eq!(track["canon_mode"], "isolated");
    let track_id = track["id"].as_i64().expect("track id") as i32;
    let chapter = runtime.app.create_chapter(track_id, "番外一").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    (track_id, chapter_id)
}

/// Maintenance in an isolated track (spec/06 §2, D-014, D-036):
/// - a shared row update is refused (`error.isolated_readonly`) and leaves the row unchanged;
/// - new rows are private (`track_id = T`);
/// - appearances of shared entities are recorded, but never move a shared entity's
///   `first_chapter_id` / `last_chapter_id`;
/// - the chapter's canon ends `synced`.
#[tokio::test]
async fn isolated_maintenance_writes_private_rows_and_never_touches_shared_canon() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "update_entity",
            json!({ "entity": "林凡", "state": "番外中的状态", "note": "试图改写正史" }),
        ),
        ScriptStep::new().tool(
            "create_entity",
            json!({ "kind": "character", "name": "守墓人", "summary": "番外特有的人物" }),
        ),
        ScriptStep::new().tool(
            "set_relation",
            json!({
                "source": "守墓人",
                "target": "林凡",
                "kind": "看守",
                "inverse_kind": "闯入者",
                "note": "番外中相遇"
            }),
        ),
        ScriptStep::new().tool(
            "record_appearances",
            json!({ "appearances": [
                { "entity": "林凡", "role": "major" },
                { "entity": "守墓人", "role": "pov" }
            ] }),
        ),
        ScriptStep::new().tool(
            "set_chapter_summary",
            json!({ "summary": "番外第一章：林凡在剑冢遇见守墓人。" }),
        ),
        ScriptStep::new().tool("finish_maintenance", json!({ "report": "番外档案完成" })),
    ])
    .await;
    let anchor = official_main_chapter(&runtime, "第一章").await;
    // A shared character whose state the isolated run must not change, with an appearance in the
    // anchor chapter so its bounds are set before the side story exists.
    let (status, hero) = runtime
        .app
        .post(
            &format!("/api/books/{}/entities", runtime.book_id),
            json!({ "kind": "character", "name": "林凡", "summary": "主角", "importance": 5 }),
        )
        .await;
    assert_eq!(status, 201, "{hero}");
    let hero_id = hero["id"].as_i64().expect("id") as i32;
    let (status, body) = runtime
        .app
        .put(
            &format!("/api/chapters/{anchor}/appearances"),
            json!([{ "entity_id": hero_id, "role": "pov", "note": "" }]),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (_, before) = runtime.app.get(&format!("/api/entities/{hero_id}")).await;
    let first_before = before["first_chapter_id"].clone();
    let last_before = before["last_chapter_id"].clone();
    let state_before = before["state"].as_str().unwrap_or_default().to_owned();

    let (track_id, side_chapter_id) = isolated_side_story(&runtime, anchor).await;
    runtime
        .app
        .put_text(side_chapter_id, 0, "番外正文第一段。")
        .await;
    runtime
        .app
        .set_chapter_status(side_chapter_id, "official")
        .await;
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET canon_status = 'pending' WHERE id = {side_chapter_id}"
        ))
        .await;

    let (session_id, _) = nekowriter::services::sessions::create(
        runtime.app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: SessionKind::ChapterMaintenance,
            book_id: Some(runtime.book_id),
            chapter_id: Some(side_chapter_id),
            ..Default::default()
        },
    )
    .await
    .expect("create isolated maintenance");
    runtime
        .wait_status_within(
            session_id,
            SessionStatus::Completed,
            Duration::from_secs(20),
        )
        .await;

    let rows = calls(&runtime, session_id).await;
    let update = rows
        .iter()
        .find(|(name, _, _)| name == "update_entity")
        .expect("update call");
    assert!(
        update.2,
        "a shared entity is read-only in an isolated track"
    );
    assert!(
        update.1.contains("独立番外"),
        "the error explains the isolated rule: {}",
        update.1
    );
    for (name, text, is_error) in rows.iter().filter(|(name, _, _)| name != "update_entity") {
        assert!(!is_error, "{name} failed: {text}");
    }

    // The shared row is untouched.
    let (_, after) = runtime.app.get(&format!("/api/entities/{hero_id}")).await;
    assert_eq!(
        after["state"], state_before,
        "the shared state is unchanged"
    );
    assert_eq!(after["first_chapter_id"], first_before);
    assert_eq!(after["last_chapter_id"], last_before);

    // The private entity belongs to the track, not the shared canon.
    let (status, list) = runtime
        .app
        .get(&format!(
            "/api/books/{}/entities?track_id={track_id}",
            runtime.book_id
        ))
        .await;
    assert_eq!(status, 200);
    let keeper = list
        .as_array()
        .expect("entities")
        .iter()
        .find(|row| row["name"] == json!("守墓人"))
        .expect("the private entity exists");
    assert_eq!(
        keeper["track_id"], track_id,
        "private rows carry the track id"
    );

    // The private relation is visible only through the track: the shared list has none of it.
    let (status, relations) = runtime
        .app
        .get(&format!(
            "/api/books/{}/relations?entity_id={hero_id}",
            runtime.book_id
        ))
        .await;
    assert_eq!(status, 200, "{relations}");
    assert!(
        !relations
            .as_array()
            .expect("relations")
            .iter()
            .any(|row| row["track_id"] == json!(track_id)),
        "the private relation does not appear in the shared view"
    );

    // The chapter's canon is synced and its summary belongs to the track.
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{side_chapter_id}"))
        .await;
    assert_eq!(chapter["canon_status"], "synced");
}

/// The author's canon list adds an isolated track's private rows through `track_id`, and the
/// shared view never shows them (spec/06 §2, the UI's track filter).
#[tokio::test]
async fn the_track_filter_adds_private_rows_to_the_shared_view() {
    let runtime = runtime(Vec::new()).await;
    let anchor = official_main_chapter(&runtime, "第一章").await;
    let (track_id, side_chapter_id) = isolated_side_story(&runtime, anchor).await;

    // One shared and one private character, both created through the author API.
    let (status, shared) = runtime
        .app
        .post(
            &format!("/api/books/{}/entities", runtime.book_id),
            json!({ "kind": "character", "name": "林凡", "summary": "shared" }),
        )
        .await;
    assert_eq!(status, 201, "{shared}");
    let (status, private) = runtime
        .app
        .post(
            &format!("/api/books/{}/entities", runtime.book_id),
            json!({ "kind": "character", "name": "守墓人", "summary": "private", "track_id": track_id }),
        )
        .await;
    assert_eq!(status, 201, "{private}");

    let (status, shared_view) = runtime
        .app
        .get(&format!("/api/books/{}/entities", runtime.book_id))
        .await;
    assert_eq!(status, 200);
    let names: Vec<&str> = shared_view
        .as_array()
        .expect("entities")
        .iter()
        .filter_map(|row| row["name"].as_str())
        .collect();
    assert!(names.contains(&"林凡"), "shared rows are always visible");
    assert!(
        !names.contains(&"守墓人"),
        "private rows are hidden by default"
    );

    let (status, track_view) = runtime
        .app
        .get(&format!(
            "/api/books/{}/entities?track_id={track_id}",
            runtime.book_id
        ))
        .await;
    assert_eq!(status, 200, "{track_view}");
    let names: Vec<&str> = track_view
        .as_array()
        .expect("entities")
        .iter()
        .filter_map(|row| row["name"].as_str())
        .collect();
    assert!(
        names.contains(&"林凡") && names.contains(&"守墓人"),
        "the track filter adds the track's private rows: {names:?}"
    );
    let _ = side_chapter_id;
}

/// The retrieval tools mark main-story items after a side story's anchor
/// (`（锚点之后）`, spec/05 §3, spec/06 §2): `get_entity`'s change notes and state, and
/// `search_chapters`' passage hits, over a side track created by the real tracks API.
#[tokio::test]
async fn the_retrieval_tools_mark_items_after_the_anchor() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("get_entity", json!({ "entity": "林凡" })),
        ScriptStep::new().tool("query_timeline", json!({})),
        ScriptStep::new().tool(
            "submit_outline",
            json!({ "outline": "番外细纲", "reference_notes": "- 要点" }),
        ),
    ])
    .await;
    let app = &runtime.app;
    let book_id = runtime.book_id;
    let first = official_main_chapter(&runtime, "第一章").await;
    let second = official_main_chapter(&runtime, "第二章").await;

    // A shared character whose state changed once before and once after the anchor.
    let (status, hero) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "林凡", "summary": "主角", "importance": 5 }),
        )
        .await;
    assert_eq!(status, 201, "{hero}");
    let hero_id = hero["id"].as_i64().expect("id") as i32;
    for (chapter_id, state, note) in [
        (first, "身处外门", "第一章后"),
        (second, "身处剑冢", "第二章后"),
    ] {
        let (status, body) = app
            .patch(
                &format!("/api/entities/{hero_id}"),
                json!({ "state": state, "note": note }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        app.execute(&format!(
            "UPDATE change_log SET chapter_id = {chapter_id} WHERE id = \
             (SELECT max(id) FROM change_log WHERE target_type = 'entity' AND target_id = {hero_id})"
        ))
        .await;
    }

    let (status, track) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "title": "剑冢旧事", "anchor_chapter_id": first }),
        )
        .await;
    assert_eq!(status, 201, "{track}");
    let track_id = track["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "番外一").await;
    let chapter_id = chapter["id"].as_i64().expect("id") as i32;

    // A timeline event narrated in the chapter after the anchor.
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/timeline"),
            json!({ "title": "After the anchor event" }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    app.execute(&format!(
        "UPDATE timeline_events SET chapter_id = {second} WHERE book_id = {book_id} \
         AND title = 'After the anchor event'"
    ))
    .await;

    let (session_id, _) = nekowriter::services::sessions::create(
        app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: SessionKind::ChapterOutline,
            book_id: Some(book_id),
            chapter_id: Some(chapter_id),
            pause_after_outline: Some(false),
            ..Default::default()
        },
    )
    .await
    .expect("create side-track writer");
    runtime.wait_any_status(session_id).await;
    let rows = calls(&runtime, session_id).await;
    let (_, text, is_error) = rows
        .iter()
        .find(|(name, _, _)| name == "get_entity")
        .expect("the get_entity call");
    assert!(!is_error, "{text}");
    assert!(
        text.contains("当前状态（锚点之后）"),
        "a state changed after the anchor is marked (spec/06 §4.2): {text}"
    );
    assert!(
        text.contains("（锚点之后）"),
        "changes after the anchor are marked: {text}"
    );
    let note_line = |needle: &str| -> &str {
        text.lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("the {needle} line: {text}"))
    };
    assert!(
        !note_line("第一章后").contains("（锚点之后）"),
        "the change before the anchor is not marked: {text}"
    );
    assert!(
        note_line("第二章后").contains("（锚点之后）"),
        "the change after the anchor is marked: {text}"
    );

    // `query_timeline` marks events narrated after the anchor too (spec/05 §3).
    let (_, timeline_text, timeline_error) = rows
        .iter()
        .find(|(name, _, _)| name == "query_timeline")
        .unwrap_or_else(|| {
            panic!(
                "the query_timeline call is missing; calls: {:?}",
                rows.iter().map(|row| &row.0).collect::<Vec<_>>()
            )
        });
    assert!(!timeline_error, "{timeline_text}");
    let event_line = timeline_text
        .lines()
        .find(|line| line.contains("After the anchor event"))
        .unwrap_or_else(|| panic!("the timeline event is missing: {timeline_text}"));
    assert!(
        event_line.contains("（锚点之后）"),
        "the event after the anchor is marked: {timeline_text}"
    );
}

/// `search_chapters` marks main-story passages that lie after a side story's anchor
/// (spec/05 §3), through the real tracks API and the passage index.
#[tokio::test]
async fn search_chapters_marks_passages_after_the_anchor() {
    let mut runtime = runtime(vec![
        ScriptStep::new().tool("search_chapters", json!({ "query": "tomb" })),
        ScriptStep::new().tool(
            "submit_outline",
            json!({ "outline": "番外细纲", "reference_notes": "- 要点" }),
        ),
    ])
    .await;
    let first = official_main_chapter(&runtime, "第一章").await;
    let second = official_main_chapter(&runtime, "第二章").await;
    let app = &runtime.app;
    // Distinctive English text per chapter so one keyword finds both passages (the manual
    // `search_docs` rows bypass the jieba segmentation the indexer applies to CJK text).
    app.put_text(first, 1, "Outside the tomb the wind howls.")
        .await;
    app.put_text(second, 1, "Inside the tomb the fire rises.")
        .await;
    // The approval flow already indexed the chapters; replace the docs with the edited text so
    // the query finds both passages deterministically.
    app.execute(&format!(
        "DELETE FROM search_docs WHERE book_id = {0} AND kind = 'passage'; \
         INSERT INTO search_docs(book_id, track_id, kind, ref_id, title, body, meta, content_hash, updated_at) \
         SELECT {0}, NULL, 'passage', {1}, 'x', \
                group_concat(json_extract(value, '$.text'), ' '), '{{\"pids\":[\"p1\"]}}', 'hash-{1}', '2026-01-01T00:00:00Z' \
         FROM json_each((SELECT content FROM chapters WHERE id = {1}), '$.paragraphs'); \
         INSERT INTO search_docs(book_id, track_id, kind, ref_id, title, body, meta, content_hash, updated_at) \
         SELECT {0}, NULL, 'passage', {2}, 'x', \
                group_concat(json_extract(value, '$.text'), ' '), '{{\"pids\":[\"p1\"]}}', 'hash-{2}', '2026-01-01T00:00:00Z' \
         FROM json_each((SELECT content FROM chapters WHERE id = {2}), '$.paragraphs'); \
         INSERT INTO fts_book(rowid, title, body) \
         SELECT id, title, body FROM search_docs WHERE book_id = {0} AND kind = 'passage'",
        runtime.book_id, first, second
    ))
    .await;

    runtime.app.without_embedder();
    let app = &runtime.app;
    // A side story anchored after chapter 1: chapter 2's passage is after the anchor.
    let (status, track) = app
        .post(
            &format!("/api/books/{}/tracks", runtime.book_id),
            json!({ "title": "剑冢旧事", "anchor_chapter_id": first }),
        )
        .await;
    assert_eq!(status, 201, "{track}");
    let track_id = track["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "番外一").await;
    let chapter_id = chapter["id"].as_i64().expect("id") as i32;

    let (session_id, _) = nekowriter::services::sessions::create(
        app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: SessionKind::ChapterOutline,
            book_id: Some(runtime.book_id),
            chapter_id: Some(chapter_id),
            pause_after_outline: Some(false),
            ..Default::default()
        },
    )
    .await
    .expect("create side-track writer");
    runtime.wait_any_status(session_id).await;
    let rows = calls(&runtime, session_id).await;
    let (_, text, is_error) = rows
        .iter()
        .find(|(name, _, _)| name == "search_chapters")
        .expect("the search_chapters call");
    assert!(!is_error, "{text}");
    let outside = text
        .find("Outside the tomb")
        .unwrap_or_else(|| panic!("the pre-anchor passage is missing: {text}"));
    let _inside = text
        .find("Inside the tomb")
        .unwrap_or_else(|| panic!("the post-anchor passage is missing: {text}"));
    let _ = outside;
    // Each hit is a header line (label, pids, marker) followed by its snippet line.
    let lines: Vec<&str> = text.lines().collect();
    let marked_line = lines
        .iter()
        .position(|line| line.contains("Inside the tomb"))
        .unwrap_or_else(|| panic!("the post-anchor passage is missing: {text}"));
    assert!(
        lines[marked_line - 1].contains("（锚点之后）"),
        "the passage after the anchor is marked: {text}"
    );
    let unmarked_line = lines
        .iter()
        .position(|line| line.contains("Outside the tomb"))
        .unwrap_or_else(|| panic!("the pre-anchor passage is missing: {text}"));
    assert!(
        !lines[unmarked_line - 1].contains("（锚点之后）"),
        "the passage before the anchor is not marked: {text}"
    );
}
