//! `change_log` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "change_log")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub chapter_id: Option<i32>,
    pub session_id: Option<i32>,
    pub actor: String,
    pub target_type: String,
    pub target_id: i32,
    pub op: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub note: String,
    pub reverts_id: Option<i32>,
    pub reverted_by_id: Option<i32>,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
