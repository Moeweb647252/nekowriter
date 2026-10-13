use nekowriter_core::{
    citation::Citations,
    feedback::FeedbackStatus,
    ids::{FeedbackId, ModelRequestId, RunId, ToolCallId},
    time::Timestamp,
};
use sea_orm::entity::prelude::*;

/// Feedback the user gave a writer (§9.2). The run's queued and bound feedback form its
/// one unsealed batch, delivered with a tool result.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "feedback")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: FeedbackId,
    #[sea_orm(unique_key = "run_sequence")]
    pub run_id: RunId,
    /// Position among the run's feedback, starting at 1; delivery keeps this order.
    #[sea_orm(unique_key = "run_sequence")]
    pub sequence: i32,
    /// Empty when the feedback consists of citations only (a declined proposal).
    #[sea_orm(column_type = "Text")]
    pub text: String,
    /// The passages it quotes, verified.
    pub citations: Option<Citations>,
    pub status: FeedbackStatus,
    /// For queued feedback: the highest call sequence with a result when its batch
    /// opened. The batch binds to the result of a later call.
    pub after_tool_sequence: Option<i32>,
    /// The call whose result carries it, once bound.
    pub bound_tool_call_id: Option<ToolCallId>,
    /// The request that carried it, once included.
    pub included_in: Option<ModelRequestId>,
    pub created_at: Timestamp,
    #[sea_orm(belongs_to, from = "run_id", to = "id")]
    pub run: BelongsTo<super::run::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
