//! Search indexing and hybrid retrieval (spec/07 §2–§5): chunking, jieba FTS, the runtime vec
//! tables, `search_docs` maintenance and the reciprocal-rank-fusion query.
//!
//! Layering: [`chunk`] and [`fts`] are pure functions; [`index`] owns the runtime vec schema and
//! the embedder state; [`maintenance`] keeps `search_docs`/`fts_book` in step with the source rows
//! inside the writing transaction; [`query`] fuses keyword and vector hits.

pub mod chunk;
pub mod fts;
pub mod index;
pub mod maintenance;

pub use index::{
    IndexState, SearchService, ensure_schema, initialize, knn_book, knn_knowledge, switch_model,
};
pub use maintenance::sync_events;
