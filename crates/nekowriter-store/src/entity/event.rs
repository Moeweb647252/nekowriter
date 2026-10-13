use nekowriter_core::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "events")]
pub struct Model {
    /// Assigned by SQLite in commit order; clients resume the event stream after it.
    #[sea_orm(primary_key)]
    pub seq: i64,
    /// The `nekowriter_core::event::Event`, as JSON.
    pub payload: String,
    #[sea_orm(indexed)]
    pub occurred_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}
