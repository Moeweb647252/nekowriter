use nekowriter_core::{
    document::DocumentRole,
    ids::{DocumentId, RevisionId, VersionId},
};
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "documents")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DocumentId,
    #[sea_orm(unique_key = "version_role")]
    pub version_id: VersionId,
    #[sea_orm(unique_key = "version_role")]
    pub role: DocumentRole,
    /// The latest revision. Revisions reference their document, so this reference has no
    /// foreign key; the revision's own key keeps both consistent.
    pub current_revision_id: Option<RevisionId>,
    #[sea_orm(belongs_to, from = "version_id", to = "id")]
    pub version: BelongsTo<super::version::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
