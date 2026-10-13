//! Documents and their immutable revisions.
//!
//! Views reference revisions by [`RevisionSummary`]; clients load a revision's content once
//! by its ID ([`Store::revision`]), since a revision never changes.

use nekowriter_core::{
    document::DocumentRole,
    ids::{ChapterId, RevisionId, RunId, VersionId},
    preparation::ChapterPlan,
    time::Timestamp,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Store, StoreError,
    entity::{document, revision},
    workflow::load_version,
};

/// A revision with its content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RevisionView {
    pub id: RevisionId,
    /// Position among the document's revisions, starting at 1.
    pub number: i32,
    pub content: String,
    /// The chapter plan, for revisions of the preparation outline.
    pub chapter_plan: Option<ChapterPlan>,
    /// The word count, for revisions of a chapter body.
    pub word_count: Option<i32>,
    pub created_at: Timestamp,
}

impl From<revision::Model> for RevisionView {
    fn from(revision: revision::Model) -> Self {
        Self {
            id: revision.id,
            number: revision.number,
            content: revision.content,
            chapter_plan: revision.chapter_plan,
            word_count: revision.word_count,
            created_at: revision.created_at,
        }
    }
}

/// A revision without its content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RevisionSummary {
    pub id: RevisionId,
    pub number: i32,
    pub word_count: Option<i32>,
    pub created_at: Timestamp,
}

impl From<&revision::Model> for RevisionSummary {
    fn from(revision: &revision::Model) -> Self {
        Self {
            id: revision.id,
            number: revision.number,
            word_count: revision.word_count,
            created_at: revision.created_at,
        }
    }
}

impl Store {
    /// A revision with its content, unless its version or project is deleted.
    pub async fn revision(&self, id: RevisionId) -> Result<RevisionView, StoreError> {
        let not_found = || StoreError::NotFound(nekowriter_core::conflict::Resource::Revision);
        let revision = revision::Entity::find_by_id(id)
            .one(&self.db)
            .await?
            .ok_or_else(not_found)?;
        let document = document::Entity::find_by_id(revision.document_id)
            .one(&self.db)
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("revision {id} has no document")))?;
        load_version(&self.db, document.version_id)
            .await
            .map_err(|error| match error {
                StoreError::NotFound(_) => not_found(),
                error => error,
            })?;
        Ok(RevisionView::from(revision))
    }
}

/// The version-level document with `role`.
pub(crate) async fn version_document<C: ConnectionTrait>(
    db: &C,
    version_id: VersionId,
    role: DocumentRole,
) -> Result<document::Model, StoreError> {
    document::Entity::find()
        .filter(document::Column::VersionId.eq(version_id))
        .filter(document::Column::ChapterId.is_null())
        .filter(document::Column::Role.eq(role))
        .one(db)
        .await?
        .ok_or_else(|| {
            StoreError::Corrupt(format!("version {version_id} has no {role:?} document"))
        })
}

/// The chapter's document with `role`.
pub(crate) async fn chapter_document<C: ConnectionTrait>(
    db: &C,
    chapter_id: ChapterId,
    role: DocumentRole,
) -> Result<document::Model, StoreError> {
    document::Entity::find()
        .filter(document::Column::ChapterId.eq(chapter_id))
        .filter(document::Column::Role.eq(role))
        .one(db)
        .await?
        .ok_or_else(|| {
            StoreError::Corrupt(format!("chapter {chapter_id} has no {role:?} document"))
        })
}

pub(crate) async fn current_revision<C: ConnectionTrait>(
    db: &C,
    document: &document::Model,
) -> Result<Option<revision::Model>, StoreError> {
    let Some(id) = document.current_revision_id else {
        return Ok(None);
    };
    let revision = revision::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| {
            StoreError::Corrupt(format!(
                "document {} points at a missing revision",
                document.id
            ))
        })?;
    Ok(Some(revision))
}

/// The new content of a document.
pub(crate) struct NewRevision {
    pub(crate) content: String,
    pub(crate) chapter_plan: Option<ChapterPlan>,
    pub(crate) word_count: Option<u32>,
    pub(crate) created_by_run: Option<RunId>,
}

impl NewRevision {
    /// Content without a chapter plan or a word count.
    pub(crate) fn text(content: String, created_by_run: Option<RunId>) -> Self {
        Self {
            content,
            chapter_plan: None,
            word_count: None,
            created_by_run,
        }
    }
}

/// Commits a new revision and makes it the document's current one.
pub(crate) async fn add_revision<C: ConnectionTrait>(
    db: &C,
    document: document::Model,
    new: NewRevision,
    now: Timestamp,
) -> Result<revision::Model, StoreError> {
    let number = match current_revision(db, &document).await? {
        Some(current) => current.number + 1,
        None => 1,
    };
    let revision = revision::ActiveModel {
        id: Set(RevisionId::generate()),
        document_id: Set(document.id),
        number: Set(number),
        content: Set(new.content),
        chapter_plan: Set(new.chapter_plan),
        word_count: Set(new
            .word_count
            .map(|count| i32::try_from(count).unwrap_or(i32::MAX))),
        created_by_run: Set(new.created_by_run),
        created_at: Set(now),
    }
    .insert(db)
    .await?;
    let mut document = document.into_active_model();
    document.current_revision_id = Set(Some(revision.id));
    document.update(db).await?;
    Ok(revision)
}
