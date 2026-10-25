//! Web-search DTOs (spec/07 §6, spec/10 §2.1).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One search hit (spec/07 §6). `published` is whatever the provider reported, unparsed.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchHitDto {
    pub title: String,
    pub url: String,
    pub snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
}

/// `POST /api/app-settings/search/test` (spec/10 §2.1).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchTestResponse {
    pub hits: Vec<SearchHitDto>,
}

/// The body of the search test.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchTestInput {
    pub query: String,
}
