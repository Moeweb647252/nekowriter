//! Knowledge base services, API and index rows (PLAN 7.4, spec/07 §1, spec/10 §2.5):
//! scope/entry CRUD with unique names, chunking in the same transaction as the change, cursor
//! pagination, hybrid search, book visibility, `knowledge.changed` events and the cascade that
//! leaves no orphan index row (spec/02 §5.11).

use axum::http::StatusCode;
use nekowriter::db::entities::knowledge_chunks;
use nekowriter::events::ServerEvent;
use nekowriter::services::indexer;
use nekowriter::testing::{TestApp, assert_error, assert_error_detail};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

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

async fn create_scope(app: &TestApp, name: &str) -> i32 {
    let (status, body) = app
        .post(
            "/api/knowledge/scopes",
            serde_json::json!({ "name": name, "description": "period detail" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create scope: {body}");
    body["id"].as_i64().expect("scope id") as i32
}

async fn create_entry(app: &TestApp, scope_id: i32, title: &str, content: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/knowledge/scopes/{scope_id}/entries"),
            serde_json::json!({
                "title": title,
                "content": content,
                "source_urls": ["https://example.com/song"],
                "tags": ["song"],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create entry: {body}");
    body["id"].as_i64().expect("entry id") as i32
}

#[tokio::test]
async fn scope_and_entry_crud_with_chunking_in_the_same_transaction() {
    let app = TestApp::new().await;
    let scope_id = create_scope(&app, "Song dynasty").await;

    // Unique scope names are `conflict.exists` with the existing id (spec/10 §4).
    let (status, body) = app
        .post(
            "/api/knowledge/scopes",
            serde_json::json!({ "name": "Song dynasty" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&body, "conflict.exists");
    assert_error_detail(&body, "conflict.exists", "id");

    let content = "# 宋代服饰\n\n宋代男子常穿圆领袍。\n\n## 男子常服\n\n颜色以青、白为主。";
    let entry_id = create_entry(&app, scope_id, "Song dress", content).await;

    // The chunks and their FTS rows exist immediately: keyword search works before any embedding
    // (spec/07 §1.1, §2).
    let chunks = knowledge_chunks::Entity::find()
        .filter(knowledge_chunks::Column::EntryId.eq(entry_id))
        .all(app.db().conn())
        .await
        .expect("chunks");
    assert_eq!(chunks.len(), 2, "{chunks:?}");
    assert_eq!(chunks[0].embed_state, "pending");
    assert!(
        count(
            &app,
            "SELECT count(*) AS c FROM fts_knowledge WHERE fts_knowledge MATCH '\"男子\"'",
        )
        .await
            >= 1
    );
    // The scope shows its entry count.
    let (status, scopes) = app.get("/api/knowledge/scopes").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(scopes[0]["entry_count"], 1);

    // Duplicate titles within a scope are `conflict.exists` too; another scope may reuse them.
    let (status, body) = app
        .post(
            &format!("/api/knowledge/scopes/{scope_id}/entries"),
            serde_json::json!({ "title": "Song dress", "content": "other" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&body, "conflict.exists");
    let other_scope = create_scope(&app, "Tang dynasty").await;
    create_entry(
        &app,
        other_scope,
        "Song dress",
        "Another scope may reuse a title.",
    )
    .await;

    let (status, entry) = app.get(&format!("/api/knowledge/entries/{entry_id}")).await;
    assert_eq!(status, StatusCode::OK, "{entry}");
    assert_eq!(entry["source"], "manual");
    assert_eq!(entry["source_urls"][0], "https://example.com/song");
    assert_eq!(entry["tags"][0], "song");

    // An edit re-chunks in the same transaction; the second section changed, the first did not.
    let first = chunks[0].clone();
    let (status, body) = app
        .patch(
            &format!("/api/knowledge/entries/{entry_id}"),
            serde_json::json!({ "content": "# 宋代服饰\n\n宋代男子常穿圆领袍。\n\n## 男子常服\n\n颜色以黑、红为主。" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let chunks_after = knowledge_chunks::Entity::find()
        .filter(knowledge_chunks::Column::EntryId.eq(entry_id))
        .all(app.db().conn())
        .await
        .expect("chunks");
    let kept = chunks_after
        .iter()
        .find(|chunk| chunk.id == first.id)
        .expect("an unchanged chunk keeps its row (D-039)");
    assert_eq!(kept.content_hash, first.content_hash);

    // Renaming updates the FTS title of every chunk.
    let (status, body) = app
        .patch(
            &format!("/api/knowledge/entries/{entry_id}"),
            serde_json::json!({ "title": "Song clothing" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        count(
            &app,
            "SELECT count(*) AS c FROM fts_knowledge WHERE fts_knowledge MATCH '\"clothing\"'",
        )
        .await
            >= 1
    );

    // Deleting the entry cascades its chunks and FTS rows.
    let (status, _) = app
        .delete(&format!("/api/knowledge/entries/{entry_id}"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        knowledge_chunks::Entity::find()
            .filter(knowledge_chunks::Column::EntryId.eq(entry_id))
            .count(app.db().conn())
            .await
            .expect("count"),
        0
    );
    assert_eq!(count_rows(&app, "fts_knowledge").await, 1); // the other scope's entry
}

#[tokio::test]
async fn entry_pages_use_an_id_cursor_and_a_text_filter() {
    let app = TestApp::new().await;
    let scope_id = create_scope(&app, "History").await;
    for index in 0..3 {
        create_entry(
            &app,
            scope_id,
            &format!("Entry {index}"),
            &format!("Body {index}"),
        )
        .await;
    }
    let (status, page) = app
        .get(&format!("/api/knowledge/scopes/{scope_id}/entries?limit=2"))
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["items"].as_array().expect("items").len(), 2);
    let cursor = page["next_cursor"].as_str().expect("next cursor");
    let (status, second) = app
        .get(&format!(
            "/api/knowledge/scopes/{scope_id}/entries?limit=2&cursor={cursor}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["items"].as_array().expect("items").len(), 1);
    assert!(second["next_cursor"].is_null());

    let (status, filtered) = app
        .get(&format!(
            "/api/knowledge/scopes/{scope_id}/entries?q=Body+1"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{filtered}");
    let items = filtered["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Entry 1");
}

#[tokio::test]
async fn deleting_a_scope_removes_entries_chunks_and_index_rows() {
    let app = TestApp::new().await;
    let scope_id = create_scope(&app, "History").await;
    create_entry(
        &app,
        scope_id,
        "Song dress",
        "# Song\n\nRound-collar robes.",
    )
    .await;
    indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(count_rows(&app, "vec_knowledge").await, 1);
    assert_eq!(count_rows(&app, "fts_knowledge").await, 1);

    let (status, _) = app
        .delete(&format!("/api/knowledge/scopes/{scope_id}"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(count_rows(&app, "knowledge_entries").await, 0);
    assert_eq!(
        knowledge_chunks::Entity::find()
            .count(app.db().conn())
            .await
            .expect("chunks"),
        0
    );
    assert_eq!(count_rows(&app, "fts_knowledge").await, 0);
    assert_eq!(count_rows(&app, "vec_knowledge").await, 0);
}

#[tokio::test]
async fn knowledge_search_returns_hybrid_hits_with_snippets() {
    let mut app = TestApp::new().await;
    let history = create_scope(&app, "History").await;
    let lore = create_scope(&app, "Tomb lore").await;
    create_entry(
        &app,
        history,
        "Song dress",
        "宋代男子常穿圆领袍，颜色以青、白为主。",
    )
    .await;
    create_entry(
        &app,
        lore,
        "Sword tomb",
        "The sword tomb hides an ancient spirit.",
    )
    .await;
    indexer::run_once(app.state()).await.expect("indexer");

    // The exact Chinese term is found through the keyword list, and the fused result puts it
    // first with a snippet from the original text (spec/07 §5).
    let (status, hits) = app
        .get("/api/knowledge/search?q=%E5%9C%86%E9%A2%86%E8%A2%8D")
        .await;
    assert_eq!(status, StatusCode::OK, "{hits}");
    let hits = hits.as_array().expect("hits");
    assert!(!hits.is_empty(), "{hits:?}");
    assert_eq!(hits[0]["title"], "Song dress");
    assert_eq!(hits[0]["scope_name"], "History");
    assert!(
        hits[0]["snippet"].as_str().unwrap_or("").contains("圆领袍"),
        "the snippet quotes the matched text: {hits:?}"
    );

    // The scope filter narrows the search; an empty filter searches every scope.
    let (status, hits) = app
        .get(&format!("/api/knowledge/search?q=sword&scope_ids={lore}"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hits.as_array().expect("hits").len(), 1, "{hits}");
    let (status, hits) = app.get("/api/knowledge/search?q=sword").await;
    assert_eq!(status, StatusCode::OK);
    let all = hits.as_array().expect("hits");
    assert_eq!(
        all.len(),
        2,
        "the vector list also returns the near other scope: {hits}"
    );
    assert_eq!(all[0]["title"], "Sword tomb");

    // Without an embedder the endpoint still answers from the keyword list (spec/07 §5).
    app.without_embedder();
    let (status, hits) = app.get("/api/knowledge/search?q=sword").await;
    assert_eq!(status, StatusCode::OK, "{hits}");
    assert_eq!(hits.as_array().expect("hits").len(), 1);

    // An empty query is an empty page, not an error.
    let (status, hits) = app.get("/api/knowledge/search?q=").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hits.as_array().expect("hits").len(), 0);
}

#[tokio::test]
async fn book_scopes_attach_and_detach_visibility() {
    let app = TestApp::new().await;
    let book = app.create_book("Knowledge Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let first = create_scope(&app, "History").await;
    let second = create_scope(&app, "Lore").await;

    let (status, scopes) = app.get(&format!("/api/books/{book_id}/scopes")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(scopes.as_array().expect("scopes").len(), 0);

    let (status, scopes) = app
        .put(
            &format!("/api/books/{book_id}/scopes"),
            serde_json::json!({ "scope_ids": [first, second] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{scopes}");
    assert_eq!(scopes.as_array().expect("scopes").len(), 2);

    // Detaching only removes visibility (spec/07 §1): the entries stay.
    let (status, scopes) = app
        .put(
            &format!("/api/books/{book_id}/scopes"),
            serde_json::json!({ "scope_ids": [second] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(scopes.as_array().expect("scopes").len(), 1);
    assert_eq!(scopes[0]["name"], "Lore");
    assert_eq!(count_rows(&app, "knowledge_entries").await, 0);

    // An unknown scope id is a 404 before anything is replaced.
    let (status, body) = app
        .put(
            &format!("/api/books/{book_id}/scopes"),
            serde_json::json!({ "scope_ids": [999] }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (_, scopes) = app.get(&format!("/api/books/{book_id}/scopes")).await;
    assert_eq!(scopes.as_array().expect("scopes").len(), 1, "unchanged");
}

#[tokio::test]
async fn knowledge_changes_publish_their_events() {
    let app = TestApp::new().await;
    let mut receiver = app.events().subscribe();
    let scope_id = create_scope(&app, "History").await;
    let entry_id = create_entry(&app, scope_id, "Song dress", "Round-collar robes.").await;
    app.patch(
        &format!("/api/knowledge/entries/{entry_id}"),
        serde_json::json!({ "content": "Changed content." }),
    )
    .await;
    app.delete(&format!("/api/knowledge/entries/{entry_id}"))
        .await;

    let mut seen: Vec<(i32, i32, String)> = Vec::new();
    while let Ok(envelope) = receiver.try_recv() {
        if let ServerEvent::KnowledgeChanged {
            scope_id,
            entry_id,
            op,
        } = envelope.event
        {
            seen.push((scope_id, entry_id, op.as_str().to_owned()));
        }
    }
    assert_eq!(seen.len(), 4, "{seen:?}");
    assert_eq!(seen[0], (scope_id, 0, "create".to_owned()), "scope created");
    assert_eq!(seen[1], (scope_id, entry_id, "create".to_owned()));
    assert_eq!(seen[2], (scope_id, entry_id, "update".to_owned()));
    assert_eq!(seen[3], (scope_id, entry_id, "delete".to_owned()));
}
