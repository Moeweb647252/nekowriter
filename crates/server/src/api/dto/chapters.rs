//! Chapter DTOs (spec/10 §2.2, §2.8).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::api::dto::ContentOp;
use crate::api::dto::ContentOpInput;
use crate::api::dto::SessionDto;
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
    /// `!chapters.outline.trim().is_empty()`: the navigator draws a planned row with an outline
    /// differently, and the header's start action depends on it (spec/08 §7, D-146).
    pub has_outline: bool,
    pub updated_at: DateTime<Utc>,
}

/// `POST /chapters/:id/approve` response (spec/10 §2.2).
///
/// `warnings` carries codes the UI localizes (spec/10 §1); with `auto_maintenance` on and no
/// archivist model configured the chapter still becomes official, stays `canon_status = pending`
/// and the response says `["model.missing_role"]` (spec/08 §1.1). `maintenance_session_id` is
/// present once approval starts the maintenance session (M3+).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApproveChapterResponse {
    pub chapter: ChapterDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance_session_id: Option<i32>,
    pub warnings: Vec<String>,
}

/// `GET /chapters/:id` (spec/10 §2.8).
///
/// `open_sessions` holds the chapter's open writer, reviewer and archivist sessions (spec/03 §3:
/// open means not `completed`, `superseded` or `abandoned`), oldest first. Conception, research
/// and canon-seed sessions are not per-chapter and never appear here. The field is always
/// serialized, so a client can rely on its presence.
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
    pub open_sessions: Vec<SessionDto>,
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

/// `POST /chapters/:id/outline` body (spec/08 §1.1, spec/10 §2.2).
///
/// `pause_after_outline` defaults to the book setting; `force` overrides the "previous chapter is
/// ready" precondition after the Outline dialog explained why it failed (the same rule as Write).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct ChapterOutlineInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_after_outline: Option<bool>,
    #[serde(default)]
    pub force: bool,
}

/// `POST /chapters/:id/write` body (spec/08 §1.1, spec/10 §2.2).
///
/// The writer switches default to the book's settings; `force` overrides the "previous chapter is
/// ready" precondition after the Write dialog explained why it failed. `pause_after_outline` is
/// not here: it belongs to the outline session (D-146).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct ChapterWriteInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granularity: Option<crate::db::json::Granularity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_pause: Option<bool>,
    #[serde(default)]
    pub force: bool,
}

/// `POST /chapters/:id/write` response (spec/10 §2.2).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChapterWriteResponse {
    pub session_id: i32,
    pub warnings: Vec<String>,
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
            status: row.status,
            canon_status: row.canon_status,
            word_count: row.word_count,
            has_outline: !row.outline.trim().is_empty(),
            updated_at: row.updated_at,
        }
    }
}
