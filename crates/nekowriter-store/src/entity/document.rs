use nekowriter_core::{
    document::DocumentRole,
    ids::{ChapterId, DocumentId, RevisionId, VersionId},
};
use sea_orm::entity::prelude::*;

/// A version-level document (`chapter_id` is `None`) or a chapter's document. The two sets
/// of roles are disjoint. SQLite treats `NULL`s as distinct in unique keys, so the key
/// enforces one document per role for chapters only; version-level documents are created
/// once, with their version.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "documents")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DocumentId,
    #[sea_orm(unique_key = "owner_role")]
    pub version_id: VersionId,
    #[sea_orm(unique_key = "owner_role")]
    pub chapter_id: Option<ChapterId>,
    #[sea_orm(unique_key = "owner_role")]
    pub role: DocumentRole,
    /// The latest revision. Revisions reference their document, so this reference has no
    /// foreign key; the revision's own key keeps both consistent.
    pub current_revision_id: Option<RevisionId>,
    #[sea_orm(belongs_to, from = "version_id", to = "id")]
    pub version: BelongsTo<super::version::Entity>,
    #[sea_orm(belongs_to, from = "chapter_id", to = "id")]
    pub chapter: BelongsTo<Option<super::chapter::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
