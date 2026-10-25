//! `FastEmbedder`: the real embedding model behind `fastembed` (spec/07 §4).
//!
//! `TextEmbedding::embed` is CPU-bound, so it runs through `spawn_blocking` behind a
//! `std::sync::Mutex` (spec/01 §7). Loading is lazy: the model downloads on first use and the
//! status is observable for `index.status` events.
//!
//! The observable states are `not_loaded → loading (downloading) → ready | failed(message)`
//! (spec/07 §4). A failed load is reported, never retried implicitly: the indexer waits, and the
//! model reloads on a model change or on Rebuild index (spec/07 §4).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

use crate::fake::{EmbedError, EmbedKind, Embedder, normalize};
use crate::models::{ModelEntry, SUPPORTED_MODELS};

/// Observable loading state of the model (spec/07 §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedderStatus {
    NotLoaded,
    /// Loading or downloading; the UI shows an indeterminate state (spec/07 §4).
    Loading,
    Ready,
    Failed(String),
}

impl EmbedderStatus {
    /// True once the model can embed.
    pub fn is_ready(&self) -> bool {
        matches!(self, EmbedderStatus::Ready)
    }
}

/// Maps a table variant name to the fastembed enum. `None` means this fastembed version does
/// not offer it.
fn fastembed_variant(name: &str) -> Option<EmbeddingModel> {
    match name {
        "MultilingualE5Small" => Some(EmbeddingModel::MultilingualE5Small),
        "BGESmallZHV15" => Some(EmbeddingModel::BGESmallZHV15),
        "EmbeddingGemma300MQ" => Some(EmbeddingModel::EmbeddingGemma300MQ),
        _ => None,
    }
}

/// The model table entry plus its resolved fastembed enum.
fn resolve(key: &str) -> Result<(&'static ModelEntry, EmbeddingModel), EmbedError> {
    let entry = crate::models::model(key)
        .or_else(|| SUPPORTED_MODELS.iter().find(|entry| entry.key == key))
        .ok_or_else(|| EmbedError::UnknownModel(key.to_owned()))?;
    let variant = fastembed_variant(&entry.variant)
        .ok_or_else(|| EmbedError::UnknownModel(format!("{} ({})", entry.key, entry.variant)))?;
    Ok((entry, variant))
}

/// Loads one model synchronously. Kept separate from the async plumbing so the dimension
/// assertion (spec/07 §4) is unit-testable without a network: `get_model_info` is static
/// metadata, and a mismatch is rejected before any download starts.
fn load_model(
    variant: &EmbeddingModel,
    expected: usize,
    cache_dir: PathBuf,
    entry_key: &str,
) -> Result<TextEmbedding, String> {
    let info = TextEmbedding::get_model_info(variant)
        .map_err(|err| format!("model info unavailable: {err}"))?;
    if info.dim != expected {
        return Err(format!(
            "dimension mismatch for {entry_key}: model reports {}, the table says {expected}",
            info.dim
        ));
    }
    // fastembed's own progress bar is off (spec/07 §4): the download is reported through the
    // `index.status` event stream and the log lines below instead.
    let mut options = TextInitOptions::default();
    options.model_name = variant.clone();
    options.cache_dir = cache_dir;
    options.show_download_progress = false;
    TextEmbedding::try_new(options).map_err(|err| format!("cannot load the embedding model: {err}"))
}

/// The real embedder.
pub struct FastEmbedder {
    entry: &'static ModelEntry,
    variant: EmbeddingModel,
    cache_dir: PathBuf,
    session: Arc<Mutex<Option<TextEmbedding>>>,
    status: Arc<Mutex<EmbedderStatus>>,
    /// Serializes a load, so two concurrent first calls cannot start two downloads (spec/07 §4:
    /// the status moves through `loading` exactly once).
    load_lock: tokio::sync::Mutex<()>,
}

impl FastEmbedder {
    /// Builds a lazy embedder; no download happens until the first `embed` or `warm_up`.
    pub fn new(key: &str, cache_dir: PathBuf) -> Result<Self, EmbedError> {
        let (entry, variant) = resolve(key)?;
        Ok(Self {
            entry,
            variant,
            cache_dir,
            session: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new(EmbedderStatus::NotLoaded)),
            load_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// The current loading state.
    pub fn status(&self) -> EmbedderStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|_| EmbedderStatus::Failed("status lock poisoned".to_owned()))
    }

