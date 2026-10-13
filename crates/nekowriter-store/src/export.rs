//! A version's committed chapter text for export (`docs/product-spec.md` §16.4).

use nekowriter_core::{
    chapter::{ChapterKind, ChapterStatus},
    document::DocumentRole,
    ids::VersionId,
    project::WritingLanguage,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, TransactionTrait};

use crate::{
    Store, StoreError,
    entity::{chapter, document, revision},
    workflow::load_version,
};

/// What an export contains: committed bodies only, never proposals, previews, prompts,
/// usage, review notes or run records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedVersion {
    pub novel_title: String,
    pub writing_language: WritingLanguage,
    /// Plan chapters in order, then extra chapters in order.
    pub chapters: Vec<ExportedChapter>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedChapter {
    pub kind: ChapterKind,
    pub number: i32,
    pub title: String,
    /// The current body; `None` before the chapter has one.
    pub body: Option<String>,
    /// Whether the whole-chapter review approved the current body.
    pub approved: bool,
}

impl Store {
    /// Reads a version's chapters and bodies in one transaction, so they are consistent.
    pub async fn export_version(
        &self,
        version_id: VersionId,
    ) -> Result<ExportedVersion, StoreError> {
        let txn = self.db.begin().await?;
        let exported = export_version(&txn, version_id).await?;
        txn.commit().await?;
        Ok(exported)
    }
}

async fn export_version<C: ConnectionTrait>(
    db: &C,
    version_id: VersionId,
) -> Result<ExportedVersion, StoreError> {
    let context = load_version(db, version_id).await?;
    let mut chapters = chapter::Entity::find()
        .filter(chapter::Column::VersionId.eq(version_id))
        .all(db)
        .await?;
    chapters.sort_by_key(|chapter| (chapter.kind == ChapterKind::Extra, chapter.number));
    let bodies = document::Entity::find()
        .filter(document::Column::VersionId.eq(version_id))
        .filter(document::Column::Role.eq(DocumentRole::ChapterBody))
        .all(db)
        .await?;
    let mut exported = Vec::with_capacity(chapters.len());
    for chapter in chapters {
        let current = bodies
            .iter()
            .find(|body| body.chapter_id == Some(chapter.id))
            .and_then(|body| body.current_revision_id);
        let body = match current {
            Some(id) => Some(
                revision::Entity::find_by_id(id)
                    .one(db)
                    .await?
                    .ok_or_else(|| StoreError::Corrupt(format!("revision {id} is missing")))?
                    .content,
            ),
            None => None,
        };
        exported.push(ExportedChapter {
            kind: chapter.kind,
            number: chapter.number,
            title: chapter.title,
            body,
            approved: chapter.status == ChapterStatus::Approved,
        });
    }
    Ok(ExportedVersion {
        novel_title: context.version.novel_title.as_str().to_owned(),
        writing_language: context.project.writing_language,
        chapters: exported,
    })
}
