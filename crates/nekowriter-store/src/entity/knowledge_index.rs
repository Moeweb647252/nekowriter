use nekowriter_core::{
    embedding::EmbeddingFailure,
    ids::{EmbeddingProfileId, KnowledgeEntryId},
    time::Timestamp,
};
use sea_orm::{entity::prelude::*, sea_query::StringLen};

/// An entry's index under one profile: the task that builds it and its outcome (§12.2).
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_indexes")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub entry_id: KnowledgeEntryId,
    #[sea_orm(primary_key, auto_increment = false)]
    pub profile_id: EmbeddingProfileId,
    /// The entry revision the index is for.
    pub entry_revision: i64,
    #[sea_orm(indexed)]
    pub status: IndexStatus,
    pub failure: Option<EmbeddingFailure>,
    /// When the task last joined the queue; the indexer serves the oldest first.
    pub queued_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
pub enum IndexStatus {
    Pending,
    Running,
    Ready,
    Failed,
}
