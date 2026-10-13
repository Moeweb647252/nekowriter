//! Project endpoints.

use axum::{extract::State, http::StatusCode};
use nekowriter_core::{
    ids::{CommandId, ProjectId},
    project::{NovelTitle, ProjectName, WritingLanguage},
};
use nekowriter_store::{NewProject, ProjectListFilter, ProjectView};
use serde::Deserialize;
use ts_rs::TS;

use crate::http::{
    AppState,
    error::ApiError,
    extract::{Json, Path, Query},
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

/// The request of commands that only need the project's current revision.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectRevisionRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
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
    Json(request): Json<ProjectRevisionRequest>,
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
    Json(request): Json<ProjectRevisionRequest>,
) -> Result<axum::Json<ProjectView>, ApiError> {
    let project = state
        .store
        .restore_project(request.command_id, id, request.expected_revision)
        .await?;
    Ok(axum::Json(project))
}
