//! `knowledge_entries` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_entries")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub scope_id: i32,
    pub title: String,
    pub content: String,
    pub source: String,
    pub source_urls: String,
    pub tags: String,
    pub research_task_id: Option<i32>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
