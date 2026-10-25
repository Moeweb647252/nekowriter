//! Embedding/index endpoints (spec/10 §2.1): the live `index.status` and the index rebuild.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use crate::api::health::IndexStatusDto;
use crate::error::AppResult;
use crate::state::AppState;

/// `GET /api/embedding/status` — the `IndexStatusDto` the settings page and the health badge show
/// (spec/07 §4, spec/10 §3).
pub async fn status(State(state): State<AppState>) -> Json<IndexStatusDto> {
    Json(state.search().status_dto().await)
}

/// `POST /api/embedding/rebuild` — queue every document for re-embedding and retry a failed
/// model load in the background (spec/07 §4). The work is asynchronous, hence 202.
pub async fn rebuild(State(state): State<AppState>) -> AppResult<StatusCode> {
    crate::services::indexer::rebuild(&state).await?;
    let retry = state.clone();
    tokio::spawn(async move {
        // A `failed` state is only retried here or on a model reload (spec/07 §4).
        if retry.search().ensure_ready().await {
            crate::search::index::wake();
        }
    });
    Ok(StatusCode::ACCEPTED)
}
