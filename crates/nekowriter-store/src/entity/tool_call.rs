use nekowriter_core::{
    call::CallRequest,
    ids::{ModelRequestId, RevisionId, RunId, ToolCallId},
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
    #[sea_orm(unique_key = "run_sequence")]
    pub run_id: RunId,
    /// Position among the run's calls, starting at 1. Feedback batches are anchored to it.
    #[sea_orm(unique_key = "run_sequence")]
    pub sequence: i32,
    pub request_id: ModelRequestId,
    /// The call's identifier from the model service, which pairs it with its result.
    pub provider_id: String,
    pub name: String,
    #[sea_orm(column_type = "Text")]
    pub arguments: String,
    /// The parsed call; absent when the call broke the protocol.
    pub parsed: Option<CallRequest>,
    /// For a proposed edit: the body revision it was computed against (`None` for an empty
    /// body).
    pub base_revision_id: Option<RevisionId>,
    pub status: ToolCallStatus,
    pub outcome: Option<ToolOutcome>,
    pub created_at: Timestamp,
    #[sea_orm(belongs_to, from = "run_id", to = "id")]
    pub run: BelongsTo<super::run::Entity>,
    #[sea_orm(belongs_to, from = "request_id", to = "id")]
    pub request: BelongsTo<super::model_request::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}

/// Where a call is between arriving and its result (§9.1). A result is sent when the next
/// request seals its message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
pub enum ToolCallStatus {
    /// Just arrived; it is carried out or held in the same transaction.
    Received,
    /// Waiting for the user to continue a paused run.
    Held,
    /// A proposed edit waiting for its review.
    AwaitingReview,
    /// Questions waiting for the user's answers.
    AwaitingAnswer,
    /// The outcome is recorded and its result message queued.
    Resolved,
}
