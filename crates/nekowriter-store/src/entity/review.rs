use nekowriter_core::{
    citation::Citations,
    ids::{ChapterId, ReviewId, RevisionId, RunId, ToolCallId, VersionId},
    review::{DecisionSource, ReviewKind, ReviewStatus},
    run::RunHalt,
    time::Timestamp,
};
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "reviews")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: ReviewId,
    pub version_id: VersionId,
    /// The chapter a chapter review is about.
    #[sea_orm(indexed)]
    pub chapter_id: Option<ChapterId>,
    /// The run whose submission is reviewed.
    #[sea_orm(indexed)]
    pub run_id: RunId,
    /// The submission round of that run.
    pub round: i32,
    pub kind: ReviewKind,
    /// The revision under review; absent for an edit review.
    pub target_revision_id: Option<RevisionId>,
    /// The call whose proposed edit is under review, for an edit review.
    pub target_call_id: Option<ToolCallId>,
    pub status: ReviewStatus,
    /// What the writer says the submitted chapter contains, for whole-chapter reviews.
    #[sea_orm(column_type = "Text", nullable)]
    pub summary: Option<String>,
    /// The reviewer's feedback on a rejection; a review agent may reject without issues.
    #[sea_orm(column_type = "Text", nullable)]
    pub feedback: Option<String>,
    /// The passages a rejection quotes, verified.
    pub citations: Option<Citations>,
    pub decided_at: Option<Timestamp>,
    pub decided_by: Option<DecisionSource>,
    /// The review run deciding it, while it is delegated. A verdict from any other run is
    /// stale.
    pub delegated_run_id: Option<RunId>,
    /// The user's draft feedback when they delegated, for the review agent.
    #[sea_orm(column_type = "Text", nullable)]
    pub delegation_notes: Option<String>,
    /// The passages the delegation notes quote, verified.
    pub delegation_citations: Option<Citations>,
    /// Why the last delegation failed, until the review is delegated again or decided.
    pub delegation_failure: Option<RunHalt>,
    /// A review agent's confidence in thousandths; shown only (D08).
    pub confidence_permille: Option<i32>,
    pub created_at: Timestamp,
    #[sea_orm(belongs_to, from = "run_id", to = "id")]
    pub run: BelongsTo<super::run::Entity>,
    #[sea_orm(belongs_to, from = "target_revision_id", to = "id")]
    pub target: BelongsTo<Option<super::revision::Entity>>,
    #[sea_orm(belongs_to, from = "chapter_id", to = "id")]
    pub chapter: BelongsTo<Option<super::chapter::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
