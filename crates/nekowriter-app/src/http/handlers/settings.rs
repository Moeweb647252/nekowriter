//! System settings and the model connection test (`docs/product-spec.md` §13.3).

use std::time::Instant;

use axum::extract::State;
use nekowriter_agent::client::connection_test_body;
use nekowriter_core::{
    field::FieldError,
    ids::CommandId,
    model::{
        ApiBaseUrl, ApiKey, EffectiveModel, ModelId, ModelProblem, ModelProfile, ModelService,
        ThinkingLevel,
    },
};
use nekowriter_store::{ApiKeyChange, SystemSettingsInput, SystemSettingsView};
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

use crate::http::{
    AppState,
    error::{ApiError, ApiProblem},
    extract::Json,
};

/// A model profile as a request carries it. [`ModelProfileInput::parse`] validates it, so
/// an invalid value is reported for its field.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfileInput {
    model_id: String,
    context_window: u32,
    max_output_tokens: u32,
    supports_thinking: bool,
    reports_stream_usage: bool,
}

impl ModelProfileInput {
    pub(crate) fn parse(self) -> Result<ModelProfile, FieldError> {
        let profile = ModelProfile {
            model_id: ModelId::parse(&self.model_id)?,
            context_window: self.context_window,
            max_output_tokens: self.max_output_tokens,
            supports_thinking: self.supports_thinking,
            reports_stream_usage: self.reports_stream_usage,
        };
        profile.validate()?;
        Ok(profile)
    }
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveSettingsRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    pub api_base_url: Option<String>,
    /// Omitted keeps the stored key, `null` clears it, a string replaces it.
    #[serde(default)]
    #[ts(optional = nullable, as = "Option<String>")]
    pub api_key: ApiKeyInput,
    #[ts(as = "Option<ModelProfile>")]
    pub default_model: Option<ModelProfileInput>,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TestModelRequest {
    /// The API root to test; the stored one when omitted.
    #[ts(optional)]
    pub api_base_url: Option<String>,
    /// The key to test: omitted uses the stored key, `null` tests without a key.
    #[serde(default)]
    #[ts(optional = nullable, as = "Option<String>")]
    pub api_key: ApiKeyInput,
    #[ts(as = "ModelProfile")]
    pub model: ModelProfileInput,
}

/// A connection test that succeeded.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModelTestView {
    /// How long the complete answer took.
    pub elapsed_millis: u32,
    /// Whether the service reported token usage, as the profile may declare.
    pub usage_reported: bool,
}

/// An API key field that tells an omitted value from `null`: an omitted field takes the
/// default through `#[serde(default)]`, while `null` and strings go through
/// `Deserialize`.
#[derive(Clone, Debug, Default)]
pub enum ApiKeyInput {
    #[default]
    Omitted,
    Null,
    Key(String),
}

impl<'de> Deserialize<'de> for ApiKeyInput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<String>::deserialize(deserializer)? {
            None => Self::Null,
            Some(key) => Self::Key(key),
        })
    }
}

pub(crate) async fn get(
    State(state): State<AppState>,
) -> Result<axum::Json<SystemSettingsView>, ApiError> {
    Ok(axum::Json(state.store.system_settings().await?))
}

pub(crate) async fn save(
    State(state): State<AppState>,
    Json(request): Json<SaveSettingsRequest>,
) -> Result<axum::Json<SystemSettingsView>, ApiError> {
    let input = SystemSettingsInput {
        api_base_url: request
            .api_base_url
            .as_deref()
            .map(ApiBaseUrl::parse)
            .transpose()?,
        api_key: match request.api_key {
            ApiKeyInput::Omitted => ApiKeyChange::Keep,
            ApiKeyInput::Null => ApiKeyChange::Clear,
            ApiKeyInput::Key(key) => ApiKeyChange::Replace(ApiKey::parse(&key)?),
        },
        default_model: request
            .default_model
            .map(ModelProfileInput::parse)
            .transpose()?,
    };
    let view = state
        .store
        .save_system_settings(request.command_id, request.expected_revision, input)
        .await?;
    Ok(axum::Json(view))
}

/// Sends one small real request with the profile under test (§13.3). A profile that
/// declares thinking support is tested with thinking on.
pub(crate) async fn test_model(
    State(state): State<AppState>,
    Json(request): Json<TestModelRequest>,
) -> Result<axum::Json<ModelTestView>, ApiError> {
    let profile = request.model.parse()?;
    let stored = state.store.model_service().await?;
    let base_url = match request.api_base_url {
        Some(url) => ApiBaseUrl::parse(&url)?,
        None => stored
            .as_ref()
            .map(|service| service.base_url.clone())
            .ok_or_else(|| {
                ApiError::new(
                    ApiProblem::ModelNotReady {
                        problem: ModelProblem::ServiceNotConfigured,
                    },
                    None,
                )
            })?,
    };
    let api_key = match request.api_key {
        ApiKeyInput::Omitted => stored.and_then(|service| service.api_key),
        ApiKeyInput::Null => None,
        ApiKeyInput::Key(key) => Some(ApiKey::parse(&key)?),
    };
    let thinking = if profile.supports_thinking {
        ThinkingLevel::Low
    } else {
        ThinkingLevel::Off
    };
    let model = EffectiveModel {
        service: ModelService { base_url, api_key },
        profile,
        thinking,
    };
    let body = connection_test_body(&model);
    let started = Instant::now();
    let completion = tokio::select! {
        result = state.client.complete(&model.service, &body, |_| {}) => result,
        () = state.shutdown.triggered() => {
            return Err(ApiError::new(ApiProblem::ServiceStopping, None));
        }
    }
    .map_err(|error| {
        ApiError::new(
            ApiProblem::ModelServiceFailed {
                failure: error.failure(),
            },
            error.to_string(),
        )
    })?;
    Ok(axum::Json(ModelTestView {
        elapsed_millis: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
        usage_reported: completion.raw_usage.is_some(),
    }))
}
