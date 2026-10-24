//! `entities` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "entities")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub kind: String,
    pub name: String,
    pub aliases: String,
    pub summary: String,
    pub description: String,
    pub state: String,
    pub attributes: String,
    pub importance: i32,
    pub status: String,
    pub is_protagonist: i32,
    pub first_chapter_id: Option<i32>,
    pub last_chapter_id: Option<i32>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
