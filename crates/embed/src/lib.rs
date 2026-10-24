//! Embedding support: the `Embedder` trait, the fastembed-backed implementation and a
//! deterministic fake embedder for tests (spec/07 §4).

pub mod fake;
pub mod fast;
pub mod models;

pub use fake::{EmbedError, EmbedKind, Embedder, FakeEmbedder, normalize, to_blob};
pub use fast::{EmbedderStatus, FastEmbedder};
pub use models::{DEFAULT_MODEL_KEY, ModelEntry, SUPPORTED_MODELS, default_model, model};
