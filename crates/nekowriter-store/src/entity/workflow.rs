use nekowriter_core::{
    ids::{RunId, VersionId},
    time::Timestamp,
    workflow::WorkflowStage,
};
use sea_orm::entity::prelude::*;

/// Where a version is in the story (the workflow axis) and which run is working on it.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "workflows")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub version_id: VersionId,
    pub stage: WorkflowStage,
    /// The run of the current stage, if one exists.
    pub active_run_id: Option<RunId>,
    /// Increases with every change of the workspace; commands quote it.
    pub revision: i64,
    /// When the user first confirmed the novel complete; extra chapters keep it (§5.3).
    pub completed_at: Option<Timestamp>,
    pub updated_at: Timestamp,
    #[sea_orm(belongs_to, from = "version_id", to = "id")]
    pub version: BelongsTo<super::version::Entity>,
    #[sea_orm(belongs_to, from = "active_run_id", to = "id")]
    pub active_run: BelongsTo<Option<super::run::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
