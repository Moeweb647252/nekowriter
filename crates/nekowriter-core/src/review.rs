//! Reviews: decisions about submitted revisions (`docs/product-spec.md` §10).

use sea_orm::{DeriveActiveEnum, EnumIter, sea_query::StringLen};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    field::{Field, multi_line_text},
    preparation::ArtifactKind,
};

multi_line_text!(
    /// What the reviewer asks the writer to change.
    ReviewFeedback,
    Field::ReviewFeedback,
    20_000
);

/// What a review is about. Each kind has its own review prompt for delegation.
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
pub enum ReviewKind {
    PreparationBackground,
    PreparationResearch,
    PreparationCharacters,
    PreparationOutline,
}

impl From<ArtifactKind> for ReviewKind {
    fn from(kind: ArtifactKind) -> Self {
        match kind {
            ArtifactKind::Background => Self::PreparationBackground,
            ArtifactKind::Research => Self::PreparationResearch,
            ArtifactKind::Characters => Self::PreparationCharacters,
            ArtifactKind::Outline => Self::PreparationOutline,
        }
    }
}

impl ReviewKind {
    /// The preparation artifact a preparation review is about.
    #[must_use]
    pub fn artifact(self) -> Option<ArtifactKind> {
        match self {
            Self::PreparationBackground => Some(ArtifactKind::Background),
            Self::PreparationResearch => Some(ArtifactKind::Research),
            Self::PreparationCharacters => Some(ArtifactKind::Characters),
            Self::PreparationOutline => Some(ArtifactKind::Outline),
        }
    }
}

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
pub enum ReviewStatus {
    /// Waiting for a decision.
    Open,
    Approved,
    /// Rejected with feedback for the writer.
    Rejected,
    /// An artifact it depends on changed after it was submitted, so it must be resubmitted
    /// rather than decided (§6).
    Stale,
}

/// A reviewer's decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewDecision {
    Approve,
    Reject { feedback: ReviewFeedback },
}
