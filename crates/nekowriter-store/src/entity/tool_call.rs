use nekowriter_core::{
    ids::{ModelRequestId, RunId, ToolCallId},
    run_message::ToolOutcome,
    time::Timestamp,
};
use sea_orm::entity::prelude::*;

/// A tool call the model made, and its outcome.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "tool_calls")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: ToolCallId,
    #[sea_orm(indexed)]
    pub run_id: RunId,
    pub request_id: ModelRequestId,
    /// The call's identifier from the model service, which pairs it with its result.
    pub provider_id: String,
    pub name: String,
    #[sea_orm(column_type = "Text")]
    pub arguments: String,
    pub outcome: Option<ToolOutcome>,
    pub created_at: Timestamp,
    #[sea_orm(belongs_to, from = "run_id", to = "id")]
    pub run: BelongsTo<super::run::Entity>,
    #[sea_orm(belongs_to, from = "request_id", to = "id")]
    pub request: BelongsTo<super::model_request::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
