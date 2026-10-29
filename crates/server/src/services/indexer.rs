//! The indexer worker (spec/07 §4).
//!
//! Wake on a process-wide [`crate::search::index::wake`] signal or every 5 s; take up to 32
//! pending items (book documents and knowledge chunks, alternating); embed them as passages
//! **outside any transaction**; then, in one transaction, write each vector and `embed_state =
//! done` only when the row still exists with the same `content_hash` under the same active model
//! (D-050). A changed or deleted item never gets a stale vector, and a failed batch is marked
//! `failed` so it retries on a model reload or on Rebuild index (spec/07 §4).

use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};

use crate::db::entities::{knowledge_chunks, knowledge_entries, search_docs};
use crate::db::json::EmbedState;
use crate::error::AppError;
use crate::search::{chunk, index};
use crate::state::AppState;

/// The batch size of one indexing pass (spec/07 §4).
pub const BATCH_SIZE: usize = 32;

/// What one batch did, for tests and logs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchReport {
    /// Vectors stored under the same hash and model.
    pub stored: usize,
    /// Items that changed, were deleted or were skipped before the store.
    pub skipped: usize,
    /// Items marked `failed` by an embedding error.
    pub failed: usize,
}

impl BatchReport {
    pub fn is_empty(&self) -> bool {
        self.stored == 0 && self.skipped == 0 && self.failed == 0
    }
}

struct PendingDoc {
    id: i32,
    content_hash: String,
    title: String,
    body: String,
}

struct PendingChunk {
    id: i32,
    content_hash: String,
    title: String,
    heading: String,
    text: String,
}

/// Runs one indexing pass over the pending items (spec/07 §4).
pub async fn run_once(state: &AppState) -> Result<BatchReport, AppError> {
    let search = state.search();
    if !search.is_ready() {
        return Ok(BatchReport::default());
    }
    let (docs, chunks) = take_pending(state).await?;
    if docs.is_empty() && chunks.is_empty() {
        return Ok(BatchReport::default());
    }

    // The embedding input of a book document carries its title as context, like a chunk's
    // heading (spec/07 §1.1 item 3).
    let mut embeddable: Vec<String> = Vec::new();
    let mut positions: Vec<ItemPosition> = Vec::new();
    for (position, doc) in docs.iter().enumerate() {
        let text = doc_embedding_input(&doc.title, &doc.body);
        if !text.trim().is_empty() {
            embeddable.push(text);
            positions.push(ItemPosition::Doc(position));
        }
    }
    for (position, item) in chunks.iter().enumerate() {
        let text = chunk::knowledge_embedding_input(&item.title, &item.heading, &item.text);
        if !text.trim().is_empty() {
            embeddable.push(text);
            positions.push(ItemPosition::Chunk(position));
        }
    }

    // Embed outside any transaction (spec/01 §7, D-045).
    let vectors = if embeddable.is_empty() {
        Vec::new()
    } else {
        match search.embed_passages(embeddable).await {
            Ok(vectors) => vectors,
            Err(err) => {
                tracing::warn!(error = %err, items = docs.len() + chunks.len(), "the indexer batch failed to embed");
                let failed = mark_failed(state, &docs, &chunks).await?;
                let report = BatchReport {
                    failed,
                    skipped: docs.len() + chunks.len() - failed,
                    stored: 0,
                };
                search.publish_status().await;
                return Ok(report);
            }
        }
    };

    // In ONE transaction store the vectors that are still valid (D-050).
    let model_key = search.model_key();
    let mut doc_vectors: std::collections::HashMap<usize, Vec<f32>> =
        std::collections::HashMap::new();
    let mut chunk_vectors: std::collections::HashMap<usize, Vec<f32>> =
        std::collections::HashMap::new();
    for (position, vector) in positions.iter().zip(vectors) {
        match position {
            ItemPosition::Doc(index) => {
                doc_vectors.insert(*index, vector);
            }
            ItemPosition::Chunk(index) => {
                chunk_vectors.insert(*index, vector);
            }
        }
    }
    let report = crate::services::write(state.db(), move |tx| {
        Box::pin(async move {
            let mut report = BatchReport::default();
            for (position, doc) in docs.iter().enumerate() {
                match doc_vectors.get(&position) {
                    Some(vector) => {
                        if index::store_book_vector(
                            tx,
                            doc.id,
                            &doc.content_hash,
                            vector,
                            &model_key,
                        )
                        .await?
                        {
                            report.stored += 1;
                        } else {
                            report.skipped += 1;
                        }
                    }
                    None => {
                        mark_doc_done(tx, doc.id, &doc.content_hash).await?;
                        report.skipped += 1;
                    }
                }
            }
            for (position, item) in chunks.iter().enumerate() {
                match chunk_vectors.get(&position) {
                    Some(vector) => {
                        if index::store_knowledge_vector(
                            tx,
                            item.id,
                            &item.content_hash,
                            vector,
                            &model_key,
                        )
                        .await?
                        {
                            report.stored += 1;
                        } else {
                            report.skipped += 1;
                        }
                    }
                    None => {
                        mark_chunk_done(tx, item.id, &item.content_hash).await?;
                        report.skipped += 1;
                    }
                }
            }
            Ok(report)
        })
    })
    .await?;

    if !report.is_empty() {
        tracing::debug!(
            stored = report.stored,
            skipped = report.skipped,
            failed = report.failed,
            "indexer batch complete"
        );
        search.publish_status().await;
    }
    Ok(report)
}

