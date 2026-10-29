//! `appearances` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "appearances")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique_key = "chapter_entity")]
    pub chapter_id: i32,
    #[sea_orm(unique_key = "chapter_entity")]
    pub entity_id: i32,
    pub role: String,
    pub note: String,
    #[sea_orm(belongs_to, from = "chapter_id", to = "id")]
    pub chapter: BelongsTo<super::chapters::Entity>,
    #[sea_orm(belongs_to, from = "entity_id", to = "id")]
    pub entity: BelongsTo<super::entities::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
