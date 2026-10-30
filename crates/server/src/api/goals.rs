//! Goal endpoints (spec/10 §2.4).
//!
//! The handlers are thin: validation, scope, change history and the evaluator live in
//! `services::goals`, so the Goals API, the overview and the navigator share the same derived
//! `ready`/`closable` state.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;

use crate::api::dto::goals::{GoalNodeDto, GoalNodeInput, GoalNodePatch, GoalQuery};
use crate::error::AppResult;
use crate::services::goals;
use crate::state::AppState;

/// `GET /api/books/:id/goals` (spec/10 §2.4).
pub async fn list(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Query(query): Query<GoalQuery>,
) -> AppResult<Json<Vec<GoalNodeDto>>> {
    Ok(Json(goals::list(state.db().conn(), book_id, &query).await?))
}

/// `POST /api/books/:id/goals` (spec/10 §2.4).
pub async fn create(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Json(input): Json<GoalNodeInput>,
) -> AppResult<(StatusCode, Json<GoalNodeDto>)> {
    Ok((
        StatusCode::CREATED,
        Json(goals::create(state.db(), state.events(), book_id, input).await?),
    ))
}

/// `PATCH /api/goals/:id` (spec/10 §2.4).
pub async fn patch(
    State(state): State<AppState>,
    Path(goal_id): Path<i32>,
    Json(input): Json<GoalNodePatch>,
) -> AppResult<Json<GoalNodeDto>> {
    Ok(Json(
        goals::update(state.db(), state.events(), goal_id, input).await?,
    ))
}

/// `DELETE /api/goals/:id` (spec/10 §2.4).
pub async fn delete(
    State(state): State<AppState>,
    Path(goal_id): Path<i32>,
) -> AppResult<StatusCode> {
    goals::delete(state.db(), state.events(), goal_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
