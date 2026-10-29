//! `books` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

use crate::db::json::BookStatus;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "books")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub title: String,
    pub idea: String,
    pub genre: String,
    pub writing_language: String,
    pub status: BookStatus,
    pub settings: String,
    pub model_overrides: String,
    pub conception: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(has_many)]
    pub tracks: HasMany<super::tracks::Entity>,
    #[sea_orm(has_many)]
    pub chapters: HasMany<super::chapters::Entity>,
    #[sea_orm(has_many)]
    pub setting_docs: HasMany<super::setting_docs::Entity>,
    #[sea_orm(has_many)]
    pub character_designs: HasMany<super::character_designs::Entity>,
    #[sea_orm(has_many)]
    pub entities: HasMany<super::entities::Entity>,
    #[sea_orm(has_many)]
    pub relations: HasMany<super::relations::Entity>,
    #[sea_orm(has_many)]
    pub timeline_events: HasMany<super::timeline_events::Entity>,
    #[sea_orm(has_many)]
    pub memory_notes: HasMany<super::memory_notes::Entity>,
    #[sea_orm(has_many)]
    pub foreshadowing: HasMany<super::foreshadowing::Entity>,
    #[sea_orm(has_many)]
    pub change_log: HasMany<super::change_log::Entity>,
    #[sea_orm(has_many)]
    pub research_tasks: HasMany<super::research_tasks::Entity>,
    #[sea_orm(has_many)]
    pub search_docs: HasMany<super::search_docs::Entity>,
    #[sea_orm(has_many)]
    pub agent_sessions: HasMany<super::agent_sessions::Entity>,
    #[sea_orm(has_many)]
    pub prompt_overrides: HasMany<super::prompt_overrides::Entity>,
    #[sea_orm(has_many)]
    pub setting_proposals: HasMany<super::setting_proposals::Entity>,
    #[sea_orm(has_many, via = "book_scopes")]
    pub knowledge_scopes: HasMany<super::knowledge_scopes::Entity>,
    #[sea_orm(has_many, via = "book_writing_skills")]
    pub writing_skills: HasMany<super::writing_skills::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
