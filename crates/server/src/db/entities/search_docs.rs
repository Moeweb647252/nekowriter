//! `search_docs` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "search_docs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: i32,
    pub track_id: Option<i32>,
    #[sea_orm(unique_key = "doc_ref")]
    pub kind: String,
    #[sea_orm(unique_key = "doc_ref")]
    pub ref_id: i32,
    #[sea_orm(unique_key = "doc_ref")]
    pub ref_part: i32,
    pub title: String,
    pub body: String,
    pub meta: String,
    pub content_hash: String,
    pub embed_state: String,
    pub updated_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<super::books::Entity>,
    #[sea_orm(belongs_to, from = "track_id", to = "id")]
    pub track: BelongsTo<Option<super::tracks::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
