//! `timeline_event_entities` join table entity (spec/02 §2).
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "timeline_event_entities")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub event_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_id: i32,
    #[sea_orm(belongs_to, from = "event_id", to = "id")]
    pub event: BelongsTo<super::timeline_events::Entity>,
    #[sea_orm(belongs_to, from = "entity_id", to = "id")]
    pub entity: BelongsTo<super::entities::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
