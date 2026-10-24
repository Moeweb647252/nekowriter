//! Chapter DTOs (spec/10 §2.2, §2.8).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::api::dto::ContentOp;
use crate::api::dto::ContentOpInput;
use crate::db::json::{CanonStatus, ChapterContent, ChapterStatus, VersionKind};

/// A chapter without its content (spec/10 §2.8 `ChapterListItemDto`).
///
/// Labels are composed by the UI (`number` plus the track), never by the API (DECISIONS D-054).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChapterListItemDto {
    pub id: i32,
    pub book_id: i32,
    pub track_id: i32,
    /// The chapter's number within its track, equal to `ord`.
    pub number: i32,
    pub title: String,
    pub status: ChapterStatus,
    pub canon_status: CanonStatus,
    pub word_count: i32,
    pub updated_at: DateTime<Utc>,
}

/// `GET /chapters/:id` (spec/10 §2.8). `open_sessions` stays empty until M3.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChapterDto {
    #[serde(flatten)]
    pub list_item: ChapterListItemDto,
    pub guidance: String,
    pub outline: String,
    pub reference_notes: String,
    pub summary: String,
    pub content: ChapterContent,
    pub content_version: i32,
    pub official_version_id: Option<i32>,
}

/// `POST /tracks/:id/chapters` body (spec/08 §1.1).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChapterInput {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub guidance: String,
}

/// `PATCH /chapters/:id` body (spec/10 §2.2).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChapterPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// `PATCH /chapters/:id/content` body (spec/10 §2.2).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChapterContentPatch {
    pub base_version: i32,
    pub ops: Vec<ContentOpInput>,
}

/// `PUT /chapters/:id/content/text` body; paragraphs are separated by blank lines.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChapterContentTextPut {
    pub base_version: i32,
    pub text: String,
}

/// The response of both content writes (spec/10 §2.2).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChapterContentResult {
    pub version: i32,
    pub ops: Vec<ContentOp>,
    pub word_count: i32,
}

/// `POST /tracks/:id/chapters/reorder` body (spec/08 §1.1).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChapterReorderInput {
    pub ids: Vec<i32>,
}

/// A version row without its content (`GET /chapters/:id/versions`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChapterVersionMetaDto {
    pub id: i32,
    pub chapter_id: i32,
    pub version_no: i32,
    pub kind: VersionKind,
    pub title: String,
    pub word_count: i32,
    pub note: String,
    pub session_id: Option<i32>,
    pub created_at: DateTime<Utc>,
}

/// `GET /chapter-versions/:id` — a version with its content.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChapterVersionDto {
    #[serde(flatten)]
    pub meta: ChapterVersionMetaDto,
    pub content: ChapterContent,
}

/// `POST /chapters/:id/versions` body: the author's manual snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ManualVersionInput {
    #[serde(default)]
    pub note: String,
}

impl From<crate::db::entities::chapters::Model> for ChapterListItemDto {
    fn from(row: crate::db::entities::chapters::Model) -> Self {
        Self {
            id: row.id,
            book_id: row.book_id,
            track_id: row.track_id,
            number: row.ord,
            title: row.title,
            status: ChapterStatus::from_str_opt(&row.status).unwrap_or(ChapterStatus::Planned),
            canon_status: CanonStatus::from_str_opt(&row.canon_status).unwrap_or(CanonStatus::None),
            word_count: row.word_count,
            updated_at: row.updated_at,
        }
    }
}
