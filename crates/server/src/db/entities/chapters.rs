//! `chapters` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "chapters")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: i32,
    pub track_id: i32,
    pub ord: i32,
    pub title: String,
    pub status: String,
    pub guidance: String,
    pub outline: String,
    pub reference_notes: String,
    pub summary: String,
    pub content: String,
    pub content_version: i32,
    pub word_count: i32,
    pub official_version_id: Option<i32>,
    pub canon_status: String,
    pub synced_version_id: Option<i32>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
