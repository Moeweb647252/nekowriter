//! `setting_docs` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

use crate::db::json::SettingDocKind;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "setting_docs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique_key = "book_kind")]
    pub book_id: i32,
    #[sea_orm(unique_key = "book_kind")]
    pub kind: SettingDocKind,
    pub content: String,
    pub revision: i32,
    pub approved_revision: Option<i32>,
    pub updated_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<super::books::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
