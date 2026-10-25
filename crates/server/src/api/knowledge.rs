//! Knowledge-base endpoints (spec/10 §2.5): scopes, entries, hybrid search, book visibility and
//! the embedding status of the search page.
//!
//! Handlers stay thin: they parse the path and body and return the service DTO.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;

use crate::api::dto::knowledge::{
    BookScopesInput, EntryDto, EntryInput, EntryPageDto, EntryPatch, EntryQuery, KnowledgeHitDto,
    KnowledgeSearchQuery, ScopeDto, ScopeInput, ScopePatch,
};
use crate::error::AppResult;
use crate::services::knowledge;
use crate::state::AppState;

/// `GET /api/knowledge/scopes`.
pub async fn list_scopes(State(state): State<AppState>) -> AppResult<Json<Vec<ScopeDto>>> {
    Ok(Json(knowledge::scopes(state.db_conn()).await?))
}

/// `POST /api/knowledge/scopes`.
pub async fn create_scope(
    State(state): State<AppState>,
    Json(input): Json<ScopeInput>,
) -> AppResult<(StatusCode, Json<ScopeDto>)> {
    let dto = knowledge::create_scope(state.db(), state.events(), input).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// `GET /api/knowledge/scopes/:id`.
pub async fn get_scope(
    State(state): State<AppState>,
    Path(scope_id): Path<i32>,
) -> AppResult<Json<ScopeDto>> {
    Ok(Json(knowledge::get_scope(state.db_conn(), scope_id).await?))
}

/// `PATCH /api/knowledge/scopes/:id`.
pub async fn patch_scope(
    State(state): State<AppState>,
    Path(scope_id): Path<i32>,
    Json(patch): Json<ScopePatch>,
) -> AppResult<Json<ScopeDto>> {
    Ok(Json(
        knowledge::update_scope(state.db(), state.events(), scope_id, patch).await?,
    ))
}

/// `DELETE /api/knowledge/scopes/:id`.
pub async fn delete_scope(
    State(state): State<AppState>,
    Path(scope_id): Path<i32>,
) -> AppResult<StatusCode> {
    knowledge::delete_scope(state.db(), state.events(), scope_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/knowledge/scopes/:id/entries`.
pub async fn list_entries(
    State(state): State<AppState>,
    Path(scope_id): Path<i32>,
    Query(query): Query<EntryQuery>,
) -> AppResult<Json<EntryPageDto>> {
    Ok(Json(
        knowledge::list_entries(state.db_conn(), scope_id, &query).await?,
    ))
}

/// `POST /api/knowledge/scopes/:id/entries`.
pub async fn create_entry(
    State(state): State<AppState>,
    Path(scope_id): Path<i32>,
    Json(input): Json<EntryInput>,
) -> AppResult<(StatusCode, Json<EntryDto>)> {
    let dto = knowledge::create_entry(state.db(), state.events(), scope_id, input).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// `GET /api/knowledge/entries/:id`.
pub async fn get_entry(
    State(state): State<AppState>,
    Path(entry_id): Path<i32>,
) -> AppResult<Json<EntryDto>> {
    Ok(Json(knowledge::get_entry(state.db_conn(), entry_id).await?))
}

/// `PATCH /api/knowledge/entries/:id`.
pub async fn patch_entry(
    State(state): State<AppState>,
    Path(entry_id): Path<i32>,
    Json(patch): Json<EntryPatch>,
) -> AppResult<Json<EntryDto>> {
    Ok(Json(
        knowledge::update_entry(state.db(), state.events(), entry_id, patch).await?,
    ))
}

/// `DELETE /api/knowledge/entries/:id`.
pub async fn delete_entry(
    State(state): State<AppState>,
    Path(entry_id): Path<i32>,
) -> AppResult<StatusCode> {
    knowledge::delete_entry(state.db(), state.events(), entry_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/knowledge/search`.
pub async fn search(
    State(state): State<AppState>,
    Query(query): Query<KnowledgeSearchQuery>,
) -> AppResult<Json<Vec<KnowledgeHitDto>>> {
    Ok(Json(knowledge::search(&state, &query).await?))
}

/// `GET /api/books/:id/scopes`.
pub async fn list_book_scopes(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
) -> AppResult<Json<Vec<ScopeDto>>> {
    Ok(Json(
        knowledge::book_scopes(state.db_conn(), book_id).await?,
    ))
}

/// `PUT /api/books/:id/scopes`.
pub async fn put_book_scopes(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Json(input): Json<BookScopesInput>,
) -> AppResult<Json<Vec<ScopeDto>>> {
    Ok(Json(
        knowledge::set_book_scopes(state.db(), state.events(), book_id, input).await?,
    ))
}
