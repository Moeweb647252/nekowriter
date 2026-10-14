//! User prompt templates (`docs/product-spec.md` §13.2).

use axum::{extract::State, http::StatusCode};
use nekowriter_core::{
    ids::{CommandId, PromptTemplateId},
    prompt_template::{TemplateContent, TemplateVariable},
};
use nekowriter_store::PromptTemplateView;
use serde::Deserialize;
use ts_rs::TS;

use crate::http::{
    AppState,
    error::ApiError,
    extract::{Json, Path},
    handlers::RevisionRequest,
};

/// A new template.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateTemplateRequest {
    pub command_id: CommandId,
    pub title: String,
    pub body: String,
    pub variables: Vec<TemplateVariable>,
}

/// A template's new content.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateTemplateRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    pub title: String,
    pub body: String,
    pub variables: Vec<TemplateVariable>,
}

pub(crate) async fn list(
    State(state): State<AppState>,
) -> Result<axum::Json<Vec<PromptTemplateView>>, ApiError> {
    Ok(axum::Json(state.store.list_templates().await?))
}

pub(crate) async fn create(
    State(state): State<AppState>,
    Json(request): Json<CreateTemplateRequest>,
) -> Result<(StatusCode, axum::Json<PromptTemplateView>), ApiError> {
    let content = TemplateContent::parse(&request.title, &request.body, request.variables)?;
    let template = state
        .store
        .create_template(request.command_id, content)
        .await?;
    Ok((StatusCode::CREATED, axum::Json(template)))
}

pub(crate) async fn update(
    State(state): State<AppState>,
    Path(id): Path<PromptTemplateId>,
    Json(request): Json<UpdateTemplateRequest>,
) -> Result<axum::Json<PromptTemplateView>, ApiError> {
    let content = TemplateContent::parse(&request.title, &request.body, request.variables)?;
    Ok(axum::Json(
        state
            .store
            .update_template(request.command_id, id, request.expected_revision, content)
            .await?,
    ))
}

pub(crate) async fn delete(
    State(state): State<AppState>,
    Path(id): Path<PromptTemplateId>,
    Json(request): Json<RevisionRequest>,
) -> Result<axum::Json<PromptTemplateView>, ApiError> {
    Ok(axum::Json(
        state
            .store
            .delete_template(request.command_id, id, request.expected_revision)
            .await?,
    ))
}
