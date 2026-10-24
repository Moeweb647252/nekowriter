//! Book endpoints (spec/10 §2.2).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;

use crate::api::dto::books::{
    BookDto, BookInput, BookOverviewDto, BookPatch, BookSummaryDto, CreateBookResponse,
};
use crate::db::json::BookStatus;
use crate::error::AppResult;
use crate::services::books as service;
use crate::state::AppState;

/// `?status=` filter of `GET /api/books`.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
}

/// `GET /api/books` and `POST /api/books`.
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<Vec<BookSummaryDto>>> {
    let status = match query.status.as_deref() {
        None | Some("") | Some("all") => None,
        Some(raw) => Some(BookStatus::from_str_opt(raw).ok_or_else(|| {
            crate::error::AppError::validation(format!("unknown book status {raw:?}"))
        })?),
    };
    Ok(Json(service::list(state.db().conn(), status).await?))
}

pub async fn create(
    State(state): State<AppState>,
    Json(input): Json<BookInput>,
) -> AppResult<(StatusCode, Json<CreateBookResponse>)> {
    let response = service::create(state.db(), state.events(), input).await?;
    Ok((StatusCode::CREATED, Json(response)))
}

/// `GET`, `PATCH` and `DELETE /api/books/:id`.
pub async fn get(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
) -> AppResult<Json<BookDto>> {
    Ok(Json(service::get(state.db().conn(), book_id).await?))
}

pub async fn patch(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Json(patch): Json<BookPatch>,
) -> AppResult<Json<BookDto>> {
    Ok(Json(
        service::update(state.db(), state.events(), book_id, patch).await?,
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
) -> AppResult<StatusCode> {
    service::delete(state.db(), state.events(), book_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/books/:id/overview` (spec/08 §7).
pub async fn overview(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
) -> AppResult<Json<BookOverviewDto>> {
    Ok(Json(service::overview(state.db().conn(), book_id).await?))
}
