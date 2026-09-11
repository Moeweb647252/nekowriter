use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "runs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    pub project_id: TextUuid,
    pub version_id: TextUuid,
    pub kind: String,
    pub parent_run_id: Option<String>,
    pub stage: String,
    pub status: String,
    pub resume_count: i64,
    pub request_count: i64,
    pub request_allowance_end: i64,
    pub consecutive_protocol_errors: i64,
    pub checkpoint_json: String,
    pub quota_scope_id: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    #[sea_orm(belongs_to, from = "project_id", to = "id")]
    pub project: BelongsTo<super::project::Entity>,
    #[sea_orm(belongs_to, from = "version_id", to = "id")]
    pub version: BelongsTo<super::novel_version::Entity>,
    #[sea_orm(
        self_ref,
        relation_enum = "ParentRun",
        from = "parent_run_id",
        to = "id"
    )]
    pub parent_run: BelongsTo<Option<Entity>>,
    #[sea_orm(has_many)]
    pub messages: HasMany<super::agent_message::Entity>,
    #[sea_orm(has_many)]
    pub model_requests: HasMany<super::model_request::Entity>,
    #[sea_orm(has_many)]
    pub tool_calls: HasMany<super::tool_call::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