enum ItemPosition {
    Doc(usize),
    Chunk(usize),
}

/// The embedding input of one `search_docs` row.
pub fn doc_embedding_input(title: &str, body: &str) -> String {
    if title.trim().is_empty() {
        body.to_owned()
    } else {
        format!("{title}\n{body}")
    }
}

/// Loads up to [`BATCH_SIZE`] pending items, alternating book documents and knowledge chunks
/// (spec/07 §4).
async fn take_pending(state: &AppState) -> Result<(Vec<PendingDoc>, Vec<PendingChunk>), AppError> {
    let doc_rows = search_docs::Entity::find()
        .filter(search_docs::COLUMN.embed_state.eq(EmbedState::Pending))
        .order_by_asc(search_docs::COLUMN.id)
        .limit(BATCH_SIZE as u64)
        .all(state.db_conn())
        .await?;
    let chunk_rows = knowledge_chunks::Entity::find()
        .filter(knowledge_chunks::COLUMN.embed_state.eq(EmbedState::Pending))
        .order_by_asc(knowledge_chunks::COLUMN.id)
        .limit(BATCH_SIZE as u64)
        .all(state.db_conn())
        .await?;
    let entry_ids: Vec<i32> = chunk_rows.iter().map(|row| row.entry_id).collect();
    let titles: std::collections::HashMap<i32, String> = knowledge_entries::Entity::find()
        .filter(knowledge_entries::COLUMN.id.is_in(entry_ids))
        .all(state.db_conn())
        .await?
        .into_iter()
        .map(|entry| (entry.id, entry.title))
        .collect();
    let mut chunk_items = Vec::with_capacity(chunk_rows.len());
    for row in chunk_rows {
        let title = titles.get(&row.entry_id).cloned().unwrap_or_default();
        chunk_items.push(PendingChunk {
            id: row.id,
            content_hash: row.content_hash,
            title,
            heading: row.heading,
            text: row.text,
        });
    }
    let docs: Vec<PendingDoc> = doc_rows
        .into_iter()
        .map(|row| PendingDoc {
            id: row.id,
            content_hash: row.content_hash,
            title: row.title,
            body: row.body,
        })
        .collect();
    // Alternate: doc, chunk, doc, chunk … up to BATCH_SIZE items overall.
    let mut selected_docs = Vec::new();
    let mut selected_chunks = Vec::new();
    let mut doc_iter = docs.into_iter();
    let mut chunk_iter = chunk_items.into_iter();
    loop {
        if selected_docs.len() + selected_chunks.len() >= BATCH_SIZE {
            break;
        }
        let mut progressed = false;
        if let Some(doc) = doc_iter.next() {
            selected_docs.push(doc);
            progressed = true;
            if selected_docs.len() + selected_chunks.len() >= BATCH_SIZE {
                break;
            }
        }
        if let Some(chunk) = chunk_iter.next() {
            selected_chunks.push(chunk);
            progressed = true;
        }
        if !progressed {
            break;
        }
    }
    Ok((selected_docs, selected_chunks))
}

