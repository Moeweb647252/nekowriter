//! Why a request cannot be carried out in the current state of the data.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

/// A kind of stored object, used to say which object a request referred to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS, Error)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Resource {
    #[error("project")]
    Project,
    #[error("version")]
    Version,
}

/// A request that is valid in itself but conflicts with the current state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS, Error)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StateConflict {
    /// Only a deleted project can be restored.
    #[error("the project is not deleted")]
    ProjectNotDeleted,
}
