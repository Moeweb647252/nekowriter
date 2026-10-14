use nekowriter_core::{
    ids::{KnowledgeEntryId, KnowledgeSpaceId},
    knowledge::{EntryTitle, Keywords},
    time::Timestamp,
};
use sea_orm::entity::prelude::*;

/// A knowledge entry (§12.2). Its revision counts content changes. Deleting it is soft.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_entries")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: KnowledgeEntryId,
    #[sea_orm(indexed)]
    pub space_id: KnowledgeSpaceId,
    pub title: EntryTitle,
    pub keywords: Keywords,
    #[sea_orm(column_type = "Text")]
    pub content: String,
    pub revision: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub deleted_at: Option<Timestamp>,
    #[sea_orm(belongs_to, from = "space_id", to = "id")]
    pub space: BelongsTo<super::knowledge_space::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
