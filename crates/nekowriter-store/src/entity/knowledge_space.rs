use nekowriter_core::{ids::KnowledgeSpaceId, knowledge::SpaceName, time::Timestamp};
use sea_orm::entity::prelude::*;

/// A space that sorts knowledge entries (§12.2). Deleting it is soft, and only an empty
/// space can be deleted.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_spaces")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: KnowledgeSpaceId,
    pub name: SpaceName,
    pub revision: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub deleted_at: Option<Timestamp>,
}

impl ActiveModelBehavior for ActiveModel {}
