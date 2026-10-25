//! The index manager (spec/07 §3, §4): the runtime vec tables, the embedder state machine and
//! vector storage.
//!
//! The dimension depends on the active embedding model, so `vec_book` and `vec_knowledge` are
//! created at runtime (and recreated on a model change) together with their delete triggers.
//! FTS tables never change with the model (spec/07 §3).
//!
//! Vectors are stored as little-endian `f32` BLOBs; replace is `DELETE` then `INSERT` (spec/07
//! §3). `vec_book.corpus` is `passage` for passages and `canon` for every other kind, and it is
//! an equality constraint in KNN so the many passages cannot crowd canon hits out (D-030).

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement};
use tokio::sync::Notify;

pub use crate::api::health::IndexState;
use crate::api::health::IndexStatusDto;
use crate::db::Db;
use crate::db::entities::index_meta;
use crate::error::AppError;
use crate::events::{EventBus, ServerEvent};
use nekowriter_embed::{EmbedError, EmbedKind, Embedder};

/// `index_meta` keys (spec/02 §2).
pub const META_MODEL: &str = "embedding_model";
pub const META_DIM: &str = "embedding_dim";

/// The process-wide wake signal of the indexer (spec/07 §4: services signal after their commit;
/// the worker also polls every 5 s).
static INDEX_WAKE: Notify = Notify::const_new();

/// Wakes the indexer after a committed write that may have queued documents.
pub fn wake() {
    INDEX_WAKE.notify_one();
}

/// Waits for a wake signal or the 5-second poll interval (spec/07 §4).
pub async fn wait_for_work() {
    let poll = tokio::time::sleep(std::time::Duration::from_secs(5));
    tokio::select! {
        _ = INDEX_WAKE.notified() => {},
        _ = poll => {},
    }
}

/// One KNN hit: the source row id and the cosine distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KnnHit {
    pub id: i32,
    pub distance: f32,
}

// ───────────────────────────── schema ─────────────────────────────

/// Creates or recreates the vec tables and their delete triggers for `dimension`, and marks every
/// document `pending` when the embedding model changed (spec/07 §3).
///
/// Returns `true` when the schema was (re)created. One transaction, called at startup and on a
/// model change.
pub async fn ensure_schema(
    tx: &impl ConnectionTrait,
    model_key: &str,
    dimension: usize,
) -> Result<bool, AppError> {
    let current_model = meta_value(tx, META_MODEL).await?;
    let current_dim = meta_value(tx, META_DIM).await?;
    let tables_exist = vec_tables_exist(tx).await?;
    let dimension = dimension.clamp(1, 4_096);
    let wanted_dim = dimension.to_string();
    if tables_exist
        && current_model.as_deref() == Some(model_key)
        && current_dim.as_deref() == Some(wanted_dim.as_str())
    {
        return Ok(false);
    }

    // The dimension is not user input: it comes from `crates/embed/models.toml` or a test
    // embedder, and is clamped above.
    let ddl = format!(
        "DROP TRIGGER IF EXISTS search_docs_vec_ad;
         DROP TRIGGER IF EXISTS knowledge_chunks_vec_ad;
         DROP TABLE IF EXISTS vec_book;
         DROP TABLE IF EXISTS vec_knowledge;
         CREATE VIRTUAL TABLE vec_book USING vec0(
           doc_id INTEGER PRIMARY KEY,
           embedding float[{dimension}] distance_metric=cosine,
           book_id INTEGER PARTITION KEY,
           corpus TEXT
         );
         CREATE VIRTUAL TABLE vec_knowledge USING vec0(
           chunk_id INTEGER PRIMARY KEY,
           embedding float[{dimension}] distance_metric=cosine,
           scope_id INTEGER PARTITION KEY
         );
         CREATE TRIGGER search_docs_vec_ad AFTER DELETE ON search_docs BEGIN
           DELETE FROM vec_book WHERE doc_id = old.id;
         END;
         CREATE TRIGGER knowledge_chunks_vec_ad AFTER DELETE ON knowledge_chunks BEGIN
           DELETE FROM vec_knowledge WHERE chunk_id = old.id;
         END;
         UPDATE search_docs SET embed_state = 'pending' WHERE embed_state <> 'pending';
         UPDATE knowledge_chunks SET embed_state = 'pending' WHERE embed_state <> 'pending';"
    );
    tx.execute_unprepared(&ddl).await?;
    upsert_meta(tx, META_MODEL, model_key).await?;
    upsert_meta(tx, META_DIM, &dimension.to_string()).await?;
    tracing::info!(
        model = model_key,
        dimension,
        "rebuilt the vector index tables for the active embedding model"
    );
    Ok(true)
}

