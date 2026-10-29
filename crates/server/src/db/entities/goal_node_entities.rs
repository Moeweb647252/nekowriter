//! `goal_node_entities` join table entity (spec/02 §2).
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "goal_node_entities")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub node_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_id: i32,
    #[sea_orm(belongs_to, from = "node_id", to = "id")]
    pub node: BelongsTo<super::goal_nodes::Entity>,
    #[sea_orm(belongs_to, from = "entity_id", to = "id")]
    pub entity: BelongsTo<super::entities::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
