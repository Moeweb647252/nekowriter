//! 回收站与番外删除的 HTTP 表面。
//!
//! 这里只做三件事：把路由参数解析成类型、把请求交给 Store、把结果原样返回。
//! 恢复与彻底删除的前提（父对象是否可见、对象是否还在回收站里）由 Store 判定，
//! 路由层不重复实现一份判断。

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use nekowriter_core::recycle::{RecycleBin, parse_recycle_item_kind};
use nekowriter_core::{ChapterId, PreparationSnapshot, ProjectId, VersionId, decimal_i64};
use serde::Deserialize;
use uuid::Uuid;

use crate::{ApiError, AppState, ensure_trusted_write_request};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecycleItemActionRequest {
    pub command_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteExtraChapterRequest {
    pub command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteProjectRequest {
    pub command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub expected_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteVersionRequest {
    pub command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub expected_revision: i64,
}

pub async fn get_recycle_bin(State(state): State<AppState>) -> Result<Json<RecycleBin>, ApiError> {
    Ok(Json(state.store.get_recycle_bin().await?))
}

pub async fn restore_recycle_item(
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    Json(request): Json<RecycleItemActionRequest>,
) -> Result<Json<RecycleBin>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let kind = parse_recycle_item_kind(&kind)?;
    Ok(Json(
        state
            .store
            .restore_recycle_item(kind, id, request.command_id)
            .await?,
    ))
}

pub async fn purge_recycle_item(
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    Json(request): Json<RecycleItemActionRequest>,
) -> Result<Json<RecycleBin>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let kind = parse_recycle_item_kind(&kind)?;
    Ok(Json(
        state
            .store
            .purge_recycle_item(kind, id, request.command_id)
            .await?,
    ))
}

/// 软删除一个项目；返回刷新后的回收站。
pub async fn delete_project(
    State(state): State<AppState>,
    Path(project_id): Path<ProjectId>,
    headers: HeaderMap,
    Json(request): Json<DeleteProjectRequest>,
) -> Result<Json<RecycleBin>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    Ok(Json(
        state
            .store
            .delete_project(project_id, request.command_id, request.expected_revision)
            .await?,
    ))
}

/// 软删除一份稿本；返回刷新后的回收站。
pub async fn delete_version(
    State(state): State<AppState>,
    Path(version_id): Path<VersionId>,
    headers: HeaderMap,
    Json(request): Json<DeleteVersionRequest>,
) -> Result<Json<RecycleBin>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    Ok(Json(
        state
            .store
            .delete_version(version_id, request.command_id, request.expected_revision)
            .await?,
    ))
}

/// 软删除一份番外章；正文计划章会被拒绝。
pub async fn delete_extra_chapter(
    State(state): State<AppState>,
    Path(chapter_id): Path<ChapterId>,
    headers: HeaderMap,
    Json(request): Json<DeleteExtraChapterRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    Ok(Json(
        state
            .store
            .delete_extra_chapter(
                chapter_id,
                request.command_id,
                request.expected_state_revision,
            )
            .await?,
    ))
}
