//! Provider and model-profile endpoints (spec/10 §2.1).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use crate::api::dto::providers::{
    ModelProfileDto, ModelProfileInput, ProviderDto, ProviderInput, ProviderPresetDto,
    ProviderTestDto, RemoteModelDto,
};
use crate::error::AppResult;
use crate::services::providers as service;
use crate::state::AppState;

/// `GET /api/providers`.
pub async fn list(State(state): State<AppState>) -> AppResult<Json<Vec<ProviderDto>>> {
    Ok(Json(service::list(state.db().conn()).await?))
}

/// `POST /api/providers`.
pub async fn create(
    State(state): State<AppState>,
    Json(input): Json<ProviderInput>,
) -> AppResult<(StatusCode, Json<ProviderDto>)> {
    let dto = service::create(&state, &input).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// `GET /api/providers/:id`.
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> AppResult<Json<ProviderDto>> {
    Ok(Json(service::get(state.db().conn(), id).await?))
}

/// `PATCH /api/providers/:id`.
pub async fn patch(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(input): Json<ProviderInput>,
) -> AppResult<Json<ProviderDto>> {
    Ok(Json(service::update(&state, id, &input).await?))
}

/// `DELETE /api/providers/:id`.
pub async fn delete(State(state): State<AppState>, Path(id): Path<i32>) -> AppResult<StatusCode> {
    service::delete(&state, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/providers/:id/test` body.
#[derive(Debug, serde::Deserialize)]
pub struct ProviderTestInput {
    #[serde(default)]
    pub model: Option<String>,
}

/// `POST /api/providers/:id/test` (spec/04 §8).
pub async fn test(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    body: Option<Json<ProviderTestInput>>,
) -> AppResult<Json<ProviderTestDto>> {
    let model = body.and_then(|Json(body)| body.model);
    Ok(Json(service::test_connection(&state, id, model).await?))
}

/// `GET /api/providers/:id/remote-models` (spec/04 §8).
pub async fn remote_models(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> AppResult<Json<Vec<RemoteModelDto>>> {
    Ok(Json(service::remote_models(&state, id).await?))
}

/// `GET /api/provider-presets` (spec/04 §8; the Demo preset only when enabled).
pub async fn presets(State(state): State<AppState>) -> Json<Vec<ProviderPresetDto>> {
    Json(service::presets(state.config().demo_provider_enabled()))
}

/// `GET /api/model-profiles`.
pub async fn list_models(State(state): State<AppState>) -> AppResult<Json<Vec<ModelProfileDto>>> {
    Ok(Json(service::list_models(state.db().conn()).await?))
}

/// `POST /api/model-profiles`.
pub async fn create_model(
    State(state): State<AppState>,
    Json(input): Json<ModelProfileInput>,
) -> AppResult<(StatusCode, Json<ModelProfileDto>)> {
    let dto = service::create_model(&state, &input).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// `GET /api/model-profiles/:id`.
pub async fn get_model(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> AppResult<Json<ModelProfileDto>> {
    Ok(Json(service::get_model(state.db().conn(), id).await?))
}

/// `PATCH /api/model-profiles/:id`.
pub async fn patch_model(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(input): Json<ModelProfileInput>,
) -> AppResult<Json<ModelProfileDto>> {
    Ok(Json(service::update_model(&state, id, &input).await?))
}

/// `DELETE /api/model-profiles/:id`.
pub async fn delete_model(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> AppResult<StatusCode> {
    service::delete_model(&state, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
