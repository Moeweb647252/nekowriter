//! `character_designs` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "character_designs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique_key = "book_name")]
    pub book_id: i32,
    pub rank: i32,
    #[sea_orm(unique_key = "book_name")]
    pub name: String,
    pub role_label: String,
    pub summary: String,
    pub profile: String,
    pub entity_id: Option<i32>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<super::books::Entity>,
    #[sea_orm(belongs_to, from = "entity_id", to = "id")]
    pub entity: BelongsTo<Option<super::entities::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
