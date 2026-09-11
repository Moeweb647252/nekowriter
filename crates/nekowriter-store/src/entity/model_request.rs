use sea_orm::entity::prelude::*;

/// 一次已经封存的模型请求尝试。
///
/// 快照字段采用 JSON 文本保存：Store 在边界处解析为类型化 `serde_json::Value`，
/// 防止任意损坏文本被当作可执行状态。凭据不属于这里；请求快照只保存配置引用和
/// 已解析的非敏感有效设置。
#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "model_requests")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    #[sea_orm(unique_key = "run_attempt")]
    pub run_id: TextUuid,
    #[sea_orm(unique_key = "run_attempt")]
    pub attempt_no: i64,
    pub status: String,
    pub settings_snapshot_json: String,
    pub prompt_snapshot_json: String,
    pub context_snapshot_json: String,
    pub provider_request_id: Option<String>,
    pub usage_json: Option<String>,
    pub error_json: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub dispatched_at: Option<i64>,
    pub completed_at: Option<i64>,
    #[sea_orm(belongs_to, from = "run_id", to = "id")]
    pub run: BelongsTo<super::run::Entity>,
    #[sea_orm(has_many)]
    pub tool_calls: HasMany<super::tool_call::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
