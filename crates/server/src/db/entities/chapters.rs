//! `chapters` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

use crate::db::json::{CanonStatus, ChapterContent, ChapterStatus, JsonText};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "chapters")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: i32,
    #[sea_orm(unique_key = "track_ord")]
    pub track_id: i32,
    #[sea_orm(unique_key = "track_ord")]
    pub ord: i32,
    pub title: String,
    pub status: ChapterStatus,
    pub guidance: String,
    pub outline: String,
    pub reference_notes: String,
    pub summary: String,
    pub content: JsonText<ChapterContent>,
    pub content_version: i32,
    pub word_count: i32,
    pub official_version_id: Option<i32>,
    pub canon_status: CanonStatus,
    pub synced_version_id: Option<i32>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<super::books::Entity>,
    #[sea_orm(belongs_to, relation_enum = "Track", from = "track_id", to = "id")]
    pub track: BelongsTo<super::tracks::Entity>,
    #[sea_orm(
        belongs_to,
        relation_enum = "OfficialVersion",
        from = "official_version_id",
        to = "id"
    )]
    pub official_version: BelongsTo<Option<super::chapter_versions::Entity>>,
    #[sea_orm(
        belongs_to,
        relation_enum = "SyncedVersion",
        from = "synced_version_id",
        to = "id"
    )]
    pub synced_version: BelongsTo<Option<super::chapter_versions::Entity>>,
    #[sea_orm(has_many)]
    pub change_log: HasMany<super::change_log::Entity>,
    #[sea_orm(has_many)]
    pub chapter_versions: HasMany<super::chapter_versions::Entity>,
    #[sea_orm(has_many)]
    pub appearances: HasMany<super::appearances::Entity>,
    #[sea_orm(has_many)]
    pub timeline_events: HasMany<super::timeline_events::Entity>,
    #[sea_orm(has_many)]
    pub memory_notes: HasMany<super::memory_notes::Entity>,
    #[sea_orm(has_many)]
    pub agent_sessions: HasMany<super::agent_sessions::Entity>,
    #[sea_orm(has_many)]
    pub review_reports: HasMany<super::review_reports::Entity>,
    #[sea_orm(has_many)]
    pub chapter_feedback: HasMany<super::chapter_feedback::Entity>,
    #[sea_orm(has_many)]
    pub setting_proposals: HasMany<super::setting_proposals::Entity>,
    #[sea_orm(has_many, via_rel = "AnchorChapter")]
    pub tracks: HasMany<super::tracks::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
