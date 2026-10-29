//! `chapter_feedback` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

use crate::db::json::FeedbackMode;

#[sea_orm::model]
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
    pub mode: FeedbackMode,
    pub session_id: Option<i32>,
    pub created_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "chapter_id", to = "id")]
    pub chapter: BelongsTo<super::chapters::Entity>,
    #[sea_orm(belongs_to, from = "review_report_id", to = "id")]
    pub review_report: BelongsTo<Option<super::review_reports::Entity>>,
    #[sea_orm(belongs_to, from = "session_id", to = "id")]
    pub session: BelongsTo<Option<super::agent_sessions::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