/// Marks a batch `failed`; an item whose content changed meanwhile stays `pending` for the next
/// batch (spec/07 §4).
async fn mark_failed(
    state: &AppState,
    docs: &[PendingDoc],
    chunks: &[PendingChunk],
) -> Result<usize, AppError> {
    let changes: Vec<(i32, String, bool)> = docs
        .iter()
        .map(|doc| (doc.id, doc.content_hash.clone(), true))
        .chain(
            chunks
                .iter()
                .map(|item| (item.id, item.content_hash.clone(), false)),
        )
        .collect();
    crate::services::write(state.db(), move |tx| {
        Box::pin(async move {
            let mut failed = 0;
            for (id, hash, is_doc) in changes {
                let result = if is_doc {
                    search_docs::Entity::update_many()
                        .set(search_docs::ActiveModel {
                            embed_state: Set(EmbedState::Failed),
                            ..Default::default()
                        })
                        .filter(search_docs::COLUMN.id.eq(id))
                        .filter(search_docs::COLUMN.content_hash.eq(hash))
                        .exec(tx)
                        .await?
                } else {
                    knowledge_chunks::Entity::update_many()
                        .set(knowledge_chunks::ActiveModel {
                            embed_state: Set(EmbedState::Failed),
                            ..Default::default()
                        })
                        .filter(knowledge_chunks::COLUMN.id.eq(id))
                        .filter(knowledge_chunks::COLUMN.content_hash.eq(hash))
                        .exec(tx)
                        .await?
                };
                failed += result.rows_affected as usize;
            }
            Ok(failed)
        })
    })
    .await
}

async fn mark_doc_done(conn: &impl ConnectionTrait, id: i32, hash: &str) -> Result<(), AppError> {
    search_docs::Entity::update_many()
        .set(search_docs::ActiveModel {
            embed_state: Set(EmbedState::Done),
            ..Default::default()
        })
        .filter(search_docs::COLUMN.id.eq(id))
        .filter(search_docs::COLUMN.content_hash.eq(hash))
        .exec(conn)
        .await?;
    Ok(())
}

async fn mark_chunk_done(conn: &impl ConnectionTrait, id: i32, hash: &str) -> Result<(), AppError> {
    knowledge_chunks::Entity::update_many()
        .set(knowledge_chunks::ActiveModel {
            embed_state: Set(EmbedState::Done),
            ..Default::default()
        })
        .filter(knowledge_chunks::COLUMN.id.eq(id))
        .filter(knowledge_chunks::COLUMN.content_hash.eq(hash))
        .exec(conn)
        .await?;
    Ok(())
}

/// The worker loop: wait for a wake signal or 5 s, then process one batch (spec/07 §4).
pub fn spawn(state: AppState) {
    tracing::info!("starting the indexer worker");
    tokio::spawn(async move {
        // The first load (a download on the first run) happens here, off the request path.
        state.search().ensure_ready().await;
        tracing::info!(
            ready = state.search().is_ready(),
            "the indexer worker is ready"
        );
        state.search().publish_status().await;
        loop {
            index::wait_for_work().await;
            if !state.search().is_ready() {
                continue;
            }
            if let Err(err) = run_once(&state).await {
                tracing::warn!(error = %err, "the indexer batch failed");
            }
        }
    });
}

/// Rebuilds the index: every document becomes `pending` and the embedder reloads if needed
/// (spec/07 §4 "Failed items are retried when the model reloads or on Rebuild index").
pub async fn rebuild(state: &AppState) -> Result<(), AppError> {
    crate::services::write(state.db(), |tx| {
        Box::pin(async move {
            search_docs::Entity::update_many()
                .set(search_docs::ActiveModel {
                    embed_state: Set(EmbedState::Pending),
                    ..Default::default()
                })
                .filter(search_docs::COLUMN.embed_state.ne(EmbedState::Pending))
                .exec(tx)
                .await?;
            knowledge_chunks::Entity::update_many()
                .set(knowledge_chunks::ActiveModel {
                    embed_state: Set(EmbedState::Pending),
                    ..Default::default()
                })
                .filter(knowledge_chunks::COLUMN.embed_state.ne(EmbedState::Pending))
                .exec(tx)
                .await?;
            Ok(())
        })
    })
    .await?;
    index::wake();
    state.search().publish_status().await;
    Ok(())
}