async fn vec_tables_exist(conn: &impl ConnectionTrait) -> Result<bool, AppError> {
    let rows = conn
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type = 'table' \
             AND name IN ('vec_book', 'vec_knowledge')",
        ))
        .await?;
    Ok(rows.len() == 2)
}

/// Reads one `index_meta` value.
pub async fn meta_value(
    conn: &impl ConnectionTrait,
    key: &str,
) -> Result<Option<String>, AppError> {
    Ok(index_meta::Entity::find_by_id(key.to_owned())
        .one(conn)
        .await?
        .map(|row| row.value))
}

async fn upsert_meta(conn: &impl ConnectionTrait, key: &str, value: &str) -> Result<(), AppError> {
    conn.execute_unprepared(&format!(
        "INSERT INTO index_meta(key, value) VALUES ('{}', '{}') \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        key.replace('\'', "''"),
        value.replace('\'', "''")
    ))
    .await?;
    Ok(())
}

// ───────────────────────────── vectors ─────────────────────────────

/// Packs a vector as a little-endian `f32` BLOB (spec/07 §3).
pub fn vector_blob(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// The KNN over `vec_book` with the mandatory partition and corpus filters (spec/07 §3).
pub async fn knn_book(
    conn: &impl ConnectionTrait,
    query: &[f32],
    book_id: i32,
    corpus: &str,
    k: usize,
) -> Result<Vec<KnnHit>, AppError> {
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "SELECT doc_id AS id, distance FROM vec_book \
         WHERE embedding MATCH ? AND k = ? AND book_id = ? AND corpus = ?",
        vec![
            sea_orm::Value::Bytes(Some(vector_blob(query))),
            (k as i64).into(),
            (book_id as i64).into(),
            corpus.into(),
        ],
    );
    let rows = conn.query_all_raw(statement).await?;
    let mut hits = Vec::with_capacity(rows.len());
    for row in rows {
        hits.push(KnnHit {
            id: row.try_get("", "id")?,
            distance: row.try_get::<f64>("", "distance")? as f32,
        });
    }
    Ok(hits)
}

/// The KNN over `vec_knowledge` for one scope (partition keys support only `=`, so a multi-scope
/// search runs one query per scope, spec/07 §3).
pub async fn knn_knowledge(
    conn: &impl ConnectionTrait,
    query: &[f32],
    scope_id: i32,
    k: usize,
) -> Result<Vec<KnnHit>, AppError> {
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "SELECT chunk_id AS id, distance FROM vec_knowledge \
         WHERE embedding MATCH ? AND k = ? AND scope_id = ?",
        vec![
            sea_orm::Value::Bytes(Some(vector_blob(query))),
            (k as i64).into(),
            (scope_id as i64).into(),
        ],
    );
    let rows = conn.query_all_raw(statement).await?;
    let mut hits = Vec::with_capacity(rows.len());
    for row in rows {
        hits.push(KnnHit {
            id: row.try_get("", "id")?,
            distance: row.try_get::<f64>("", "distance")? as f32,
        });
    }
    Ok(hits)
}

/// Stores the vector of one `search_docs` row and marks it `done`, but only when the row still
/// exists with the same `content_hash` under the same active model (D-050, spec/07 §4).
///
/// Returns `false` when the vector was skipped because the item changed, was deleted or the model
/// changed while the batch was being embedded.
pub async fn store_book_vector(
    conn: &impl ConnectionTrait,
    doc_id: i32,
    expected_hash: &str,
    vector: &[f32],
    model_key: &str,
) -> Result<bool, AppError> {
    if meta_value(conn, META_MODEL).await?.as_deref() != Some(model_key) {
        return Ok(false);
    }
    let Some(row) = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT content_hash, kind FROM search_docs WHERE id = ?",
            vec![(doc_id as i64).into()],
        ))
        .await?
    else {
        return Ok(false);
    };
    let hash: String = row.try_get("", "content_hash")?;
    if hash != expected_hash {
        return Ok(false);
    }
    let kind: String = row.try_get("", "kind")?;
    let corpus = if kind == "passage" {
        "passage"
    } else {
        "canon"
    };
    conn.execute_unprepared(&format!("DELETE FROM vec_book WHERE doc_id = {doc_id}"))
        .await?;
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO vec_book(doc_id, embedding, book_id, corpus) \
         SELECT id, ?, book_id, ? FROM search_docs WHERE id = ?",
        vec![
            sea_orm::Value::Bytes(Some(vector_blob(vector))),
            corpus.into(),
            (doc_id as i64).into(),
        ],
    ))
    .await?;
    conn.execute_unprepared(&format!(
        "UPDATE search_docs SET embed_state = 'done' WHERE id = {doc_id}"
    ))
    .await?;
    Ok(true)
}

