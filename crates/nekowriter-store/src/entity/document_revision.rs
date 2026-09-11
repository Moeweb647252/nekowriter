use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "document_revisions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    #[sea_orm(unique_key = "document_revision_no")]
    pub document_id: TextUuid,
    #[sea_orm(unique_key = "document_revision_no")]
    pub revision_no: i64,
    pub content: String,
    pub structured_payload: Option<String>,
    pub word_count: i64,
    pub created_by: String,
    pub source_revision_id: Option<String>,
    pub created_at: i64,
    #[sea_orm(belongs_to, from = "document_id", to = "id")]
    pub document: BelongsTo<super::document::Entity>,
    #[sea_orm(
        self_ref,
        relation_enum = "SourceRevision",
        from = "source_revision_id",
        to = "id"
    )]
    pub source_revision: BelongsTo<Option<Entity>>,
    #[sea_orm(has_many)]
    pub reviews: HasMany<super::review::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
