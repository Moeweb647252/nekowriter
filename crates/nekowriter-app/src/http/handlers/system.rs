//! Diagnostics and shutdown.

use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;
use ts_rs::TS;

use crate::{VERSION, http::AppState};

/// Facts about the running service, for the diagnostics view.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HealthView {
    pub app_version: String,
    pub data_dir: String,
    pub log_dir: String,
    /// Whether this build serves the web UI itself (release builds do).
    pub ui_embedded: bool,
}

pub(crate) async fn health(State(state): State<AppState>) -> Json<HealthView> {
    Json(HealthView {
        app_version: VERSION.to_owned(),
        data_dir: state.diagnostics.data_dir.display().to_string(),
        log_dir: state.diagnostics.log_dir.display().to_string(),
        ui_embedded: crate::http::ui_embedded(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ShutdownStatus {
    ShuttingDown,
    AlreadyShuttingDown,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShutdownView {
    pub status: ShutdownStatus,
}

/// Stops the service. The response is sent before connections close.
pub(crate) async fn shutdown(State(state): State<AppState>) -> (StatusCode, Json<ShutdownView>) {
    let status = if state.shutdown.is_triggered() {
        ShutdownStatus::AlreadyShuttingDown
    } else {
        tracing::info!("shutdown requested through the web UI");
        state.shutdown.trigger();
        ShutdownStatus::ShuttingDown
    };
    (StatusCode::ACCEPTED, Json(ShutdownView { status }))
}
