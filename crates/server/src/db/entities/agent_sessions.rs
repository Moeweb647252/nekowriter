//! `agent_sessions` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_sessions")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: Option<i32>,
    pub role: String,
    pub kind: String,
    pub chapter_id: Option<i32>,
    pub track_id: Option<i32>,
    pub research_task_id: Option<i32>,
    pub model_profile_id: Option<i32>,
    pub model_snapshot: String,
    pub system_prompt: String,
    pub tools: String,
    pub status: String,
    pub phase: String,
    pub granularity: Option<String>,
    pub auto_pause: i32,
    pub pause_after_outline: i32,
    pub pause_requested: i32,
    pub held_tool_call_id: Option<i32>,
    pub base_content_version: Option<i32>,
    pub pending_comment: Option<String>,
    pub view_snapshot: Option<String>,
    pub counters: String,
    pub usage: String,
    pub error: Option<String>,
    pub parent_session_id: Option<i32>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub finished_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
