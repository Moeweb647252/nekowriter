//! `agent_tool_calls` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

use crate::db::json::{Appendix, JsonText, ToolEffects};

use crate::db::json::ToolCallStatus;

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
    pub status: ToolCallStatus,
    pub raw_result_text: Option<String>,
    pub appendices: JsonText<Vec<Appendix>>,
    pub result_text: Option<String>,
    pub is_error: bool,
    pub ui: Option<JsonText<serde_json::Value>>,
    pub effects: Option<JsonText<ToolEffects>>,
    pub sealed: bool,
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
