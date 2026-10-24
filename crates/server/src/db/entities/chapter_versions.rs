//! `chapter_versions` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "chapter_versions")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub chapter_id: i32,
    pub version_no: i32,
    pub kind: String,
    pub title: String,
    pub content: String,
    pub word_count: i32,
    pub note: String,
    pub session_id: Option<i32>,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
