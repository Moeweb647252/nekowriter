//! `setting_proposals` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "setting_proposals")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: i32,
    pub target_type: String,
    pub target_kind: Option<String>,
    pub target_id: Option<i32>,
    pub edits: String,
    pub rationale: String,
    pub status: String,
    pub chapter_id: Option<i32>,
    pub session_id: Option<i32>,
    pub created_at: DateTimeUtc,
    pub decided_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