    /// True once the model can embed (shown as `ready` in Settings › Embeddings).
    pub fn is_ready(&self) -> bool {
        self.status().is_ready()
    }

    /// Asserts the dimension reported by fastembed against the table (spec/07 §4).
    pub fn table_dimension(&self) -> usize {
        self.entry.dimension
    }

    /// The model key this embedder was built for.
    pub fn model_key(&self) -> &str {
        &self.entry.key
    }

    /// Loads the model if needed. Safe to call from a background warm-up task.
    pub async fn warm_up(&self) -> Result<(), EmbedError> {
        self.load_model_once().await
    }

    /// Sets an observable failure state. Only used by the test that drives the transition table
    /// without a network download; production failures come from [`load_model`].
    #[cfg(test)]
    pub(crate) fn fail_for_test(&self, message: &str) {
        if let Ok(mut status) = self.status.lock() {
            *status = EmbedderStatus::Failed(message.to_owned());
        }
    }

    fn set_status(&self, status: EmbedderStatus) {
        if let Ok(mut guard) = self.status.lock() {
            *guard = status;
        }
    }

    async fn load_model_once(&self) -> Result<(), EmbedError> {
        if self
            .session
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
        {
            return Ok(());
        }
        // One load at a time; a second caller waits and then observes the result.
        let _load_guard = self.load_lock.lock().await;
        if self
            .session
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
        {
            return Ok(());
        }
        self.set_status(EmbedderStatus::Loading);
        tracing::info!(
            model = %self.entry.key,
            "loading the embedding model (the first run downloads it)"
        );

        let cache_dir = self.cache_dir.clone();
        let variant = self.variant.clone();
        let expected = self.entry.dimension;
        let entry_key = self.entry.key.clone();

        let loaded = tokio::task::spawn_blocking(move || {
            load_model(&variant, expected, cache_dir, &entry_key)
        })
        .await;

        match loaded {
            Ok(Ok(model)) => {
                if let Ok(mut guard) = self.session.lock() {
                    *guard = Some(model);
                }
                self.set_status(EmbedderStatus::Ready);
                tracing::info!(
                    model = %self.entry.key,
                    "the embedding model is ready"
                );
                Ok(())
            }
            Ok(Err(message)) => {
                tracing::warn!(
                    model = %self.entry.key,
                    error = %message,
                    "the embedding model could not be loaded"
                );
                self.set_status(EmbedderStatus::Failed(message.clone()));
                Err(EmbedError::Failed(message))
            }
            Err(err) => {
                let message = format!("the model-loading task failed: {err}");
                self.set_status(EmbedderStatus::Failed(message.clone()));
                Err(EmbedError::Failed(message))
            }
        }
    }
}

#[async_trait::async_trait]
impl Embedder for FastEmbedder {
    async fn ensure_loaded(&self) -> Result<(), EmbedError> {
        self.load_model_once().await
    }

    fn model(&self) -> &ModelEntry {
        self.entry
    }

    fn dimension(&self) -> usize {
        self.entry.dimension
    }

    async fn embed(
        &self,
        texts: Vec<String>,
        kind: EmbedKind,
    ) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.load_model_once().await?;
        let prefixed: Vec<String> = texts
            .into_iter()
            .map(|text| match kind {
                EmbedKind::Query => self.entry.query_text(&text),
                EmbedKind::Passage => self.entry.passage_text(&text),
            })
            .collect();

        let session = self.session.clone();
        let vectors = tokio::task::spawn_blocking(move || -> Result<Vec<Vec<f32>>, String> {
            let mut guard = session
                .lock()
                .map_err(|_| "embedding session lock poisoned".to_owned())?;
            let model = guard
                .as_mut()
                .ok_or_else(|| "the embedding model is not loaded".to_owned())?;
            let embeddings = model
                .embed(prefixed, None)
                .map_err(|err| format!("embedding failed: {err}"))?;
            let mut vectors: Vec<Vec<f32>> = embeddings;
            // Cosine distance in vec0 wants unit vectors (spec/07 §4).
            for vector in &mut vectors {
                normalize(vector);
            }
            Ok(vectors)
        })
        .await
        .map_err(|err| EmbedError::Failed(format!("the embedding task failed: {err}")))?
        .map_err(EmbedError::Failed)?;

