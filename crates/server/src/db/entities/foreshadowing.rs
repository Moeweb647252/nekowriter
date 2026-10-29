//! `foreshadowing` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
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
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<super::books::Entity>,
    #[sea_orm(belongs_to, from = "track_id", to = "id")]
    pub track: BelongsTo<Option<super::tracks::Entity>>,
    #[sea_orm(
        belongs_to,
        relation_enum = "PlantedChapter",
        from = "planted_chapter_id",
        to = "id"
    )]
    pub planted_chapter: BelongsTo<Option<super::chapters::Entity>>,
    #[sea_orm(
        belongs_to,
        relation_enum = "ResolvedChapter",
        from = "resolved_chapter_id",
        to = "id"
    )]
    pub resolved_chapter: BelongsTo<Option<super::chapters::Entity>>,
    #[sea_orm(
        has_many,
        relation_enum = "Entity",
        via = "foreshadowing_entities::Foreshadowing"
    )]
    pub entities: HasMany<super::entities::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
