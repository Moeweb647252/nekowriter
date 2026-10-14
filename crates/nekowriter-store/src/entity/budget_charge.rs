use nekowriter_core::{ids::ToolCallId, time::Timestamp};
use sea_orm::entity::prelude::*;

/// One charged auxiliary tool call (§12.1). A scope's used count is the number of its
/// charges; the call's ID makes charging a call twice impossible.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "budget_charges")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tool_call_id: ToolCallId,
    /// The budget scope: the preparation of a version, or the outline of a chapter.
    #[sea_orm(indexed)]
    pub scope: String,
    pub charged_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}
