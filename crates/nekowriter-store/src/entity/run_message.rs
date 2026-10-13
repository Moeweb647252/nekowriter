use nekowriter_core::{
    ids::{ModelRequestId, RunId},
    run_message::{RunMessageBody, TranscriptMessage},
    time::Timestamp,
};
use sea_orm::entity::prelude::*;

/// A message of a run's conversation after its stage input (§9.1).
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "run_messages")]
pub struct Model {
    /// Assigned in insertion order, which is the conversation's order.
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(indexed)]
    pub run_id: RunId,
    pub body: RunMessageBody,
    /// The message as the model sees it; `None` until a request seals it.
    pub rendered: Option<TranscriptMessage>,
    /// The request that first carried the message.
    pub sealed_by: Option<ModelRequestId>,
    pub created_at: Timestamp,
    #[sea_orm(belongs_to, from = "run_id", to = "id")]
    pub run: BelongsTo<super::run::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
