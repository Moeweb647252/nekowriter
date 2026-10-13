//! The single row that identifies a NekoWriter database and the schema it was created with.

use nekowriter_core::time::Timestamp;
use sea_orm::entity::prelude::*;

/// The primary key of the only row.
pub(crate) const ROW_ID: i32 = 1;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "schema_info")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i32,
    /// SHA-256 of the schema definition; see `crate::schema::fingerprint`.
    pub fingerprint: String,
    /// The application version that created the database, for error messages.
    pub created_by_version: String,
    pub created_at: Timestamp,
    /// The highest event sequence number deleted by retention pruning (0 if none).
    pub events_pruned_through: i64,
}

impl ActiveModelBehavior for ActiveModel {}
