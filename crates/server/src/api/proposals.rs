//! Setting proposal endpoints (spec/10 §2.3, spec/06 §5).

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::api::dto::proposals::{ProposalDto, ProposalQuery};
use crate::error::AppResult;
use crate::services::proposals as service;
use crate::state::AppState;

/// `GET /api/books/:id/proposals` (spec/10 §2.3).
pub async fn list(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Query(query): Query<ProposalQuery>,
) -> AppResult<Json<Vec<ProposalDto>>> {
    Ok(Json(
        service::list(state.db().conn(), book_id, query.status).await?,
    ))
}

/// `POST /api/proposals/:id/accept` body (spec/10 §2.3, plan §4.7).
#[derive(Debug, Default, serde::Deserialize)]
pub struct AcceptProposalInput {
    #[serde(default)]
    pub confirm_open_children: bool,
}

/// `POST /api/proposals/:id/accept` (spec/10 §2.3).
pub async fn accept(
    State(state): State<AppState>,
    Path(proposal_id): Path<i32>,
    body: Option<Json<AcceptProposalInput>>,
) -> AppResult<Json<ProposalDto>> {
    let confirm_open_children = body
        .map(|Json(body)| body.confirm_open_children)
        .unwrap_or(false);
    Ok(Json(
        service::accept(
            state.db(),
            state.events(),
            proposal_id,
            confirm_open_children,
        )
        .await?,
    ))
}

/// `POST /api/proposals/:id/reject` (spec/10 §2.3).
pub async fn reject(
    State(state): State<AppState>,
    Path(proposal_id): Path<i32>,
) -> AppResult<Json<ProposalDto>> {
    Ok(Json(
        service::reject(state.db(), state.events(), proposal_id).await?,
    ))
}
