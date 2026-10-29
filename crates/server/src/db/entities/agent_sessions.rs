//! `agent_sessions` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

use crate::db::json::{AgentRole, Granularity, Phase, SessionKind, SessionStatus};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_sessions")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: Option<i32>,
    pub role: AgentRole,
    pub kind: SessionKind,
    pub chapter_id: Option<i32>,
    pub track_id: Option<i32>,
    pub research_task_id: Option<i32>,
    pub model_profile_id: Option<i32>,
    pub model_snapshot: String,
    pub system_prompt: String,
    pub tools: String,
    pub status: SessionStatus,
    pub phase: Phase,
    pub granularity: Option<Granularity>,
    pub auto_pause: bool,
    pub pause_after_outline: bool,
    pub pause_requested: bool,
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
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<Option<super::books::Entity>>,
    #[sea_orm(belongs_to, from = "chapter_id", to = "id")]
    pub chapter: BelongsTo<Option<super::chapters::Entity>>,
    #[sea_orm(belongs_to, from = "track_id", to = "id")]
    pub track: BelongsTo<Option<super::tracks::Entity>>,
    #[sea_orm(
        belongs_to,
        relation_enum = "ResearchTask",
        from = "research_task_id",
        to = "id"
    )]
    pub research_task: BelongsTo<Option<super::research_tasks::Entity>>,
    #[sea_orm(belongs_to, from = "model_profile_id", to = "id")]
    pub model_profile: BelongsTo<Option<super::model_profiles::Entity>>,
    #[sea_orm(
        belongs_to,
        relation_enum = "HeldToolCall",
        from = "held_tool_call_id",
        to = "id"
    )]
    pub held_tool_call: BelongsTo<Option<super::agent_tool_calls::Entity>>,
    #[sea_orm(
        self_ref,
        relation_enum = "ParentSession",
        relation_reverse = "ChildSessions",
        from = "parent_session_id",
        to = "id"
    )]
    pub parent_session: BelongsTo<Option<Entity>>,
    #[sea_orm(has_many)]
    pub agent_inbox: HasMany<super::agent_inbox::Entity>,
    #[sea_orm(has_many)]
    pub agent_messages: HasMany<super::agent_messages::Entity>,
    #[sea_orm(has_many, via_rel = "Session")]
    pub agent_tool_calls: HasMany<super::agent_tool_calls::Entity>,
    #[sea_orm(has_many)]
    pub chapter_versions: HasMany<super::chapter_versions::Entity>,
    #[sea_orm(has_many)]
    pub chapter_feedback: HasMany<super::chapter_feedback::Entity>,
    #[sea_orm(has_many)]
    pub change_log: HasMany<super::change_log::Entity>,
    #[sea_orm(has_many, via_rel = "Session")]
    pub research_tasks: HasMany<super::research_tasks::Entity>,
    #[sea_orm(has_many)]
    pub review_reports: HasMany<super::review_reports::Entity>,
    #[sea_orm(has_many)]
    pub setting_proposals: HasMany<super::setting_proposals::Entity>,
    #[sea_orm(
        self_ref,
        relation_enum = "ChildSessions",
        relation_reverse = "ParentSession"
    )]
    pub child_sessions: HasMany<Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
