//! Application settings endpoints (spec/10 §2.1).

use axum::Json;
use axum::extract::State;

use crate::api::dto::settings::{AppSettingsDto, AppSettingsPatch};
use crate::error::{AppError, AppResult};
use crate::services::app_settings as service;
use crate::state::AppState;

/// `GET /api/app-settings`.
pub async fn get(State(state): State<AppState>) -> AppResult<Json<AppSettingsDto>> {
    Ok(Json(service::get(state.db().conn()).await?))
}

/// `PATCH /api/app-settings`.
pub async fn patch(
    State(state): State<AppState>,
    Json(patch): Json<AppSettingsPatch>,
) -> AppResult<Json<AppSettingsDto>> {
    let before = service::get(state.db().conn()).await?.embedding.model;
    let dto = crate::services::write(state.db(), move |tx| {
        Box::pin(async move { service::update(tx, &patch).await })
    })
    .await
    .map_err(|error| AppError {
        code: error.code,
        message: error.message,
        details: error.details,
    })?;
    if before != dto.embedding.model {
        // A model change recreates the vec tables for the new dimension and marks every document
        // pending (spec/07 §3).
        crate::search::switch_model(&state, &dto.embedding.model).await?;
    }
    Ok(Json(dto))
}
