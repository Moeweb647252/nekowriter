//! 用户提示词模板的 HTTP 表面。
//!
//! 模板是应用级的独立内容，不属于任何稿本；保存与修改都携带 `command_id` 与各自
//! 的 `expected_revision`，校验与渲染统一由 `nekowriter_core::template` 完成。

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
};
use nekowriter_core::{NewPromptTemplate, PromptTemplate, TemplateVariable, decimal_i64};
use serde::Deserialize;
use uuid::Uuid;

use crate::{ApiError, AppState};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePromptTemplateRequest {
    pub command_id: Uuid,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub variables: Vec<TemplateVariable>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePromptTemplateRequest {
    pub command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub expected_revision: i64,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub variables: Vec<TemplateVariable>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePromptTemplateRequest {
    pub command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub expected_revision: i64,
}

pub async fn list_prompt_templates(
    State(state): State<AppState>,
) -> Result<Json<Vec<PromptTemplate>>, ApiError> {
    Ok(Json(state.store.list_prompt_templates().await?))
}

pub async fn get_prompt_template(
    State(state): State<AppState>,
    AxumPath(template_id): AxumPath<Uuid>,
) -> Result<Json<PromptTemplate>, ApiError> {
    Ok(Json(state.store.get_prompt_template(template_id).await?))
}

pub async fn create_prompt_template(
    State(state): State<AppState>,
    Json(request): Json<CreatePromptTemplateRequest>,
) -> Result<(StatusCode, Json<PromptTemplate>), ApiError> {
    let template = state
        .store
        .create_prompt_template(
            request.command_id,
            NewPromptTemplate {
                title: request.title,
                body: request.body,
                variables: request.variables,
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(template)))
}

pub async fn update_prompt_template(
    State(state): State<AppState>,
    AxumPath(template_id): AxumPath<Uuid>,
    Json(request): Json<UpdatePromptTemplateRequest>,
) -> Result<Json<PromptTemplate>, ApiError> {
    let template = state
        .store
        .update_prompt_template(
            template_id,
            request.command_id,
            request.expected_revision,
            NewPromptTemplate {
                title: request.title,
                body: request.body,
                variables: request.variables,
            },
        )
        .await?;
    Ok(Json(template))
}

pub async fn delete_prompt_template(
    State(state): State<AppState>,
    AxumPath(template_id): AxumPath<Uuid>,
    Json(request): Json<DeletePromptTemplateRequest>,
) -> Result<Json<PromptTemplate>, ApiError> {
    let template = state
        .store
        .delete_prompt_template(template_id, request.command_id, request.expected_revision)
        .await?;
    Ok(Json(template))
}
