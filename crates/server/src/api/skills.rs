//! Writing-skill endpoints (spec/10 §2.9, design §8).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;

use crate::api::dto::skills::{
    BookWritingSkillsInput, WritingSkillDeleteResponse, WritingSkillDto, WritingSkillInput,
    WritingSkillPatch,
};
use crate::error::AppResult;
use crate::services::skills as service;
use crate::state::AppState;

/// `?book_id=` of `GET /api/writing-skills`.
#[derive(Debug, Deserialize)]
pub struct CatalogueQuery {
    pub book_id: Option<i32>,
}

/// `GET /api/writing-skills?book_id=` (design §8): the catalogue with `selected` per book.
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<CatalogueQuery>,
) -> AppResult<Json<Vec<WritingSkillDto>>> {
    Ok(Json(
        service::catalogue(state.db().conn(), query.book_id).await?,
    ))
}

/// `POST /api/writing-skills` (design §8): create a user skill.
pub async fn create(
    State(state): State<AppState>,
    Json(input): Json<WritingSkillInput>,
) -> AppResult<(StatusCode, Json<WritingSkillDto>)> {
    Ok((
        StatusCode::CREATED,
        Json(service::create(&state, input).await?),
    ))
}

/// `PATCH /api/writing-skills/:id` (design §8): user rows only.
pub async fn patch(
    State(state): State<AppState>,
    Path(skill_id): Path<i32>,
    Json(patch): Json<WritingSkillPatch>,
) -> AppResult<Json<WritingSkillDto>> {
    Ok(Json(service::update(&state, skill_id, patch).await?))
}

/// `DELETE /api/writing-skills/:id` (design §8): user rows only; names the books that used it.
pub async fn delete(
    State(state): State<AppState>,
    Path(skill_id): Path<i32>,
) -> AppResult<Json<WritingSkillDeleteResponse>> {
    Ok(Json(service::delete(&state, skill_id).await?))
}

/// `PUT /api/books/:id/writing-skills` (design §8): replace the book's selection.
pub async fn replace_selection(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Json(input): Json<BookWritingSkillsInput>,
) -> AppResult<Json<Vec<WritingSkillDto>>> {
    Ok(Json(
        service::replace_selection(&state, book_id, input).await?,
    ))
}
