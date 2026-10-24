//! `research_tasks` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

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
    pub status: String,
    pub session_id: Option<i32>,
    pub report: String,
    pub entries_created: i32,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
