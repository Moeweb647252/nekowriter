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
