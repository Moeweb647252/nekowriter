//! Prompt registry endpoints (spec/10 §2.6).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;

use crate::api::dto::prompts::{PromptEntryDto, PromptResetQuery, PromptSaveInput};
use crate::error::AppResult;
use crate::services::prompts as service;
use crate::state::AppState;

/// Query of `GET /api/prompts`.
#[derive(Debug, serde::Deserialize)]
pub struct PromptListQuery {
    #[serde(default)]
    pub book_id: Option<i32>,
}

/// `GET /api/prompts?book_id=` — every entry with its overrides and the effective source.
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<PromptListQuery>,
) -> AppResult<Json<Vec<PromptEntryDto>>> {
    Ok(Json(service::list(&state, query.book_id).await?))
}

/// `PUT /api/prompts/:key` — validate and save an override (400 `prompt.template_invalid`).
pub async fn save(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(input): Json<PromptSaveInput>,
) -> AppResult<Json<PromptEntryDto>> {
    Ok(Json(service::save(&state, &key, &input).await?))
}

/// `DELETE /api/prompts/:key?scope=&book_id=` — reset one level.
pub async fn reset(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(query): Query<PromptResetQuery>,
) -> AppResult<(StatusCode, Json<PromptEntryDto>)> {
    let dto = service::reset(&state, &key, query.scope, query.book_id).await?;
    Ok((StatusCode::OK, Json(dto)))
}
