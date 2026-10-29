//! `knowledge_entries` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_entries")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique_key = "scope_title")]
    pub scope_id: i32,
    #[sea_orm(unique_key = "scope_title")]
    pub title: String,
    pub content: String,
    pub source: String,
    pub source_urls: String,
    pub tags: String,
    pub research_task_id: Option<i32>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "scope_id", to = "id")]
    pub scope: BelongsTo<super::knowledge_scopes::Entity>,
    #[sea_orm(belongs_to, from = "research_task_id", to = "id")]
    pub research_task: BelongsTo<Option<super::research_tasks::Entity>>,
    #[sea_orm(has_many)]
    pub knowledge_chunks: HasMany<super::knowledge_chunks::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
