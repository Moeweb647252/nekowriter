//! Research task DTOs (spec/02 §2, spec/10 §2.5).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::json::{ResearchTaskKind, ResearchTaskOrigin, ResearchTaskStatus};

/// One entry a research task created (spec/10 §2.5: the task carries "the created entries when
/// done", which the task page lists).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResearchTaskEntryDto {
    pub id: i32,
    pub title: String,
}

/// A research task row (spec/10 §2.5).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResearchTaskDto {
    pub id: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<i32>,
    pub scope_name: String,
    pub topic: String,
    pub kind: ResearchTaskKind,
    pub origin: ResearchTaskOrigin,
    pub requirements: String,
    pub language: String,
    pub status: ResearchTaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i32>,
    pub report: String,
    pub entries_created: i32,
    /// The entries this task created, in id order; filled by the single-task endpoints when the
    /// task is `done` (spec/10 §2.5). Events and the list leave it empty to stay cheap: the UI
    /// refetches the task on `research.updated`.
    #[serde(default)]
    pub entries: Vec<ResearchTaskEntryDto>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `POST /api/books/:id/knowledge-check` (spec/10 §2.2, D-158).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct KnowledgeCheckInput {
    #[serde(default)]
    pub requirements: String,
}

/// The knowledge-check response: the preparation task and the research session it started.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct KnowledgeCheckResponse {
    pub task: ResearchTaskDto,
    pub session_id: i32,
}

/// `POST /api/research-tasks` (spec/10 §2.5).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct ResearchTaskCreate {
    #[serde(default)]
    pub book_id: Option<i32>,
    pub scope_name: String,
    pub topic: String,
    #[serde(default)]
    pub requirements: String,
    /// Required without a book, ignored with one (spec/10 §2.5).
    #[serde(default)]
    pub language: Option<String>,
}

/// `PATCH /api/research-tasks/:id` (spec/10 §2.5).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct ResearchTaskPatch {
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub requirements: Option<String>,
    #[serde(default)]
    pub scope_name: Option<String>,
    /// Only `ready` is accepted: it confirms a draft (spec/10 §2.5).
    #[serde(default)]
    pub status: Option<ResearchTaskStatus>,
}

/// `GET /api/research-tasks` filters (spec/10 §2.5).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct ResearchTaskQuery {
    #[serde(default)]
    pub book_id: Option<i32>,
    #[serde(default)]
    pub status: Option<ResearchTaskStatus>,
}

/// The response of `POST /api/research-tasks/:id/run` (spec/10 §2.5).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResearchRunResponse {
    pub session_id: i32,
}
