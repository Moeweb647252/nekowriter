//! Setting proposal DTOs (spec/06 §5, spec/10 §2.3).
//!
//! A proposal stores the edits exactly as the agent created them; the DTO also carries a computed
//! preview (`before` / `after`), so the UI can show the target document with the change applied
//! without reproducing the find/replace logic.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::json::{ProposalEdits, ProposalStatus, ProposalTargetType, SettingDocKind};

/// The computed preview of a proposal (spec/10 §2.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[ts(export)]
pub struct ProposalPreviewDto {
    /// The target text as it is now (empty for a new character).
    pub before: String,
    /// The target text with the proposal applied, when it still applies; otherwise equal to
    /// `before` (the accept would turn into `conflict`).
    pub after: String,
}

/// One setting proposal (spec/10 §2.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProposalDto {
    pub id: i32,
    pub book_id: i32,
    pub target_type: ProposalTargetType,
    /// The document kind for a `setting_doc` target.
    pub target_kind: Option<SettingDocKind>,
    /// The character design for a `character_design` target.
    pub target_id: Option<i32>,
    pub edits: ProposalEdits,
    pub rationale: String,
    pub status: ProposalStatus,
    pub chapter_id: Option<i32>,
    pub session_id: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub preview: ProposalPreviewDto,
}

/// `GET /books/:id/proposals` query (spec/10 §2.3).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProposalQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ProposalStatus>,
}
