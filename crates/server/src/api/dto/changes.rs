//! Change-log DTOs (spec/10 §2.4, spec/06 §3).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::json::{ChangeActor, ChangeOp, ChangeTargetType, Snapshot};

/// One `change_log` row.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChangeDto {
    pub id: i32,
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub chapter_id: Option<i32>,
    pub session_id: Option<i32>,
    pub actor: ChangeActor,
    pub target_type: ChangeTargetType,
    pub target_id: i32,
    pub op: ChangeOp,
    pub before: Option<Snapshot>,
    pub after: Option<Snapshot>,
    /// Written text only: the author's change note, an agent's note, a proposal's rationale, or
    /// the merge form `#B → #A`. Never generated UI wording (spec/06 §3, DECISIONS D-032).
    pub note: String,
    pub reverts_id: Option<i32>,
    pub reverted_by_id: Option<i32>,
    pub created_at: DateTime<Utc>,
}

/// `{ items, next_cursor }` for the append-mostly, id-ordered lists (spec/10 §1).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChangePageDto {
    pub items: Vec<ChangeDto>,
    pub next_cursor: Option<String>,
}

/// `GET /books/:id/changes` query (spec/10 §2.4).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChangeQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_type: Option<ChangeTargetType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ChangeActor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
}
