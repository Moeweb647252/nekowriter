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
    // Read by both adapters (`reasoning_replay` selects the replayed assistant payload,
    // `cache_ttl` becomes `CacheHints.ttl`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_replay: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_ttl: Option<String>,
    // OpenAI-compatible only.
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
    // Anthropic only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal_fallback: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eager_input_streaming: Option<bool>,
}

impl From<crate::db::json::ModelOptions> for ModelOptionsDto {
    fn from(value: crate::db::json::ModelOptions) -> Self {
        Self {
            reasoning_replay: value.reasoning_replay,
            cache_ttl: value.cache_ttl,
            reasoning_effort: value.reasoning_effort,
            max_tokens_field: value.max_tokens_field,
            send_parallel_tool_calls: value.send_parallel_tool_calls,
            stream_usage: value.stream_usage,
            temperature: value.temperature,
            extra_body: value.extra_body,
            effort: value.effort,
            thinking_display: value.thinking_display,
            refusal_fallback: value.refusal_fallback,
            eager_input_streaming: value.eager_input_streaming,
        }
    }
}

impl From<ModelOptionsDto> for crate::db::json::ModelOptions {
    fn from(value: ModelOptionsDto) -> Self {
        Self {
            reasoning_replay: value.reasoning_replay,
            cache_ttl: value.cache_ttl,
            reasoning_effort: value.reasoning_effort,
            max_tokens_field: value.max_tokens_field,
            send_parallel_tool_calls: value.send_parallel_tool_calls,
            stream_usage: value.stream_usage,
            temperature: value.temperature,
            extra_body: value.extra_body,
            effort: value.effort,
            thinking_display: value.thinking_display,
            refusal_fallback: value.refusal_fallback,
            eager_input_streaming: value.eager_input_streaming,
        }
    }
}

impl ModelOptionsDto {
    /// The neutral options these fields describe, or `None` when a value is not one the adapters
    /// accept (spec/04 §5). The write path rejects such a blob, so `None` only happens for a row
    /// written before that rule.
    pub fn to_model_options(&self) -> Option<nekowriter_llm::ModelOptions> {
        let value = serde_json::to_value(self).ok()?;
        serde_json::from_value(value).ok()
    }
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
#[ts(optional_fields = nullable)]
pub struct ModelProfileDto {
    pub id: i32,
    pub provider_id: i32,
    pub display_name: String,
    /// The provider's model id.
    pub model: String,
    pub context_window: i32,
    pub max_output_tokens: i32,
    pub options: ModelOptionsDto,
    /// Options this profile sets that its provider's protocol cannot send (spec/04 §5, D-132).
    ///
    /// A **stored** mismatch is reported here instead of failing the read: the author's database
    /// already holds one (an Anthropic `effort` on an OpenAI-protocol provider), and failing the
    /// read would lock them out of the page that has to fix it. Writing such a profile is refused.
    pub ignored_options: Vec<String>,
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

/// `GET /api/provider-presets/options` (spec/04 §5, spec/10 §2.1): the option table the settings
/// page renders.
///
/// The classification lives in `crates/llm` next to the adapters and is verified against them by
/// `crates/llm/tests/model_options.rs`, so the UI cannot drift from what the wire can carry. It is
/// served instead of hard-coded in TypeScript for exactly that reason: a client-side copy would be
/// a second source of truth for the rule this bug report is about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelOptionSpecDto {
    /// The JSON field name in `model_profiles.options`.
    pub key: String,
    /// The protocols whose adapter sends this option at all.
    pub protocols: Vec<Protocol>,
    pub kind: ModelOptionKindDto,
    /// The closed set of values, or empty when the option is free-form.
    pub values: Vec<String>,
    /// The value an unset option behaves as, when `ModelOptions::default` gives it one; `None` for
    /// an option that is simply absent unless set (spec/04 §5).
    pub default_value: Option<String>,
}

/// How the UI edits an option (spec/04 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ModelOptionKindDto {
    Choice,
    Boolean,
    Number,
    Text,
    Json,
}

impl From<nekowriter_llm::ModelOptionKind> for ModelOptionKindDto {
    fn from(kind: nekowriter_llm::ModelOptionKind) -> Self {
        match kind {
            nekowriter_llm::ModelOptionKind::Choice => ModelOptionKindDto::Choice,
            nekowriter_llm::ModelOptionKind::Boolean => ModelOptionKindDto::Boolean,
            nekowriter_llm::ModelOptionKind::Number => ModelOptionKindDto::Number,
            nekowriter_llm::ModelOptionKind::Text => ModelOptionKindDto::Text,
            nekowriter_llm::ModelOptionKind::Json => ModelOptionKindDto::Json,
        }
    }
}
