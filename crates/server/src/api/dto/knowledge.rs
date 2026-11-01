//! Knowledge-base DTOs (spec/07 §1, spec/10 §2.5).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::json::EntrySource;

/// A knowledge scope with its entry count (spec/10 §2.5).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ScopeDto {
    pub id: i32,
    pub name: String,
    pub description: String,
    /// `count(*)` crosses the wire as a JSON number (spec/10 §2.8 note on i64).
    #[ts(type = "number")]
    pub entry_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `POST /api/knowledge/scopes`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ScopeInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// `PATCH /api/knowledge/scopes/:id`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct ScopePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A knowledge entry in full (spec/07 §1).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EntryDto {
    pub id: i32,
    pub scope_id: i32,
    pub title: String,
    pub content: String,
    pub source: EntrySource,
    pub source_urls: Vec<String>,
    pub tags: Vec<String>,
    pub research_task_id: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One row of the entries list (the content is loaded on selection).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EntrySummaryDto {
    pub id: i32,
    pub scope_id: i32,
    pub title: String,
    pub source: EntrySource,
    pub tags: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

/// A page of entries (spec/10 §1: `{ items, next_cursor }`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EntryPageDto {
    pub items: Vec<EntrySummaryDto>,
    pub next_cursor: Option<String>,
}

/// `POST /api/knowledge/scopes/:id/entries`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EntryInput {
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub source_urls: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// `PATCH /api/knowledge/entries/:id`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct EntryPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_urls: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// `GET /api/knowledge/scopes/:id/entries?q&cursor&limit`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct EntryQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// One hybrid knowledge hit (spec/10 §2.5).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct KnowledgeHitDto {
    pub entry_id: i32,
    pub scope_id: i32,
    pub scope_name: String,
    pub title: String,
    pub snippet: String,
}

/// `GET /api/knowledge/search?q&scope_ids=1,2&limit`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct KnowledgeSearchQuery {
    #[serde(default)]
    pub q: Option<String>,
    /// Comma-separated scope ids; absent searches every scope.
    #[serde(default)]
    pub scope_ids: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// `PUT /api/books/:id/scopes`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BookScopesInput {
    /// Scope ids the book unchecks; every other scope stays visible (spec/10 §2.5, D-158).
    pub excluded_scope_ids: Vec<i32>,
}
