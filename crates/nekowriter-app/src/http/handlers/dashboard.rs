//! The dashboard (`docs/product-spec.md` §15).

use axum::extract::State;
use nekowriter_core::{dashboard::TimeBuckets, ids::ProjectId, time::Timestamp};
use nekowriter_store::{DashboardQuery, DashboardView};
use serde::Deserialize;

use crate::http::{
    AppState,
    error::{ApiError, ApiProblem},
    extract::Query,
};

#[derive(Debug, Deserialize)]
pub(crate) struct DashboardParams {
    /// The time buckets' boundaries in milliseconds since the Unix epoch, comma-separated
    /// and increasing; the client picks them in the user's time zone.
    boundaries: String,
    /// Only this project; all live projects when absent.
    project: Option<ProjectId>,
}

pub(crate) async fn get(
    State(state): State<AppState>,
    Query(params): Query<DashboardParams>,
) -> Result<axum::Json<DashboardView>, ApiError> {
    let buckets = params
        .boundaries
        .split(',')
        .map(|boundary| boundary.parse().map(Timestamp::from_millis))
        .collect::<Result<Vec<_>, _>>()
        .ok()
        .and_then(TimeBuckets::new)
        .ok_or_else(|| {
            ApiError::new(
                ApiProblem::MalformedRequest,
                format!(
                    "the boundaries are not 2 to {} increasing timestamps",
                    TimeBuckets::MAX + 1
                ),
            )
        })?;
    let query = DashboardQuery {
        buckets,
        project: params.project,
    };
    Ok(axum::Json(state.store.dashboard(&query).await?))
}
