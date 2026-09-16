//! 准备阶段 handler：快照、小说简介、准备启动、产物提交与人工审核。
//!
//! 从 `app/src/main.rs` 整块搬来（`docs/08` 的 P1.8b），代码一行未改，只有可见性从私有放宽到
//! `pub(crate)`（`http::router` 要挂这些路由）。
//!
//! 本文件在 `main.rs` 里是**两段**：`get_preparation`…`save_novel_brief`…`start_preparation`，
//! 以及 `submit_preparation_artifacts`…`cancel_review_delegation`；中间隔着两个跨域 helper
//! （它们在 `http::handlers` 根上，见那里的文档）。本文件按原顺序排列，去掉了那两个 helper。

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
};
use nekowriter_core::{
    ChapterPlanDraft, ExecutionStatus, HumanReviewDecision, NewNovelBrief,
    PreparationArtifactInput, PreparationSnapshot, ReviewId, VersionId,
};
use nekowriter_store::StoreError;
use serde_json::json;
use tracing::warn;

use crate::http::dto::{
    CancelReviewDelegationRequest, DecidePreparationReviewRequest, DelegateReviewRequest,
    SaveNovelBriefRequest, StartPreparationRequest, SubmitPreparationArtifactsRequest,
};
use crate::http::error::ApiError;
use crate::http::handlers::{enqueue_queued_writing_run, ensure_model_ready};
use crate::http::state::AppState;

pub(crate) async fn get_preparation(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    Ok(Json(
        state.store.get_preparation_snapshot(version_id).await?,
    ))
}

pub(crate) async fn save_novel_brief(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<SaveNovelBriefRequest>,
) -> Result<Json<nekowriter_core::CurrentNovelBrief>, ApiError> {
    let input = NewNovelBrief::new(request.content)?;
    Ok(Json(
        state
            .store
            .save_novel_brief(version_id, request.command_id, input)
            .await?,
    ))
}

pub(crate) async fn start_preparation(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<StartPreparationRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    let project_id = state.store.project_id_for_version(version_id).await?;
    ensure_model_ready(&state, project_id).await?;
    let snapshot = state
        .store
        .start_preparation(
            version_id,
            request.command_id,
            request.expected_state_revision,
        )
        .await?;
    if snapshot.workflow.execution == ExecutionStatus::Queued {
        let run_id = snapshot.workflow.active_run_id.ok_or_else(|| {
            StoreError::InvalidPreparationState("准备阶段缺少活跃 Run".to_owned())
        })?;
        state
            .scheduler
            .enqueue_preparation(version_id, run_id)
            .await?;
    }
    Ok(Json(snapshot))
}

pub(crate) async fn submit_preparation_artifacts(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<SubmitPreparationArtifactsRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    let artifacts = request
        .artifacts
        .into_iter()
        .map(|artifact| PreparationArtifactInput::new(artifact.kind, artifact.content))
        .collect::<Result<Vec<_>, _>>()?;
    let chapter_plan = request
        .chapter_plan
        .map(|drafts| {
            drafts
                .into_iter()
                .map(|draft| ChapterPlanDraft::new(draft.title, draft.goal))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    Ok(Json(
        state
            .store
            .submit_preparation_artifacts(
                version_id,
                request.command_id,
                request.expected_state_revision,
                artifacts,
                chapter_plan,
            )
            .await?,
    ))
}

pub(crate) async fn decide_review(
    State(state): State<AppState>,
    AxumPath(review_id): AxumPath<ReviewId>,
    Json(request): Json<DecidePreparationReviewRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    let decision = HumanReviewDecision::new(request.passed, request.issues.unwrap_or_default())?;
    let snapshot = state
        .store
        .decide_review(
            review_id,
            request.command_id,
            request.expected_state_revision,
            decision,
            request.citations,
        )
        .await?;
    enqueue_queued_writing_run(&state, &snapshot.workflow).await?;
    Ok(Json(snapshot))
}

pub(crate) async fn delegate_review(
    State(state): State<AppState>,
    AxumPath(review_id): AxumPath<ReviewId>,
    Json(request): Json<DelegateReviewRequest>,
) -> Result<(StatusCode, Json<nekowriter_store::ReviewDelegationResult>), ApiError> {
    let project_id = state.store.project_id_for_review(review_id).await?;
    ensure_model_ready(&state, project_id).await?;
    let delegation = state
        .store
        .delegate_review(
            review_id,
            request.command_id,
            request.expected_state_revision,
            request.notes,
            request.citations,
        )
        .await?;
    if let Err(error) = state
        .scheduler
        .enqueue_review(
            delegation.snapshot.workflow.version_id,
            delegation.review_id,
            delegation.run_id,
        )
        .await
    {
        let failure = json!({
            "kind": "scheduler_enqueue",
            "message": error.to_string(),
        });
        if let Err(recovery_error) = state
            .store
            .fail_delegated_review_attempt(
                delegation.review_id,
                delegation.run_id,
                None,
                None,
                failure,
                None,
                false,
            )
            .await
        {
            warn!(
                review_id = %delegation.review_id,
                run_id = %delegation.run_id,
                error = %recovery_error,
                "审核 Agent 调度失败后未能重新开放审核"
            );
        }
        return Err(error.into());
    }
    Ok((StatusCode::ACCEPTED, Json(delegation)))
}

pub(crate) async fn cancel_review_delegation(
    State(state): State<AppState>,
    AxumPath(review_id): AxumPath<ReviewId>,
    Json(request): Json<CancelReviewDelegationRequest>,
) -> Result<Json<nekowriter_store::CancelReviewDelegationResult>, ApiError> {
    let cancellation = state
        .store
        .cancel_review_delegation(
            review_id,
            request.command_id,
            request.expected_state_revision,
        )
        .await?;
    state
        .scheduler
        .stop_run(
            cancellation.snapshot.workflow.version_id,
            cancellation.run_id,
            &cancellation.cancelled_request_ids,
        )
        .await;
    Ok(Json(cancellation))
}
