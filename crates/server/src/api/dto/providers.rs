//! Provider, model-profile and preset DTOs (spec/10 §2.1, spec/04 §8).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::json::Protocol;
use crate::error::ErrorCode;
use crate::services::providers::PresetKey;

/// `model_profiles.options` (spec/04 §5) as the API carries it. Every field is optional so the UI
/// can send only what it changed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "snake_case")]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct ModelOptionsDto {
    // OpenAI-compatible
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_replay: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_usage: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<BTreeMap<String, serde_json::Value>>,
    // Anthropic
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_ttl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal_fallback: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eager_input_streaming: Option<bool>,
}

/// One provider (spec/10 §2.1). The key is never returned in clear text.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct ProviderDto {
    pub id: i32,
    pub name: String,
    pub protocol: Protocol,
    pub base_url: String,
    /// `…abcd` for a stored literal key, `env:NAME` verbatim for an environment key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_masked: Option<String>,
    pub has_api_key: bool,
    pub extra_headers: BTreeMap<String, String>,
    /// The provider's model profiles, so the settings page needs one request.
    pub models: Vec<ModelProfileDto>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `POST /api/providers` and `PATCH /api/providers/:id` (spec/10 §2.1). An omitted `api_key`
/// keeps the stored key.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct ProviderInput {
    pub name: String,
    pub protocol: Protocol,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
}

/// One model profile (spec/10 §2.1, spec/02 §2).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelProfileDto {
    pub id: i32,
    pub provider_id: i32,
    pub display_name: String,
    /// The provider's model id.
    pub model: String,
    pub context_window: i32,
    pub max_output_tokens: i32,
    pub options: ModelOptionsDto,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `POST /api/model-profiles` and `PATCH /api/model-profiles/:id`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelProfileInput {
    pub provider_id: i32,
    pub display_name: String,
    pub model: String,
    pub context_window: i32,
    pub max_output_tokens: i32,
    #[serde(default)]
    pub options: ModelOptionsDto,
}

/// `GET /api/provider-presets` (spec/04 §8). The values are editable defaults.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProviderPresetDto {
    pub key: PresetKey,
    pub name: String,
    pub protocol: Protocol,
    pub base_url: String,
    /// Suggested model ids, in preference order.
    pub suggested_models: Vec<String>,
    pub context_window: i32,
    pub max_output_tokens: i32,
    pub options: ModelOptionsDto,
    /// True when the provider needs an API key (Ollama and LM Studio do not).
    pub requires_key: bool,
}

/// `POST /api/providers/:id/test` response (spec/10 §2.1, spec/04 §8).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct ProviderTestDto {
    pub ok: bool,
    pub latency_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProviderTestErrorDto>,
}

/// The classified failure of a connection test (spec/10 §4).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProviderTestErrorDto {
    /// `provider.auth`, `provider.bad_request` or `provider.unavailable`.
    pub code: ErrorCode,
    pub message: String,
}

/// `GET /api/providers/:id/remote-models` (spec/04 §8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct RemoteModelDto {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i64>,
}
