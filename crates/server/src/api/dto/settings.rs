//! Change-log and app-settings DTOs (spec/10 §2.1, §2.4, spec/06 §3).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::json::{BookSettings, RoleModels};

/// `GET /app-settings` (spec/10 §2.1). Secrets are masked.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AppSettingsDto {
    pub roles_defaults: RoleModels,
    /// The settings new books start with (spec/02 §4.1).
    pub book_defaults: BookSettings,
    /// The web search configuration; `api_key` is masked like a provider key.
    pub search: SearchSettingsDto,
    pub embedding: EmbeddingSettingsDto,
}

/// `app_settings["search"]` (spec/02 §4.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "snake_case")]
#[ts(export)]
pub struct SearchSettingsDto {
    pub provider: SearchProvider,
    /// Masked in responses; an absent field keeps the stored key (spec/10 §2.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tavily_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub searxng_base_url: Option<String>,
    pub max_results: i32,
}

impl Default for SearchSettingsDto {
    fn default() -> Self {
        Self {
            provider: SearchProvider::None,
            tavily_api_key: None,
            searxng_base_url: None,
            max_results: 6,
        }
    }
}

/// Web search providers (spec/02 §4.1, spec/07 §6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SearchProvider {
    #[default]
    None,
    Tavily,
    Searxng,
}

/// `app_settings["embedding"]` (spec/02 §4.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "snake_case")]
#[ts(export)]
pub struct EmbeddingSettingsDto {
    /// One of the keys in `crates/embed/models.toml`.
    pub model: String,
}

impl Default for EmbeddingSettingsDto {
    fn default() -> Self {
        Self {
            model: "multilingual-e5-small".to_owned(),
        }
    }
}

/// `PATCH /app-settings` body. Omitted sections are unchanged (spec/10 §2.1).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct AppSettingsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles_defaults: Option<RoleModels>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_defaults: Option<BookSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<SearchSettingsPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<EmbeddingSettingsDto>,
}

/// The patchable part of `app_settings["search"]`. `tavily_api_key: null` clears the key.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct SearchSettingsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<SearchProvider>,
    /// Masked values are detected and ignored, so a round-tripped masked key cannot erase it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tavily_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub searxng_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
}
