//! Book DTOs (spec/10 §2.2, §2.8, spec/08 §7).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::json::{
    BookSettings, BookStatus, ChapterStatus, ConceptionState, RoleModels, Usage,
};

/// `GET /books/:id`, and the `book` field of the create response.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BookDto {
    pub id: i32,
    pub title: String,
    pub idea: String,
    pub genre: String,
    /// BCP-47 tag, e.g. `zh-CN`.
    pub writing_language: String,
    pub status: BookStatus,
    pub settings: BookSettings,
    pub model_overrides: RoleModels,
    pub conception: ConceptionState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One row of the library grid (spec/10 §2.2: counts, words, latest activity, attention count).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BookSummaryDto {
    pub id: i32,
    pub title: String,
    pub genre: String,
    pub writing_language: String,
    pub status: BookStatus,
    pub chapter_count: i32,
    pub official_chapter_count: i32,
    /// Sum of `chapters.word_count` over the book's chapters.
    pub word_count: i32,
    /// The chapter the card's status line shows (highest `ord` of the main track).
    pub latest_chapter: Option<ChapterPointerDto>,
    /// Open sessions needing the author plus chapters in review; always 0 until agents exist.
    pub attention_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The minimum a card needs to describe a chapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChapterPointerDto {
    pub id: i32,
    /// The chapter's number within its track (= `ord`).
    pub number: i32,
    pub title: String,
    pub status: ChapterStatus,
}

/// `POST /books` body (spec/10 §2.2, spec/08 §5).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BookInput {
    pub title: String,
    #[serde(default)]
    pub idea: String,
    #[serde(default)]
    pub genre: String,
    /// BCP-47 tag. Required: the writing language decides prose style and word counting.
    pub writing_language: String,
    /// Starts the conception studio with the book in `conceiving` (spec/08 §5). The response
    /// carries the conception session's id when a conception model is configured; otherwise the
    /// book is still created and `warnings` reports `model.missing_role` (D-069).
    #[serde(default)]
    pub start_conception: bool,
}

/// `PATCH /books/:id` body. Omitted fields are unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BookPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writing_language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<BookSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_overrides: Option<RoleModels>,
    /// Only `archived` (hides the book) and `active` (unhides it) are accepted here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<BookStatus>,
}

/// `POST /books` response (spec/10 §2.2). `session_id` is the conception session a
/// `start_conception` book started (absent when no conception model was configured, in which
/// case `warnings` carries `model.missing_role`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct CreateBookResponse {
    pub book: BookDto,
    /// Always present (`null` when no session started): the generated TypeScript type declares
    /// `number | null`, and the New book dialog's landing decision tests it against `null`.
    pub session_id: Option<i32>,
    pub warnings: Vec<String>,
}

impl Default for CreateBookResponse {
    fn default() -> Self {
        Self {
            book: BookDto {
                id: 0,
                title: String::new(),
                idea: String::new(),
                genre: String::new(),
                writing_language: String::new(),
                status: BookStatus::Active,
                settings: BookSettings::default(),
                model_overrides: RoleModels::default(),
                conception: ConceptionState::default(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            session_id: None,
            warnings: Vec::new(),
        }
    }
}

/// `POST /books/:id/conception/finalize` response (spec/10 §2.2, spec/08 §5).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct ConceptionFinalizeResponse {
    pub book: BookDto,
    /// The `canon_seed` session finalize started, when an archivist model is configured; present
    /// as `null` otherwise, matching the generated `number | null` type.
    pub seed_session_id: Option<i32>,
    pub warnings: Vec<String>,
}

impl Default for ConceptionFinalizeResponse {
    fn default() -> Self {
        Self {
            book: CreateBookResponse::default().book,
            seed_session_id: None,
            warnings: Vec::new(),
        }
    }
}

/// `GET /books/:id/overview` (spec/08 §7). M1 fills `progress` and `recent_changes`; the next
/// action, the attention list and usage arrive with later milestones.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BookOverviewDto {
    pub book: BookDto,
    /// The single most useful next step (spec/08 §7). Always present for an existing book.
    pub next_action: Option<NextActionDto>,
    /// Everything that needs the author, in a stable order (spec/08 §7).
    pub attention: Vec<AttentionItemDto>,
    pub progress: BookProgressDto,
    pub recent_changes: Vec<super::changes::ChangeDto>,
    /// Token totals of the book's sessions (spec/08 §7 `usage`).
    pub usage: Usage,
}

/// The single next action of the overview (spec/08 §7, first match). The label is composed in the
/// UI from the kind and the data beside it (D-054).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NextActionKind {
    ContinueConception,
    SeedCanon,
    ReviewChapter,
    ContinueChapter,
    SyncCanon,
    WriteNext,
}

/// `next_action` (spec/08 §7). `chapter`, `session_id` and `track_id` are the targets the action
/// names; the DTO carries data only.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NextActionDto {
    pub kind: NextActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter: Option<ChapterPointerDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<i32>,
}

/// One kind of `attention[]` item (spec/08 §7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AttentionKind {
    OpenSession,
    ChapterInReview,
    CanonUnsynced,
    PendingProposals,
    ReadyResearch,
}

/// One entry of `attention[]` (spec/08 §7). Sessions, chapters in review and unsynced canon are
/// listed one per row (`count = 1`, with the target); pending proposals and ready research tasks
/// are one row carrying their count.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AttentionItemDto {
    pub kind: AttentionKind,
    pub count: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i32>,
    /// The open session's kind and status, so the UI composes its title and status label from
    /// data instead of fetching every session (D-054).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_kind: Option<crate::db::json::SessionKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_status: Option<crate::db::json::SessionStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter: Option<ChapterPointerDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<i32>,
}

/// Chapter counts and words per track (spec/08 §7).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BookProgressDto {
    pub total_chapters: i32,
    pub official_chapters: i32,
    pub total_words: i32,
    pub official_words: i32,
    pub tracks: Vec<TrackProgressDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TrackProgressDto {
    pub track_id: i32,
    pub kind: crate::db::json::TrackKind,
    /// Empty for the main track; the UI composes its name.
    pub title: String,
    pub chapters: i32,
    pub words: i32,
    /// One entry per chapter status, in the enum's order.
    pub by_status: Vec<StatusCountDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StatusCountDto {
    pub status: ChapterStatus,
    pub count: i32,
}
