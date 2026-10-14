//! Events: notices that committed data changed.
//!
//! An event is written in the same transaction as the change it describes and is then
//! streamed to clients, which refresh whatever the event affects. Events name what changed;
//! they do not carry the new state.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{ProjectId, VersionId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum Event {
    /// A project was created, renamed, deleted or restored, or its settings or selected
    /// version changed.
    ProjectChanged { project_id: ProjectId },
    /// Something in a version's workspace changed: its workflow, runs, documents, reviews
    /// or chapters.
    VersionChanged {
        project_id: ProjectId,
        version_id: VersionId,
    },
    /// The system settings changed.
    SettingsChanged,
    /// A user prompt template was created, changed or deleted.
    TemplatesChanged,
    /// A knowledge space or entry was created, changed or deleted, or an entry's index
    /// changed.
    KnowledgeChanged,
}
