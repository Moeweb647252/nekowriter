use nekowriter_core::{
    chapter::{ChapterKind, ChapterStatus},
    ids::{ChapterId, PlanItemId, VersionId},
    time::Timestamp,
};
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "chapters")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: ChapterId,
    #[sea_orm(unique_key = "version_kind_number")]
    pub version_id: VersionId,
    #[sea_orm(unique_key = "version_kind_number")]
    pub kind: ChapterKind,
    /// Position among the version's chapters of the same kind, starting at 1.
    #[sea_orm(unique_key = "version_kind_number")]
    pub number: i32,
    /// The plan item a main chapter was created from.
    pub plan_item_id: Option<PlanItemId>,
    pub title: String,
    #[sea_orm(column_type = "Text")]
    pub goal: String,
    pub status: ChapterStatus,
    pub created_at: Timestamp,
    #[sea_orm(belongs_to, from = "version_id", to = "id")]
    pub version: BelongsTo<super::version::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
