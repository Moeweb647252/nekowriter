//! Research task DTOs (spec/02 §2, spec/10 §2.5).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::json::ResearchTaskStatus;

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
    pub requirements: String,
    pub language: String,
    pub status: ResearchTaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i32>,
    pub report: String,
    pub entries_created: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
