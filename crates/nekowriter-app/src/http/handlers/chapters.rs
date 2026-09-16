//! 章节与正文 handler：章节列表与推进、细纲、正文、评审与运行控制。
//!
//! 从 `app/src/main.rs` 整块搬来（`docs/08` 的 P1.8a），代码一行未改，只有可见性从私有放宽到
//! `pub(crate)`（`http::router` 要挂这些路由）。
//!
//! 两个跨域 helper（`ensure_model_ready`、`enqueue_queued_writing_run`）在 `http::handlers`
//! 根上：它们同时被本章与准备域使用，放在任一域文件里都会让另一域反向依赖。

use axum::{
    Json,
    extract::{Path as AxumPath, State},
};
use nekowriter_core::{
    AppendChapterContent, ChapterId, DocumentId, ExecutionStatus, NewChapterGuidance,
    NewChapterOutline, NewExtraChapter, PreparationSnapshot, ReplaceChapterContent, RevisionId,
    UserQuestionAnswers, VersionId, WorkflowSnapshot,
};
use nekowriter_store::StoreError;
use uuid::Uuid;

use crate::http::dto::{
    AddExtraChapterRequest, AdvanceToNextChapterRequest, AnswerQuestionRequest,
    AppendChapterBodyRequest, ConfirmNovelCompletionRequest, DecideChapterBodyProposalRequest,
    ReplaceChapterBodyRequest, RunWorkflowControlRequest, SaveChapterGuidanceRequest,
    SetApprovalModeRequest, StartChapterOutlineRequest, SubmitChapterFinalReviewRequest,
    SubmitChapterOutlineRequest, SubmitRunFeedbackRequest,
};
use crate::http::error::ApiError;
use crate::http::handlers::{enqueue_queued_writing_run, ensure_model_ready};
use crate::http::state::AppState;

pub(crate) async fn list_chapters(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
) -> Result<Json<Vec<nekowriter_core::ChapterSummary>>, ApiError> {
    Ok(Json(state.store.list_chapters(version_id).await?))
}

pub(crate) async fn advance_to_next_chapter(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<AdvanceToNextChapterRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    Ok(Json(
        state
            .store
            .advance_to_next_chapter(
                version_id,
                request.command_id,
                request.expected_state_revision,
            )
            .await?,
    ))
}

pub(crate) async fn confirm_novel_completion(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<ConfirmNovelCompletionRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    Ok(Json(
        state
            .store
            .confirm_novel_completion(
                version_id,
                request.command_id,
                request.expected_state_revision,
            )
            .await?,
    ))
}

pub(crate) async fn add_extra_chapter(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<AddExtraChapterRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    let input = NewExtraChapter::new(request.title)?;
    Ok(Json(
        state
            .store
            .add_extra_chapter(
                version_id,
                request.command_id,
                request.expected_state_revision,
                input,
            )
            .await?,
    ))
}

pub(crate) async fn get_chapter_guidance(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
) -> Result<Json<nekowriter_core::CurrentChapterGuidance>, ApiError> {
    Ok(Json(state.store.get_chapter_guidance(chapter_id).await?))
}

pub(crate) async fn save_chapter_guidance(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
    Json(request): Json<SaveChapterGuidanceRequest>,
) -> Result<Json<nekowriter_core::CurrentChapterGuidance>, ApiError> {
    let input = NewChapterGuidance::new(request.content)?;
    Ok(Json(
        state
            .store
            .save_chapter_guidance(
                chapter_id,
                request.command_id,
                request.expected_state_revision,
                input,
            )
            .await?,
    ))
}

pub(crate) async fn start_chapter_outline(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
    Json(request): Json<StartChapterOutlineRequest>,
) -> Result<Json<nekowriter_core::WorkflowSnapshot>, ApiError> {
    let project_id = state.store.project_id_for_chapter(chapter_id).await?;
    ensure_model_ready(&state, project_id).await?;
    let workflow = state
        .store
        .start_chapter_outline(
            chapter_id,
            request.command_id,
            request.expected_state_revision,
        )
        .await?;
    if workflow.execution == ExecutionStatus::Queued {
        let run_id = workflow.active_run_id.ok_or_else(|| {
            StoreError::InvalidChapterState("章节细纲任务缺少活跃 Run".to_owned())
        })?;
        state
            .scheduler
            .enqueue_chapter_outline(workflow.version_id, chapter_id, run_id)
            .await?;
    }
    Ok(Json(workflow))
}

pub(crate) async fn get_chapter_outline(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
) -> Result<Json<nekowriter_core::ChapterOutlineSnapshot>, ApiError> {
    Ok(Json(state.store.get_chapter_outline(chapter_id).await?))
}

pub(crate) async fn submit_chapter_outline(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
    Json(request): Json<SubmitChapterOutlineRequest>,
) -> Result<Json<nekowriter_core::ChapterOutlineSnapshot>, ApiError> {
    let input = NewChapterOutline::new(request.content)?;
    Ok(Json(
        state
            .store
            .submit_chapter_outline(
                chapter_id,
                request.command_id,
                request.expected_state_revision,
                input,
            )
            .await?,
    ))
}

pub(crate) async fn get_chapter_body(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    Ok(Json(state.store.get_chapter_body(chapter_id).await?))
}

