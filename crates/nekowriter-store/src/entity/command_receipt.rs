use nekowriter_core::{ids::CommandId, time::Timestamp};
use sea_orm::entity::prelude::*;

/// The recorded outcome of a successful command, so a retry returns the same result.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "command_receipts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub command_id: CommandId,
    /// The command kind, such as `project.create`.
    pub kind: String,
    /// SHA-256 of the kind and the command's input; a retry must match it.
    pub input_hash: String,
    /// The command's output, as JSON.
    pub output: String,
    pub committed_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}
