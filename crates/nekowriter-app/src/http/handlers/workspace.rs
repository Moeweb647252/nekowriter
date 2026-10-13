//! A version's workspace: its view, revisions, the brief, preparation, chapters, reviews
//! and runs.

use axum::extract::State;
use nekowriter_core::{
    chapter::{ChapterGuidance, ChapterTitle},
    citation::Citation,
    feedback::FeedbackText,
    field::{FieldError, FieldProblem},
    ids::{CommandId, ReviewId, RevisionId, RunId, ToolCallId, VersionId},
    preparation::NovelBrief,
    question::Answer,
    review::{Rejection, ReviewDecision, ReviewFeedback},
    workflow::ApprovalMode,
};
use nekowriter_store::{RevisionView, WorkspaceAck, WorkspaceView};
use serde::Deserialize;
use ts_rs::TS;

use crate::http::{
    AppState,
    error::ApiError,
    extract::{Json, Path},
    handlers::{ControlRequest, RevisionRequest},
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

/// Feedback for a writing run; a control command (§16.3).
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FeedbackRequest {
    pub command_id: CommandId,
    pub text: String,
    /// Passages the feedback quotes.
    pub citations: Vec<Citation>,
}

/// Switching how body edits are approved; a control command (§9.6).
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApprovalModeRequest {
    pub command_id: CommandId,
    pub mode: ApprovalMode,
}

/// The user's answers to the agent's questions (§8.6).
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AnswerRequest {
    pub command_id: CommandId,
    pub answers: Vec<Answer>,
}

/// Delegating a review to a review agent (§10), with the user's draft feedback as notes.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DelegateReviewRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    /// The draft feedback, which may be empty, as notes for the review agent.
    pub notes: String,
    pub citations: Vec<Citation>,
}

/// A review decision as a request carries it.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ReviewDecisionInput {
    Approve,
    /// The writer needs to know what to change: feedback, citations, or both.
    Reject {
        feedback: String,
        citations: Vec<Citation>,
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
        ReviewDecisionInput::Reject {
            feedback,
            citations,
        } => ReviewDecision::Reject(Rejection::new(optional_feedback(&feedback)?, citations)?),
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

pub(crate) async fn stop_run(
    State(state): State<AppState>,
    Path(run_id): Path<RunId>,
    Json(request): Json<ControlRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    Ok(axum::Json(
        state.store.stop_run(request.command_id, run_id).await?,
    ))
}

pub(crate) async fn submit_feedback(
    State(state): State<AppState>,
    Path(run_id): Path<RunId>,
    Json(request): Json<FeedbackRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    let text = FeedbackText::parse(&request.text)?;
    Ok(axum::Json(
        state
            .store
            .submit_feedback(request.command_id, run_id, text, request.citations)
            .await?,
    ))
}

pub(crate) async fn pause_run(
    State(state): State<AppState>,
    Path(run_id): Path<RunId>,
    Json(request): Json<ControlRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    Ok(axum::Json(
        state
            .store
            .request_pause(request.command_id, run_id)
            .await?,
    ))
}

pub(crate) async fn cancel_pause(
    State(state): State<AppState>,
    Path(run_id): Path<RunId>,
    Json(request): Json<ControlRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    Ok(axum::Json(
        state.store.cancel_pause(request.command_id, run_id).await?,
    ))
}

pub(crate) async fn resume_run(
    State(state): State<AppState>,
    Path(run_id): Path<RunId>,
    Json(request): Json<ControlRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    Ok(axum::Json(
        state.store.resume_run(request.command_id, run_id).await?,
    ))
}

pub(crate) async fn set_approval_mode(
    State(state): State<AppState>,
    Path(version_id): Path<VersionId>,
    Json(request): Json<ApprovalModeRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    Ok(axum::Json(
        state
            .store
            .set_approval_mode(request.command_id, version_id, request.mode)
            .await?,
    ))
}

pub(crate) async fn answer_questions(
    State(state): State<AppState>,
    Path(call_id): Path<ToolCallId>,
    Json(request): Json<AnswerRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    Ok(axum::Json(
        state
            .store
            .answer_questions(request.command_id, call_id, request.answers)
            .await?,
    ))
}

/// Review feedback that may be empty: empty feedback is none.
fn optional_feedback(text: &str) -> Result<Option<ReviewFeedback>, FieldError> {
    match ReviewFeedback::parse(text) {
        Ok(feedback) => Ok(Some(feedback)),
        Err(FieldError {
            problem: FieldProblem::Empty,
            ..
        }) => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) async fn delegate_review(
    State(state): State<AppState>,
    Path(review_id): Path<ReviewId>,
    Json(request): Json<DelegateReviewRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    let notes = optional_feedback(&request.notes)?;
    Ok(axum::Json(
        state
            .store
            .delegate_review(
                request.command_id,
                review_id,
                request.expected_revision,
                notes,
                request.citations,
            )
            .await?,
    ))
}

pub(crate) async fn cancel_delegation(
    State(state): State<AppState>,
    Path(review_id): Path<ReviewId>,
    Json(request): Json<ControlRequest>,
) -> Result<axum::Json<WorkspaceAck>, ApiError> {
    Ok(axum::Json(
        state
            .store
            .cancel_delegation(request.command_id, review_id)
            .await?,
    ))
}
