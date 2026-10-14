use nekowriter_core::{ids::EmbeddingProfileId, time::Timestamp};
use sea_orm::{entity::prelude::*, sea_query::StringLen};

/// One request to the embedding service, for the dashboard's usage (§15).
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "embedding_requests")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub profile_id: EmbeddingProfileId,
    pub purpose: EmbeddingPurpose,
    /// Whether vectors came back.
    pub answered: bool,
    /// The input tokens, if the service reported them.
    pub input_tokens: Option<i64>,
    #[sea_orm(indexed)]
    pub sent_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
pub enum EmbeddingPurpose {
    Index,
    Search,
}
