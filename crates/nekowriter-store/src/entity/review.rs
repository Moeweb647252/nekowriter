use nekowriter_core::{
    ids::{ChapterId, ReviewId, RevisionId, RunId, VersionId},
    review::{ReviewKind, ReviewStatus},
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
    pub target_revision_id: RevisionId,
    pub status: ReviewStatus,
    /// What the writer says the submitted chapter contains, for whole-chapter reviews.
    #[sea_orm(column_type = "Text", nullable)]
    pub summary: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub feedback: Option<String>,
    pub decided_at: Option<Timestamp>,
    pub created_at: Timestamp,
    #[sea_orm(belongs_to, from = "run_id", to = "id")]
    pub run: BelongsTo<super::run::Entity>,
    #[sea_orm(belongs_to, from = "target_revision_id", to = "id")]
    pub target: BelongsTo<super::revision::Entity>,
    #[sea_orm(belongs_to, from = "chapter_id", to = "id")]
    pub chapter: BelongsTo<Option<super::chapter::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
