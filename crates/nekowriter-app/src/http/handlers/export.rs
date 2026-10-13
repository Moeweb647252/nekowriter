//! Downloading a version's novel (`docs/product-spec.md` §16.4).

use axum::{
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};
use nekowriter_core::{ids::VersionId, time::Timestamp};
use serde::Deserialize;

use crate::{
    export::{self, ExportFormat},
    http::{
        AppState,
        error::ApiError,
        extract::{Path, Query},
    },
};

#[derive(Debug, Deserialize)]
pub(crate) struct ExportQuery {
    format: ExportFormat,
}

/// The version's committed chapters as a file named after the novel.
pub(crate) async fn export(
    State(state): State<AppState>,
    Path(version_id): Path<VersionId>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    let novel = state.store.export_version(version_id).await?;
    let body = export::render(&novel, query.format, Timestamp::now());
    Ok((
        [
            (header::CONTENT_TYPE, query.format.content_type().to_owned()),
            (
                header::CONTENT_DISPOSITION,
                export::content_disposition(&novel.novel_title, query.format),
            ),
        ],
        body,
    )
        .into_response())
}
