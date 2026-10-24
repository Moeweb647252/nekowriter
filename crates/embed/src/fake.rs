//! The `Embedder` trait and the deterministic `FakeEmbedder` used by every test (spec/07 §4).

use std::collections::HashMap;

use crate::models::ModelEntry;

/// Query vs passage input convention (spec/07 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedKind {
    Query,
    Passage,
}

/// What a search index needs from an embedder.
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    /// The model currently backing this embedder.
    fn model(&self) -> &ModelEntry;

    /// The vector dimension.
    fn dimension(&self) -> usize;

    /// Embeds a batch of texts. The order of the result matches the input order.
    async fn embed(&self, texts: Vec<String>, kind: EmbedKind)
    -> Result<Vec<Vec<f32>>, EmbedError>;
}

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("the embedding model is not ready: {0}")]
    NotReady(String),
    #[error("embedding failed: {0}")]
    Failed(String),
    #[error("the embedding model {0} does not exist")]
    UnknownModel(String),
}

/// Deterministic 32-dimension vectors from blake3 hashes of character trigrams, so similar
/// texts get similar vectors (spec/07 §4).
pub struct FakeEmbedder {
    model: ModelEntry,
}

impl FakeEmbedder {
    /// A fake embedder that reports the default model's metadata but 32 dimensions.
    pub fn new(fallback_dimension: usize) -> Self {
        let mut model = crate::models::default_model().clone();
        model.dimension = fallback_dimension;
        Self { model }
    }

    /// Builds a fake with the given dimension (32 by default in tests).
    pub fn with_dimension(dimension: usize) -> Self {
        Self::new(dimension)
    }

    /// Computes one vector, exposed for direct assertions in tests.
    pub fn vector(text: &str, dimension: usize) -> Vec<f32> {
        let chars: Vec<char> = text.chars().collect();
        let mut buckets = vec![0.0f32; dimension];
        if chars.is_empty() {
            return buckets;
        }
        // Character trigrams (padded so short strings still produce signal).
        let padded: Vec<char> = std::iter::once('^')
            .chain(chars.iter().copied())
            .chain(std::iter::once('$'))
            .collect();
        for window in padded.windows(3) {
            let token: String = window.iter().collect();
            let hash = blake3::hash(token.as_bytes());
            let bytes = hash.as_bytes();
            let index =
                u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize % dimension;
            let sign = if bytes[4].is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            buckets[index] += sign;
        }
        normalize(&mut buckets);
        buckets
    }
}

impl Default for FakeEmbedder {
    fn default() -> Self {
        Self::new(32)
    }
}

#[async_trait::async_trait]
impl Embedder for FakeEmbedder {
    fn model(&self) -> &ModelEntry {
        &self.model
    }

    fn dimension(&self) -> usize {
        self.model.dimension
    }

    async fn embed(
        &self,
        texts: Vec<String>,
        kind: EmbedKind,
    ) -> Result<Vec<Vec<f32>>, EmbedError> {
        let _ = kind;
        Ok(texts
            .iter()
            .map(|text| Self::vector(text, self.model.dimension))
            .collect())
    }
}

/// L2-normalizes a vector in place (cosine distance in vec0 wants unit vectors).
pub fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in vector.iter_mut() {
            *value /= norm;
        }
    }
}

/// Packs a vector as little-endian `f32` bytes for the vec0 BLOB column (spec/07 §3).
pub fn to_blob(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// A registry of named embedders, used by the server to swap models at runtime.
pub type EmbedderMap = HashMap<String, Box<dyn Embedder>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_embeddings_are_deterministic_and_similar() {
        let embedder = FakeEmbedder::default();
        let a = embedder
            .embed(vec!["hello world".to_owned()], EmbedKind::Passage)
            .await
            .expect("embed");
        let b = embedder
            .embed(vec!["hello world".to_owned()], EmbedKind::Passage)
            .await
            .expect("embed");
        assert_eq!(a, b);
        assert_eq!(a[0].len(), 32);
        let norm: f32 = a[0].iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "unit length, got {norm}");

        let near = embedder
            .embed(vec!["hello worlds".to_owned()], EmbedKind::Passage)
            .await
            .expect("embed");
        let far = embedder
            .embed(
                vec!["completely different text".to_owned()],
                EmbedKind::Passage,
            )
            .await
            .expect("embed");
        let dot_near: f32 = a[0].iter().zip(&near[0]).map(|(x, y)| x * y).sum();
        let dot_far: f32 = a[0].iter().zip(&far[0]).map(|(x, y)| x * y).sum();
        assert!(
            dot_near > dot_far,
            "similar texts must be closer: {dot_near} vs {dot_far}"
        );
    }

    #[test]
    fn blob_round_trip() {
        let vector = vec![0.5f32, -1.25, 0.0];
        let blob = to_blob(&vector);
        assert_eq!(blob.len(), 12);
        let restored: Vec<f32> = blob
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| f32::from_le_bytes(*chunk))
            .collect();
        assert_eq!(restored, vector);
    }

    #[test]
    fn normalize_handles_zero_vectors() {
        let mut zero = vec![0.0f32; 4];
        normalize(&mut zero);
        assert_eq!(zero, vec![0.0; 4]);
        let mut unit = vec![3.0f32, 4.0];
        normalize(&mut unit);
        assert!((unit[0] - 0.6).abs() < 1e-6);
        assert!((unit[1] - 0.8).abs() < 1e-6);
    }
}
