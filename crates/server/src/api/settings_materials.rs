//! Setting material endpoints (spec/10 §2.3).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use crate::api::dto::changes::ChangeDto;
use crate::api::dto::settings_materials::{
    CharacterDto, CharacterInput, CharacterPatch, CharacterReorderInput, SettingDocDto,
    SettingDocInput,
};
use crate::error::AppResult;
use crate::services::characters as character_service;
use crate::services::setting_docs as doc_service;
use crate::state::AppState;

/// `GET /api/books/:id/setting-docs`.
pub async fn list_docs(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
) -> AppResult<Json<Vec<SettingDocDto>>> {
    Ok(Json(doc_service::list(state.db().conn(), book_id).await?))
}

/// `GET` and `PUT /api/books/:id/setting-docs/:kind`.
pub async fn get_doc(
    State(state): State<AppState>,
    Path((book_id, kind)): Path<(i32, String)>,
) -> AppResult<Json<SettingDocDto>> {
    let kind = doc_service::parse_kind(&kind)?;
    Ok(Json(
        doc_service::get(state.db().conn(), book_id, kind).await?,
    ))
}

pub async fn put_doc(
    State(state): State<AppState>,
    Path((book_id, kind)): Path<(i32, String)>,
    Json(input): Json<SettingDocInput>,
) -> AppResult<Json<SettingDocDto>> {
    let kind = doc_service::parse_kind(&kind)?;
    Ok(Json(
        doc_service::put(state.db(), state.events(), book_id, kind, input).await?,
    ))
}

/// `GET /api/books/:id/setting-docs/:kind/history`.
pub async fn doc_history(
    State(state): State<AppState>,
    Path((book_id, kind)): Path<(i32, String)>,
) -> AppResult<Json<Vec<ChangeDto>>> {
    let kind = doc_service::parse_kind(&kind)?;
    Ok(Json(
        doc_service::history(state.db().conn(), book_id, kind, 100).await?,
    ))
}

/// `GET` and `POST /api/books/:id/characters`.
pub async fn list_characters(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
) -> AppResult<Json<Vec<CharacterDto>>> {
    Ok(Json(
        character_service::list(state.db().conn(), book_id).await?,
    ))
}

pub async fn create_character(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Json(input): Json<CharacterInput>,
) -> AppResult<(StatusCode, Json<CharacterDto>)> {
    let dto = character_service::create(state.db(), state.events(), book_id, input).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// `GET /api/characters/:id`.
pub async fn get_character(
    State(state): State<AppState>,
    Path(character_id): Path<i32>,
) -> AppResult<Json<CharacterDto>> {
    let row = character_service::load(state.db().conn(), character_id).await?;
    Ok(Json(character_service::to_dto(&row)?))
}

/// `PATCH` and `DELETE /api/characters/:id`.
pub async fn patch_character(
    State(state): State<AppState>,
    Path(character_id): Path<i32>,
    Json(patch): Json<CharacterPatch>,
) -> AppResult<Json<CharacterDto>> {
    Ok(Json(
        character_service::update(state.db(), state.events(), character_id, patch).await?,
    ))
}

pub async fn delete_character(
    State(state): State<AppState>,
    Path(character_id): Path<i32>,
) -> AppResult<StatusCode> {
    character_service::delete(state.db(), state.events(), character_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/books/:id/characters/reorder`.
pub async fn reorder_characters(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Json(input): Json<CharacterReorderInput>,
) -> AppResult<Json<Vec<CharacterDto>>> {
    Ok(Json(
        character_service::reorder(state.db(), state.events(), book_id, input).await?,
    ))
}

/// `GET /api/characters/:id/history`.
pub async fn character_history(
    State(state): State<AppState>,
    Path(character_id): Path<i32>,
) -> AppResult<Json<Vec<ChangeDto>>> {
    Ok(Json(
        character_service::history(state.db().conn(), character_id, 100).await?,
    ))
}
