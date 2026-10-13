//! Request extractors whose rejections are [`ApiError`]s, so a malformed request gets the
//! same error body as every other failure.

use axum::extract::{
    FromRequest, FromRequestParts,
    rejection::{JsonRejection, PathRejection, QueryRejection},
};

use super::error::{ApiError, ApiProblem};

/// A JSON request body.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub(crate) struct Json<T>(pub(crate) T);

/// Path parameters.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub(crate) struct Path<T>(pub(crate) T);

/// Query parameters.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(ApiError))]
pub(crate) struct Query<T>(pub(crate) T);

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        Self::new(ApiProblem::MalformedRequest, rejection.body_text())
    }
}

impl From<PathRejection> for ApiError {
    fn from(rejection: PathRejection) -> Self {
        Self::new(ApiProblem::MalformedRequest, rejection.body_text())
    }
}

impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> Self {
        Self::new(ApiProblem::MalformedRequest, rejection.body_text())
    }
}
