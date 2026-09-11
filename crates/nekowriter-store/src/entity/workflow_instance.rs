use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "workflow_instances")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub version_id: TextUuid,
    pub state_json: String,
    pub state_revision: i64,
    pub active_run_id: Option<String>,
    pub approval_mode: String,
    pub pause_requested: bool,
    #[sea_orm(belongs_to, from = "version_id", to = "id")]
    pub version: BelongsTo<super::novel_version::Entity>,
    #[sea_orm(
        belongs_to,
        relation_enum = "ActiveRun",
        from = "active_run_id",
        to = "id"
    )]
    pub active_run: BelongsTo<Option<super::run::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
