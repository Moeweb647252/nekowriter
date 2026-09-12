//! 长篇阅读的 HTTP 表面。
//!
//! 阅读区按窗口取正文（`offset` / `limit`），目录与内容因此是两次独立查询：打开
//! 一部几百章的作品只读取要显示的那几章。查找是用户显式动作，它读遍全部已提交
//! 正文，返回的偏移与引用使用同一套单位。

use axum::{
    Json,
    extract::{Path, Query, State},
};
use nekowriter_core::{
    ManuscriptMatch, ManuscriptWindow, VersionId, parse_manuscript_window, parse_search_query,
};
use serde::Deserialize;

use crate::{ApiError, AppState};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManuscriptWindowRequest {
    /// 窗口第一个章节在阅读顺序中的下标；省略表示从第一章开始。
    #[serde(default)]
    pub offset: Option<i64>,
    /// 窗口长度；省略使用默认值。
    #[serde(default)]
    pub limit: Option<i64>,
}

pub async fn get_manuscript_window(
    State(state): State<AppState>,
    Path(version_id): Path<VersionId>,
    Query(request): Query<ManuscriptWindowRequest>,
) -> Result<Json<ManuscriptWindow>, ApiError> {
    let (offset, limit) = parse_manuscript_window(request.offset, request.limit)?;
    Ok(Json(
        state
            .store
            .read_manuscript_window(version_id, offset, limit)
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManuscriptSearchRequest {
    /// 查找文本；空或只有空白的查询由核心校验拒绝，而不是悄悄返回全部内容。
    #[serde(default)]
    pub q: Option<String>,
}

pub async fn search_manuscript(
    State(state): State<AppState>,
    Path(version_id): Path<VersionId>,
    Query(request): Query<ManuscriptSearchRequest>,
) -> Result<Json<Vec<ManuscriptMatch>>, ApiError> {
    let query = parse_search_query(request.q.as_deref().unwrap_or_default())?;
    Ok(Json(
        state.store.search_manuscript(version_id, &query).await?,
    ))
}
