//! `goal_node_foreshadowing` join table entity (spec/02 §2).
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "goal_node_foreshadowing")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub node_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    pub foreshadowing_id: i32,
    #[sea_orm(belongs_to, from = "node_id", to = "id")]
    pub node: BelongsTo<super::goal_nodes::Entity>,
    #[sea_orm(belongs_to, from = "foreshadowing_id", to = "id")]
    pub foreshadowing: BelongsTo<super::foreshadowing::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
