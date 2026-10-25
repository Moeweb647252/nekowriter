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
    /// The resolved fastembed cache directory (`$HF_HOME` or `<data_dir>/models`, spec/01 §6).
    /// Settings › Embeddings shows it read-only (PROJECT §8.12); spec/10 §3 documents the field.
    pub model_cache_dir: String,
    pub state: IndexState,
    /// Explains a `failed` state (spec/07 §4).
    pub message: Option<String>,
    /// Counts are `i64` in Rust but cross the wire as JSON numbers, which `JSON.parse` always
    /// yields as `number`, so the exported TS type is `number` rather than `bigint`.
    #[ts(type = "number")]
    pub pending: i64,
    #[ts(type = "number")]
    pub failed: i64,
    #[ts(type = "number")]
    pub done: i64,
}

/// Embedder/index state (spec/07 §4: `not_loaded → loading (downloading) → ready | failed`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum IndexState {
    /// No embedding model is configured yet, so nothing is indexed (the M0 default).
    #[default]
    NotLoaded,
    /// The model is loading or downloading.
    Loading,
    /// The model is loaded and the index is usable.
    Ready,
    /// Loading or downloading failed; `message` explains why.
    Failed,
}

pub async fn health(State(state): State<AppState>) -> Json<HealthDto> {
    Json(HealthDto {
        ok: true,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        vec_version: state.vec_version().to_owned(),
        embedding: state.search().status_dto().await,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire values are the ones spec/07 §4 names; the frontend switches on them.
    #[test]
    fn index_state_wire_values_match_the_spec() {
        let values: Vec<String> = [
            IndexState::NotLoaded,
            IndexState::Loading,
            IndexState::Ready,
            IndexState::Failed,
        ]
        .into_iter()
        .map(|state| serde_json::to_string(&state).expect("serialize state"))
        .collect();
        assert_eq!(
            values,
            vec!["\"not_loaded\"", "\"loading\"", "\"ready\"", "\"failed\""]
        );
    }

    /// `failed` carries its reason in `message` (spec/07 §4).
    #[test]
    fn failed_state_serializes_with_a_message() {
        let dto = IndexStatusDto {
            model: "multilingual-e5-small".to_owned(),
            model_cache_dir: "/data/models".to_owned(),
            state: IndexState::Failed,
            message: Some("download failed".to_owned()),
            pending: 0,
            failed: 0,
            done: 0,
        };
        let json = serde_json::to_value(&dto).expect("serialize");
        assert_eq!(json["state"], "failed");
        assert_eq!(json["message"], "download failed");
    }
}
