//! Change-log endpoints (spec/10 §2.4).

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::api::dto::changes::{ChangeDto, ChangePageDto, ChangeQuery};
use crate::error::AppResult;
use crate::services::change_log as service;
use crate::state::AppState;

/// `GET /api/books/:id/changes`.
pub async fn list(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Query(query): Query<ChangeQuery>,
) -> AppResult<Json<ChangePageDto>> {
    Ok(Json(
        service::page(state.db().conn(), book_id, &query).await?,
    ))
}

/// `GET /api/changes/:id`.
pub async fn get(
    State(state): State<AppState>,
    Path(change_id): Path<i32>,
) -> AppResult<Json<ChangeDto>> {
    Ok(Json(service::get(state.db().conn(), change_id).await?))
}
