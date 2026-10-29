//! `knowledge_scopes` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_scopes")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique)]
    pub name: String,
    pub description: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(has_many)]
    pub knowledge_entries: HasMany<super::knowledge_entries::Entity>,
    #[sea_orm(has_many)]
    pub knowledge_chunks: HasMany<super::knowledge_chunks::Entity>,
    #[sea_orm(has_many)]
    pub research_tasks: HasMany<super::research_tasks::Entity>,
    #[sea_orm(has_many, relation_enum = "Book", via = "book_scopes::Scope")]
    pub books: HasMany<super::books::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
