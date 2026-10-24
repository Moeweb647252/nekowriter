//! Health endpoint (spec/10 §2.1).

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::state::AppState;

/// `GET /api/health`.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthDto {
    pub ok: bool,
    pub version: String,
    pub vec_version: String,
    pub embedding: IndexStatusDto,
}

/// Embedding/index state (spec/10 §3 `index.status`). Filled in from M7 onwards.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IndexStatusDto {
    pub model: String,
    pub state: IndexState,
    pub message: Option<String>,
    pub pending: i64,
    pub failed: i64,
    pub done: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum IndexState {
    #[default]
    Unavailable,
    Loading,
    Ready,
    Building,
}

pub async fn health(State(state): State<AppState>) -> Json<HealthDto> {
    Json(HealthDto {
        ok: true,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        vec_version: state.vec_version().to_owned(),
        embedding: IndexStatusDto {
            model: String::new(),
            state: IndexState::Unavailable,
            message: None,
            pending: 0,
            failed: 0,
            done: 0,
        },
    })
}
