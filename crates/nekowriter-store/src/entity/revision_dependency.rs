use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "revision_dependencies")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub revision_id: TextUuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub dependency_revision_id: TextUuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub purpose: String,
    #[sea_orm(belongs_to, from = "revision_id", to = "id")]
    pub revision: BelongsTo<super::document_revision::Entity>,
    #[sea_orm(
        belongs_to,
        relation_enum = "DependencyRevision",
        from = "dependency_revision_id",
        to = "id"
    )]
    pub dependency_revision: BelongsTo<super::document_revision::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
