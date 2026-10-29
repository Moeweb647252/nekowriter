//! `agent_tool_calls` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_tool_calls")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub session_id: i32,
    #[sea_orm(unique_key = "message_call")]
    pub message_id: i32,
    #[sea_orm(unique_key = "message_call")]
    pub call_index: i32,
    pub provider_call_id: String,
    pub name: String,
    pub input: String,
    pub status: String,
    pub raw_result_text: Option<String>,
    pub appendices: String,
    pub result_text: Option<String>,
    pub is_error: i32,
    pub ui: Option<String>,
    pub effects: Option<String>,
    pub sealed: i32,
    pub created_at: DateTimeUtc,
    pub started_at: Option<DateTimeUtc>,
    pub finished_at: Option<DateTimeUtc>,
    #[sea_orm(belongs_to, relation_enum = "Session", from = "session_id", to = "id")]
    pub session: BelongsTo<super::agent_sessions::Entity>,
    #[sea_orm(belongs_to, from = "message_id", to = "id")]
    pub message: BelongsTo<super::agent_messages::Entity>,
    #[sea_orm(has_many)]
    pub agent_inbox: HasMany<super::agent_inbox::Entity>,
    #[sea_orm(has_many, via_rel = "HeldToolCall")]
    pub agent_sessions: HasMany<super::agent_sessions::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
