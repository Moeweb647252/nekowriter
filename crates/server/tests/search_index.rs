//! Search index maintenance and the indexer worker (PLAN 7.2, spec/07 §1.1–§4):
//! chunking/shape of the rows, upsert/delete flows through the services and the agent path,
//! rebuilds on a dimension change, orphan cleanup after deleting a book and the stale-vector
//! guard of D-050.

use axum::http::StatusCode;
use nekowriter::db::entities::{knowledge_chunks, search_docs};
use nekowriter::db::json::EmbedState;
use nekowriter::search::{index, maintenance};
use nekowriter::services::indexer;
use nekowriter::testing::TestApp;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};

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

/// Creates a book and returns `(book_id, main_track_id)`.
async fn book_with_track(app: &TestApp) -> (i32, i32) {
    let book = app.create_book("Search Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (status, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    assert_eq!(status, StatusCode::OK, "tracks: {tracks}");
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    (book_id, track_id)
}

async fn create_entity(app: &TestApp, book_id: i32, name: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            serde_json::json!({
                "kind": "character",
                "name": name,
                "aliases": ["Fan"],
                "summary": "the protagonist",
                "description": "a young cultivator",
                "state": "qi refining",
                "attributes": { "age": "17" },
                "importance": 4,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create entity: {body}");
    body["id"].as_i64().expect("entity id") as i32
}

async fn doc_row(app: &TestApp, kind: &str, ref_id: i32) -> Option<search_docs::Model> {
    search_docs::Entity::find()
        .filter(search_docs::COLUMN.kind.eq(kind))
        .filter(search_docs::COLUMN.ref_id.eq(ref_id))
        .one(app.db().conn())
        .await
        .expect("doc query")
}

async fn fts_hits(app: &TestApp, query: &str) -> i64 {
    count(
        app,
        &format!(
            "SELECT count(*) AS c FROM fts_book WHERE fts_book MATCH '{}'",
            query.replace('\'', "''")
        ),
    )
    .await
}

/// Creates an official chapter with the given paragraph texts.
async fn official_chapter(app: &TestApp, track_id: i32, title: &str, text: &str) -> i32 {
    let chapter = app.create_chapter(track_id, title).await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.put_text(chapter_id, 0, text).await;
    let (status, body) = app
        .post(
            &format!("/api/chapters/{chapter_id}/submit"),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "submit: {body}");
    let (status, body) = app
        .post(
            &format!("/api/chapters/{chapter_id}/approve"),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "approve: {body}");
    chapter_id
}

#[tokio::test]
async fn entity_documents_follow_the_crud_flow() {
    let app = TestApp::new().await;
    let (book_id, _track) = book_with_track(&app).await;
    let entity_id = create_entity(&app, book_id, "Lin Fan").await;

    let doc = doc_row(&app, "entity", entity_id)
        .await
        .expect("entity doc");
    assert_eq!(doc.title, "Lin Fan");
    assert_eq!(
        doc.body, "the protagonist · Fan · qi refining · age: 17 · a young cultivator",
        "spec/07 §2 entity body"
    );
    assert_eq!(doc.embed_state, EmbedState::Pending);
    assert_eq!(doc.track_id, None);
    // The FTS row exists and holds jieba-segmented tokens.
    assert_eq!(fts_hits(&app, "\"lin\"").await, 1);
    assert_eq!(fts_hits(&app, "\"protagonist\"").await, 1);

    // A change with a new body resets the embed state (spec/07 §2).
    let (status, body) = app
        .patch(
            &format!("/api/entities/{entity_id}"),
            serde_json::json!({ "summary": "the hero" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "patch entity: {body}");
    let doc = doc_row(&app, "entity", entity_id).await.expect("doc");
    assert_eq!(doc.embed_state, EmbedState::Pending);
    assert_eq!(fts_hits(&app, "\"hero\"").await, 1);

    // Indexing stores the vector and marks the row done.
    let report = indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(report.stored, 1, "{report:?}");
    let doc = doc_row(&app, "entity", entity_id).await.expect("doc");
    assert_eq!(doc.embed_state, EmbedState::Done);
    assert_eq!(count_rows(&app, "vec_book").await, 1);

    // Deleting the entity removes the document, its FTS row and its vector (spec/02 §5.11).
    let (status, _) = app.delete(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(doc_row(&app, "entity", entity_id).await.is_none());
    assert_eq!(fts_hits(&app, "\"hero\"").await, 0);
    assert_eq!(count_rows(&app, "vec_book").await, 0);
}

#[tokio::test]
async fn approving_a_chapter_indexes_its_passages() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let text = "First paragraph of chapter one.\n\nSecond paragraph of the chapter.\n\nThird one.";
    let chapter_id = official_chapter(&app, track_id, "Sword Tomb", text).await;

    let docs = search_docs::Entity::find()
        .filter(search_docs::COLUMN.kind.eq("passage"))
        .filter(search_docs::COLUMN.ref_id.eq(chapter_id))
        .all(app.db().conn())
        .await
        .expect("passages");
    assert_eq!(docs.len(), 1, "three short paragraphs are one passage");
    assert_eq!(docs[0].ref_part, 0);
    assert_eq!(docs[0].title, "Sword Tomb");
    let meta: serde_json::Value = docs[0].meta.clone().into_inner();
    assert_eq!(meta["pids"], serde_json::json!(["p1", "p2", "p3"]));
    assert!(docs[0].body.contains("First paragraph"));
    // An empty summary has no document.
    assert!(doc_row(&app, "summary", chapter_id).await.is_none());

    let report = indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(report.stored, 1, "{report:?}");
    let passage_id = docs[0].id;

    // Editing one paragraph rebuilds passages: the unchanged prefix keeps its row id and vector,
    // the changed content gets a new hash (spec/07 §2, D-039).
    let patch = serde_json::json!({
        "base_version": 1,
        "ops": [{ "op": "update", "pid": "p2", "text": "Second paragraph, rewritten." }],
    });
    let (status, body) = app
        .patch(&format!("/api/chapters/{chapter_id}/content"), patch)
        .await;
    assert_eq!(status, StatusCode::OK, "content patch: {body}");
    let docs_after = search_docs::Entity::find()
        .filter(search_docs::COLUMN.kind.eq("passage"))
        .filter(search_docs::COLUMN.ref_id.eq(chapter_id))
        .all(app.db().conn())
        .await
        .expect("passages");
    assert_eq!(docs_after.len(), 1);
    assert_eq!(docs_after[0].id, passage_id, "the hash diff keeps the row");
    assert_eq!(docs_after[0].embed_state, EmbedState::Pending);
    assert!(docs_after[0].body.contains("rewritten"));
    // The old vector is replaced only by the next batch, which stores one vector for the row.
    let report = indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(report.stored, 1, "{report:?}");
    assert_eq!(count_rows(&app, "vec_book").await, 1);

    // The archivist's summary fills the summary document in the same transaction.
    let (status, body) = app
        .patch(
            &format!("/api/chapters/{chapter_id}"),
            serde_json::json!({ "summary": "Lin Fan enters the sword tomb." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "summary patch: {body}");
    let summary = doc_row(&app, "summary", chapter_id)
        .await
        .expect("summary doc");
    assert_eq!(summary.body, "Lin Fan enters the sword tomb.");
}

#[tokio::test]
async fn a_model_change_rebuilds_the_vec_tables_and_resets_the_states() {
    let app = TestApp::new().await;
    let (book_id, _track) = book_with_track(&app).await;
    let entity_id = create_entity(&app, book_id, "Lin Fan").await;
    indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(count_rows(&app, "vec_book").await, 1);
    let fts_before = count_rows(&app, "fts_book").await;

    let dim_before: String = index::meta_value(app.db().conn(), index::META_DIM)
        .await
        .expect("meta")
        .expect("dim");
    assert_eq!(dim_before, "384");

    // Switching the model through the settings endpoint recreates the vec tables at the new
    // dimension, marks every document pending and leaves FTS untouched (spec/07 §3).
    let (status, body) = app
        .patch(
            "/api/app-settings",
            serde_json::json!({ "embedding": { "model": "bge-small-zh-v1.5" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "settings patch: {body}");
    assert_eq!(
        index::meta_value(app.db().conn(), index::META_MODEL)
            .await
            .expect("meta")
            .as_deref(),
        Some("bge-small-zh-v1.5")
    );
    assert_eq!(
        index::meta_value(app.db().conn(), index::META_DIM)
            .await
            .expect("meta")
            .as_deref(),
        Some("512"),
        "the fake embedder follows the model table's dimension in tests"
    );
    let doc = doc_row(&app, "entity", entity_id).await.expect("doc");
    assert_eq!(doc.embed_state, EmbedState::Pending);
    assert_eq!(
        fts_before,
        count_rows(&app, "fts_book").await,
        "FTS is independent"
    );
    assert_eq!(
        count_rows(&app, "vec_book").await,
        0,
        "the old dimensions are gone"
    );

    // The new schema accepts the new model's vectors.
    let report = indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(report.stored, 1, "{report:?}");
    assert_eq!(count_rows(&app, "vec_book").await, 1);
}

#[tokio::test]
async fn a_startup_with_a_stale_index_meta_rebuilds_the_schema() {
    let app = TestApp::new().await;
    let (book_id, _track) = book_with_track(&app).await;
    let entity_id = create_entity(&app, book_id, "Lin Fan").await;
    indexer::run_once(app.state()).await.expect("indexer");
    app.execute(
        "UPDATE index_meta SET value = '999' WHERE key = 'embedding_dim'; \
         UPDATE search_docs SET embed_state = 'done'",
    )
    .await;

    // The production startup sequence compares `index_meta` with the active model.
    nekowriter::search::initialize(app.state())
        .await
        .expect("initialize");
    assert_eq!(
        index::meta_value(app.db().conn(), index::META_DIM)
            .await
            .expect("meta")
            .as_deref(),
        Some("384")
    );
    let doc = doc_row(&app, "entity", entity_id).await.expect("doc");
    assert_eq!(doc.embed_state, EmbedState::Pending);
}

#[tokio::test]
async fn an_item_changed_or_deleted_during_embedding_gets_no_stale_vector() {
    let app = TestApp::new().await;
    let (book_id, _track) = book_with_track(&app).await;
    let entity_id = create_entity(&app, book_id, "Lin Fan").await;
    let doc = doc_row(&app, "entity", entity_id).await.expect("doc");
    let old_hash = doc.content_hash.clone();

    // The batch read the hash; the author renames the entity meanwhile.
    let (status, _) = app
        .patch(
            &format!("/api/entities/{entity_id}"),
            serde_json::json!({ "summary": "changed while embedding" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let vector = vec![0.1f32; app.state().search().dimension()];
    let stored = index::store_book_vector(
        app.db().conn(),
        doc.id,
        &old_hash,
        &vector,
        &app.state().search().model_key(),
    )
    .await
    .expect("store");
    assert!(!stored, "a changed row must not get the old vector");
    assert_eq!(count_rows(&app, "vec_book").await, 0);
    let doc = doc_row(&app, "entity", entity_id).await.expect("doc");
    assert_eq!(doc.embed_state, EmbedState::Pending);

    // A deleted row gets no vector either, and the next batch stores the fresh content.
    let old_hash = doc.content_hash.clone();
    let (status, _) = app.delete(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let stored = index::store_book_vector(
        app.db().conn(),
        doc.id,
        &old_hash,
        &vector,
        &app.state().search().model_key(),
    )
    .await
    .expect("store");
    assert!(!stored);
    assert_eq!(count_rows(&app, "vec_book").await, 0);
}

#[tokio::test]
async fn deleting_a_book_removes_every_index_row() {
    let app = TestApp::new().await;
    let (book_id, track_id) = book_with_track(&app).await;
    let entity_id = create_entity(&app, book_id, "Lin Fan").await;
    let chapter_id = official_chapter(&app, track_id, "Sword Tomb", "Paragraph one.\n\nTwo.").await;
    app.patch(
        &format!("/api/chapters/{chapter_id}"),
        serde_json::json!({ "summary": "A summary." }),
    )
    .await;
    indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(count_rows(&app, "search_docs").await, 3); // entity + summary + passage
    assert_eq!(count_rows(&app, "fts_book").await, 3);
    assert_eq!(count_rows(&app, "vec_book").await, 3);

    let (status, body) = app.delete(&format!("/api/books/{book_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(count_rows(&app, "search_docs").await, 0);
    assert_eq!(count_rows(&app, "fts_book").await, 0);
    assert_eq!(count_rows(&app, "vec_book").await, 0);
    let _ = entity_id;
}

#[tokio::test]
async fn the_worker_takes_book_documents_and_knowledge_chunks_in_one_batch() {
    let app = TestApp::new().await;
    let (book_id, _track) = book_with_track(&app).await;
    for name in ["A", "B", "C"] {
        let (status, body) = app
            .post(
                &format!("/api/books/{book_id}/entities"),
                serde_json::json!({ "kind": "character", "name": name }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "create entity: {body}");
    }
    // Seed one scope with three chunks (the knowledge services arrive with PLAN 7.4).
    app.execute(
        "INSERT INTO knowledge_scopes(id, name, created_at, updated_at) \
         VALUES (1, 'History', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'); \
         INSERT INTO knowledge_entries(id, scope_id, title, content, source, created_at, updated_at) \
         VALUES (1, 1, 'Song dress', 'content', 'manual', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .await;
    for ord in 0..3 {
        app.execute(&format!(
            "INSERT INTO knowledge_chunks(entry_id, scope_id, ord, heading, text, content_hash) \
             VALUES (1, 1, {ord}, '', 'chunk {ord}', 'hash-{ord}')"
        ))
        .await;
    }
    let report = indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(report.stored, 6, "{report:?}");
    assert_eq!(count_rows(&app, "vec_book").await, 3);
    assert_eq!(
        knowledge_chunks::Entity::find()
            .filter(knowledge_chunks::COLUMN.embed_state.eq("done"))
            .count(app.db().conn())
            .await
            .expect("count"),
        3
    );
    let remaining = knowledge_chunks::Entity::find()
        .filter(knowledge_chunks::COLUMN.embed_state.eq("pending"))
        .count(app.db().conn())
        .await
        .expect("count");
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn index_status_is_reported_and_published() {
    let app = TestApp::new().await;
    let mut receiver = app.events().subscribe();
    let (book_id, _track) = book_with_track(&app).await;
    create_entity(&app, book_id, "Lin Fan").await;

    let (status, body) = app.get("/api/embedding/status").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["model"], "multilingual-e5-small");
    assert_eq!(body["state"], "ready");
    assert!(body["pending"].as_i64().unwrap_or(0) >= 1, "{body}");

    let report = indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(report.stored, 1);
    let (status, body) = app.get("/api/embedding/status").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["done"], 1, "{body}");
    assert_eq!(body["pending"], 0);

    // `index.status` is published with the counts (spec/10 §3).
    let mut saw_status = false;
    while let Ok(envelope) = receiver.try_recv() {
        if let nekowriter::events::ServerEvent::IndexStatus {
            model, state, done, ..
        } = &envelope.event
        {
            assert_eq!(model, "multilingual-e5-small");
            assert_eq!(*state, nekowriter::api::health::IndexState::Ready);
            assert_eq!(*done, 1);
            saw_status = true;
        }
    }
    assert!(saw_status, "an index.status event must be published");
}

#[tokio::test]
async fn rebuild_marks_everything_pending_and_retries_failed_items() {
    let app = TestApp::new().await;
    let (book_id, _track) = book_with_track(&app).await;
    let entity_id = create_entity(&app, book_id, "Lin Fan").await;
    app.execute("UPDATE search_docs SET embed_state = 'failed'")
        .await;
    let doc = doc_row(&app, "entity", entity_id).await.expect("doc");
    assert_eq!(doc.embed_state, EmbedState::Failed);

    let (status, _) = app
        .post("/api/embedding/rebuild", serde_json::json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let doc = doc_row(&app, "entity", entity_id).await.expect("doc");
    assert_eq!(doc.embed_state, EmbedState::Pending);
}

#[tokio::test]
async fn knowledge_entry_chunks_are_rebuilt_with_a_hash_diff() {
    let app = TestApp::new().await;
    app.execute(
        "INSERT INTO knowledge_scopes(id, name, created_at, updated_at) \
         VALUES (1, 'History', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'); \
         INSERT INTO knowledge_entries(id, scope_id, title, content, source, created_at, updated_at) \
         VALUES (1, 1, 'Song dress', '# 宋代服饰\n\n男子常穿圆领袍。\n\n## 男子常服\n\n颜色以青、白为主。', \
                 'manual', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .await;
    nekowriter::db::Db::write(app.db(), |tx| {
        Box::pin(async move {
            maintenance::sync_knowledge_entry(tx, 1)
                .await
                .map_err(|err| sea_orm::DbErr::Custom(err.message))?;
            Ok(())
        })
    })
    .await
    .expect("sync");
    let rows = knowledge_chunks::Entity::find()
        .filter(knowledge_chunks::COLUMN.entry_id.eq(1))
        .all(app.db().conn())
        .await
        .expect("chunks");
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0].heading, "宋代服饰");
    assert_eq!(rows[1].heading, "宋代服饰 > 男子常服");
    assert_eq!(rows[1].ord, 1);
    let first = rows[0].clone();

    // Change the second section only: the first chunk keeps its row id and hash.
    app.execute(
        "UPDATE knowledge_entries SET content = '# 宋代服饰\n\n男子常穿圆领袍。\n\n## 男子常服\n\n颜色以黑、红为主。' WHERE id = 1",
    )
    .await;
    nekowriter::db::Db::write(app.db(), |tx| {
        Box::pin(async move {
            maintenance::sync_knowledge_entry(tx, 1)
                .await
                .map_err(|err| sea_orm::DbErr::Custom(err.message))?;
            Ok(())
        })
    })
    .await
    .expect("resync");
    let rows = knowledge_chunks::Entity::find()
        .filter(knowledge_chunks::COLUMN.entry_id.eq(1))
        .order_by_asc(knowledge_chunks::COLUMN.ord)
        .all(app.db().conn())
        .await
        .expect("chunks");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, first.id, "unchanged chunk keeps its row");
    assert_eq!(rows[0].content_hash, first.content_hash);
    assert_ne!(rows[1].content_hash, first.content_hash);
    // FTS rows follow: keyword search works immediately (spec/07 §2), through the same jieba
    // segmentation as the index (spec/07 §5).
    let query = nekowriter::search::fts::query_string("男子常服").expect("query string");
    assert_eq!(
        count(
            &app,
            &format!(
                "SELECT count(*) AS c FROM fts_knowledge WHERE fts_knowledge MATCH '{}'",
                query.replace('\'', "''")
            ),
        )
        .await,
        2,
        "both chunks are found by the section name"
    );
}