        Ok(vectors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_model_resolves() {
        for entry in SUPPORTED_MODELS.iter() {
            assert!(
                resolve(&entry.key).is_ok(),
                "{} must map to a fastembed variant",
                entry.key
            );
        }
    }

    #[test]
    fn unknown_model_is_an_error() {
        assert!(matches!(resolve("nope"), Err(EmbedError::UnknownModel(_))));
    }

    #[test]
    fn status_starts_not_loaded() {
        let embedder =
            FastEmbedder::new("multilingual-e5-small", PathBuf::from(".")).expect("embedder");
        assert_eq!(embedder.status(), EmbedderStatus::NotLoaded);
        assert_eq!(embedder.table_dimension(), 384);
        assert!(!embedder.is_ready());
    }

    /// spec/07 §4: the dimension comes from `TextEmbedding::get_model_info` and is asserted
    /// against the table. `get_model_info` is static metadata, so this test needs no network.
    #[test]
    fn every_model_info_dimension_matches_the_table() {
        for entry in SUPPORTED_MODELS.iter() {
            let variant = fastembed_variant(&entry.variant).expect("a fastembed variant");
            let info = TextEmbedding::get_model_info(&variant).expect("model info");
            assert_eq!(
                info.dim, entry.dimension,
                "{}: fastembed reports {}, models.toml says {}",
                entry.key, info.dim, entry.dimension
            );
        }
    }

    /// A mismatch is rejected before the download starts, so a wrong table cannot yield vectors
    /// of the wrong width (spec/07 §4).
    #[test]
    fn dimension_mismatch_is_rejected_before_loading() {
        let variant = EmbeddingModel::MultilingualE5Small;
        let error = match load_model(&variant, 999, PathBuf::from("."), "test-model") {
            Ok(_) => panic!("a mismatched dimension is an error"),
            Err(error) => error,
        };
        assert!(
            error.contains("dimension mismatch"),
            "unexpected error: {error}"
        );
    }

    /// The transition table of spec/07 §4: `not_loaded → loading → ready | failed`, with the
    /// failure carrying its message.
    #[test]
    fn failure_is_observable_with_its_message() {
        let embedder =
            FastEmbedder::new("multilingual-e5-small", PathBuf::from(".")).expect("embedder");
        assert_eq!(embedder.status(), EmbedderStatus::NotLoaded);
        embedder.fail_for_test("download failed");
        assert_eq!(
            embedder.status(),
            EmbedderStatus::Failed("download failed".to_owned())
        );
        assert!(!embedder.is_ready());
    }

    /// Loads the default model for real. Ignored by default because the first run downloads
    /// several hundred megabytes (PLAN 7.1: run manually only).
    #[tokio::test]
    #[ignore = "downloads the default model; run manually with --ignored"]
    async fn the_default_model_loads_and_normalizes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let embedder =
            FastEmbedder::new("multilingual-e5-small", dir.path().to_path_buf()).expect("embedder");
        embedder.warm_up().await.expect("the model loads");
        assert_eq!(embedder.status(), EmbedderStatus::Ready);
        let vectors = embedder
            .embed(
                vec!["a fox jumps over the dog".to_owned()],
                EmbedKind::Passage,
            )
            .await
            .expect("embedding");
        assert_eq!(vectors[0].len(), 384);
        let norm: f32 = vectors[0]
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-4,
            "vectors are L2-normalized: {norm}"
        );
        // A query and a passage of similar meaning are closer than an unrelated passage.
        let query = embedder
            .embed(vec!["a fox".to_owned()], EmbedKind::Query)
            .await
            .expect("query");
        let other = embedder
            .embed(
                vec!["quantum chromodynamics".to_owned()],
                EmbedKind::Passage,
            )
            .await
            .expect("other");
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        assert!(
            dot(&query[0], &vectors[0]) > dot(&query[0], &other[0]),
            "the related passage must be closer"
        );
    }
}
