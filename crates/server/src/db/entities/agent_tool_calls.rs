//! `agent_tool_calls` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_tool_calls")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub session_id: i32,
    pub message_id: i32,
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
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
