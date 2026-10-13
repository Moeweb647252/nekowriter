use nekowriter_core::ids::CommandId;
use serde::Deserialize;
use ts_rs::TS;

pub(crate) mod events;
pub(crate) mod projects;
pub(crate) mod settings;
pub(crate) mod system;
pub(crate) mod workspace;

/// The request of commands that need nothing but the target's current revision.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RevisionRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
}
