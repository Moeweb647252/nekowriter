//! The embedding settings and their connection test (`docs/product-spec.md` §12.2, §13.3).

use std::time::Instant;

use axum::extract::State;
use nekowriter_agent::model_text::model_text;
use nekowriter_core::{
    embedding::EmbeddingModel,
    field::FieldError,
    ids::CommandId,
    model::{ApiBaseUrl, ApiKey, ModelId, ModelProblem, ModelService},
    usage::TokenUsage,
};
use nekowriter_store::{ApiKeyChange, EmbeddingSettingsInput, EmbeddingSettingsView};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::http::{
    AppState,
    error::{ApiError, ApiProblem},
    extract::Json,
    handlers::settings::ApiKeyInput,
};

/// An embedding model as a request carries it; [`EmbeddingModelInput::parse`] validates
/// it, so an invalid value is reported for its field.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingModelInput {
    model_id: String,
    dimensions: u32,
}

impl EmbeddingModelInput {
    fn parse(self) -> Result<EmbeddingModel, FieldError> {
        let model = EmbeddingModel {
            model_id: ModelId::parse(&self.model_id)?,
            dimensions: self.dimensions,
        };
        model.validate()?;
        Ok(model)
    }
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveEmbeddingRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    pub api_base_url: Option<String>,
    /// Omitted keeps the stored key, `null` clears it, a string replaces it.
    #[serde(default)]
    #[ts(optional = nullable, as = "Option<String>")]
    pub api_key: ApiKeyInput,
    #[ts(as = "Option<EmbeddingModel>")]
    pub model: Option<EmbeddingModelInput>,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TestEmbeddingRequest {
    /// The API root to test; the stored one when omitted.
    #[ts(optional)]
    pub api_base_url: Option<String>,
    /// The key to test: omitted uses the stored key, `null` tests without a key.
    #[serde(default)]
    #[ts(optional = nullable, as = "Option<String>")]
    pub api_key: ApiKeyInput,
    #[ts(as = "EmbeddingModel")]
    pub model: EmbeddingModelInput,
}

/// An embedding test that succeeded: the vector had the declared size and finite values.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EmbeddingTestView {
    pub elapsed_millis: u32,
    pub dimensions: u32,
}

pub(crate) async fn get(
    State(state): State<AppState>,
) -> Result<axum::Json<EmbeddingSettingsView>, ApiError> {
    Ok(axum::Json(state.store.embedding_settings().await?))
}

pub(crate) async fn save(
    State(state): State<AppState>,
    Json(request): Json<SaveEmbeddingRequest>,
) -> Result<axum::Json<EmbeddingSettingsView>, ApiError> {
    let input = EmbeddingSettingsInput {
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
        model: request.model.map(EmbeddingModelInput::parse).transpose()?,
    };
    Ok(axum::Json(
        state
            .store
            .save_embedding_settings(request.command_id, request.expected_revision, input)
            .await?,
    ))
}

/// Embeds one short text with the model under test and checks the vector (§13.3).
pub(crate) async fn test(
    State(state): State<AppState>,
    Json(request): Json<TestEmbeddingRequest>,
) -> Result<axum::Json<EmbeddingTestView>, ApiError> {
    let model = request.model.parse()?;
    let stored = state.store.embedding_endpoint().await?;
    let base_url = match request.api_base_url {
        Some(url) => ApiBaseUrl::parse(&url)?,
        None => stored
            .as_ref()
            .map(|stored| stored.base_url.clone())
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
        ApiKeyInput::Omitted => stored.and_then(|stored| stored.api_key),
        ApiKeyInput::Null => None,
        ApiKeyInput::Key(key) => Some(ApiKey::parse(&key)?),
    };
    let service = ModelService { base_url, api_key };
    let texts = [model_text().connection_test.embedding.render(&[])];
    let started = Instant::now();
    let result = tokio::select! {
        result = state.client.embed(&service, &model, &texts) => result,
        () = state.shutdown.triggered() => {
            return Err(ApiError::new(ApiProblem::ServiceStopping, None));
        }
    };
    // Embeddings have no output tokens; the dashboard counts the input (§15).
    let reported = result
        .as_ref()
        .ok()
        .and_then(|embeddings| embeddings.input_tokens)
        .map(|input| TokenUsage {
            input_tokens: Some(input),
            output_tokens: Some(0),
            cached_input_tokens: None,
            reasoning_tokens: None,
        });
    state
        .store
        .record_connection_test(result.is_ok(), reported)
        .await?;
    result.map_err(|error| {
        ApiError::new(
            ApiProblem::EmbeddingFailed {
                failure: error.failure(),
            },
            error.to_string(),
        )
    })?;
    Ok(axum::Json(EmbeddingTestView {
        elapsed_millis: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
        dimensions: model.dimensions,
    }))
}
