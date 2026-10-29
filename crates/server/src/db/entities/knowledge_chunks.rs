//! `knowledge_chunks` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

use crate::db::json::EmbedState;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_chunks")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique_key = "entry_ord")]
    pub entry_id: i32,
    pub scope_id: i32,
    #[sea_orm(unique_key = "entry_ord")]
    pub ord: i32,
    pub heading: String,
    pub text: String,
    pub content_hash: String,
    pub embed_state: EmbedState,
    #[sea_orm(belongs_to, from = "entry_id", to = "id")]
    pub entry: BelongsTo<super::knowledge_entries::Entity>,
    #[sea_orm(belongs_to, from = "scope_id", to = "id")]
    pub scope: BelongsTo<super::knowledge_scopes::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
