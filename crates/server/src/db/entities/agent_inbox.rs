//! `agent_inbox` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_inbox")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub session_id: i32,
    pub kind: String,
    pub content: String,
    pub created_at: DateTimeUtc,
    pub consumed_at: Option<DateTimeUtc>,
    pub consumed_by_tool_call_id: Option<i32>,
    pub consumed_by_message_id: Option<i32>,
    #[sea_orm(belongs_to, from = "consumed_by_message_id", to = "id")]
    pub consumed_by_message: BelongsTo<Option<super::agent_messages::Entity>>,
    #[sea_orm(belongs_to, from = "consumed_by_tool_call_id", to = "id")]
    pub consumed_by_tool_call: BelongsTo<Option<super::agent_tool_calls::Entity>>,
    #[sea_orm(belongs_to, from = "session_id", to = "id")]
    pub session: BelongsTo<super::agent_sessions::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
