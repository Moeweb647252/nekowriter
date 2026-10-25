//! Research task endpoints (spec/10 §2.5).
//!
//! Handlers stay thin: they parse the path and body and return the service DTO.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;

use crate::api::dto::research::{
    ResearchRunResponse, ResearchTaskCreate, ResearchTaskDto, ResearchTaskPatch, ResearchTaskQuery,
};
use crate::error::AppResult;
use crate::services::research;
use crate::state::AppState;

/// `GET /api/research-tasks`.
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ResearchTaskQuery>,
) -> AppResult<Json<Vec<ResearchTaskDto>>> {
    Ok(Json(research::list(state.db_conn(), query).await?))
}

/// `POST /api/research-tasks`.
pub async fn create(
    State(state): State<AppState>,
    Json(input): Json<ResearchTaskCreate>,
) -> AppResult<(StatusCode, Json<ResearchTaskDto>)> {
    let (dto, _warnings) = research::create(&state, input).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// `GET /api/research-tasks/:id`.
pub async fn get(
    State(state): State<AppState>,
    Path(task_id): Path<i32>,
) -> AppResult<Json<ResearchTaskDto>> {
    Ok(Json(research::get(state.db_conn(), task_id).await?))
}

/// `PATCH /api/research-tasks/:id`.
pub async fn patch(
    State(state): State<AppState>,
    Path(task_id): Path<i32>,
    Json(input): Json<ResearchTaskPatch>,
) -> AppResult<Json<ResearchTaskDto>> {
    Ok(Json(research::patch(&state, task_id, input).await?))
}

/// `DELETE /api/research-tasks/:id`.
pub async fn delete(
    State(state): State<AppState>,
    Path(task_id): Path<i32>,
) -> AppResult<StatusCode> {
    research::delete(&state, task_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/research-tasks/:id/run`.
pub async fn run(
    State(state): State<AppState>,
    Path(task_id): Path<i32>,
) -> AppResult<(StatusCode, Json<ResearchRunResponse>)> {
    let session_id = research::run(&state, task_id).await?;
    Ok((
        StatusCode::CREATED,
        Json(ResearchRunResponse { session_id }),
    ))
}
