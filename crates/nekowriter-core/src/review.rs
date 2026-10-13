//! Reviews: decisions about submitted revisions (`docs/product-spec.md` §10).

use sea_orm::{DeriveActiveEnum, EnumIter, sea_query::StringLen};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    citation::{Citation, VerifiedCitation},
    field::{Field, FieldError, FieldProblem, multi_line_text},
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
    /// A chapter's outline, before its body is written.
    ChapterOutline,
    /// A finished chapter as a whole.
    ChapterFinal,
    /// One proposed body edit, in `ApproveRequired` mode.
    ChapterEdit,
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
            Self::ChapterOutline | Self::ChapterFinal | Self::ChapterEdit => None,
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
    /// A review agent decides it (§10).
    Delegated,
    Approved,
    /// Rejected with feedback for the writer.
    Rejected,
    /// An artifact it depends on changed after it was submitted, so it must be resubmitted
    /// rather than decided (§6).
    Stale,
}

/// A user's decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewDecision {
    Approve,
    Reject(Rejection),
}

/// What a rejection asks the writer to change: in words, by citing passages, or both, but
/// never nothing (§6, §9.5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejection {
    feedback: Option<ReviewFeedback>,
    citations: Vec<Citation>,
}

impl Rejection {
    pub fn new(
        feedback: Option<ReviewFeedback>,
        citations: Vec<Citation>,
    ) -> Result<Self, FieldError> {
        if feedback.is_none() && citations.is_empty() {
            return Err(FieldError {
                field: Field::ReviewFeedback,
                problem: FieldProblem::Empty,
            });
        }
        Ok(Self {
            feedback,
            citations,
        })
    }

    #[must_use]
    pub fn feedback(&self) -> Option<&ReviewFeedback> {
        self.feedback.as_ref()
    }

    #[must_use]
    pub fn citations(&self) -> &[Citation] {
        &self.citations
    }
}

/// Who decided a review.
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
pub enum DecisionSource {
    User,
    /// A review agent the user delegated the review to.
    Delegate,
}

/// A decision as it is applied, by the user or a delegate. A delegate's rejection may
/// come without issues; the writer is then told the review failed without specifics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub approved: bool,
    pub feedback: Option<String>,
    pub citations: Vec<VerifiedCitation>,
    pub source: DecisionSource,
}