pub(crate) async fn get_document_revision(
    State(state): State<AppState>,
    AxumPath((document_id, revision_id)): AxumPath<(DocumentId, RevisionId)>,
) -> Result<Json<nekowriter_core::DocumentRevisionView>, ApiError> {
    Ok(Json(
        state
            .store
            .get_document_revision(document_id, revision_id)
            .await?,
    ))
}

pub(crate) async fn append_chapter_body(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
    Json(request): Json<AppendChapterBodyRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    let input = AppendChapterContent::new(request.content)?;
    Ok(Json(
        state
            .store
            .append_chapter_body(
                chapter_id,
                request.command_id,
                request.expected_state_revision,
                input,
            )
            .await?,
    ))
}

pub(crate) async fn replace_chapter_body(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
    Json(request): Json<ReplaceChapterBodyRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    let input = ReplaceChapterContent::new(request.old_str, request.new_str)?;
    Ok(Json(
        state
            .store
            .replace_chapter_body(
                chapter_id,
                request.command_id,
                request.expected_state_revision,
                input,
            )
            .await?,
    ))
}

pub(crate) async fn decide_chapter_body_proposal(
    State(state): State<AppState>,
    AxumPath((chapter_id, tool_call_id)): AxumPath<(ChapterId, Uuid)>,
    Json(request): Json<DecideChapterBodyProposalRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    let snapshot = state
        .store
        .decide_chapter_body_proposal(
            chapter_id,
            tool_call_id,
            request.command_id,
            request.expected_state_revision,
            request.feedback,
            request.citations,
        )
        .await?;
    enqueue_queued_writing_run(&state, &snapshot.workflow).await?;
    Ok(Json(snapshot))
}

pub(crate) async fn submit_run_feedback(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<nekowriter_core::RunId>,
    Json(request): Json<SubmitRunFeedbackRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    let snapshot = state
        .store
        .submit_run_feedback(
            run_id,
            request.command_id,
            request.expected_state_revision,
            request.text,
            request.citations,
        )
        .await?;
    enqueue_queued_writing_run(&state, &snapshot.workflow).await?;
    Ok(Json(snapshot))
}

pub(crate) async fn answer_question(
    State(state): State<AppState>,
    AxumPath(question_id): AxumPath<nekowriter_core::QuestionId>,
    Json(request): Json<AnswerQuestionRequest>,
) -> Result<Json<nekowriter_store::QuestionAnswerResult>, ApiError> {
    let response = state
        .store
        .answer_user_question(
            question_id,
            request.command_id,
            request.expected_state_revision,
            UserQuestionAnswers {
                answers: request.answers,
            },
        )
        .await?;
    enqueue_queued_writing_run(&state, &response.workflow).await?;
    Ok(Json(response))
}

pub(crate) async fn continue_writing_run(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<nekowriter_core::RunId>,
    Json(request): Json<RunWorkflowControlRequest>,
) -> Result<Json<WorkflowSnapshot>, ApiError> {
    let project_id = state.store.project_id_for_run(run_id).await?;
    ensure_model_ready(&state, project_id).await?;
    let workflow = state
        .store
        .continue_writing_run(run_id, request.command_id, request.expected_state_revision)
        .await?;
    enqueue_queued_writing_run(&state, &workflow).await?;
    Ok(Json(workflow))
}

pub(crate) async fn stop_writing_run(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<nekowriter_core::RunId>,
    Json(request): Json<RunWorkflowControlRequest>,
) -> Result<Json<WorkflowSnapshot>, ApiError> {
    let stopped = state
        .store
        .stop_writing_run(run_id, request.command_id, request.expected_state_revision)
        .await?;
    state
        .scheduler
        .stop_run(
            stopped.workflow.version_id,
            run_id,
            &stopped.cancelled_request_ids,
        )
        .await;
    Ok(Json(stopped.workflow))
}

pub(crate) async fn request_chapter_body_pause(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<nekowriter_core::RunId>,
    Json(request): Json<RunWorkflowControlRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    Ok(Json(
        state
            .store
            .request_chapter_body_pause(run_id, request.command_id, request.expected_state_revision)
            .await?,
    ))
}

pub(crate) async fn resume_chapter_body_pause(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<nekowriter_core::RunId>,
    Json(request): Json<RunWorkflowControlRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    let snapshot = state
        .store
        .resume_chapter_body_pause(run_id, request.command_id, request.expected_state_revision)
        .await?;
    enqueue_queued_writing_run(&state, &snapshot.workflow).await?;
    Ok(Json(snapshot))
}

pub(crate) async fn set_approval_mode(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<SetApprovalModeRequest>,
) -> Result<Json<WorkflowSnapshot>, ApiError> {
    Ok(Json(
        state
            .store
            .set_approval_mode(
                version_id,
                request.command_id,
                request.expected_state_revision,
                request.mode,
            )
            .await?,
    ))
}

pub(crate) async fn submit_chapter_final_review(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
    Json(request): Json<SubmitChapterFinalReviewRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    Ok(Json(
        state
            .store
            .submit_chapter_final_review(
                chapter_id,
                request.command_id,
                request.expected_state_revision,
            )
            .await?,
    ))
}
