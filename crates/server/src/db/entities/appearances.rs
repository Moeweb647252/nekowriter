//! `appearances` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "appearances")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub chapter_id: i32,
    pub entity_id: i32,
    pub role: String,
    pub note: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
