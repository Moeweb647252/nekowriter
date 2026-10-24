//! `tracks` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "tracks")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: i32,
    pub kind: String,
    pub title: String,
    pub synopsis: String,
    pub canon_mode: String,
    pub anchor_kind: String,
    pub anchor_chapter_id: Option<i32>,
    pub focus_entity_id: Option<i32>,
    pub ord: i32,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
