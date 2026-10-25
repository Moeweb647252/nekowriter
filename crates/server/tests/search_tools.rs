//! The four search tools (PLAN 7.5, spec/05 §3): results and `ui` payloads, scope restriction and
//! errors, offset continuation, passage labels and the visibility rules of scopes and tracks.

mod agent_write_harness;

use agent_write_harness::base::runtime;
use nekowriter::db::json::{Granularity, Phase};
use nekowriter::search::{BookCorpus, BookSearch, hybrid_book};
use nekowriter::services::indexer;
use nekowriter::testing::TestApp;
use nekowriter_llm::mock::ScriptStep;
use serde_json::{Value, json};

/// The calls of the finished session, as `(name, result_text, ui, is_error)`.
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

/// Starts a writer session in `planning`, runs the scripted calls and returns the tool calls.
async fn run_writer(runtime: &agent_write_harness::Runtime) -> Vec<(String, String, Value, bool)> {
    let session_id =
        agent_write_harness::start_writer(runtime, Granularity::Paragraph, false, false).await;
    runtime.wait_any_status(session_id).await;
    calls(runtime, session_id).await
}

async fn entity(runtime: &agent_write_harness::Runtime, name: &str, summary: &str) -> i32 {
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{}/entities", runtime.book_id),
            json!({ "kind": "character", "name": name, "summary": summary, "importance": 3 }),
        )
        .await;
    assert_eq!(status, 201, "create entity: {body}");
    body["id"].as_i64().expect("id") as i32
}

async fn disable_auto_maintenance(app: &TestApp, book_id: i32) {
    let (_, book) = app.get(&format!("/api/books/{book_id}")).await;
    let mut settings = book["settings"].clone();
    settings["auto_maintenance"] = json!(false);
    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "settings": settings }),
        )
        .await;
    assert_eq!(status, 200, "book settings: {body}");
}

/// Creates an official chapter on a track and returns its id.
async fn official_chapter(
    app: &TestApp,
    book_id: i32,
    track_id: i32,
    title: &str,
    text: &str,
) -> i32 {
    disable_auto_maintenance(app, book_id).await;
    let chapter = app.create_chapter(track_id, title).await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.put_text(chapter_id, 0, text).await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, 200, "submit: {body}");
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/approve"), json!({}))
        .await;
    assert_eq!(status, 200, "approve: {body}");
    chapter_id
}

async fn create_scope(app: &TestApp, name: &str) -> i32 {
    let (status, body) = app
        .post("/api/knowledge/scopes", json!({ "name": name }))
        .await;
    assert_eq!(status, 201, "create scope: {body}");
    body["id"].as_i64().expect("scope id") as i32
}

async fn create_entry(app: &TestApp, scope_id: i32, title: &str, content: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/knowledge/scopes/{scope_id}/entries"),
            json!({ "title": title, "content": content }),
        )
        .await;
    assert_eq!(status, 201, "create entry: {body}");
    body["id"].as_i64().expect("entry id") as i32
}

async fn attach_scopes(app: &TestApp, book_id: i32, scope_ids: Vec<i32>) {
    let (status, body) = app
        .put(
            &format!("/api/books/{book_id}/scopes"),
            json!({ "scope_ids": scope_ids }),
        )
        .await;
    assert_eq!(status, 200, "attach scopes: {body}");
}

#[tokio::test]
async fn search_canon_returns_the_archive_hits_and_ui() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("search_canon", json!({ "query": "Lin Fan" })),
    ])
    .await;
    entity(&runtime, "Lin Fan", "an outer disciple of the sword sect").await;
    indexer::run_once(runtime.app.state())
        .await
        .expect("indexer");
    let calls = run_writer(&runtime).await;
    let (name, text, ui, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "search_canon")
        .expect("the search_canon call");
    assert_eq!(name, "search_canon");
    assert!(!is_error, "the search succeeded: {text}");
    assert!(
        text.contains("【实体】Lin Fan(#"),
        "the kind label and the hit: {text}"
    );
    assert_eq!(ui["query"], json!("Lin Fan"));
    assert_eq!(ui["count"], json!(1));
}

#[tokio::test]
async fn search_canon_honours_the_kind_filter_and_the_limit() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "search_canon",
            json!({ "query": "sword", "kinds": ["entity"] }),
        ),
        ScriptStep::new().tool("search_canon", json!({ "query": "sword", "limit": 1 })),
    ])
    .await;
    entity(&runtime, "Sword Sect", "an ancient sect of the sword").await;
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
    let searches: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "search_canon")
        .collect();
    assert_eq!(searches.len(), 2);
    let (_, filtered, filtered_ui, filtered_error) = searches[0];
    assert!(!filtered_error, "{filtered}");
    assert!(filtered.contains("【实体】"), "{filtered}");
    assert!(
        !filtered.contains("【笔记】"),
        "the kind filter excludes notes: {filtered}"
    );
    assert_eq!(filtered_ui["count"], json!(1));

    let (_, limited, limited_ui, limited_error) = searches[1];
    assert!(!limited_error, "{limited}");
    assert_eq!(
        limited_ui["count"],
        json!(1),
        "limit 1 returns one hit: {limited}"
    );
}

