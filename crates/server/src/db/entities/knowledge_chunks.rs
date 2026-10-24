//! `knowledge_chunks` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_chunks")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub entry_id: i32,
    pub scope_id: i32,
    pub ord: i32,
    pub heading: String,
    pub text: String,
    pub content_hash: String,
    pub embed_state: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
