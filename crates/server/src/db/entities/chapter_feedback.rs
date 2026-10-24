//! `chapter_feedback` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "chapter_feedback")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub chapter_id: i32,
    pub content_version: i32,
    pub review_report_id: Option<i32>,
    pub text: String,
    pub issues: String,
    pub mode: String,
    pub session_id: Option<i32>,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
