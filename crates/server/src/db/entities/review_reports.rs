//! `review_reports` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

use crate::db::json::ReviewVerdict;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "review_reports")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub chapter_id: i32,
    pub content_version: i32,
    pub session_id: Option<i32>,
    pub attempt: i32,
    pub verdict: ReviewVerdict,
    pub summary: String,
    pub issues: String,
    pub feedback: String,
    pub user_comment: String,
    pub created_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "chapter_id", to = "id")]
    pub chapter: BelongsTo<super::chapters::Entity>,
    #[sea_orm(belongs_to, from = "session_id", to = "id")]
    pub session: BelongsTo<Option<super::agent_sessions::Entity>>,
    #[sea_orm(has_many)]
    pub chapter_feedback: HasMany<super::chapter_feedback::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