#[tokio::test]
async fn search_knowledge_without_a_visible_scope_is_an_error() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("search_knowledge", json!({ "query": "tea" })),
    ])
    .await;
    let calls = run_writer(&runtime).await;
    let (_, text, _, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "search_knowledge")
        .expect("the search_knowledge call");
    assert!(is_error, "no visible scope is an error");
    assert!(text.contains("本书没有可检索的知识库范围"), "{text}");
}

#[tokio::test]
async fn search_knowledge_honours_scope_names_and_restricts_to_them() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("search_knowledge", json!({ "query": "tea" })),
        ScriptStep::new().tool(
            "search_knowledge",
            json!({ "query": "tea", "scope": "Nope" }),
        ),
    ])
    .await;
    // Two attached scopes; the query matches only the first one's entry.
    let history = create_scope(&runtime.app, "Song life").await;
    let lore = create_scope(&runtime.app, "Tomb lore").await;
    create_entry(
        &runtime.app,
        history,
        "Tea houses",
        "Song tea houses served tea all day.",
    )
    .await;
    create_entry(
        &runtime.app,
        lore,
        "Sword tomb",
        "The sword tomb hides a spirit.",
    )
    .await;
    attach_scopes(&runtime.app, runtime.book_id, vec![history, lore]).await;
    indexer::run_once(runtime.app.state())
        .await
        .expect("indexer");
    let calls = run_writer(&runtime).await;
    let searches: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "search_knowledge")
        .collect();
    assert_eq!(searches.len(), 2);
    let (_, found, ui, is_error) = searches[0];
    assert!(!is_error, "the attached scope is searchable: {found}");
    assert!(found.contains("[条目#"), "{found}");
    assert!(found.contains("Tea houses"), "{found}");
    assert!(
        found.contains("Song life"),
        "the scope name is shown: {found}"
    );
    let first_hit = found
        .lines()
        .find(|line| line.contains("[条目#"))
        .expect("a hit line");
    assert!(
        first_hit.contains("Tea houses"),
        "the keyword hit ranks first: {found}"
    );
    assert!(ui["count"].as_i64().unwrap_or(0) >= 1);
    let (_, unknown, _, unknown_error) = searches[1];
    assert!(unknown_error, "the unknown scope name is an error");
    assert!(
        unknown.contains("没有名为「Nope」的可检索知识范围"),
        "{unknown}"
    );
    assert!(
        unknown.contains("Song life"),
        "the visible names are listed: {unknown}"
    );
}

#[tokio::test]
async fn read_knowledge_entry_pages_from_an_offset() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("read_knowledge_entry", json!({ "entry_id": 1 })),
        ScriptStep::new().tool(
            "read_knowledge_entry",
            json!({ "entry_id": 1, "offset": 6000 }),
        ),
        ScriptStep::new().tool("read_knowledge_entry", json!({ "entry_id": 2 })),
    ])
    .await;
    let scope = create_scope(&runtime.app, "Long form").await;
    let long = "字".repeat(7_000);
    create_entry(&runtime.app, scope, "Long entry", &long).await;
    attach_scopes(&runtime.app, runtime.book_id, vec![scope]).await;
    let other = create_scope(&runtime.app, "Private").await;
    create_entry(
        &runtime.app,
        other,
        "Hidden entry",
        "Not attached to this book.",
    )
    .await;
    let calls = run_writer(&runtime).await;
    let reads: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "read_knowledge_entry")
        .collect();
    assert_eq!(reads.len(), 3);
    let (_, first, ui, is_error) = reads[0];
    assert!(!is_error, "{first}");
    assert!(first.contains("[条目#1]《Long entry》"), "{first}");
    assert!(first.contains("第 0–6000 字，共 7000 字"), "{first}");
    assert!(first.contains("offset=6000"), "{first}");
    assert_eq!(ui["entry_id"], json!(1));
    // The second page is the tail: it carries no continuation hint.
    let (_, second, _, second_error) = reads[1];
    assert!(!second_error, "{second}");
    assert!(!second.contains("继续阅读"), "{second}");
    // An entry of an invisible scope cannot be read (spec/05 §3).
    let (_, _, _, hidden_error) = reads[2];
    assert!(hidden_error, "an invisible scope is an error");
}

#[tokio::test]
async fn search_chapters_returns_passages_with_the_chapter_label_and_pids() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("search_chapters", json!({ "query": "sword tomb" })),
    ])
    .await;
    let chapter = official_chapter(
        &runtime.app,
        runtime.book_id,
        runtime.track_id,
        "Sword Tomb",
        "Lin Fan enters the sword tomb.\n\nThe ancient swords hum in the dark.",
    )
    .await;
    indexer::run_once(runtime.app.state())
        .await
        .expect("indexer");
    let calls = run_writer(&runtime).await;
    let (_, text, ui, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "search_chapters")
        .expect("the search_chapters call");
    assert!(!is_error, "{text}");
    let (_, chapter_dto) = runtime.app.get(&format!("/api/chapters/{chapter}")).await;
    let number = chapter_dto["number"].as_i64().expect("chapter number");
    assert!(
        text.contains(&format!("第{number}章《Sword Tomb》")),
        "{text}"
    );
    assert!(text.contains("p1–p2"), "the covered pid range: {text}");
    assert!(text.contains("sword tomb"), "the snippet: {text}");
    assert_eq!(ui["count"], json!(1));
}

