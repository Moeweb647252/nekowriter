//! `research_tasks` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

use crate::db::json::ResearchTaskStatus;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "research_tasks")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: Option<i32>,
    pub scope_id: Option<i32>,
    pub scope_name: String,
    pub topic: String,
    pub requirements: String,
    pub language: String,
    pub status: ResearchTaskStatus,
    pub session_id: Option<i32>,
    pub report: String,
    pub entries_created: i32,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<Option<super::books::Entity>>,
    #[sea_orm(belongs_to, from = "scope_id", to = "id")]
    pub scope: BelongsTo<Option<super::knowledge_scopes::Entity>>,
    #[sea_orm(belongs_to, relation_enum = "Session", from = "session_id", to = "id")]
    pub session: BelongsTo<Option<super::agent_sessions::Entity>>,
    #[sea_orm(has_many)]
    pub knowledge_entries: HasMany<super::knowledge_entries::Entity>,
    #[sea_orm(has_many, via_rel = "ResearchTask")]
    pub agent_sessions: HasMany<super::agent_sessions::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
