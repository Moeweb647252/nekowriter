use nekowriter_core::{
    prompt::{PromptKey, PromptRole},
    time::Timestamp,
};
use sea_orm::entity::prelude::*;

/// A system-wide override of a built-in prompt (§13.1). Resetting deletes the row.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "system_prompts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: PromptKey,
    #[sea_orm(primary_key, auto_increment = false)]
    pub role: PromptRole,
    /// The override's text; empty is real empty content.
    #[sea_orm(column_type = "Text")]
    pub content: String,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}
