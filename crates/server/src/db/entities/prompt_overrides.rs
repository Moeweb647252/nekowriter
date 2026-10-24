//! `prompt_overrides` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "prompt_overrides")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub scope: String,
    pub book_id: Option<i32>,
    pub key: String,
    pub content: String,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
