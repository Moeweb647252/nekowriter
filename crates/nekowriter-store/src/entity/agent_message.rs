use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_messages")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    #[sea_orm(unique_key = "run_message_sequence")]
    pub run_id: TextUuid,
    #[sea_orm(unique_key = "run_message_sequence")]
    pub sequence: i64,
    pub role: String,
    pub content_json: String,
    pub request_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub created_at: i64,
    #[sea_orm(belongs_to, from = "run_id", to = "id")]
    pub run: BelongsTo<super::run::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
