use nekowriter_core::ids::CommandId;
use serde::Deserialize;
use ts_rs::TS;

pub(crate) mod dashboard;
pub(crate) mod embedding;
pub(crate) mod engines;
pub(crate) mod events;
pub(crate) mod export;
pub(crate) mod knowledge;
pub(crate) mod projects;
pub(crate) mod prompts;
pub(crate) mod settings;
pub(crate) mod system;
pub(crate) mod templates;
pub(crate) mod workspace;

/// The request of control commands, which quote no revision (`docs/product-spec.md`
/// §16.3).
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ControlRequest {
    pub command_id: CommandId,
}

/// The request of commands that need nothing but the target's current revision.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RevisionRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
}
