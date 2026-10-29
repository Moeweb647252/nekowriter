//! `chapter_versions` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "chapter_versions")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique_key = "chapter_version")]
    pub chapter_id: i32,
    #[sea_orm(unique_key = "chapter_version")]
    pub version_no: i32,
    pub kind: String,
    pub title: String,
    pub content: String,
    pub word_count: i32,
    pub note: String,
    pub session_id: Option<i32>,
    pub created_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "chapter_id", to = "id")]
    pub chapter: BelongsTo<super::chapters::Entity>,
    #[sea_orm(belongs_to, from = "session_id", to = "id")]
    pub session: BelongsTo<Option<super::agent_sessions::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
