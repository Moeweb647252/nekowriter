//! Embeddings for semantic knowledge search (`docs/product-spec.md` §12.2, D10).

use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    field::{Field, FieldError, FieldProblem},
    model::ModelId,
    run::ProviderFailure,
};

/// An embedding model and the size of the vectors it produces. Vectors of different
/// models or sizes are never compared.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, FromJsonQueryResult)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EmbeddingModel {
    pub model_id: ModelId,
    pub dimensions: u32,
}

impl EmbeddingModel {
    pub const MAX_DIMENSIONS: u32 = 8_192;

    pub fn validate(&self) -> Result<(), FieldError> {
        if (1..=Self::MAX_DIMENSIONS).contains(&self.dimensions) {
            Ok(())
        } else {
            Err(FieldError {
                field: Field::Dimensions,
                problem: FieldProblem::OutOfRange {
                    min: 1,
                    max: Self::MAX_DIMENSIONS,
                },
            })
        }
    }
}

/// Why texts could not be embedded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, FromJsonQueryResult)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum EmbeddingFailure {
    Provider {
        failure: ProviderFailure,
    },
    /// The vectors have another size than the settings declare.
    WrongDimensions {
        expected: u32,
        actual: u32,
    },
    /// A vector holds a value that is not a finite number.
    NotFinite,
    /// The service returned another number of vectors than texts were sent.
    WrongCount {
        expected: u32,
        actual: u32,
    },
}

/// A vector as stored: its values as little-endian `f32`.
#[must_use]
pub fn vector_bytes(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// A stored vector; `None` if the bytes are not a whole number of `f32` values.
#[must_use]
pub fn vector_from_bytes(bytes: &[u8]) -> Option<Vec<f32>> {
    let (values, rest) = bytes.as_chunks::<4>();
    rest.is_empty().then(|| {
        values
            .iter()
            .map(|value| f32::from_le_bytes(*value))
            .collect()
    })
}

/// The cosine similarity of two vectors of the same size; 0 when either is zero.
#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (mut dot, mut norm_a, mut norm_b) = (0.0_f32, 0.0_f32, 0.0_f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a.sqrt() * norm_b.sqrt())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_round_trip_and_compare_by_direction() {
        let vector = [1.0, -0.5, 0.25];
        assert_eq!(
            vector_from_bytes(&vector_bytes(&vector)),
            Some(vector.to_vec())
        );
        assert_eq!(vector_from_bytes(&[0, 1, 2]), None);
        assert!((cosine(&[1.0, 0.0], &[2.0, 0.0]) - 1.0).abs() < f32::EPSILON);
        assert!(cosine(&[1.0, 0.0], &[0.0, 3.0]).abs() < f32::EPSILON);
        assert!(cosine(&[0.0, 0.0], &[1.0, 0.0]).abs() < f32::EPSILON);
    }
}
