//! `foreshadowing` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "foreshadowing")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub title: String,
    pub description: String,
    pub planned_payoff: String,
    pub status: String,
    pub planted_chapter_id: Option<i32>,
    pub resolved_chapter_id: Option<i32>,
    pub resolution: String,
    pub importance: i32,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