/// Stores the vector of one `knowledge_chunks` row (same guards as [`store_book_vector`]).
pub async fn store_knowledge_vector(
    conn: &impl ConnectionTrait,
    chunk_id: i32,
    expected_hash: &str,
    vector: &[f32],
    model_key: &str,
) -> Result<bool, AppError> {
    if meta_value(conn, META_MODEL).await?.as_deref() != Some(model_key) {
        return Ok(false);
    }
    let Some(row) = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT content_hash FROM knowledge_chunks WHERE id = ?",
            vec![(chunk_id as i64).into()],
        ))
        .await?
    else {
        return Ok(false);
    };
    let hash: String = row.try_get("", "content_hash")?;
    if hash != expected_hash {
        return Ok(false);
    }
    conn.execute_unprepared(&format!(
        "DELETE FROM vec_knowledge WHERE chunk_id = {chunk_id}"
    ))
    .await?;
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO vec_knowledge(chunk_id, embedding, scope_id) \
         SELECT id, ?, scope_id FROM knowledge_chunks WHERE id = ?",
        vec![
            sea_orm::Value::Bytes(Some(vector_blob(vector))),
            (chunk_id as i64).into(),
        ],
    ))
    .await?;
    conn.execute_unprepared(&format!(
        "UPDATE knowledge_chunks SET embed_state = 'done' WHERE id = {chunk_id}"
    ))
    .await?;
    Ok(true)
}

// ───────────────────────────── the embedder service ─────────────────────────────

/// Shared, swappable embedder state, published as `index.status` events (spec/07 §4).
#[derive(Clone)]
pub struct SearchService {
    inner: Arc<SearchInner>,
}

struct SearchInner {
    db: Db,
    cache_dir: PathBuf,
    events: EventBus,
    state: RwLock<SearchState>,
    /// Serializes loading so the status moves through `loading` exactly once.
    load_lock: tokio::sync::Mutex<()>,
}

struct SearchState {
    model_key: String,
    dimension: usize,
    embedder: Option<Arc<dyn Embedder>>,
    status: IndexState,
    message: Option<String>,
}