#[tokio::test]
async fn the_search_tools_fall_back_to_keywords_without_an_embedder() {
    let mut runtime = runtime(vec![
        ScriptStep::new().tool("search_canon", json!({ "query": "Lin Fan" })),
    ])
    .await;
    entity(&runtime, "Lin Fan", "the protagonist").await;
    runtime.app.without_embedder();
    let calls = run_writer(&runtime).await;
    let (_, text, _, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "search_canon")
        .expect("the call");
    assert!(!is_error, "{text}");
    assert!(text.contains("【实体】Lin Fan(#"), "{text}");
    assert!(
        text.contains("本次只进行了关键词匹配"),
        "the result says vectors were unavailable: {text}"
    );
}

/// Private documents of an isolated track are invisible to the main track and visible when the
/// search passes the track (spec/06 §2, D-014).
#[tokio::test]
async fn isolated_track_documents_are_filtered_by_visibility() {
    let app = TestApp::new().await;
    let book = app.create_book("Visibility Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    // Side tracks are created by M10; the search filter only needs the rows, so seed them.
    app.execute(&format!(
        "INSERT INTO tracks(id, book_id, kind, title, canon_mode, ord, created_at, updated_at) \
         VALUES (900, {book_id}, 'side', 'Side story', 'isolated', 1, \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'); \
         INSERT INTO chapters(id, book_id, track_id, ord, title, status, content, created_at, updated_at) \
         VALUES (900, {book_id}, 900, 1, 'Side One', 'official', \
                 '{{\"paragraphs\":[{{\"pid\":\"p1\",\"text\":\"The secret keeper waits.\"}}],\"next_pid\":2,\"tail_open\":false}}', \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'); \
         INSERT INTO search_docs(id, book_id, track_id, kind, ref_id, title, body, content_hash, updated_at) \
         VALUES (900, {book_id}, 900, 'passage', 900, 'Side One', 'The secret keeper waits.', 'hash-passage', \
                 '2026-01-01T00:00:00Z'), \
                (901, {book_id}, 900, 'entity', 42, 'Secret Keeper', 'the secret keeper of the side story', 'hash-entity', \
                 '2026-01-01T00:00:00Z'); \
         INSERT INTO fts_book(rowid, title, body) VALUES \
           (900, 'side one', 'secret keeper waits'), \
           (901, 'secret keeper', 'the secret keeper of the side story')"
    ))
    .await;

    let main = hybrid_book(
        app.db().conn(),
        "secret keeper",
        None,
        &BookSearch {
            book_id,
            corpus: BookCorpus::Canon,
            kinds: None,
            isolated_track: None,
            limit: 10,
        },
    )
    .await
    .expect("canon");
    assert!(main.hits.is_empty(), "{main:?}");
    let main_passages = hybrid_book(
        app.db().conn(),
        "secret keeper",
        None,
        &BookSearch {
            book_id,
            corpus: BookCorpus::Passage,
            kinds: None,
            isolated_track: None,
            limit: 10,
        },
    )
    .await
    .expect("passages");
    assert!(main_passages.hits.is_empty(), "{main_passages:?}");

    let side_canon = hybrid_book(
        app.db().conn(),
        "secret keeper",
        None,
        &BookSearch {
            book_id,
            corpus: BookCorpus::Canon,
            kinds: None,
            isolated_track: Some(900),
            limit: 10,
        },
    )
    .await
    .expect("canon");
    assert!(
        side_canon.hits.iter().all(|hit| hit.track_id == Some(900)),
        "{side_canon:?}"
    );
    let side_passages = hybrid_book(
        app.db().conn(),
        "secret keeper",
        None,
        &BookSearch {
            book_id,
            corpus: BookCorpus::Passage,
            kinds: None,
            isolated_track: Some(900),
            limit: 10,
        },
    )
    .await
    .expect("passages");
    assert_eq!(side_passages.hits.len(), 1, "{side_passages:?}");
}

/// The phase matrix keeps the search tools out of the drafting phase (spec/03 §1.2):
/// `search_canon` is a retrieval tool, so a drafting call is rejected without effect.
#[tokio::test]
async fn search_tools_are_permitted_in_planning_only() {
    use nekowriter::agent::tools::{WRITER_TOOLS, is_permitted};
    let frozen: Vec<String> = WRITER_TOOLS.iter().map(|name| (*name).to_owned()).collect();
    for tool in ["search_canon", "search_knowledge", "search_chapters"] {
        assert!(is_permitted(
            nekowriter::db::json::SessionKind::ChapterWrite,
            Phase::Planning,
            tool,
            &frozen
        ));
        assert!(!is_permitted(
            nekowriter::db::json::SessionKind::ChapterWrite,
            Phase::Drafting,
            tool,
            &frozen
        ));
    }
}
