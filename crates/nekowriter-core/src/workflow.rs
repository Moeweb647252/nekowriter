//! Where a version is in the story: the workflow axis of `docs/product-spec.md` §5.
//!
//! The workflow is hierarchical: a phase (preparation, a chapter, completed) and a step
//! inside it. What the agent is doing at that point is the separate execution axis, the
//! status of the version's active run (`crate::run`).

use sea_orm::{DeriveActiveEnum, EnumIter, FromJsonQueryResult, sea_query::StringLen};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::ChapterId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, FromJsonQueryResult)]
#[serde(
    tag = "phase",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum WorkflowStage {
    Preparation {
        step: PreparationStep,
    },
    Chapter {
        chapter_id: ChapterId,
        step: ChapterStep,
    },
    /// Every plan chapter is approved and the user confirmed completion.
    Completed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PreparationStep {
    /// Waiting for the brief and the user's start.
    NotStarted,
    /// The agent is producing the four artifacts.
    Generating,
    /// The submitted artifacts wait for review.
    Reviewing,
    /// The agent is revising rejected or stale artifacts.
    Revising,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChapterStep {
    /// Waiting for the chapter's guidance and the user's start.
    Ready,
    OutlineGenerating,
    OutlineReviewing,
    Drafting,
    /// The finished chapter waits for its whole-chapter review.
    Reviewing,
    /// The chapter is approved.
    Done,
}

impl WorkflowStage {
    pub const NEW_VERSION: Self = Self::Preparation {
        step: PreparationStep::NotStarted,
    };
}

/// Whether body edits apply at once or wait for the user's review (§9.5, D14).
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    TS,
    EnumIter,
    DeriveActiveEnum,
)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ApprovalMode {
    /// Every valid edit is committed at once.
    #[default]
    AutoApprove,
    /// Every edit that changes the body waits for the user as a proposal.
    ApproveRequired,
}
