//! The web-search settings endpoint (spec/10 §2.1).

use axum::Json;
use axum::extract::State;

use crate::api::dto::SearchTestInput;
use crate::api::dto::search::SearchTestResponse;
use crate::error::AppResult;
use crate::services::search_settings;
use crate::state::AppState;

/// `POST /api/app-settings/search/test` — one real search with the saved provider.
pub async fn test(
    State(state): State<AppState>,
    Json(input): Json<SearchTestInput>,
) -> AppResult<Json<SearchTestResponse>> {
    Ok(Json(
        search_settings::test_query(&state, &input.query).await?,
    ))
}
