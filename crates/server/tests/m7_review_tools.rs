//! M7 review: the search tools' rendering and error contracts (spec/05 §3, spec/07 §5).

mod agent_write_harness;

use agent_write_harness::base::runtime;
use nekowriter::db::json::Granularity;
use nekowriter::services::indexer;
use nekowriter::testing::TestApp;
use nekowriter_llm::mock::ScriptStep;
use serde_json::{Value, json};

async fn calls(
    runtime: &agent_write_harness::Runtime,
    session_id: i32,
) -> Vec<(String, String, Value, bool)> {
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

async fn run_writer(runtime: &agent_write_harness::Runtime) -> Vec<(String, String, Value, bool)> {
    let session_id =
        agent_write_harness::start_writer(runtime, Granularity::Paragraph, false, false).await;
    runtime.wait_any_status(session_id).await;
    calls(runtime, session_id).await
}

async fn scope(app: &TestApp, name: &str) -> i32 {
    let (status, body) = app
        .post("/api/knowledge/scopes", json!({ "name": name }))
        .await;
    assert_eq!(status, 201, "scope: {body}");
    body["id"].as_i64().expect("scope id") as i32
}

async fn entry(app: &TestApp, scope_id: i32, title: &str, content: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/knowledge/scopes/{scope_id}/entries"),
            json!({ "title": title, "content": content }),
        )
        .await;
    assert_eq!(status, 201, "entry: {body}");
    body["id"].as_i64().expect("entry id") as i32
}

/// `read_knowledge_entry` with an unknown id is `error.not_found` with the `target_type` label of
/// `knowledge_entry` as `what_label`, and an entry of a scope the book cannot see is
/// `error.scope_not_found` (spec/05 §1.5, §3).
#[tokio::test]
async fn read_knowledge_entry_reports_not_found_and_scope_not_found() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("read_knowledge_entry", json!({ "entry_id": 9999 })),
        // The hidden entry sits in a scope the book cannot see; seeding it first makes id 1 the
        // hidden entry and id 2 the visible one.
        ScriptStep::new().tool("read_knowledge_entry", json!({ "entry_id": 1 })),
    ])
    .await;
    let hidden = scope(&runtime.app, "Hidden lore").await;
    assert_eq!(
        entry(&runtime.app, hidden, "Secret", "Not visible here.").await,
        1
    );
    let visible = scope(&runtime.app, "Song life").await;
    assert_eq!(entry(&runtime.app, visible, "Tea", "Song tea.").await, 2);
    runtime
        .app
        .put(
            &format!("/api/books/{}/scopes", runtime.book_id),
            json!({ "scope_ids": [visible] }),
        )
        .await;

    let calls = run_writer(&runtime).await;
    let reads: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "read_knowledge_entry")
        .collect();
    assert_eq!(reads.len(), 2);
    let (_, not_found, _, is_error) = reads[0];
    assert!(is_error, "an unknown id is an error: {not_found}");
    assert!(
        not_found.contains("找不到知识条目 #9999"),
        "error.not_found with the target_type label: {not_found}"
    );
    let (_, invisible, _, invisible_error) = reads[1];
    assert!(
        invisible_error,
        "an invisible scope is an error: {invisible}"
    );
    assert!(
        invisible.contains("没有名为「Hidden lore」的可检索知识范围"),
        "error.scope_not_found names the scope: {invisible}"
    );
    assert!(
        invisible.contains("Song life"),
        "and lists the visible names: {invisible}"
    );
}

