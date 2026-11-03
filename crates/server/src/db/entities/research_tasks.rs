//! `research_tasks` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

use crate::db::json::{ResearchTaskKind, ResearchTaskOrigin, ResearchTaskStatus};

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
    pub kind: ResearchTaskKind,
    pub origin: ResearchTaskOrigin,
    pub requirements: String,
    pub language: String,
    pub status: ResearchTaskStatus,
    pub session_id: Option<i32>,
    /// The knowledge check that listed this topic (spec/02 §5 invariant 21, D-168).
    pub check_id: Option<i32>,
    pub report: String,
    pub entries_created: i32,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<Option<super::books::Entity>>,
    #[sea_orm(belongs_to, from = "scope_id", to = "id")]
    pub scope: BelongsTo<Option<super::knowledge_scopes::Entity>>,
    #[sea_orm(
        self_ref,
        relation_enum = "Check",
        relation_reverse = "CheckTopics",
        from = "check_id",
        to = "id"
    )]
    pub check: BelongsTo<Option<Entity>>,
    #[sea_orm(belongs_to, relation_enum = "Session", from = "session_id", to = "id")]
    pub session: BelongsTo<Option<super::agent_sessions::Entity>>,
    #[sea_orm(has_many)]
    pub knowledge_entries: HasMany<super::knowledge_entries::Entity>,
    #[sea_orm(has_many, via_rel = "ResearchTask")]
    pub agent_sessions: HasMany<super::agent_sessions::Entity>,
    #[sea_orm(self_ref, relation_enum = "CheckTopics", relation_reverse = "Check")]
    pub check_topics: HasMany<Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
