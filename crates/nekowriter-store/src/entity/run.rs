use nekowriter_core::{
    ids::{ProjectId, RunId, VersionId},
    run::{Fold, RunHalt, RunKind, RunStatus},
    time::Timestamp,
};
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "runs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: RunId,
    /// Kept on the run so the project's single writing slot can be checked directly.
    #[sea_orm(indexed)]
    pub project_id: ProjectId,
    pub version_id: VersionId,
    pub kind: RunKind,
    #[sea_orm(indexed)]
    pub status: RunStatus,
    /// Why the run stopped, failed or was interrupted.
    pub halt: Option<RunHalt>,
    /// How many submissions were accepted; preparation review rounds are numbered by it.
    pub round: i32,
    pub request_count: i32,
    /// The request count this run may reach before the user continues it (§8.7).
    pub request_allowance_end: i32,
    pub consecutive_protocol_errors: i32,
    pub resume_count: i32,
    /// Set when the run's stage is approved.
    pub fold: Option<Fold>,
    /// When the run last joined the queue; the dispatcher serves the oldest first.
    pub queued_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[sea_orm(belongs_to, from = "version_id", to = "id")]
    pub version: BelongsTo<super::version::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
