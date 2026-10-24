//! M5 review regressions for the retrieval tools (spec/05 §3, spec/06 §2).
//!
//! - `get_entity` reports appearances "count in the mid-term window" (spec/05 §3). That window is
//!   the one `block.memory` builds (spec/06 §4.1): the last M chapters of P before the session
//!   chapter, topped up from the main story for a side track. The tool used to count only the
//!   side track's own chapters and to include a session chapter that is itself official, so the
//!   card disagreed with the memory the model received.
//! - `read_chapter` accepts the three chapter-reference forms (`"12"`, `"main 12"`, `"#345"`),
//!   rejects a non-official chapter, and an isolated track's chapters are invisible from shared
//!   canon (spec/05 §3).

mod agent_write_harness;

use agent_write_harness::base::runtime;
use nekowriter::db::json::Granularity;
use nekowriter_llm::mock::{ScriptStep, ScriptedProvider};
use serde_json::json;

/// Creates a shared entity through the API and returns its id.
async fn shared_entity(app: &nekowriter::testing::TestApp, book_id: i32, name: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": name, "summary": "shared row" }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_i64().expect("entity id") as i32
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

/// One integer from a fixture query.
async fn scalar_i64(app: &nekowriter::testing::TestApp, sql: &str) -> i64 {
    use sea_orm::ConnectionTrait;
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

/// A side track's entity card counts appearances in the same mid-term window the memory block
/// describes (spec/05 §3, spec/06 §4.1): for its first chapter with an `after_chapter` anchor,
/// that window is the main story's chapters up to the anchor.
#[tokio::test]
async fn get_entity_counts_the_memory_midterm_window_for_a_side_track() {
    let mut runtime = runtime(Vec::new()).await;
    let book_id = runtime.book_id;
    // Main chapter 1 (the harness chapter) and 2, both official; a shared character appears in
    // both. The side story is anchored after chapter 2.
    runtime
        .app
        .set_chapter_status(runtime.chapter_id, "official")
        .await;
    let second = runtime.app.create_chapter(runtime.track_id, "Two").await;
    let second_id = second["id"].as_i64().expect("chapter id") as i32;
    runtime.app.set_chapter_status(second_id, "official").await;
    let shared = shared_entity(&runtime.app, book_id, "Shared One").await;
    for chapter_id in [runtime.chapter_id, second_id] {
        let (status, body) = runtime
            .app
            .put(
                &format!("/api/chapters/{chapter_id}/appearances"),
                json!([{ "entity_id": shared, "role": "major", "note": "" }]),
            )
            .await;
        assert_eq!(status, 200, "{body}");
    }
    runtime
        .app
        .execute(&format!(
            "INSERT INTO tracks(book_id, kind, title, synopsis, canon_mode, anchor_kind, \
             anchor_chapter_id, ord, created_at, updated_at) VALUES ({book_id}, 'side', 'Side', \
             '', 'isolated', 'after_chapter', {second_id}, 1, '2026-01-01T00:00:00Z', \
             '2026-01-01T00:00:00Z')"
        ))
        .await;
    let side_track = scalar_i64(
        &runtime.app,
        &format!("SELECT max(id) AS value FROM tracks WHERE book_id = {book_id}"),
    )
    .await as i32;
    let side_chapter = runtime.app.create_chapter(side_track, "Side one").await;
    let side_chapter_id = side_chapter["id"].as_i64().expect("chapter id") as i32;

    runtime.app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new().tool("get_entity", json!({ "entity": "Shared One" })),
    ]));
    let session_id = agent_write_harness::start_writer_on(
        &runtime,
        side_chapter_id,
        Granularity::Paragraph,
        false,
        false,
        false,
    )
    .await;
    runtime.wait_any_status(session_id).await;
    let calls = tool_calls(&runtime, session_id).await;
    let (_, text, _, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "get_entity")
        .expect("the get_entity call");
    assert!(!is_error, "{text}");
    assert!(
        text.contains("近 10 章出场 2 次"),
        "the count covers the topped-up main-story window (chapters 1 and 2): {text}"
    );
}

