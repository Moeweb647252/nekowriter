//! `index_meta` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "index_meta")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub key: Option<String>,
    pub value: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
