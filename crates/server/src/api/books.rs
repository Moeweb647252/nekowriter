//! Book endpoints (spec/10 §2.2).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::api::dto::books::{
    BookAgentStart, BookAgentStarted, BookDto, BookInput, BookOverviewDto, BookPatch,
    BookSummaryDto, CreateBookResponse,
};
use crate::api::dto::research::{KnowledgeCheckInput, KnowledgeCheckResponse};
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
    let response = service::create(&state, input).await?;
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
    service::delete(&state, book_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/books/:id/overview` (spec/08 §7).
pub async fn overview(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
) -> AppResult<Json<BookOverviewDto>> {
    Ok(Json(service::overview(state.db().conn(), book_id).await?))
}

/// `POST /api/books/:id/knowledge-check` (spec/07 §8.1, spec/10 §2.2).
pub async fn knowledge_check(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Json(input): Json<KnowledgeCheckInput>,
) -> AppResult<(StatusCode, Json<KnowledgeCheckResponse>)> {
    let (task, session_id) = crate::services::research::start_knowledge_check_for_book(
        &state,
        book_id,
        input.requirements,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(KnowledgeCheckResponse { task, session_id }),
    ))
}

/// `POST /api/books/:id/agent` (spec/10 §2.2, D-169).
pub async fn start_agent(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Json(input): Json<BookAgentStart>,
) -> AppResult<(StatusCode, Json<BookAgentStarted>)> {
    let session_id = crate::services::sessions::start_book_agent(
        &state,
        book_id,
        input.message,
        input.write_mode,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(BookAgentStarted { session_id })))
}

/// `POST /api/books/:id/conception/start` (spec/08 §5, spec/10 §2.2): the open session or a new
/// one.
pub async fn conception_start(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
) -> AppResult<Json<ConceptionStartResponse>> {
    let (session_id, warnings) = crate::services::conception::start(&state, book_id).await?;
    Ok(Json(ConceptionStartResponse {
        session_id,
        warnings,
    }))
}

/// `POST /api/books/:id/conception/finalize` (spec/08 §5, spec/10 §2.2).
pub async fn conception_finalize(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
) -> AppResult<Json<crate::services::conception::FinalizeResponse>> {
    Ok(Json(
        crate::services::conception::finalize(&state, book_id).await?,
    ))
}

/// `POST /api/books/:id/canon/seed` (spec/08 §5, spec/10 §2.2): the open seed session or a new
/// one.
pub async fn canon_seed(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
) -> AppResult<Json<CanonSeedResponse>> {
    let (session_id, warnings) = crate::services::conception::seed(&state, book_id).await?;
    Ok(Json(CanonSeedResponse {
        session_id,
        warnings,
    }))
}

/// `POST /books/:id/conception/start` response (spec/10 §2.2).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct ConceptionStartResponse {
    pub session_id: i32,
    pub warnings: Vec<String>,
}

/// `POST /books/:id/canon/seed` response (spec/10 §2.2).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct CanonSeedResponse {
    pub session_id: i32,
    pub warnings: Vec<String>,
}