/// A side story anchored `before_start` has no main-story item that predates it, so every
/// main-track item is marked `（锚点之后）` (spec/06 §2). The marker compares against the main
/// track, which the tool only looked up for `after_chapter`.
#[tokio::test]
async fn a_before_start_side_track_marks_every_main_story_item() {
    let mut runtime = runtime(Vec::new()).await;
    let book_id = runtime.book_id;
    runtime
        .app
        .set_chapter_status(runtime.chapter_id, "official")
        .await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/timeline"),
            json!({ "title": "Main event" }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    runtime
        .app
        .execute(&format!(
            "UPDATE timeline_events SET chapter_id = {} WHERE book_id = {book_id} AND              title = 'Main event'",
            runtime.chapter_id
        ))
        .await;
    runtime
        .app
        .execute(&format!(
            "INSERT INTO tracks(book_id, kind, title, synopsis, canon_mode, anchor_kind, ord,              created_at, updated_at) VALUES ({book_id}, 'side', 'Past', '', 'isolated',              'before_start', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ))
        .await;
    let side_track = scalar_i64(
        &runtime.app,
        &format!("SELECT max(id) AS value FROM tracks WHERE book_id = {book_id}"),
    )
    .await as i32;
    let side_chapter = runtime.app.create_chapter(side_track, "Side one").await;
    let side_chapter_id = side_chapter["id"].as_i64().expect("chapter id") as i32;

    runtime.app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new().tool("query_timeline", json!({})),
    ]));
    let session_id = agent_write_harness::start_writer_on(
        &runtime,
        side_chapter_id,
        Granularity::Paragraph,
        false,
        false,
        false,
    )
    .await;
    runtime.wait_any_status(session_id).await;
    let calls = tool_calls(&runtime, session_id).await;
    let (_, text, _, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "query_timeline")
        .expect("the query_timeline call");
    assert!(!is_error, "{text}");
    let event = text.find("Main event").expect("the main event");
    assert!(
        text[event..].contains("（锚点之后）"),
        "a before_start side story marks every main-story item: {text}"
    );
}

/// `read_chapter` resolves `"main 12"` and `"#345"`, and a chapter of another isolated track is
/// not visible from a main-track session (spec/05 §3).
#[tokio::test]
async fn read_chapter_resolves_every_reference_form_and_hides_isolated_tracks() {
    let mut runtime = runtime(Vec::new()).await;
    let book_id = runtime.book_id;
    let second = runtime.app.create_chapter(runtime.track_id, "Two").await;
    let second_id = second["id"].as_i64().expect("chapter id") as i32;
    runtime
        .app
        .put_text(second_id, 0, "The second chapter.")
        .await;
    runtime.app.set_chapter_status(second_id, "official").await;
    runtime
        .app
        .execute(&format!(
            "INSERT INTO tracks(book_id, kind, title, synopsis, canon_mode, anchor_kind, ord, \
             created_at, updated_at) VALUES ({book_id}, 'side', 'Side', '', 'isolated', 'none', \
             1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ))
        .await;
    let side_track = scalar_i64(
        &runtime.app,
        &format!("SELECT max(id) AS value FROM tracks WHERE book_id = {book_id}"),
    )
    .await as i32;
    let side_chapter = runtime.app.create_chapter(side_track, "Side one").await;
    let side_chapter_id = side_chapter["id"].as_i64().expect("chapter id") as i32;
    runtime
        .app
        .put_text(side_chapter_id, 0, "A private side chapter.")
        .await;
    runtime
        .app
        .set_chapter_status(side_chapter_id, "official")
        .await;

    runtime.app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new().tool("read_chapter", json!({ "chapter": "main 2" })),
        ScriptStep::new().tool(
            "read_chapter",
            json!({ "chapter": format!("#{second_id}") }),
        ),
        ScriptStep::new().tool(
            "read_chapter",
            json!({ "chapter": format!("#{side_chapter_id}") }),
        ),
    ]));
    let session_id =
        agent_write_harness::start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime.wait_any_status(session_id).await;
    let calls = tool_calls(&runtime, session_id).await;
    let reads: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "read_chapter")
        .collect();
    assert_eq!(reads.len(), 3);
    assert!(!reads[0].3, "main 2 resolves: {}", reads[0].1);
    assert!(reads[0].1.contains("The second chapter."), "{}", reads[0].1);
    assert!(!reads[1].3, "#id resolves: {}", reads[1].1);
    assert!(reads[1].1.contains("The second chapter."), "{}", reads[1].1);
    assert!(
        reads[2].3,
        "another isolated track's chapter is invisible: {}",
        reads[2].1
    );
    assert!(reads[2].1.contains("找不到章节"), "{}", reads[2].1);
}
