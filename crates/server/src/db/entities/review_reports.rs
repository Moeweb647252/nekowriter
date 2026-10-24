//! `review_reports` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "review_reports")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub chapter_id: i32,
    pub content_version: i32,
    pub session_id: Option<i32>,
    pub attempt: i32,
    pub verdict: String,
    pub summary: String,
    pub issues: String,
    pub feedback: String,
    pub user_comment: String,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
