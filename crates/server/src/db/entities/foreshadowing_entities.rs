//! `foreshadowing_entities` join table entity (spec/02 §2).
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "foreshadowing_entities")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub foreshadowing_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_id: i32,
    #[sea_orm(belongs_to, from = "foreshadowing_id", to = "id")]
    pub foreshadowing: BelongsTo<super::foreshadowing::Entity>,
    #[sea_orm(belongs_to, from = "entity_id", to = "id")]
    pub entity: BelongsTo<super::entities::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
