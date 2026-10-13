use nekowriter_core::{
    ids::{ProjectId, VersionId},
    project::NovelTitle,
    time::Timestamp,
};
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "versions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: VersionId,
    #[sea_orm(indexed)]
    pub project_id: ProjectId,
    /// Position of the version inside its project, starting at 1.
    pub number: i32,
    pub novel_title: NovelTitle,
    pub revision: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub deleted_at: Option<Timestamp>,
    #[sea_orm(belongs_to, from = "project_id", to = "id")]
    pub project: BelongsTo<super::project::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
