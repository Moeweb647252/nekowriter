use sea_orm::entity::prelude::*;

/// 一次已经完整接收的模型工具调用。
///
/// `provider_call_id` 是模型协议中的调用 ID；它和所属请求组成唯一键，使网络回调
/// 重放不会把同一正文编辑应用两次。参数与结果均作为 JSON 文本保存，具体工具
/// 协议在 Store/App 边界校验，实体只承载关系和不可变审计字段。
#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "tool_calls")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    #[sea_orm(unique_key = "request_provider_call")]
    pub request_id: TextUuid,
    pub run_id: TextUuid,
    #[sea_orm(unique_key = "request_provider_call")]
    pub provider_call_id: String,
    pub sequence: i64,
    pub name: String,
    pub args_json: String,
    pub base_revision_id: Option<String>,
    /// `ApproveRequired` 下尚未决定的候选正文；正式正文修订仍保持在 document 上。
    pub proposal_json: Option<String>,
    /// 用户暂停命中完整工具边界后保留的执行输入；恢复前绝不应用正文变更。
    pub held_json: Option<String>,
    pub status: String,
    pub result_json: Option<String>,
    /// 首次将不可变工具结果纳入模型请求快照的请求 ID；之后的新反馈不可回写它。
    pub included_request_id: Option<String>,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
    #[sea_orm(belongs_to, from = "request_id", to = "id")]
    pub request: BelongsTo<super::model_request::Entity>,
    #[sea_orm(belongs_to, from = "run_id", to = "id")]
    pub run: BelongsTo<super::run::Entity>,
    #[sea_orm(has_one)]
    pub question: HasOne<super::question::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
