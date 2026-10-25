//! Track endpoints (spec/10 §2.2).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use crate::api::dto::chapters::{
    ChapterDto, ChapterInput, ChapterListItemDto, ChapterReorderInput,
};
use crate::api::dto::tracks::{TrackDto, TrackInput, TrackPatch};
use crate::error::AppResult;
use crate::services::chapters as chapter_service;
use crate::services::tracks as service;
use crate::state::AppState;

/// `GET` and `POST /api/books/:id/tracks`.
pub async fn list(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
) -> AppResult<Json<Vec<TrackDto>>> {
    Ok(Json(service::list(state.db().conn(), book_id).await?))
}

pub async fn create(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Json(input): Json<TrackInput>,
) -> AppResult<(StatusCode, Json<TrackDto>)> {
    let dto = service::create(state.db(), state.events(), book_id, input).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// `GET`, `PATCH` and `DELETE /api/tracks/:id`.
pub async fn get(
    State(state): State<AppState>,
    Path(track_id): Path<i32>,
) -> AppResult<Json<TrackDto>> {
    Ok(Json(service::get(state.db().conn(), track_id).await?))
}

pub async fn patch(
    State(state): State<AppState>,
    Path(track_id): Path<i32>,
    Json(patch): Json<TrackPatch>,
) -> AppResult<Json<TrackDto>> {
    Ok(Json(
        service::update(state.db(), state.events(), track_id, patch).await?,
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(track_id): Path<i32>,
) -> AppResult<StatusCode> {
    service::delete(&state, track_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET` and `POST /api/tracks/:id/chapters`.
pub async fn list_chapters(
    State(state): State<AppState>,
    Path(track_id): Path<i32>,
) -> AppResult<Json<Vec<ChapterListItemDto>>> {
    Ok(Json(
        chapter_service::list(state.db().conn(), track_id).await?,
    ))
}

pub async fn create_chapter(
    State(state): State<AppState>,
    Path(track_id): Path<i32>,
    Json(input): Json<ChapterInput>,
) -> AppResult<(StatusCode, Json<ChapterDto>)> {
    let dto = chapter_service::create(state.db(), state.events(), track_id, input).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// `POST /api/tracks/:id/chapters/reorder`.
pub async fn reorder_chapters(
    State(state): State<AppState>,
    Path(track_id): Path<i32>,
    Json(input): Json<ChapterReorderInput>,
) -> AppResult<Json<Vec<ChapterListItemDto>>> {
    Ok(Json(
        chapter_service::reorder(state.db(), state.events(), track_id, input).await?,
    ))
}
