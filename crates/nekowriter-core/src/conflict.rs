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
    #[error("workspace")]
    Workspace,
    #[error("review")]
    Review,
    #[error("run")]
    Run,
    #[error("settings")]
    Settings,
}

/// A request that is valid in itself but conflicts with the current state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS, Error)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StateConflict {
    /// Only a deleted project can be restored.
    #[error("the project is not deleted")]
    ProjectNotDeleted,
    /// The brief can be edited only until preparation starts.
    #[error("preparation has started, so the brief can no longer change")]
    BriefLocked,
    /// Preparation needs a brief.
    #[error("the version has no brief yet")]
    BriefMissing,
    /// The request does not fit the current workflow step.
    #[error("the version is not at the step this request needs")]
    WrongStep,
    /// Another version of the project is being written (§4 invariant 2).
    #[error("another version of this project has an active run")]
    ProjectBusy,
    /// The review was already decided or is no longer part of the current round.
    #[error("the review is not open")]
    ReviewNotOpen,
    /// The project has a run in progress or waiting for review; it cannot be deleted now.
    #[error("the project has an active run")]
    RunActive,
    /// Only a stopped, failed or interrupted run can be continued.
    #[error("the run cannot be continued in its current state")]
    RunNotContinuable,
}
