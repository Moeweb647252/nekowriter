//! Chapter versions (spec/08 §1.2, spec/02 §2 `chapter_versions`, spec/10 §2.2).
//!
//! Version kinds: `draft` (draft complete or submit from drafting), `revision` (revision complete
//! or submit from revising), `official` (approve, re-sync completion), `manual` (the author's
//! "Save version") and `backup` (before redraft, revise or restore).

use sea_orm::{
    ConnectionTrait, DerivePartialModel, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};

use crate::api::dto::chapters::{ChapterVersionDto, ChapterVersionMetaDto};
use crate::db::entities::chapter_versions;
use crate::db::json::{ChapterContent, VersionKind};
use crate::error::AppError;

use super::now;

/// One version to append.
pub struct NewVersion<'a> {
    pub chapter_id: i32,
    pub kind: VersionKind,
    pub title: &'a str,
    pub content: &'a ChapterContent,
    pub word_count: i32,
    pub note: &'a str,
    pub session_id: Option<i32>,
}

/// Appends a version row inside the caller's transaction and returns it.
pub async fn create_version(
    tx: &impl ConnectionTrait,
    version: NewVersion<'_>,
) -> Result<chapter_versions::Model, AppError> {
    let version_no = next_version_no(tx, version.chapter_id).await?;
    let model = chapter_versions::ActiveModel {
        chapter_id: Set(version.chapter_id),
        version_no: Set(version_no),
        kind: Set(version.kind),
        title: Set(version.title.to_owned()),
        content: Set(crate::services::json_text(version.content.clone())),
        word_count: Set(version.word_count),
        note: Set(version.note.to_owned()),
        session_id: Set(version.session_id),
        created_at: Set(now()),
        ..Default::default()
    };
    let row = chapter_versions::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    Ok(row)
}

async fn next_version_no(conn: &impl ConnectionTrait, chapter_id: i32) -> Result<i32, AppError> {
    let latest = chapter_versions::Entity::find()
        .filter(chapter_versions::COLUMN.chapter_id.eq(chapter_id))
        .order_by_desc(chapter_versions::COLUMN.version_no)
        .all(conn)
        .await?
        .into_iter()
        .next();
    Ok(latest.map(|row| row.version_no + 1).unwrap_or(1))
}

/// The metadata columns of the version list; `content` is deliberately not decoded (plan §2.7).
#[derive(DerivePartialModel)]
#[sea_orm(entity = "chapter_versions::Entity")]
struct ChapterVersionListItemRow {
    id: i32,
    chapter_id: i32,
    version_no: i32,
    kind: crate::db::json::VersionKind,
    title: String,
    word_count: i32,
    note: String,
    session_id: Option<i32>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<ChapterVersionListItemRow> for ChapterVersionMetaDto {
    fn from(row: ChapterVersionListItemRow) -> Self {
        Self {
            id: row.id,
            chapter_id: row.chapter_id,
            version_no: row.version_no,
            kind: row.kind,
            title: row.title,
            word_count: row.word_count,
            note: row.note,
            session_id: row.session_id,
            created_at: row.created_at,
        }
    }
}

/// `GET /api/chapters/:id/versions`, newest first.
pub async fn list(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
) -> Result<Vec<ChapterVersionMetaDto>, AppError> {
    let rows = chapter_versions::Entity::find()
        .filter(chapter_versions::COLUMN.chapter_id.eq(chapter_id))
        .order_by_desc(chapter_versions::COLUMN.version_no)
        .select_only()
        .columns([
            chapter_versions::Column::Id,
            chapter_versions::Column::ChapterId,
            chapter_versions::Column::VersionNo,
            chapter_versions::Column::Kind,
            chapter_versions::Column::Title,
            chapter_versions::Column::WordCount,
            chapter_versions::Column::Note,
            chapter_versions::Column::SessionId,
            chapter_versions::Column::CreatedAt,
        ])
        .into_model::<ChapterVersionListItemRow>()
        .all(conn)
        .await?;
    Ok(rows.into_iter().map(ChapterVersionMetaDto::from).collect())
}

/// `GET /api/chapter-versions/:id` — a version with its content.
pub async fn get(
    conn: &impl ConnectionTrait,
    version_id: i32,
) -> Result<ChapterVersionDto, AppError> {
    let row = load(conn, version_id).await?;
    Ok(ChapterVersionDto {
        meta: to_meta(&row)?,
        content: row.content.clone().into_inner(),
    })
}

/// Loads a version row or reports `not_found`.
pub async fn load(
    conn: &impl ConnectionTrait,
    version_id: i32,
) -> Result<chapter_versions::Model, AppError> {
    chapter_versions::Entity::find_by_id(version_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("chapter version #{version_id}")))
}

/// Maps a version row to its metadata DTO.
pub fn to_meta(row: &chapter_versions::Model) -> Result<ChapterVersionMetaDto, AppError> {
    Ok(ChapterVersionMetaDto {
        id: row.id,
        chapter_id: row.chapter_id,
        version_no: row.version_no,
        kind: row.kind,
        title: row.title.clone(),
        word_count: row.word_count,
        note: row.note.clone(),
        session_id: row.session_id,
        created_at: row.created_at,
    })
}
