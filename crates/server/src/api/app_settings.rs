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
    let dto = crate::services::write(state.db(), move |tx| {
        Box::pin(async move { service::update(tx, &patch).await })
    })
    .await
    .map_err(|error| AppError {
        code: error.code,
        message: error.message,
        details: error.details,
    })?;
    Ok(Json(dto))
}
