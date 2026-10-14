use nekowriter_core::{
    ids::ProjectId,
    prompt::{PromptKey, PromptRole},
    time::Timestamp,
};
use sea_orm::entity::prelude::*;

/// A project's override of a built-in prompt, which wins over the system's (§13.1).
/// Resetting deletes the row.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "project_prompts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub project_id: ProjectId,
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: PromptKey,
    #[sea_orm(primary_key, auto_increment = false)]
    pub role: PromptRole,
    /// The override's text; empty is real empty content.
    #[sea_orm(column_type = "Text")]
    pub content: String,
    pub updated_at: Timestamp,
    #[sea_orm(belongs_to, from = "project_id", to = "id")]
    pub project: BelongsTo<super::project::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
