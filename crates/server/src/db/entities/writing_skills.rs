//! `writing_skills` entity (spec/02 §2, design §4).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "writing_skills")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    /// Built-in only; `NULL` for user rows (design §4).
    pub key: Option<String>,
    pub name: String,
    pub category: String,
    /// `builtin` | `user`.
    pub author: String,
    pub description: String,
    pub body: String,
    pub enabled_by_default: i32,
    pub retired: i32,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
