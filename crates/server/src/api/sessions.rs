//! Session endpoints (spec/10 §2.7): reads and the control operations of spec/03 §6.

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::api::dto::sessions::{
    DebugEchoDto, DebugEchoInput, MessagePageDto, ResumeInput, SessionAnswerInput, SessionDto,
    SessionMessageInput, SessionPatch, SessionSnapshotDto,
};
use crate::db::json::{SessionKind, SessionStatus};
use crate::error::AppResult;
use crate::services::sessions;
use crate::state::AppState;

/// `?kind=&status=&chapter_id=` of the book's session list (spec/10 §2.7).
#[derive(Debug, Default, Deserialize)]
pub struct SessionListQuery {
    pub kind: Option<SessionKind>,
    pub status: Option<SessionStatus>,
    pub chapter_id: Option<i32>,
}

/// `?cursor=&limit=` of the message list (spec/10 §2.7).
#[derive(Debug, Default, Deserialize)]
pub struct MessagePageQuery {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

/// `GET /api/books/:id/sessions`.
pub async fn list(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Query(query): Query<SessionListQuery>,
) -> AppResult<Json<Vec<SessionDto>>> {
    let sessions = sessions::list(
        state.db().conn(),
        book_id,
        query.kind,
        query.status,
        query.chapter_id,
    )
    .await?;
    Ok(Json(sessions))
}

/// `GET /api/sessions/:id`.
pub async fn get(
    State(state): State<AppState>,
    Path(session_id): Path<i32>,
) -> AppResult<Json<SessionSnapshotDto>> {
    Ok(Json(
        sessions::snapshot(state.db().conn(), session_id).await?,
    ))
}

/// `GET /api/sessions/:id/messages`.
pub async fn messages(
    State(state): State<AppState>,
    Path(session_id): Path<i32>,
    Query(query): Query<MessagePageQuery>,
) -> AppResult<Json<MessagePageDto>> {
    Ok(Json(
        sessions::message_page(state.db().conn(), session_id, query.cursor, query.limit).await?,
    ))
}

/// `POST /api/sessions/:id/pause`.
pub async fn pause(
    State(state): State<AppState>,
    Path(session_id): Path<i32>,
) -> AppResult<Json<SessionDto>> {
    Ok(Json(sessions::pause(&state, session_id).await?))
}

/// `POST /api/sessions/:id/resume`.
pub async fn resume(
    State(state): State<AppState>,
    Path(session_id): Path<i32>,
    Json(input): Json<ResumeInput>,
) -> AppResult<Json<SessionDto>> {
    Ok(Json(sessions::resume(&state, session_id, input).await?))
}

/// `POST /api/sessions/:id/stop`.
pub async fn stop(
    State(state): State<AppState>,
    Path(session_id): Path<i32>,
) -> AppResult<Json<SessionDto>> {
    Ok(Json(sessions::stop(&state, session_id).await?))
}

/// `POST /api/sessions/:id/continue`.
pub async fn continue_session(
    State(state): State<AppState>,
    Path(session_id): Path<i32>,
) -> AppResult<Json<SessionDto>> {
    Ok(Json(sessions::continue_session(&state, session_id).await?))
}

/// `POST /api/sessions/:id/abandon`.
pub async fn abandon(
    State(state): State<AppState>,
    Path(session_id): Path<i32>,
) -> AppResult<Json<SessionDto>> {
    Ok(Json(sessions::abandon(&state, session_id).await?))
}

/// `POST /api/sessions/:id/message`.
pub async fn send_message(
    State(state): State<AppState>,
    Path(session_id): Path<i32>,
    Json(input): Json<SessionMessageInput>,
) -> AppResult<Json<SessionDto>> {
    Ok(Json(
        sessions::send_message(&state, session_id, input).await?,
    ))
}

/// `POST /api/sessions/:id/answer`.
pub async fn answer(
    State(state): State<AppState>,
    Path(session_id): Path<i32>,
    Json(input): Json<SessionAnswerInput>,
) -> AppResult<Json<SessionDto>> {
    Ok(Json(sessions::answer(&state, session_id, input).await?))
}

/// `PATCH /api/sessions/:id`.
pub async fn patch(
    State(state): State<AppState>,
    Path(session_id): Path<i32>,
    Json(input): Json<SessionPatch>,
) -> AppResult<Json<SessionDto>> {
    Ok(Json(sessions::patch(&state, session_id, input).await?))
}

/// `POST /api/debug/sessions/echo` (spec/10 §2.7, debug builds only).
pub async fn debug_echo(
    State(state): State<AppState>,
    Json(input): Json<DebugEchoInput>,
) -> AppResult<Json<DebugEchoDto>> {
    Ok(Json(sessions::debug_echo(&state, input.book_id).await?))
}
