//! Runs: agent executions, and why they stop. The execution axis of
//! `docs/product-spec.md` §5.1.

use sea_orm::{DeriveActiveEnum, EnumIter, FromJsonQueryResult, sea_query::StringLen};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ids::RevisionId, model::ModelProblem};

/// How many model requests a run may make before the user has to continue it explicitly
/// (§8.7). Continuing grants the same number again.
pub const REQUEST_ALLOWANCE: i32 = 100;

/// Consecutive protocol errors after which a run stops (§8.7).
pub const MAX_CONSECUTIVE_PROTOCOL_ERRORS: i32 = 3;

/// What a run does.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS, EnumIter, DeriveActiveEnum,
)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunKind {
    /// Writes the four preparation artifacts and the chapter plan.
    Preparation,
    /// Writes a chapter's outline.
    ChapterOutline,
    /// Writes a chapter's body.
    ChapterBody,
}

/// What a run is doing now.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS, EnumIter, DeriveActiveEnum,
)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunStatus {
    /// Waiting for the model service.
    Queued,
    /// A model request is in flight.
    Running,
    /// Its submission waits for review.
    WaitingReview,
    /// The user paused it: a tool call or submission is held until the user continues.
    Paused,
    /// It asked the user questions and waits for the answers.
    WaitingAnswer,
    /// The process ended while the run was executing; the user decides whether to continue.
    Interrupted,
    /// Stopped by a limit or by the user; it can be continued.
    Stopped,
    /// A request or its preparation failed; it can be continued after fixing the cause.
    Failed,
    /// Its stage is approved and folded.
    Succeeded,
}

impl RunStatus {
    /// Whether the run holds its project's single writing slot (§4 invariant 2). Stopped
    /// and failed runs release it; continuing them takes it again.
    #[must_use]
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Running
                | Self::WaitingReview
                | Self::Paused
                | Self::WaitingAnswer
                | Self::Interrupted
        )
    }

    /// Whether the user can continue the run.
    #[must_use]
    pub fn can_continue(self) -> bool {
        matches!(self, Self::Interrupted | Self::Stopped | Self::Failed)
    }

    /// Whether the user can stop the run: it is waiting for the model, talking to it, or
    /// paused.
    #[must_use]
    pub fn can_stop(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Paused)
    }

    /// Whether the user can give the run feedback or pause it: it is writing, or waiting
    /// for the answer to a question it asked.
    #[must_use]
    pub fn is_writing(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Running | Self::Paused | Self::WaitingAnswer
        )
    }
}

/// Why a run stopped, failed or was interrupted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, FromJsonQueryResult)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum RunHalt {
    /// The run used its request allowance.
    RequestLimit,
    /// The model broke the protocol too many times in a row.
    ProtocolErrorLimit,
    /// The model settings do not allow a request.
    ModelNotReady { problem: ModelProblem },
    /// The context does not fit the model's window even after folding.
    ContextLimitExceeded {
        estimated_tokens: u32,
        limit_tokens: u32,
    },
    /// The model service failed.
    Provider { failure: ProviderFailure },
    /// The application stopped while the run was executing.
    ProcessStopped,
    /// The user stopped the run.
    StoppedByUser,
    /// The application could not carry out the run's next step; the log has the details.
    Internal,
}

/// The approved revisions a completed stage folds into, in the order the fold presents them
/// (§11). The fold's text is rendered from them for every later request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
pub struct Fold(pub Vec<RevisionId>);

/// How a request to the model service failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, FromJsonQueryResult)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ProviderFailure {
    /// The service could not be reached.
    Connect,
    /// The service took too long at the given phase.
    Timeout { phase: TimeoutPhase },
    /// The service answered with an HTTP error status.
    HttpStatus { status: u16 },
    /// The service answered with something that is not a valid streamed completion.
    InvalidResponse,
    /// The response exceeded the size limit.
    ResponseTooLarge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TimeoutPhase {
    Connect,
    FirstByte,
    Idle,
}
