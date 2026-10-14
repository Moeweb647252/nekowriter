//! What research and accumulation engines are available (`docs/product-spec.md` §12.3).

use axum::extract::State;
use nekowriter_core::engine::EngineList;
use serde::Serialize;
use ts_rs::TS;

use crate::http::AppState;

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EnginesView {
    pub research: EngineList,
    pub accumulation: EngineList,
}

pub(crate) async fn get(State(state): State<AppState>) -> axum::Json<EnginesView> {
    axum::Json(EnginesView {
        research: state.engines.research(),
        accumulation: state.engines.accumulation(),
    })
}
