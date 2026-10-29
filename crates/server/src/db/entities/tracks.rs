//! `tracks` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
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
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<super::books::Entity>,
    #[sea_orm(
        belongs_to,
        relation_enum = "AnchorChapter",
        from = "anchor_chapter_id",
        to = "id"
    )]
    pub anchor_chapter: BelongsTo<Option<super::chapters::Entity>>,
    #[sea_orm(
        belongs_to,
        relation_enum = "FocusEntity",
        from = "focus_entity_id",
        to = "id"
    )]
    pub focus_entity: BelongsTo<Option<super::entities::Entity>>,
    #[sea_orm(has_many, via_rel = "Track")]
    pub chapters: HasMany<super::chapters::Entity>,
    #[sea_orm(has_many, via_rel = "Track")]
    pub entities: HasMany<super::entities::Entity>,
    #[sea_orm(has_many)]
    pub relations: HasMany<super::relations::Entity>,
    #[sea_orm(has_many)]
    pub timeline_events: HasMany<super::timeline_events::Entity>,
    #[sea_orm(has_many)]
    pub memory_notes: HasMany<super::memory_notes::Entity>,
    #[sea_orm(has_many)]
    pub foreshadowing: HasMany<super::foreshadowing::Entity>,
    #[sea_orm(has_many)]
    pub change_log: HasMany<super::change_log::Entity>,
    #[sea_orm(has_many)]
    pub search_docs: HasMany<super::search_docs::Entity>,
    #[sea_orm(has_many)]
    pub agent_sessions: HasMany<super::agent_sessions::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
