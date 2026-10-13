use nekowriter_core::{
    conflict::{Resource, StateConflict},
    ids::CommandId,
    model::ModelProblem,
};
use sea_orm::DbErr;
use thiserror::Error;

/// Why a store operation failed.
///
/// The `Display` text is for logs. Callers decide what to show users from the variant.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] DbErr),
    /// The object does not exist, is deleted, or has a deleted ancestor.
    #[error("{0} not found")]
    NotFound(Resource),
    /// The client acted on an outdated copy of the object.
    #[error("the client expected {resource} revision {expected}, but it is at revision {actual}")]
    RevisionConflict {
        resource: Resource,
        expected: i64,
        actual: i64,
    },
    /// The command ID was already used by a different command or input.
    #[error("command {0} was already used for a different request")]
    IdempotencyConflict(CommandId),
    #[error("{0}")]
    StateConflict(StateConflict),
    /// The model settings do not allow requests, so work that needs the model cannot start.
    #[error("the model is not ready: {0:?}")]
    ModelNotReady(ModelProblem),
    /// Stored data violates an invariant the application maintains.
    #[error("stored data is inconsistent: {0}")]
    Corrupt(String),
}

impl From<serde_json::Error> for StoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Corrupt(format!(
            "a JSON column could not be read or written: {error}"
        ))
    }
}