impl SearchService {
    /// Builds the service for a model key; no embedder is installed yet.
    pub fn new(db: Db, cache_dir: PathBuf, events: EventBus) -> Self {
        let model_key = nekowriter_embed::DEFAULT_MODEL_KEY.to_owned();
        let dimension = nekowriter_embed::default_model().dimension;
        Self {
            inner: Arc::new(SearchInner {
                db,
                cache_dir,
                events,
                state: RwLock::new(SearchState {
                    model_key,
                    dimension,
                    embedder: None,
                    status: IndexState::NotLoaded,
                    message: None,
                }),
                load_lock: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// The same service publishing to a different bus (tests subscribe before the app is built).
    pub fn with_events(&self, events: EventBus) -> Self {
        let state = self
            .inner
            .state
            .read()
            .expect("search state lock")
            .clone_for_swap();
        Self {
            inner: Arc::new(SearchInner {
                db: self.inner.db.clone(),
                cache_dir: self.inner.cache_dir.clone(),
                events,
                state: RwLock::new(state),
                load_lock: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// The installed embedder, when there is one.
    pub fn embedder(&self) -> Option<Arc<dyn Embedder>> {
        self.inner
            .state
            .read()
            .expect("search state lock")
            .embedder
            .clone()
    }

    /// Installs the embedder of the current model. The dimension comes from the embedder, which
    /// reads it from `crates/embed/models.toml` (a fake may report a smaller test dimension).
    pub fn install(&self, embedder: Arc<dyn Embedder>) {
        let mut state = self.inner.state.write().expect("search state lock");
        state.dimension = embedder.dimension();
        state.embedder = Some(embedder);
        state.status = IndexState::NotLoaded;
        state.message = None;
    }

    /// The active model key (stored in `index_meta` and `app_settings["embedding"]`).
    pub fn model_key(&self) -> String {
        self.inner
            .state
            .read()
            .expect("search state lock")
            .model_key
            .clone()
    }

    /// The active vector dimension.
    pub fn dimension(&self) -> usize {
        self.inner
            .state
            .read()
            .expect("search state lock")
            .dimension
    }

    /// The current embedder state.
    pub fn status(&self) -> IndexState {
        self.inner.state.read().expect("search state lock").status
    }

    /// The failure message of a `failed` state.
    pub fn message(&self) -> Option<String> {
        self.inner
            .state
            .read()
            .expect("search state lock")
            .message
            .clone()
    }

    /// True when vectors can be produced (FTS keeps working regardless, spec/07 §5).
    pub fn is_ready(&self) -> bool {
        self.status() == IndexState::Ready
    }

    /// Switches the active model. The caller installs the matching embedder and rebuilds the vec
    /// schema (spec/07 §3).
    pub fn set_model(&self, model_key: &str) -> Result<(), AppError> {
        let entry = nekowriter_embed::model(model_key)
            .ok_or_else(|| AppError::validation(format!("unknown embedding model {model_key}")))?;
        let mut state = self.inner.state.write().expect("search state lock");
        state.model_key = model_key.to_owned();
        state.dimension = entry.dimension;
        state.embedder = None;
        state.status = IndexState::NotLoaded;
        state.message = None;
        Ok(())
    }

    /// Builds the production embedder ([`nekowriter_embed::FastEmbedder`]) for the active model.
    pub fn install_fast_embedder(&self) -> Result<(), AppError> {
        let key = self.model_key();
        let embedder = nekowriter_embed::FastEmbedder::new(&key, self.inner.cache_dir.clone())
            .map_err(|err| AppError::internal(format!("cannot build the embedder: {err}")))?;
        self.install(Arc::new(embedder));
        Ok(())
    }

    /// Loads the model when needed and publishes the transition through `index.status`.
    ///
    /// A `failed` state is not retried implicitly (spec/07 §4: failed items retry on a model
    /// reload or on Rebuild index).
    pub async fn ensure_ready(&self) -> bool {
        if self.is_ready() {
            return true;
        }
        let Some(embedder) = self.embedder() else {
            self.set_status(IndexState::NotLoaded, None).await;
            return false;
        };
        let _guard = self.inner.load_lock.lock().await;
        if self.is_ready() {
            return true;
        }
        self.set_status(IndexState::Loading, None).await;
        match embedder.ensure_loaded().await {
            Ok(()) => {
                self.set_status(IndexState::Ready, None).await;
                true
            }
            Err(err) => {
                self.set_status(IndexState::Failed, Some(err.to_string()))
                    .await;
                false
            }
        }
    }

    /// The query vector, or `None` when the embedder is not ready — the caller then runs keyword
    /// search only (spec/07 §5).
    pub async fn query_vector(&self, query: &str) -> Option<Vec<f32>> {
        if !self.is_ready() {
            return None;
        }
        let embedder = self.embedder()?;
        match embedder
            .embed(vec![query.to_owned()], EmbedKind::Query)
            .await
        {
            Ok(mut vectors) => vectors.pop(),
            Err(err) => {
                tracing::warn!(error = %err, "the query embedding failed; searching by keyword only");
                None
            }
        }
    }

    /// Embeds a batch of passages outside any transaction (spec/07 §4).
    pub async fn embed_passages(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, EmbedError> {
        if !self.ensure_ready().await {
            return Err(EmbedError::NotReady(
                self.message().unwrap_or_else(|| "loading".to_owned()),
            ));
        }
        let embedder = self
            .embedder()
            .ok_or_else(|| EmbedError::NotReady("no embedding model".to_owned()))?;
        embedder.embed(texts, EmbedKind::Passage).await
    }

    /// The counts and state the settings page and `/health` show.
    pub async fn status_dto(&self) -> IndexStatusDto {
        let counts = index_counts(self.inner.db.conn()).await.unwrap_or_default();
        let state = self.status();
        let message = self.message();
        IndexStatusDto {
            model: self.model_key(),
            state,
            message,
            pending: counts.0,
            failed: counts.1,
            done: counts.2,
        }
    }

    /// Publishes one `index.status` event (after the state change has been decided).
    pub async fn publish_status(&self) {
        let dto = self.status_dto().await;
        self.inner.events.publish(ServerEvent::IndexStatus {
            model: dto.model,
            state: dto.state,
            message: dto.message,
            pending: dto.pending,
            failed: dto.failed,
            done: dto.done,
        });
    }

    async fn set_status(&self, status: IndexState, message: Option<String>) {
        {
            let mut state = self.inner.state.write().expect("search state lock");
            state.status = status;
            state.message = message;
        }
        self.publish_status().await;
    }
}

impl SearchState {
    fn clone_for_swap(&self) -> SearchState {
        SearchState {
            model_key: self.model_key.clone(),
            dimension: self.dimension,
            embedder: self.embedder.clone(),
            status: self.status,
            message: self.message.clone(),
        }
    }
}

/// `(pending, failed, done)` across both index tables (spec/10 §3 `index.status`).
pub async fn index_counts(conn: &impl ConnectionTrait) -> Result<(i64, i64, i64), AppError> {
    let row = conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT \
               (SELECT count(*) FROM search_docs WHERE embed_state = 'pending') + \
               (SELECT count(*) FROM knowledge_chunks WHERE embed_state = 'pending') AS pending, \
               (SELECT count(*) FROM search_docs WHERE embed_state = 'failed') + \
               (SELECT count(*) FROM knowledge_chunks WHERE embed_state = 'failed') AS failed, \
               (SELECT count(*) FROM search_docs WHERE embed_state = 'done') + \
               (SELECT count(*) FROM knowledge_chunks WHERE embed_state = 'done') AS done",
        ))
        .await?
        .ok_or_else(|| AppError::internal("index counts returned no row"))?;
    Ok((
        row.try_get("", "pending")?,
        row.try_get("", "failed")?,
        row.try_get("", "done")?,
    ))
}

// ───────────────────────────── startup and model changes ─────────────────────────────

/// Prepares the search subsystem at startup (spec/07 §3): the configured model is installed and
/// the runtime vec tables are created (or recreated) for its dimension.
///
/// In the production build this is where the fastembed model is installed and the indexer worker
/// is spawned; the tests use the deterministic fake and drive the worker batch by batch.
pub async fn initialize(state: &crate::state::AppState) -> Result<(), AppError> {
    let search = state.search();
    #[cfg(not(any(test, feature = "testing")))]
    {
        let model = match crate::services::app_settings::get(state.db_conn()).await {
            Ok(settings) => settings.embedding.model,
            Err(err) => {
                tracing::warn!(error = %err, "cannot read the embedding settings; using the default model");
                nekowriter_embed::DEFAULT_MODEL_KEY.to_owned()
            }
        };
        search.set_model(&model)?;
        search.install_fast_embedder()?;
    }
    #[cfg(any(test, feature = "testing"))]
    {
        // `AppState::new` installed the fake embedder; loading it is a no-op.
        let _ = search.ensure_ready().await;
    }
    let model_key = search.model_key();
    let dimension = search.dimension();
    crate::services::write(state.db(), move |tx| {
        Box::pin(async move { ensure_schema(tx, &model_key, dimension).await })
    })
    .await?;
    #[cfg(not(any(test, feature = "testing")))]
    crate::services::indexer::spawn(state.clone());
    Ok(())
}

/// Switches the active embedding model (spec/07 §3): install the new model and rebuild the vec
/// tables in one transaction, then let the indexer catch up.
pub async fn switch_model(state: &crate::state::AppState, model_key: &str) -> Result<(), AppError> {
    let search = state.search();
    search.set_model(model_key)?;
    #[cfg(any(test, feature = "testing"))]
    {
        let dimension = nekowriter_embed::model(model_key)
            .map(|entry| entry.dimension)
            .unwrap_or_else(|| nekowriter_embed::default_model().dimension);
        // Model changes in tests are real dimension changes: the fake follows the table.
        search.install(Arc::new(nekowriter_embed::FakeEmbedder::with_dimension(
            dimension,
        )));
        let _ = search.ensure_ready().await;
    }
    #[cfg(not(any(test, feature = "testing")))]
    search.install_fast_embedder()?;
    let model = search.model_key();
    let dimension = search.dimension();
    crate::services::write(state.db(), move |tx| {
        Box::pin(async move { ensure_schema(tx, &model, dimension).await })
    })
    .await?;
    #[cfg(not(any(test, feature = "testing")))]
    {
        // The new model loads (and possibly downloads) in the background (D-002).
        let state = state.clone();
        tokio::spawn(async move {
            state.search().ensure_ready().await;
        });
    }
    wake();
    Ok(())
}

#[cfg(any(test, feature = "testing"))]
impl SearchService {
    /// Removes the embedder: hybrid search must fall back to keyword-only (spec/07 §5).
    pub fn uninstall_embedder(&self) {
        let mut state = self.inner.state.write().expect("search state lock");
        state.embedder = None;
        state.status = IndexState::NotLoaded;
        state.message = None;
    }
}
