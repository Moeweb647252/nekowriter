//! Review endpoints (spec/10 §2.2): ask the reviewer, re-review, send back, reports.

use axum::Json;
use axum::extract::{Path, State};

use crate::api::dto::reviews::{
    RereviewInput, ReviewSessionResponse, ReviewsResponse, SendBackInput,
};
use crate::error::AppResult;
use crate::services::reviews as service;
use crate::state::AppState;

/// `POST /api/chapters/:id/review` — ask the reviewer (spec/08 §1.1).
pub async fn review(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
) -> AppResult<Json<ReviewSessionResponse>> {
    Ok(Json(service::ask_reviewer(&state, chapter_id).await?))
}

/// `POST /api/review-reports/:id/rereview` (spec/03 §6).
pub async fn rereview(
    State(state): State<AppState>,
    Path(report_id): Path<i32>,
    Json(input): Json<RereviewInput>,
) -> AppResult<Json<ReviewSessionResponse>> {
    Ok(Json(service::rereview(&state, report_id, input).await?))
}

/// `POST /api/chapters/:id/send-back` (spec/08 §1.1).
pub async fn send_back(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
    Json(input): Json<SendBackInput>,
) -> AppResult<Json<ReviewSessionResponse>> {
    Ok(Json(service::send_back(&state, chapter_id, input).await?))
}

/// `GET /api/chapters/:id/reviews` (spec/10 §2.2).
pub async fn reports(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
) -> AppResult<Json<ReviewsResponse>> {
    Ok(Json(service::reports(state.db().conn(), chapter_id).await?))
}
