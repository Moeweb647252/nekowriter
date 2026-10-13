use nekowriter_core::ids::RevisionId;
use sea_orm::entity::prelude::*;

/// A revision was written from another one (§6), so an approval of it covers that source.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "revision_dependencies")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub revision_id: RevisionId,
    #[sea_orm(primary_key, auto_increment = false)]
    pub depends_on_revision_id: RevisionId,
    #[sea_orm(
        belongs_to,
        from = "revision_id",
        to = "id",
        relation_enum = "Revision"
    )]
    pub revision: BelongsTo<super::revision::Entity>,
    #[sea_orm(
        belongs_to,
        from = "depends_on_revision_id",
        to = "id",
        relation_enum = "DependsOn"
    )]
    pub depends_on: BelongsTo<super::revision::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
