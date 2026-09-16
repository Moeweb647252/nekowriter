//! 项目、版本、导出与项目设置 handler。
//!
//! 从 `app/src/main.rs` 整块搬来（`docs/08` 的 P1.7），代码一行未改，只有可见性从私有放宽到
//! `pub(crate)`（`http::router` 要挂这些路由）；`content_disposition` 只被 `export_version`
//! 使用，保持私有。
//!
//! 这批的来源在 `main.rs` 里是**三段不连续区间**：项目与版本查询（`list_projects` 起）、
//! 导出（`export_version` 与其文档、`content_disposition`）、创建与设置
//! （`create_project`…`save_project_settings`）。它们之间隔着事件面与章节/准备域，所以本文件
//! 里的顺序与原文件一致，只是把中间那些别的域的 handler 去掉了。

use std::fmt::Write as _;

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use nekowriter_core::{
    ExportFormat, NewCopiedNovelVersion, NewNovelVersion, NewProject, NewProjectSettings,
    ProjectBundle, ProjectId, ProjectListItem, ProjectSettings, VersionId, VersionSnapshot,
};

use crate::http::dto::{
    CopyVersionRequest, CreateProjectRequest, CreateVersionRequest, ExportQuery,
    SaveProjectSettingsRequest, SelectVersionRequest,
};
use crate::http::error::ApiError;
use crate::http::state::AppState;

pub(crate) async fn list_projects(
    State(state): State<AppState>,
) -> Result<Json<Vec<ProjectListItem>>, ApiError> {
    Ok(Json(state.store.list_projects().await?))
}

pub(crate) async fn get_project(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<ProjectId>,
) -> Result<Json<ProjectBundle>, ApiError> {
    Ok(Json(state.store.get_project(project_id).await?))
}

pub(crate) async fn list_versions(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<ProjectId>,
) -> Result<Json<Vec<nekowriter_core::NovelVersion>>, ApiError> {
    Ok(Json(state.store.list_versions(project_id).await?))
}

pub(crate) async fn get_version_snapshot(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
) -> Result<Json<VersionSnapshot>, ApiError> {
    Ok(Json(state.store.get_version_snapshot(version_id).await?))
}

/// 把稿本已提交正文导出成一个可下载的文件。
///
/// 内容是现读的：每次请求都在同一个读取快照里重新组装，不存在可以过期的导出副本。
pub(crate) async fn export_version(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    let format = ExportFormat::parse(query.format.as_deref().unwrap_or_default())?;
    let export = state.store.build_version_export(version_id).await?;
    let file_name = export.file_name(format);
    let mut response = export.render(format).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(format.media_type()),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&content_disposition(&file_name, format))
            .expect("导出文件名已去除控制字符"),
    );
    Ok(response)
}

/// RFC 6266 下载头：ASCII 回退名加上 UTF-8 百分号编码的完整名。
///
/// 中文标题不能直接写进 `filename=`，因此两者都给出：旧客户端用 ASCII 回退名，
/// 规范客户端用 `filename*` 得到原名。
fn content_disposition(file_name: &str, format: ExportFormat) -> String {
    let mut encoded = String::new();
    for byte in file_name.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(*byte));
        } else {
            write!(encoded, "%{byte:02X}").expect("写入 String 不会失败");
        }
    }
    format!(
        "attachment; filename=\"nekowriter-export.{}\"; filename*=UTF-8''{encoded}",
        format.extension()
    )
}

pub(crate) async fn create_project(
    State(state): State<AppState>,
    Json(request): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectBundle>), ApiError> {
    let input = NewProject::new(
        request.name,
        request.novel_title,
        request.version_label.as_deref(),
    )?;
    let bundle = state
        .store
        .create_project(request.command_id, input)
        .await?;
    Ok((StatusCode::CREATED, Json(bundle)))
}

pub(crate) async fn create_version(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<ProjectId>,
    Json(request): Json<CreateVersionRequest>,
) -> Result<(StatusCode, Json<ProjectBundle>), ApiError> {
    let input = NewNovelVersion::new(request.label, request.novel_title)?;
    let bundle = state
        .store
        .create_version(project_id, request.command_id, input)
        .await?;
    Ok((StatusCode::CREATED, Json(bundle)))
}

pub(crate) async fn select_version(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<ProjectId>,
    Json(request): Json<SelectVersionRequest>,
) -> Result<Json<ProjectBundle>, ApiError> {
    Ok(Json(
        state
            .store
            .select_version(
                project_id,
                request.version_id,
                request.command_id,
                request.expected_project_revision,
            )
            .await?,
    ))
}

pub(crate) async fn copy_version(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<CopyVersionRequest>,
) -> Result<(StatusCode, Json<ProjectBundle>), ApiError> {
    let input = NewCopiedNovelVersion::new(request.label)?;
    let bundle = state
        .store
        .copy_version(version_id, request.command_id, input)
        .await?;
    Ok((StatusCode::CREATED, Json(bundle)))
}

pub(crate) async fn get_project_settings(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<ProjectId>,
) -> Result<Json<ProjectSettings>, ApiError> {
    Ok(Json(state.store.get_project_settings(project_id).await?))
}

/// 项目设置写入与系统设置共用同一把短锁：调度器在读取有效设置到封存请求快照之间
/// 持有它，保存命令不会夹在中间，已 dispatch 的快照也不会被原地改写。
pub(crate) async fn save_project_settings(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<ProjectId>,
    Json(request): Json<SaveProjectSettingsRequest>,
) -> Result<Json<ProjectSettings>, ApiError> {
    let _guard = state.settings_write_lock.lock().await;
    Ok(Json(
        state
            .store
            .save_project_settings(
                project_id,
                request.command_id,
                request.expected_revision,
                NewProjectSettings {
                    model: request.model,
                    thinking_level: request.thinking_level,
                    auxiliary_tool_budget: request.auxiliary_tool_budget,
                },
            )
            .await?,
    ))
}
