//! 提示词面 handler：内置提示词的目录与覆盖，以及覆盖后的预览。
//!
//! 从 `app/src/main.rs` 整块搬来（`docs/08` 的 P1.6），代码一行未改，只有可见性从私有放宽到
//! `pub(crate)`（`http::router` 要挂这些路由）；`save_prompt_override_command` 与
//! `reset_prompt_override_command` 只被本模块的系统/项目四个 handler 使用，保持私有。
//!
//! 系统面与项目面共用同一条校验与落库路径（`save_prompt_override_command`），差别只在
//! `PromptOverrideScope` 与是否带 `project_id`——因此它们同属一个文件。

use axum::{
    Json,
    extract::{Path as AxumPath, State},
};

use nekowriter_agent::validate_builtin_prompt_pair;
use nekowriter_core::ProjectId;
use nekowriter_store::{
    NewPromptOverride, PromptOverride, PromptOverrideReset, PromptOverrideScope,
};

use crate::http::dto::{ResetPromptOverrideRequest, SavePromptOverrideRequest};
use crate::http::error::ApiError;
use crate::http::state::AppState;
use crate::prompts;

pub(crate) async fn get_system_prompts(
    State(state): State<AppState>,
) -> Result<Json<Vec<prompts::PromptCatalogEntry>>, ApiError> {
    Ok(Json(prompts::prompt_catalog(&state.store, None).await?))
}

pub(crate) async fn get_project_prompts(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<ProjectId>,
) -> Result<Json<Vec<prompts::PromptCatalogEntry>>, ApiError> {
    Ok(Json(
        prompts::prompt_catalog(&state.store, Some(project_id)).await?,
    ))
}

/// 保存覆盖前先按受限模板语言校验，拒绝未知变量与缺失的必需占位符。
async fn save_prompt_override_command(
    state: &AppState,
    scope: PromptOverrideScope,
    project_id: Option<ProjectId>,
    prompt_key: &str,
    request: SavePromptOverrideRequest,
) -> Result<Json<PromptOverride>, ApiError> {
    let key = prompts::parse_prompt_key(prompt_key)?;
    validate_builtin_prompt_pair(key, &request.system, &request.user)?;
    Ok(Json(
        state
            .store
            .save_prompt_override(
                request.command_id,
                request.expected_revision,
                NewPromptOverride {
                    prompt_key: key.as_str().to_owned(),
                    scope,
                    project_id,
                    system: request.system,
                    user: request.user,
                },
            )
            .await?,
    ))
}

async fn reset_prompt_override_command(
    state: &AppState,
    scope: PromptOverrideScope,
    project_id: Option<ProjectId>,
    prompt_key: &str,
    request: ResetPromptOverrideRequest,
) -> Result<Json<PromptOverrideReset>, ApiError> {
    let key = prompts::parse_prompt_key(prompt_key)?;
    Ok(Json(
        state
            .store
            .reset_prompt_override(
                request.command_id,
                request.expected_revision,
                scope,
                project_id,
                key.as_str(),
            )
            .await?,
    ))
}

pub(crate) async fn save_system_prompt_override(
    State(state): State<AppState>,
    AxumPath(prompt_key): AxumPath<String>,
    Json(request): Json<SavePromptOverrideRequest>,
) -> Result<Json<PromptOverride>, ApiError> {
    save_prompt_override_command(
        &state,
        PromptOverrideScope::System,
        None,
        &prompt_key,
        request,
    )
    .await
}

pub(crate) async fn reset_system_prompt_override(
    State(state): State<AppState>,
    AxumPath(prompt_key): AxumPath<String>,
    Json(request): Json<ResetPromptOverrideRequest>,
) -> Result<Json<PromptOverrideReset>, ApiError> {
    reset_prompt_override_command(
        &state,
        PromptOverrideScope::System,
        None,
        &prompt_key,
        request,
    )
    .await
}

pub(crate) async fn save_project_prompt_override(
    State(state): State<AppState>,
    AxumPath((project_id, prompt_key)): AxumPath<(ProjectId, String)>,
    Json(request): Json<SavePromptOverrideRequest>,
) -> Result<Json<PromptOverride>, ApiError> {
    save_prompt_override_command(
        &state,
        PromptOverrideScope::Project,
        Some(project_id),
        &prompt_key,
        request,
    )
    .await
}

pub(crate) async fn reset_project_prompt_override(
    State(state): State<AppState>,
    AxumPath((project_id, prompt_key)): AxumPath<(ProjectId, String)>,
    Json(request): Json<ResetPromptOverrideRequest>,
) -> Result<Json<PromptOverrideReset>, ApiError> {
    reset_prompt_override_command(
        &state,
        PromptOverrideScope::Project,
        Some(project_id),
        &prompt_key,
        request,
    )
    .await
}

pub(crate) async fn preview_prompt(
    State(state): State<AppState>,
    Json(request): Json<prompts::PreviewPromptRequest>,
) -> Result<Json<prompts::PreviewPromptResponse>, ApiError> {
    Ok(Json(prompts::preview_prompt(&state.store, request).await?))
}
