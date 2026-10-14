use nekowriter_core::{
    ids::{DocumentId, RevisionId, RunId},
    preparation::ChapterPlan,
    time::Timestamp,
};
use sea_orm::entity::prelude::*;

/// One immutable revision of a document.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "revisions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: RevisionId,
    #[sea_orm(unique_key = "document_number")]
    pub document_id: DocumentId,
    /// Position among the document's revisions, starting at 1.
    #[sea_orm(unique_key = "document_number")]
    pub number: i32,
    #[sea_orm(column_type = "Text")]
    pub content: String,
    /// The chapter plan, for revisions of the outline.
    pub chapter_plan: Option<ChapterPlan>,
    /// The content's word count, for revisions of a chapter body (§15).
    pub word_count: Option<i32>,
    /// The word count minus the previous revision's (or zero's, for the first one), for
    /// revisions of a chapter body; negative when the body shrank (§15).
    pub word_delta: Option<i32>,
    /// The run that wrote it; `None` when the user wrote it.
    pub created_by_run: Option<RunId>,
    pub created_at: Timestamp,
    #[sea_orm(belongs_to, from = "document_id", to = "id")]
    pub document: BelongsTo<super::document::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