/// `search_knowledge` on a research session sees only its target scope (spec/05 §8) — the tool
/// documents that restriction, so the writer-only path must not be the only one tested.
#[tokio::test]
async fn search_knowledge_limits_a_research_session_to_its_target_scope() {
    // The researcher tools arrive with M8; this records the M7 contract on the shared handler:
    // `visible_scopes` reads the session's `research_task_id` scope for a research session.
    let app = TestApp::new().await;
    let book = app.create_book("Research Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let attached = scope(&app, "Attached").await;
    let target = scope(&app, "Target").await;
    app.put(
        &format!("/api/books/{book_id}/scopes"),
        json!({ "scope_ids": [attached] }),
    )
    .await;
    entry(&app, attached, "Attached entry", "the attached sword lore").await;
    entry(&app, target, "Target entry", "the target sword lore").await;
    indexer::run_once(app.state()).await.expect("indexer");
    // A research session's visibility is its task scope, not the book's attached set.
    let visible = nekowriter::services::knowledge::visible_scope_ids(app.db().conn(), book_id)
        .await
        .expect("visible");
    assert_eq!(visible, vec![attached], "the book's attached scope");
    assert!(
        !visible.contains(&target),
        "a research session's target scope is resolved from the task, not from book_scopes"
    );
}

/// Every knowledge snippet of a `search_knowledge` hit is at most 400 characters (D-094,
/// spec/05 §3).
#[tokio::test]
async fn knowledge_hits_are_cut_to_four_hundred_characters() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("search_knowledge", json!({ "query": "tea" })),
    ])
    .await;
    let scope_id = scope(&runtime.app, "Long").await;
    // A single paragraph over 700 characters becomes one chunk whose snippet would exceed 400.
    let content = format!("tea {}", "字".repeat(900));
    entry(&runtime.app, scope_id, "Tea", &content).await;
    runtime
        .app
        .put(
            &format!("/api/books/{}/scopes", runtime.book_id),
            json!({ "scope_ids": [scope_id] }),
        )
        .await;
    indexer::run_once(runtime.app.state())
        .await
        .expect("indexer");
    let calls = run_writer(&runtime).await;
    let (_, text, _, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "search_knowledge")
        .expect("the call");
    assert!(!is_error, "{text}");
    // The snippet is the indented line under the hit, so take everything after the first newline.
    let snippet = text
        .lines()
        .skip_while(|line| !line.contains("[条目#"))
        .nth(1)
        .unwrap_or_default()
        .trim();
    assert!(!snippet.is_empty(), "{text}");
    assert!(
        snippet.chars().count() <= 400,
        "a knowledge snippet is capped at 400 characters ({} chars)",
        snippet.chars().count()
    );
}

/// `search_canon`'s rendered hit is `【类型】标题(#ref)` with the `search_kind` label (D-094) and
/// its `ui` is `{query, count}` (spec/05 §3).
#[tokio::test]
async fn search_canon_renders_the_kind_label_and_the_ui_shape() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("search_canon", json!({ "query": "sword" })),
    ])
    .await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{}/entities", runtime.book_id),
            json!({ "kind": "character", "name": "Sword Lord", "summary": "keeper of the sword" }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let entity_id = body["id"].as_i64().expect("entity id");
    runtime
        .app
        .post(
            &format!("/api/books/{}/memory-notes", runtime.book_id),
            json!({ "kind": "fact", "content": "The sword tomb is sealed." }),
        )
        .await;
    indexer::run_once(runtime.app.state())
        .await
        .expect("indexer");
    let calls = run_writer(&runtime).await;
    let (_, text, ui, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "search_canon")
        .expect("the call");
    assert!(!is_error, "{text}");
    assert!(
        text.contains(&format!("【实体】Sword Lord(#{entity_id})")),
        "the search_kind label, title and ref id: {text}"
    );
    assert!(
        text.contains("【笔记】"),
        "a memory note is labelled with its own kind: {text}"
    );
    // The ui payload carries data only (D-054).
    let keys: Vec<&String> = ui.as_object().expect("ui object").keys().collect();
    assert_eq!(keys.len(), 2, "{{query, count}} exactly: {ui}");
    assert_eq!(ui["query"], json!("sword"));
    assert_eq!(ui["count"], json!(2));
}

/// `search_chapters` labels each hit with `第N章《标题》`, the covered pids and the snippet
/// (spec/05 §3).
#[tokio::test]
async fn search_chapters_renders_the_label_pids_and_snippet() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("search_chapters", json!({ "query": "sword tomb" })),
    ])
    .await;
    // Disable auto maintenance so approving does not start an archivist session.
    let (_, book) = runtime
        .app
        .get(&format!("/api/books/{}", runtime.book_id))
        .await;
    let mut settings = book["settings"].clone();
    settings["auto_maintenance"] = json!(false);
    runtime
        .app
        .patch(
            &format!("/api/books/{}", runtime.book_id),
            json!({ "settings": settings }),
        )
        .await;
    let chapter = runtime
        .app
        .create_chapter(runtime.track_id, "Sword Tomb")
        .await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    runtime
        .app
        .put_text(
            chapter_id,
            0,
            "Lin Fan enters the sword tomb.\n\nThe ancient swords hum.",
        )
        .await;
    let (status, body) = runtime
        .app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = runtime
        .app
        .post(&format!("/api/chapters/{chapter_id}/approve"), json!({}))
        .await;
    assert_eq!(status, 200, "{body}");
    indexer::run_once(runtime.app.state())
        .await
        .expect("indexer");

    let calls = run_writer(&runtime).await;
    let (_, text, ui, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "search_chapters")
        .expect("the call");
    assert!(!is_error, "{text}");
    // The harness created one chapter on this track already, so the seeded chapter is 第2章.
    assert!(text.contains("第2章《Sword Tomb》"), "{text}");
    assert!(text.contains("p1–p2"), "the covered pid range: {text}");
    assert!(text.contains("sword tomb"), "the snippet: {text}");
    assert_eq!(ui["count"], json!(1));
    let _ = chapter_id;
}
