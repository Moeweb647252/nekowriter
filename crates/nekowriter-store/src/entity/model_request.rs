use nekowriter_core::{
    ids::{ModelRequestId, RunId},
    run::ProviderFailure,
    time::Timestamp,
    usage::TokenUsage,
};
use sea_orm::{entity::prelude::*, sea_query::StringLen};

/// One request to the model service and what came of it.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "model_requests")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: ModelRequestId,
    #[sea_orm(unique_key = "run_attempt")]
    pub run_id: RunId,
    /// Position among the run's requests, starting at 1.
    #[sea_orm(unique_key = "run_attempt")]
    pub attempt: i32,
    pub status: ModelRequestStatus,
    /// The request body exactly as sent, as JSON. The API key is never part of it.
    #[sea_orm(column_type = "Text")]
    pub body: String,
    pub estimated_input_tokens: i32,
    pub finish_reason: Option<String>,
    pub usage: Option<TokenUsage>,
    /// The usage object as the service sent it, as JSON.
    #[sea_orm(column_type = "Text", nullable)]
    pub raw_usage: Option<String>,
    pub failure: Option<ProviderFailure>,
    pub provider_request_id: Option<String>,
    pub dispatched_at: Timestamp,
    pub completed_at: Option<Timestamp>,
    #[sea_orm(belongs_to, from = "run_id", to = "id")]
    pub run: BelongsTo<super::run::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
pub enum ModelRequestStatus {
    /// Sent; no answer yet.
    Dispatched,
    Succeeded,
    Failed,
    /// The process stopped while the request was in flight; the service may or may not
    /// have processed (and charged for) it.
    Unknown,
    /// The user stopped the run while the request was in flight; the request was
    /// abandoned.
    Cancelled,
}
