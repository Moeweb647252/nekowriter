//! Prompt registry DTOs (spec/10 §2.6).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::json::PromptScope;
use crate::prompts::PromptKind;
use crate::prompts::PromptSource;

/// One entry of `GET /api/prompts` (spec/09 §4): metadata, the shipped default, both override
/// levels, the effective source and the variable docs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct PromptEntryDto {
    pub key: String,
    pub kind: PromptKind,
    /// The owning role (`conception`, `researcher`, `writer`, `reviewer`, `archivist`, `shared`).
    pub agent: String,
    /// UI titles from the pack; the client picks by its language (spec/09 §1).
    pub title_en: String,
    pub title_zh: String,
    /// Top-level variables the template may use (spec/09 §2).
    pub vars: Vec<String>,
    /// The shipped default template.
    pub default_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_override: Option<String>,
    /// Which level the effective template comes from.
    pub source: PromptSource,
    /// Documentation of every declared variable, shown beside the editor (spec/09 §1).
    pub variables: Vec<PromptVariableDocDto>,
}

/// Documentation of one template variable (spec/09 §1, `prompts/variables.toml`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PromptVariableDocDto {
    pub name: String,
    pub en: String,
    pub zh: String,
    pub fields: Vec<PromptVariableFieldDocDto>,
}

/// Documentation of one field of a template variable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PromptVariableFieldDocDto {
    pub name: String,
    pub en: String,
    pub zh: String,
}

/// `PUT /api/prompts/:key` body (spec/10 §2.6).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct PromptSaveInput {
    pub scope: PromptScope,
    /// Required for `scope: "book"` and ignored for `scope: "global"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_id: Option<i32>,
    pub content: String,
}

/// `DELETE /api/prompts/:key` query (spec/10 §2.6).
#[derive(Debug, Clone, Deserialize)]
pub struct PromptResetQuery {
    pub scope: PromptScope,
    #[serde(default)]
    pub book_id: Option<i32>,
}
