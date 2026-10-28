//! End-to-end evidence for M7 (PLAN 7.4): a scope and entries, keyword search working before any
//! embedding, a paraphrase found only through the vector list afterwards, the writer's
//! `search_knowledge` restricted to the attached scopes, the embedding status counts, and the
//! no-orphan invariant of spec/02 §5.11 after deleting a book and a scope.

mod agent_write_harness;

use agent_write_harness::base::runtime;
use nekowriter::services::indexer;
use nekowriter::testing::TestApp;
use nekowriter_llm::mock::ScriptStep;
use serde_json::{Value, json};

async fn count(app: &TestApp, sql: &str) -> i64 {
    use sea_orm::ConnectionTrait;
    let row = app
        .db()
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql,
        ))
        .await
        .expect("count query")
        .expect("count row");
    row.try_get::<i64>("", "c").expect("count value")
}

async fn count_rows(app: &TestApp, table: &str) -> i64 {
    count(app, &format!("SELECT count(*) AS c FROM {table}")).await
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

async fn hits(app: &TestApp, query: &str) -> Vec<Value> {
    let (status, body) = app
        .get(&format!("/api/knowledge/search?q={}", urlencode(query)))
        .await;
    assert_eq!(status, 200, "search: {body}");
    body.as_array().cloned().unwrap_or_default()
}

fn urlencode(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Keyword search works immediately (FTS is written in the entry's transaction); a paraphrase
/// that the keyword list cannot tokenize is found only after the indexer stored its vector.
#[tokio::test]
async fn keyword_then_vector_search_and_the_embedding_status() {
    let app = TestApp::new().await;
    let book = app.create_book("Knowledge Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let history = scope(&app, "Song life").await;
    entry(
        &app,
        history,
        "Tea houses",
        "Song tea houses served tea all day; the ancient sword tomb lies beneath the mountain.",
    )
    .await;
    let other = scope(&app, "Tomb lore").await;
    entry(
        &app,
        other,
        "Tomb spirit",
        "A sword spirit guards the sealed hall.",
    )
    .await;
    let (status, body) = app
        .put(
            &format!("/api/books/{book_id}/scopes"),
            json!({ "scope_ids": [history] }),
        )
        .await;
    assert_eq!(status, 200, "attach scope: {body}");

    // FTS: `ancient` is a token in the indexed text, so it is found before any embedding.
    let keyword = hits(&app, "ancient").await;
    assert_eq!(keyword.len(), 1, "{keyword:?}");
    assert_eq!(keyword[0]["title"], "Tea houses");
    // The paraphrase has no FTS token in the text (the tokenizer sees "swordtomb" as one token).
    assert!(
        hits(&app, "swordtomb").await.is_empty(),
        "the keyword list misses the concatenated paraphrase"
    );

    // The indexer embeds the pending chunks; the same query is then a vector hit.
    let before = app.get("/api/embedding/status").await.1;
    assert!(
        before["pending"].as_i64().unwrap_or(0) >= 1,
        "chunks wait for their vectors: {before}"
    );
    let report = indexer::run_once(app.state()).await.expect("indexer");
    assert!(report.stored >= 2, "{report:?}");
    let after = app.get("/api/embedding/status").await.1;
    assert_eq!(after["state"], "ready");
    assert_eq!(after["pending"], 0, "{after}");
    assert!(after["done"].as_i64().unwrap_or(0) >= 2, "{after}");
    let vector = hits(&app, "swordtomb").await;
    assert!(
        !vector.is_empty(),
        "the paraphrase now matches through its vector"
    );
    assert_eq!(
        vector[0]["title"], "Tea houses",
        "the vector list ranks the matching entry first: {vector:?}"
    );
}

/// A writer session's `search_knowledge` only searches the scopes attached to its book: an entry
/// in an unattached scope is invisible even when it matches the same keyword.
#[tokio::test]
async fn the_writer_only_searches_the_books_attached_scopes() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("search_knowledge", json!({ "query": "tea" })),
    ])
    .await;
    let attached = scope(&runtime.app, "Song life").await;
    let hidden = scope(&runtime.app, "Tomb lore").await;
    entry(
        &runtime.app,
        attached,
        "Tea houses",
        "Song tea houses served tea all day.",
    )
    .await;
    entry(
        &runtime.app,
        hidden,
        "Tea smuggling",
        "Tea smuggling crossed the border at night.",
    )
    .await;
    let (status, body) = runtime
        .app
        .put(
            &format!("/api/books/{}/scopes", runtime.book_id),
            json!({ "scope_ids": [attached] }),
        )
        .await;
    assert_eq!(status, 200, "attach scope: {body}");
    let session_id = agent_write_harness::start_planner(&runtime).await;
    runtime.wait_any_status(session_id).await;
    let session = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await
        .1;
    let calls = session["tool_calls"].as_array().expect("tool calls");
    let call = calls
        .iter()
        .find(|call| call["name"] == "search_knowledge")
        .expect("the search_knowledge call");
    let text = call["result_text"].as_str().unwrap_or_default();
    assert_eq!(call["is_error"], false, "{text}");
    assert!(text.contains("Tea houses"), "{text}");
    assert!(
        !text.contains("Tea smuggling"),
        "an unattached scope is not searched: {text}"
    );
}

/// Deleting a book and a scope leaves no `search_docs`/`fts_*`/`vec_*` row without its source
/// (spec/02 §5.11, D-030).
#[tokio::test]
async fn deleting_a_book_and_a_scope_leaves_no_orphan_index_rows() {
    let app = TestApp::new().await;
    let book = app.create_book("Orphan Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    // One entity document.
    let (status, _) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "Lin Fan", "summary": "the protagonist" }),
        )
        .await;
    assert_eq!(status, 201);
    // One official chapter: summary + passage documents.
    let chapter = app.create_chapter(track_id, "Sword Tomb").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.put_text(chapter_id, 0, "Lin Fan enters the sword tomb.\n\nHe waits.")
        .await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, 200, "submit: {body}");
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/approve"), json!({}))
        .await;
    assert_eq!(status, 200, "approve: {body}");
    app.patch(
        &format!("/api/chapters/{chapter_id}"),
        json!({ "summary": "Lin Fan enters the sword tomb." }),
    )
    .await;
    // One knowledge scope with its chunk.
    let scope_id = scope(&app, "History").await;
    entry(
        &app,
        scope_id,
        "Sword tomb",
        "# Tomb\n\nThe tomb hides a sword spirit.",
    )
    .await;

    indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(count_rows(&app, "search_docs").await, 3);
    assert_eq!(count_rows(&app, "fts_book").await, 3);
    assert_eq!(count_rows(&app, "vec_book").await, 3);
    assert_eq!(count_rows(&app, "knowledge_chunks").await, 1);
    assert_eq!(count_rows(&app, "vec_knowledge").await, 1);

    // Delete the book: every book index row goes with it, the scope stays.
    let (status, _) = app.delete(&format!("/api/books/{book_id}")).await;
    assert_eq!(status, 204);
    assert_eq!(count_rows(&app, "search_docs").await, 0);
    assert_eq!(count_rows(&app, "fts_book").await, 0);
    assert_eq!(count_rows(&app, "vec_book").await, 0);
    assert_eq!(
        count_rows(&app, "knowledge_chunks").await,
        1,
        "the scope is independent"
    );

    // Delete the scope: its chunks, FTS rows and vectors cascade away.
    let (status, _) = app
        .delete(&format!("/api/knowledge/scopes/{scope_id}"))
        .await;
    assert_eq!(status, 204);
    assert_eq!(count_rows(&app, "knowledge_chunks").await, 0);
    assert_eq!(count_rows(&app, "fts_knowledge").await, 0);
    assert_eq!(count_rows(&app, "vec_knowledge").await, 0);

    // No virtual-table row outlives its source (spec/02 §5.11).
    assert_eq!(
        count(
            &app,
            "SELECT count(*) AS c FROM fts_book WHERE rowid NOT IN (SELECT id FROM search_docs)",
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &app,
            "SELECT count(*) AS c FROM vec_book WHERE doc_id NOT IN (SELECT id FROM search_docs)",
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &app,
            "SELECT count(*) AS c FROM fts_knowledge WHERE rowid NOT IN (SELECT id FROM knowledge_chunks)",
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &app,
            "SELECT count(*) AS c FROM vec_knowledge WHERE chunk_id NOT IN (SELECT id FROM knowledge_chunks)",
        )
        .await,
        0
    );
}
