//! Setting material DTOs (spec/10 §2.3).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::json::SettingDocKind;

/// One of the book's three setting documents.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SettingDocDto {
    pub id: i32,
    pub book_id: i32,
    pub kind: SettingDocKind,
    pub content: String,
    pub revision: i32,
    pub approved_revision: Option<i32>,
    pub updated_at: DateTime<Utc>,
}

/// `PUT /books/:id/setting-docs/:kind` body (spec/10 §2.3).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SettingDocInput {
    pub content: String,
    pub base_revision: i32,
    /// The author's optional change note (spec/06 §3).
    #[serde(default)]
    pub note: String,
}

/// A character design (spec/02 §2 `character_designs`, spec/10 §2.3).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CharacterDto {
    pub id: i32,
    pub book_id: i32,
    /// 1 = most important; contiguous `1..n`.
    pub rank: i32,
    pub name: String,
    /// Author-written stored text (e.g. a role word); the one label that is not rendered by the
    /// prompt pack (D-055).
    pub role_label: String,
    pub summary: String,
    pub profile: String,
    pub entity_id: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `POST /books/:id/characters` body (spec/10 §2.3).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CharacterInput {
    pub name: String,
    #[serde(default)]
    pub role_label: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub note: String,
}

/// `PATCH /characters/:id` body. Omitted fields are unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CharacterPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// The link to a canon entity, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<i32>,
    #[serde(default)]
    pub note: String,
}

/// `POST /books/:id/characters/reorder` body: every character id, most important first.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CharacterReorderInput {
    pub ids: Vec<i32>,
}
