//! `relations` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "relations")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub source_id: i32,
    pub target_id: i32,
    pub kind: String,
    pub inverse_kind: String,
    pub description: String,
    pub status: String,
    pub since_chapter_id: Option<i32>,
    pub until_chapter_id: Option<i32>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<super::books::Entity>,
    #[sea_orm(belongs_to, from = "track_id", to = "id")]
    pub track: BelongsTo<Option<super::tracks::Entity>>,
    #[sea_orm(belongs_to, relation_enum = "Source", from = "source_id", to = "id")]
    pub source: BelongsTo<super::entities::Entity>,
    #[sea_orm(belongs_to, relation_enum = "Target", from = "target_id", to = "id")]
    pub target: BelongsTo<super::entities::Entity>,
    #[sea_orm(
        belongs_to,
        relation_enum = "SinceChapter",
        from = "since_chapter_id",
        to = "id"
    )]
    pub since_chapter: BelongsTo<Option<super::chapters::Entity>>,
    #[sea_orm(
        belongs_to,
        relation_enum = "UntilChapter",
        from = "until_chapter_id",
        to = "id"
    )]
    pub until_chapter: BelongsTo<Option<super::chapters::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
