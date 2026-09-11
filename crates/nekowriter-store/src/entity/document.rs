use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "documents")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    pub version_id: TextUuid,
    pub chapter_id: Option<String>,
    pub role: String,
    pub current_revision_id: Option<String>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    #[sea_orm(belongs_to, from = "version_id", to = "id")]
    pub version: BelongsTo<super::novel_version::Entity>,
    #[sea_orm(belongs_to, from = "chapter_id", to = "id")]
    pub chapter: BelongsTo<Option<super::chapter::Entity>>,
    #[sea_orm(
        belongs_to,
        relation_enum = "CurrentRevision",
        from = "current_revision_id",
        to = "id"
    )]
    pub current_revision: BelongsTo<Option<super::document_revision::Entity>>,
    #[sea_orm(has_many)]
    pub revisions: HasMany<super::document_revision::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
