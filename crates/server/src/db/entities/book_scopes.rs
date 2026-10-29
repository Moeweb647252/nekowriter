//! `book_scopes` join table entity (spec/02 §2).
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "book_scopes")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub book_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    pub scope_id: i32,
    #[sea_orm(belongs_to, relation_enum = "Book", from = "book_id", to = "id")]
    pub book: BelongsTo<super::books::Entity>,
    #[sea_orm(belongs_to, relation_enum = "Scope", from = "scope_id", to = "id")]
    pub scope: BelongsTo<super::knowledge_scopes::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
