//! Writing-skill DTOs (spec/10 §2.9, design §8).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One skill row of the catalogue.
///
/// `selected` and `used_by_books` are per-view fields: `selected` against the query's `book_id`,
/// `used_by_books` is the deletion-confirmation list the UI shows before a user row is deleted
/// (design §4: deleting a user skill cascades away every book's selection of it).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WritingSkillDto {
    pub id: i32,
    /// Built-in only; `null` for user rows.
    pub key: Option<String>,
    pub name: String,
    pub category: String,
    /// `builtin` | `user`.
    pub author: String,
    pub description: String,
    pub body: String,
    pub enabled_by_default: bool,
    pub retired: bool,
    /// True when the `book_id` of the request has selected this skill.
    pub selected: bool,
    /// The books whose selection includes this skill (delete confirmation).
    pub used_by_books: Vec<WritingSkillBookDto>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One book that uses a skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WritingSkillBookDto {
    pub id: i32,
    pub title: String,
}

/// `POST /api/writing-skills` body. `author` is forced to `user` and `key` stays `null`
/// (design §8); built-in fields are never settable.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WritingSkillInput {
    pub name: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub body: String,
}

/// `PATCH /api/writing-skills/:id` body. Omitted fields are unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WritingSkillPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// `PUT /api/books/:id/writing-skills` body: the book's complete selection.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BookWritingSkillsInput {
    #[serde(default)]
    pub skill_ids: Vec<i32>,
}

/// `DELETE /api/writing-skills/:id` response: the books the deleted skill was selected by
/// (design §4/§8 — deleting is allowed only for user rows and must name its users).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WritingSkillDeleteResponse {
    pub used_by_books: Vec<WritingSkillBookDto>,
}
