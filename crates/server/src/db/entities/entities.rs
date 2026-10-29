//! `entities` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "entities")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub kind: String,
    pub name: String,
    pub aliases: String,
    pub summary: String,
    pub description: String,
    pub state: String,
    pub attributes: String,
    pub importance: i32,
    pub status: String,
    pub is_protagonist: i32,
    pub first_chapter_id: Option<i32>,
    pub last_chapter_id: Option<i32>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<super::books::Entity>,
    #[sea_orm(belongs_to, relation_enum = "Track", from = "track_id", to = "id")]
    pub track: BelongsTo<Option<super::tracks::Entity>>,
    #[sea_orm(
        belongs_to,
        relation_enum = "FirstChapter",
        from = "first_chapter_id",
        to = "id"
    )]
    pub first_chapter: BelongsTo<Option<super::chapters::Entity>>,
    #[sea_orm(
        belongs_to,
        relation_enum = "LastChapter",
        from = "last_chapter_id",
        to = "id"
    )]
    pub last_chapter: BelongsTo<Option<super::chapters::Entity>>,
    #[sea_orm(has_many)]
    pub appearances: HasMany<super::appearances::Entity>,
    #[sea_orm(has_many)]
    pub character_designs: HasMany<super::character_designs::Entity>,
    #[sea_orm(has_many, via_rel = "FocusEntity")]
    pub tracks: HasMany<super::tracks::Entity>,
    #[sea_orm(
        has_many,
        relation_enum = "Event",
        via = "timeline_event_entities::Entity"
    )]
    pub timeline_events: HasMany<super::timeline_events::Entity>,
    #[sea_orm(has_many, relation_enum = "Note", via = "memory_note_entities::Entity")]
    pub memory_notes: HasMany<super::memory_notes::Entity>,
    #[sea_orm(
        has_many,
        relation_enum = "Foreshadowing",
        via = "foreshadowing_entities::Entity"
    )]
    pub foreshadowing: HasMany<super::foreshadowing::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
