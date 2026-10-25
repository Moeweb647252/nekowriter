//! M7 review: embeddings and query vectors are produced **outside** any transaction
//! (spec/01 §7, spec/07 §4, D-045), and the store step re-checks the row (D-050).
//!
//! The decisive probe: the embedder tries to open a *second* write transaction while it embeds.
//! If the caller held the process-wide write lock (or an open transaction), that attempt would
//! block until the first one commits — which cannot happen, because the caller is waiting for the
//! embed result. A completed probe therefore proves the lock was free.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use nekowriter::db::Db;
use nekowriter::db::entities::{knowledge_chunks, search_docs};
use nekowriter::search::index;
use nekowriter::services::indexer;
use nekowriter::testing::TestApp;
use nekowriter_embed::{EmbedError, EmbedKind, Embedder, ModelEntry};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde_json::json;

/// Records whether a write transaction could be opened from inside `embed`.
struct LockProbeEmbedder {
    db: Db,
    entered: AtomicUsize,
    write_lock_free: AtomicBool,
}

impl LockProbeEmbedder {
    fn new(db: Db) -> Self {
        Self {
            db,
            entered: AtomicUsize::new(0),
            write_lock_free: AtomicBool::new(false),
        }
    }
}

#[async_trait::async_trait]
impl Embedder for LockProbeEmbedder {
    fn model(&self) -> &ModelEntry {
        nekowriter_embed::default_model()
    }

    fn dimension(&self) -> usize {
        384
    }

    async fn embed(
        &self,
        texts: Vec<String>,
        _kind: EmbedKind,
    ) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        // If this call happens inside `Db::write`, the second transaction waits for the lock the
        // caller still holds, so the timeout fires and the flag stays false.
        let probe = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            self.db.write(|tx| {
                Box::pin(async move {
                    tx.execute_unprepared("SELECT 1").await?;
                    Ok(())
                })
            }),
        )
        .await;
        if matches!(probe, Ok(Ok(()))) {
            self.write_lock_free.store(true, Ordering::SeqCst);
        }
        Ok(texts
            .iter()
            .map(|text| nekowriter_embed::FakeEmbedder::vector(text, 384))
            .collect())
    }
}

async fn count(app: &TestApp, sql: &str) -> i64 {
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

/// The indexer's read → embed → write sequence must release the write lock before embedding.
#[tokio::test]
async fn the_indexer_embeds_outside_the_write_transaction() {
    let app = TestApp::new().await;
    let book = app.create_book("Lock Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "Lin Fan", "summary": "the protagonist" }),
        )
        .await;
    assert_eq!(status, 201, "{body}");

    let probe = Arc::new(LockProbeEmbedder::new(app.db().clone()));
    app.state().search().install(probe.clone());
    assert!(app.state().search().ensure_ready().await, "the probe loads");
    let report = indexer::run_once(app.state()).await.expect("indexer");
    println!("store report: {report:?}");
    assert_eq!(report.stored, 1, "{report:?}");
    assert_eq!(
        probe.entered.load(Ordering::SeqCst),
        1,
        "the probe embedder was used"
    );
    assert!(
        probe.write_lock_free.load(Ordering::SeqCst),
        "the embedder must run while the process-wide write lock is free (spec/01 §7, D-045)"
    );
}

/// A store step that sees a different active model must skip (D-050): the vector of the previous
/// model lives in a different space.
#[tokio::test]
async fn a_model_switch_during_embedding_leaves_no_vector() {
    let app = TestApp::new().await;
    let book = app.create_book("Model Race Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "Lin Fan", "summary": "the protagonist" }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let doc = search_docs::Entity::find()
        .filter(search_docs::Column::Kind.eq("entity"))
        .one(app.db().conn())
        .await
        .expect("doc")
        .expect("entity doc");
    let old_hash = doc.content_hash.clone();
    // The batch read this hash under the old model; the author switches models meanwhile.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "embedding": { "model": "bge-small-zh-v1.5" } }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let stored = index::store_book_vector(
        app.db().conn(),
        doc.id,
        &old_hash,
        &vec![0.1f32; 512],
        "multilingual-e5-small",
    )
    .await
    .expect("store");
    assert!(!stored, "a vector of the old model must not be stored");
    assert_eq!(count(&app, "SELECT count(*) AS c FROM vec_book").await, 0);
    let row = search_docs::Entity::find_by_id(doc.id)
        .one(app.db().conn())
        .await
        .expect("reload")
        .expect("row");
    assert_eq!(
        row.embed_state, "pending",
        "it stays pending for the new model"
    );
}

/// The same guard for knowledge chunks.
#[tokio::test]
async fn a_changed_chunk_is_not_given_its_old_vector() {
    let app = TestApp::new().await;
    let (status, body) = app
        .post("/api/knowledge/scopes", json!({ "name": "History" }))
        .await;
    assert_eq!(status, 201, "{body}");
    let scope_id = body["id"].as_i64().expect("scope") as i32;
    let (status, body) = app
        .post(
            &format!("/api/knowledge/scopes/{scope_id}/entries"),
            json!({ "title": "Tea", "content": "# Tea\n\nSong tea houses." }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let chunk = knowledge_chunks::Entity::find()
        .one(app.db().conn())
        .await
        .expect("chunk")
        .expect("a chunk");
    let old_hash = chunk.content_hash.clone();
    let entry_id = chunk.entry_id;
    // The entry changes while the batch embeds; then the batch tries to store the old vector.
    let (status, body) = app
        .patch(
            &format!("/api/knowledge/entries/{entry_id}"),
            json!({ "content": "# Tea\n\nTang tea houses." }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let stored = index::store_knowledge_vector(
        app.db().conn(),
        chunk.id,
        &old_hash,
        &vec![0.2f32; 384],
        &app.state().search().model_key(),
    )
    .await
    .expect("store");
    assert!(!stored, "the changed chunk keeps its pending state");
    assert_eq!(
        count(&app, "SELECT count(*) AS c FROM vec_knowledge").await,
        0
    );
    let row = knowledge_chunks::Entity::find_by_id(chunk.id)
        .one(app.db().conn())
        .await
        .expect("reload")
        .expect("row");
    assert_eq!(row.embed_state, "pending");
}

/// The knowledge search endpoint computes its query vector outside any transaction: the write lock
/// must be free while the query embeds.
#[tokio::test]
async fn knowledge_search_embeds_the_query_outside_the_transaction() {
    let app = TestApp::new().await;
    let (status, body) = app
        .post("/api/knowledge/scopes", json!({ "name": "History" }))
        .await;
    assert_eq!(status, 201, "{body}");
    let scope_id = body["id"].as_i64().expect("scope") as i32;
    app.post(
        &format!("/api/knowledge/scopes/{scope_id}/entries"),
        json!({ "title": "Tea", "content": "Song tea houses serve tea." }),
    )
    .await;
    let probe = Arc::new(LockProbeEmbedder::new(app.db().clone()));
    app.state().search().install(probe.clone());
    assert!(app.state().search().ensure_ready().await);
    let (status, body) = app.get("/api/knowledge/search?q=tea").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(probe.entered.load(Ordering::SeqCst), 1);
    assert!(
        probe.write_lock_free.load(Ordering::SeqCst),
        "the query vector must be computed with the write lock free (spec/07 §5, spec/05 §1.2)"
    );
}
