use nekowriter_core::ids::{EmbeddingProfileId, KnowledgeEntryId};
use sea_orm::entity::prelude::*;

/// A chunk of an entry revision and its vector under one profile (§12.2).
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_chunks")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(indexed)]
    pub entry_id: KnowledgeEntryId,
    #[sea_orm(indexed)]
    pub profile_id: EmbeddingProfileId,
    pub entry_revision: i64,
    /// Position among the entry's chunks, starting at 0.
    pub ordinal: i32,
    #[sea_orm(column_type = "Text")]
    pub text: String,
    /// The vector as little-endian `f32` values.
    #[sea_orm(column_type = "VarBinary(StringLen::None)")]
    pub vector: Vec<u8>,
}

impl ActiveModelBehavior for ActiveModel {}
