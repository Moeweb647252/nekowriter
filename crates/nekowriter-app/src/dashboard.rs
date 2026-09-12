//! Dashboard 的 HTTP 表面。
//!
//! 查询参数只描述用户意图（时区、天数、项目），时间区间换算与日期分桶都在
//! `nekowriter_core::DashboardRange` 中完成，前端不参与猜测“今天”。

use axum::{
    Json,
    extract::{Query, State},
};
use nekowriter_core::{DEFAULT_DASHBOARD_RANGE_DAYS, Dashboard, DashboardRange, utc_now_millis};
use serde::Deserialize;
use uuid::Uuid;

use crate::{ApiError, AppState};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardRequest {
    /// 用户时区相对 UTC 的偏移（分钟）；省略表示 UTC。
    #[serde(default)]
    pub timezone_offset_minutes: i32,
    /// 统计天数（含今天）；省略使用默认值。
    #[serde(default)]
    pub days: Option<i64>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

pub async fn get_dashboard(
    State(state): State<AppState>,
    Query(request): Query<DashboardRequest>,
) -> Result<Json<Dashboard>, ApiError> {
    let range = DashboardRange::new(
        utc_now_millis(),
        request.timezone_offset_minutes,
        request.days.unwrap_or(DEFAULT_DASHBOARD_RANGE_DAYS),
    )?;
    Ok(Json(
        state
            .store
            .compute_dashboard(range, request.project_id)
            .await?,
    ))
}
