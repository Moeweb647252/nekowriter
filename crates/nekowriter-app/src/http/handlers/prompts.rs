//! Built-in prompts and their overrides (`docs/product-spec.md` §13.1, §13.2).

use std::collections::BTreeMap;

use axum::extract::State;
use nekowriter_agent::prompts::builtin;
use nekowriter_core::{
    ids::{CommandId, ProjectId},
    prompt::{PromptContent, PromptKey, PromptRole, PromptVariableSpec},
    template::Template,
};
use nekowriter_store::PromptSaved;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::http::{
    AppState,
    error::ApiError,
    extract::{Json, Path, Query},
};

/// Every prompt with its built-in text and its overrides.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PromptsView {
    /// What a save at the asked scope quotes: the system settings' revision, or the
    /// project's.
    pub revision: i64,
    pub prompts: Vec<PromptView>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PromptView {
    pub key: PromptKey,
    pub role: PromptRole,
    pub builtin: String,
    pub system: Option<String>,
    /// The project's override; only when a project was asked for.
    pub project: Option<String>,
    pub variables: Vec<PromptVariableSpec>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PromptsParams {
    project: Option<ProjectId>,
}

pub(crate) async fn get(
    State(state): State<AppState>,
    Query(params): Query<PromptsParams>,
) -> Result<axum::Json<PromptsView>, ApiError> {
    let read = state.store.prompts(params.project).await?;
    let prompts = PromptKey::ALL
        .into_iter()
        .flat_map(|key| [PromptRole::System, PromptRole::User].map(|role| (key, role)))
        .map(|(key, role)| PromptView {
            key,
            role,
            builtin: builtin(key, role).to_owned(),
            system: read.overrides.system(key, role).map(str::to_owned),
            project: read.overrides.project(key, role).map(str::to_owned),
            variables: key.variables(role).to_vec(),
        })
        .collect();
    Ok(axum::Json(PromptsView {
        revision: read.revision,
        prompts,
    }))
}

/// Sets or resets an override; a `null` content deletes it, so the next level applies.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavePromptRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    pub key: PromptKey,
    pub role: PromptRole,
    pub content: Option<String>,
}

impl SavePromptRequest {
    fn content(&self) -> Result<Option<PromptContent>, ApiError> {
        self.content
            .as_deref()
            .map(|content| PromptContent::parse(self.key, self.role, content))
            .transpose()
            .map_err(ApiError::from)
    }
}

pub(crate) async fn save_system(
    State(state): State<AppState>,
    Json(request): Json<SavePromptRequest>,
) -> Result<axum::Json<PromptSaved>, ApiError> {
    let content = request.content()?;
    Ok(axum::Json(
        state
            .store
            .save_system_prompt(
                request.command_id,
                request.expected_revision,
                request.key,
                request.role,
                content,
            )
            .await?,
    ))
}

pub(crate) async fn save_project(
    State(state): State<AppState>,
    Path(project_id): Path<ProjectId>,
    Json(request): Json<SavePromptRequest>,
) -> Result<axum::Json<PromptSaved>, ApiError> {
    let content = request.content()?;
    Ok(axum::Json(
        state
            .store
            .save_project_prompt(
                request.command_id,
                project_id,
                request.expected_revision,
                request.key,
                request.role,
                content,
            )
            .await?,
    ))
}

/// Renders a template with the given values, as the agents would (§13.2).
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenderTemplateRequest {
    pub template: String,
    pub values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenderedTemplate {
    pub text: String,
}

pub(crate) async fn render(
    Json(request): Json<RenderTemplateRequest>,
) -> Result<axum::Json<RenderedTemplate>, ApiError> {
    let text = Template::parse(&request.template)?
        .render(|name| request.values.get(name).map(String::as_str))?;
    Ok(axum::Json(RenderedTemplate { text }))
}
