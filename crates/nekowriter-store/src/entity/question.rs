use sea_orm::entity::prelude::*;

/// 一次 `ask_user` 工具调用对应的持久问题组。
///
/// 模式与答案分别保存，避免 UI 或模型上下文只能从临时状态推断问题。一个工具调用
/// 最多有一个问题组；回答后工具调用获得不可变结果并按正常消息配对继续 Agent。
#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "questions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    #[sea_orm(unique_key = "tool_call_question")]
    pub tool_call_id: TextUuid,
    pub schema_json: String,
    pub answer_json: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[sea_orm(belongs_to, from = "tool_call_id", to = "id")]
    pub tool_call: BelongsTo<super::tool_call::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
