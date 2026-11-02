//! `model_profiles` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

use crate::db::json::{JsonText, ModelOptions};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "model_profiles")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique_key = "provider_display_name")]
    pub provider_id: i32,
    #[sea_orm(unique_key = "provider_display_name")]
    pub display_name: String,
    pub model: String,
    pub context_window: i32,
    pub max_output_tokens: i32,
    pub options: JsonText<ModelOptions>,
    /// The thinking-effort levels this model accepts, in display order; empty means the model
    /// takes no effort parameter (spec/04 §5.1, D-162).
    pub effort_levels: JsonText<Vec<String>>,
    /// The level a role that chooses none gets; one of `effort_levels`, or `NULL` when the
    /// provider decides (spec/04 §5.1, D-162).
    pub default_effort: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "provider_id", to = "id")]
    pub provider: BelongsTo<super::providers::Entity>,
    #[sea_orm(has_many)]
    pub agent_sessions: HasMany<super::agent_sessions::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
