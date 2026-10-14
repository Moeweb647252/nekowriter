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
    #[error("revision")]
    Revision,
    #[error("question")]
    Question,
    #[error("settings")]
    Settings,
    #[error("prompt template")]
    Template,
    #[error("knowledge space")]
    Space,
    #[error("knowledge entry")]
    Entry,
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
    /// Only a run that is waiting for the model or talking to it can be stopped.
    #[error("the run cannot be stopped in its current state")]
    RunNotStoppable,
    /// Feedback and pauses are for a chapter body that is being written.
    #[error("the run is not writing a chapter body")]
    RunNotWriting,
    /// Only a paused run can be resumed.
    #[error("the run is not paused")]
    RunNotPaused,
    /// Only a review that is delegated can have its delegation cancelled.
    #[error("the review is not delegated")]
    ReviewNotDelegated,
    /// A delegated review runs on its own; it cannot be continued.
    #[error("a delegated review cannot be continued; delegate it again")]
    ReviewRunNotContinuable,
    /// The question was already answered or is no longer waiting.
    #[error("the question is not waiting for an answer")]
    QuestionNotOpen,
    /// A space that still holds entries cannot be deleted.
    #[error("the knowledge space still has entries")]
    SpaceNotEmpty,
    /// Another space already has the name; agents choose spaces by name.
    #[error("another knowledge space has this name")]
    SpaceNameTaken,
}
