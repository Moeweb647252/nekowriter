//! `model_profiles` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

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
    pub options: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "provider_id", to = "id")]
    pub provider: BelongsTo<super::providers::Entity>,
    #[sea_orm(has_many)]
    pub agent_sessions: HasMany<super::agent_sessions::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
