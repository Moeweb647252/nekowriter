use sea_orm::entity::prelude::*;

/// 正文 Agent 的一条自由意见。
///
/// 意见不直接改写工具结果或已封存模型请求。`after_tool_sequence`、
/// `preferred_tool_call_id` 与 `bound_tool_call_id` 是同一未封存批次共享的锚点，
/// 让随后追加的意见始终和同一工具结果一起投递。
#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "feedback")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    #[sea_orm(unique_key = "run_feedback_sequence")]
    pub run_id: TextUuid,
    pub version_id: TextUuid,
    #[sea_orm(unique_key = "run_feedback_sequence")]
    pub sequence: i64,
    pub text: String,
    pub citations_json: String,
    pub after_tool_sequence: i64,
    pub preferred_tool_call_id: Option<String>,
    pub bound_tool_call_id: Option<String>,
    pub included_request_id: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[sea_orm(belongs_to, from = "run_id", to = "id")]
    pub run: BelongsTo<super::run::Entity>,
    #[sea_orm(belongs_to, from = "version_id", to = "id")]
    pub version: BelongsTo<super::novel_version::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
