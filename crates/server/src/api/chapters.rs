//! Chapter endpoints (spec/10 §2.2).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use crate::api::dto::chapters::{
    ApproveChapterResponse, ChapterContentPatch, ChapterContentResult, ChapterContentTextPut,
    ChapterDto, ChapterPatch, ChapterVersionDto, ChapterVersionMetaDto, ChapterWriteInput,
    ChapterWriteResponse, ManualVersionInput,
};
use crate::db::json::ChangeActor;
use crate::error::AppResult;
use crate::services::chapters as service;
use crate::services::versions as version_service;
use crate::state::AppState;

/// `GET`, `PATCH` and `DELETE /api/chapters/:id`.
pub async fn get(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
) -> AppResult<Json<ChapterDto>> {
    Ok(Json(service::get(state.db().conn(), chapter_id).await?))
}

pub async fn patch(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
    Json(patch): Json<ChapterPatch>,
) -> AppResult<Json<ChapterDto>> {
    Ok(Json(
        service::update(state.db(), state.events(), chapter_id, patch).await?,
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
) -> AppResult<StatusCode> {
    service::delete(state.db(), state.events(), chapter_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PATCH /api/chapters/:id/content`.
pub async fn patch_content(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
    Json(patch): Json<ChapterContentPatch>,
) -> AppResult<Json<ChapterContentResult>> {
    Ok(Json(
        service::patch_content(state.db(), state.events(), chapter_id, patch).await?,
    ))
}

/// `PUT /api/chapters/:id/content/text`.
pub async fn put_content_text(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
    Json(put): Json<ChapterContentTextPut>,
) -> AppResult<Json<ChapterContentResult>> {
    Ok(Json(
        service::put_content_text(state.db(), state.events(), chapter_id, put).await?,
    ))
}

/// `POST /api/chapters/:id/write` (spec/08 §1.1): start the writer session.
pub async fn write(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
    Json(input): Json<ChapterWriteInput>,
) -> AppResult<(StatusCode, Json<ChapterWriteResponse>)> {
    let response = service::write(&state, chapter_id, input).await?;
    Ok((StatusCode::CREATED, Json(response)))
}

/// `POST /api/chapters/:id/submit`.
pub async fn submit(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
) -> AppResult<Json<ChapterDto>> {
    Ok(Json(
        service::submit(state.db(), state.events(), chapter_id).await?,
    ))
}

/// `POST /api/chapters/:id/approve` (spec/10 §2.2).
pub async fn approve(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
) -> AppResult<Json<ApproveChapterResponse>> {
    Ok(Json(
        service::approve(state.db(), state.events(), chapter_id).await?,
    ))
}

/// `GET` and `POST /api/chapters/:id/versions`.
pub async fn list_versions(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
) -> AppResult<Json<Vec<ChapterVersionMetaDto>>> {
    Ok(Json(
        version_service::list(state.db().conn(), chapter_id).await?,
    ))
}

pub async fn create_version(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
    Json(input): Json<ManualVersionInput>,
) -> AppResult<(StatusCode, Json<ChapterVersionMetaDto>)> {
    let dto = service::save_version(state.db(), chapter_id, input).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// `GET /api/chapter-versions/:id`.
pub async fn get_version(
    State(state): State<AppState>,
    Path(version_id): Path<i32>,
) -> AppResult<Json<ChapterVersionDto>> {
    Ok(Json(
        version_service::get(state.db().conn(), version_id).await?,
    ))
}

/// `POST /api/chapter-versions/:id/restore` (spec/08 §1.2).
pub async fn restore_version(
    State(state): State<AppState>,
    Path(version_id): Path<i32>,
) -> AppResult<Json<ChapterDto>> {
    Ok(Json(
        service::restore(state.db(), state.events(), version_id, ChangeActor::User).await?,
    ))
}
