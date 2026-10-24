//! `FastEmbedder`: the real embedding model behind `fastembed` (spec/07 §4).
//!
//! `TextEmbedding::embed` is CPU-bound, so it runs through `spawn_blocking` behind a
//! `std::sync::Mutex` (spec/01 §7). Loading is lazy: the model downloads on first use and the
//! status is observable for `index.status` events.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

use crate::fake::{EmbedError, EmbedKind, Embedder, normalize};
use crate::models::{ModelEntry, SUPPORTED_MODELS};

/// Observable loading state of the model (spec/07 §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedderStatus {
    NotLoaded,
    Loading,
    Ready,
    Failed(String),
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

/// The real embedder.
pub struct FastEmbedder {
    entry: &'static ModelEntry,
    variant: EmbeddingModel,
    cache_dir: PathBuf,
    session: Arc<Mutex<Option<TextEmbedding>>>,
    status: Arc<Mutex<EmbedderStatus>>,
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
        })
    }

    /// The current loading state.
    pub fn status(&self) -> EmbedderStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|_| EmbedderStatus::Failed("status lock poisoned".to_owned()))
    }

    /// Asserts the dimension reported by fastembed against the table (spec/07 §4).
    pub fn table_dimension(&self) -> usize {
        self.entry.dimension
    }

    /// Loads the model if needed. Safe to call from a background warm-up task.
    pub async fn warm_up(&self) -> Result<(), EmbedError> {
        self.ensure_loaded().await
    }

    async fn ensure_loaded(&self) -> Result<(), EmbedError> {
        if self
            .session
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
        {
            return Ok(());
        }
        // Set the status in a scope so no guard is held across the loading await.
        if let Ok(mut status) = self.status.lock() {
            *status = EmbedderStatus::Loading;
        }

        let cache_dir = self.cache_dir.clone();
        let variant = self.variant.clone();
        let expected = self.entry.dimension;
        let entry_key = self.entry.key.clone();

        let loaded = tokio::task::spawn_blocking(move || -> Result<TextEmbedding, String> {
            tracing::info!(model = %entry_key, "loading the embedding model");
            let info = TextEmbedding::get_model_info(&variant)
                .map_err(|err| format!("model info unavailable: {err}"))?;
            if info.dim != expected {
                return Err(format!(
                    "dimension mismatch for {entry_key}: model reports {}, the table says {expected}",
                    info.dim
                ));
            }
            let mut options = TextInitOptions::default();
            options.model_name = variant;
            options.cache_dir = cache_dir;
            options.show_download_progress = false;
            let model = TextEmbedding::try_new(options)
                .map_err(|err| format!("cannot load the embedding model: {err}"))?;
            tracing::info!(model = %entry_key, "the embedding model is ready");
            Ok(model)
        })
        .await;

        match loaded {
            Ok(Ok(model)) => {
                if let Ok(mut guard) = self.session.lock() {
                    *guard = Some(model);
                }
                if let Ok(mut status) = self.status.lock() {
                    *status = EmbedderStatus::Ready;
                }
                Ok(())
            }
            Ok(Err(message)) => {
                if let Ok(mut status) = self.status.lock() {
                    *status = EmbedderStatus::Failed(message.clone());
                }
                Err(EmbedError::Failed(message))
            }
            Err(err) => {
                let message = format!("the model-loading task failed: {err}");
                if let Ok(mut status) = self.status.lock() {
                    *status = EmbedderStatus::Failed(message.clone());
                }
                Err(EmbedError::Failed(message))
            }
        }
    }
}

#[async_trait::async_trait]
impl Embedder for FastEmbedder {
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
        self.ensure_loaded().await?;
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
    }
}
