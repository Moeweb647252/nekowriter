//! `book_scopes` join table entity (spec/02 §2).
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "book_scopes")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub book_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    pub scope_id: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
