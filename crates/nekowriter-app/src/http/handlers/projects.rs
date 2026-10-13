//! Project endpoints, including the project's settings and versions.

use axum::{extract::State, http::StatusCode};
use nekowriter_core::{
    ids::{CommandId, ProjectId, VersionId},
    model::{ModelProfile, ThinkingLevel},
    project::{NovelTitle, ProjectName, WritingLanguage},
};
use nekowriter_store::{NewProject, ProjectListFilter, ProjectSettings, ProjectView};
use serde::Deserialize;
use ts_rs::TS;

use crate::http::{
    AppState,
    error::ApiError,
    extract::{Json, Path, Query},
    handlers::{RevisionRequest, settings::ModelProfileInput},
};

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ListProjectsQuery {
    /// Which projects to list; active ones when omitted.
    #[ts(optional)]
    pub filter: Option<ProjectListFilter>,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateProjectRequest {
    pub command_id: CommandId,
    pub name: String,
    pub novel_title: String,
    pub writing_language: WritingLanguage,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenameProjectRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateProjectSettingsRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    pub writing_language: WritingLanguage,
    /// The project's own model; `null` uses the system default.
    #[ts(as = "Option<ModelProfile>")]
    pub model: Option<ModelProfileInput>,
    pub thinking_level: ThinkingLevel,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateVersionRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    pub novel_title: String,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SelectVersionRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    pub version_id: VersionId,
}

pub(crate) async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListProjectsQuery>,
) -> Result<axum::Json<Vec<ProjectView>>, ApiError> {
    Ok(axum::Json(
        state
            .store
            .list_projects(query.filter.unwrap_or_default())
            .await?,
    ))
}

pub(crate) async fn create(
    State(state): State<AppState>,
    Json(request): Json<CreateProjectRequest>,
) -> Result<(StatusCode, axum::Json<ProjectView>), ApiError> {
    let input = NewProject {
        name: ProjectName::parse(&request.name)?,
        novel_title: NovelTitle::parse(&request.novel_title)?,
        writing_language: request.writing_language,
    };
    let project = state
        .store
        .create_project(request.command_id, input)
        .await?;
    Ok((StatusCode::CREATED, axum::Json(project)))
}

pub(crate) async fn rename(
    State(state): State<AppState>,
    Path(id): Path<ProjectId>,
    Json(request): Json<RenameProjectRequest>,
) -> Result<axum::Json<ProjectView>, ApiError> {
    let name = ProjectName::parse(&request.name)?;
    let project = state
        .store
        .rename_project(request.command_id, id, request.expected_revision, name)
        .await?;
    Ok(axum::Json(project))
}

pub(crate) async fn delete(
    State(state): State<AppState>,
    Path(id): Path<ProjectId>,
    Json(request): Json<RevisionRequest>,
) -> Result<axum::Json<ProjectView>, ApiError> {
    let project = state
        .store
        .delete_project(request.command_id, id, request.expected_revision)
        .await?;
    Ok(axum::Json(project))
}

pub(crate) async fn restore(
    State(state): State<AppState>,
    Path(id): Path<ProjectId>,
    Json(request): Json<RevisionRequest>,
) -> Result<axum::Json<ProjectView>, ApiError> {
    let project = state
        .store
        .restore_project(request.command_id, id, request.expected_revision)
        .await?;
    Ok(axum::Json(project))
}

pub(crate) async fn update_settings(
    State(state): State<AppState>,
    Path(id): Path<ProjectId>,
    Json(request): Json<UpdateProjectSettingsRequest>,
) -> Result<axum::Json<ProjectView>, ApiError> {
    let settings = ProjectSettings {
        writing_language: request.writing_language,
        model: request.model.map(ModelProfileInput::parse).transpose()?,
        thinking_level: request.thinking_level,
    };
    let project = state
        .store
        .update_project_settings(request.command_id, id, request.expected_revision, settings)
        .await?;
    Ok(axum::Json(project))
}

/// Creates an empty version and selects it.
pub(crate) async fn create_version(
    State(state): State<AppState>,
    Path(id): Path<ProjectId>,
    Json(request): Json<CreateVersionRequest>,
) -> Result<(StatusCode, axum::Json<ProjectView>), ApiError> {
    let novel_title = NovelTitle::parse(&request.novel_title)?;
    let project = state
        .store
        .create_version(
            request.command_id,
            id,
            request.expected_revision,
            novel_title,
        )
        .await?;
    Ok((StatusCode::CREATED, axum::Json(project)))
}

pub(crate) async fn select_version(
    State(state): State<AppState>,
    Path(id): Path<ProjectId>,
    Json(request): Json<SelectVersionRequest>,
) -> Result<axum::Json<ProjectView>, ApiError> {
    let project = state
        .store
        .select_version(
            request.command_id,
            id,
            request.expected_revision,
            request.version_id,
        )
        .await?;
    Ok(axum::Json(project))
}
