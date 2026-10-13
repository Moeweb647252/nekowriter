//! A version's workspace: its view, revisions, the brief, preparation, chapters, reviews
//! and runs.

use axum::extract::State;
use nekowriter_core::{
    chapter::{ChapterGuidance, ChapterTitle},
    ids::{CommandId, ReviewId, RevisionId, RunId, VersionId},
    preparation::NovelBrief,
    review::{ReviewDecision, ReviewFeedback},
};
use nekowriter_store::{RevisionView, WorkspaceAck, WorkspaceView};
use serde::Deserialize;
use ts_rs::TS;

use crate::http::{
    AppState,
    error::ApiError,
    extract::{Json, Path},
    handlers::RevisionRequest,
};

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveBriefRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    pub brief: String,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DecideReviewRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    pub decision: ReviewDecisionInput,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartChapterRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    /// What the user asks of the chapter; may be empty.
    pub guidance: String,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AddExtraChapterRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    pub title: String,
}

/// A review decision as a request carries it.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ReviewDecisionInput {
    Approve,
    /// Feedback is required: the writer needs to know what to change.
    Reject {
        feedback: String,
    },
}

pub(crate) async fn get(
    State(state): State<AppState>,
    Path(version_id): Path<VersionId>,
) -> Result<axum::Json<WorkspaceView>, ApiError> {
    Ok(axum::Json(state.store.workspace(version_id).await?))
}

pub(crate) async fn save_brief(
    State(state): State<AppState>,
    Path(version_id): Path<VersionId>,
    Json(request): Json<SaveBriefRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    let brief = NovelBrief::parse(&request.brief)?;
    let ack = state
        .store
        .save_brief(
            request.command_id,
            version_id,
            request.expected_revision,
            brief,
        )
        .await?;
    Ok(axum::Json(ack))
}

pub(crate) async fn start_preparation(
    State(state): State<AppState>,
    Path(version_id): Path<VersionId>,
    Json(request): Json<RevisionRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    let ack = state
        .store
        .start_preparation(request.command_id, version_id, request.expected_revision)
        .await?;
    Ok(axum::Json(ack))
}

pub(crate) async fn decide_review(
    State(state): State<AppState>,
    Path(review_id): Path<ReviewId>,
    Json(request): Json<DecideReviewRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    let decision = match request.decision {
        ReviewDecisionInput::Approve => ReviewDecision::Approve,
        ReviewDecisionInput::Reject { feedback } => ReviewDecision::Reject {
            feedback: ReviewFeedback::parse(&feedback)?,
        },
    };
    let ack = state
        .store
        .decide_review(
            request.command_id,
            review_id,
            request.expected_revision,
            decision,
        )
        .await?;
    Ok(axum::Json(ack))
}

pub(crate) async fn continue_run(
    State(state): State<AppState>,
    Path(run_id): Path<RunId>,
    Json(request): Json<RevisionRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    let ack = state
        .store
        .continue_run(request.command_id, run_id, request.expected_revision)
        .await?;
    Ok(axum::Json(ack))
}

/// A revision's content. Revisions never change, so clients can keep it.
pub(crate) async fn revision(
    State(state): State<AppState>,
    Path(revision_id): Path<RevisionId>,
) -> Result<axum::Json<RevisionView>, ApiError> {
    Ok(axum::Json(state.store.revision(revision_id).await?))
}

pub(crate) async fn start_chapter(
    State(state): State<AppState>,
    Path(version_id): Path<VersionId>,
    Json(request): Json<StartChapterRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    let guidance = ChapterGuidance::parse(&request.guidance)?;
    let ack = state
        .store
        .start_chapter(
            request.command_id,
            version_id,
            request.expected_revision,
            guidance,
        )
        .await?;
    Ok(axum::Json(ack))
}

pub(crate) async fn next_chapter(
    State(state): State<AppState>,
    Path(version_id): Path<VersionId>,
    Json(request): Json<RevisionRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    let ack = state
        .store
        .next_chapter(request.command_id, version_id, request.expected_revision)
        .await?;
    Ok(axum::Json(ack))
}

pub(crate) async fn complete(
    State(state): State<AppState>,
    Path(version_id): Path<VersionId>,
    Json(request): Json<RevisionRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    let ack = state
        .store
        .complete_version(request.command_id, version_id, request.expected_revision)
        .await?;
    Ok(axum::Json(ack))
}

pub(crate) async fn add_extra_chapter(
    State(state): State<AppState>,
    Path(version_id): Path<VersionId>,
    Json(request): Json<AddExtraChapterRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    let title = ChapterTitle::parse(&request.title)?;
    let ack = state
        .store
        .add_extra_chapter(
            request.command_id,
            version_id,
            request.expected_revision,
            title,
        )
        .await?;
    Ok(axum::Json(ack))
}
