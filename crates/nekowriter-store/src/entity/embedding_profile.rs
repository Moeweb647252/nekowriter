use nekowriter_core::{ids::EmbeddingProfileId, model::ModelId, time::Timestamp};
use sea_orm::entity::prelude::*;

/// An embedding model and vector size whose vectors are stored (§12.2).
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "embedding_profiles")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: EmbeddingProfileId,
    #[sea_orm(unique_key = "model_dimensions")]
    pub model_id: ModelId,
    #[sea_orm(unique_key = "model_dimensions")]
    pub dimensions: i32,
    /// When every entry that was live when the profile became current had been indexed;
    /// search waits for it.
    pub ready_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}
