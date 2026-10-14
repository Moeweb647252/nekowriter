use nekowriter_core::{ids::ConnectionTestId, time::Timestamp, usage::TokenUsage};
use sea_orm::entity::prelude::*;

/// One connection test against the model service (§13.3), kept for the dashboard's token
/// usage, where tests are counted apart from runs (§15).
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "connection_tests")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: ConnectionTestId,
    /// Whether the service answered.
    pub answered: bool,
    /// The counts, if the service sent a usage object.
    pub usage: Option<TokenUsage>,
    pub tested_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}
