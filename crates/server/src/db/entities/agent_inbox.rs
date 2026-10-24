//! `agent_inbox` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_inbox")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub session_id: i32,
    pub kind: String,
    pub content: String,
    pub created_at: DateTimeUtc,
    pub consumed_at: Option<DateTimeUtc>,
    pub consumed_by_tool_call_id: Option<i32>,
    pub consumed_by_message_id: Option<i32>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
