//! Error responses.
//!
//! An error response says what went wrong as a code with typed parameters, never as prose:
//! the web UI renders it in the user's language. The English description of the underlying
//! cause is logged under the response's request ID.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use nekowriter_core::{
    conflict::{Resource, StateConflict},
    field::FieldError,
    model::ModelProblem,
    question::AnswerProblem,
    run::ProviderFailure,
};
use nekowriter_store::StoreError;
use serde::Serialize;
use ts_rs::TS;
use uuid::Uuid;

/// The body of every error response.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApiErrorBody {
    #[serde(flatten)]
    pub problem: ApiProblem,
    /// Identifies the log entry that describes the cause.
    pub request_id: String,
}

/// What went wrong, as a code and its parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(
    tag = "code",
    content = "params",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ApiProblem {
    /// A field value failed validation.
    FieldInvalid(FieldError),
    /// The answers do not fit the questions they answer.
    AnswerInvalid { problem: AnswerProblem },
    /// The referenced object does not exist or is deleted.
    NotFound { resource: Resource },
    /// The client acted on an outdated copy of the object and must reload it.
    RevisionConflict { resource: Resource },
    /// The command ID was already used for a different request.
    IdempotencyConflict,
    /// The request conflicts with the current state.
    StateConflict(StateConflict),
    /// The model settings do not allow model requests.
    ModelNotReady { problem: ModelProblem },
    /// The model service did not answer a connection test properly.
    ModelServiceFailed { failure: ProviderFailure },
    /// The service is shutting down, so it abandoned the request.
    ServiceStopping,
    /// The request is not valid JSON or does not have the expected shape.
    MalformedRequest,
    /// No API route matches the request.
    RouteNotFound,
    /// The request's `Host` header is not a loopback address.
    UntrustedHost,
    /// The request's `Origin` is not the web UI.
    UntrustedOrigin,
    /// An unexpected failure; the log has the details.
    Internal,
}

impl ApiProblem {
    fn status(&self) -> StatusCode {
        match self {
            Self::FieldInvalid(_) | Self::AnswerInvalid { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::NotFound { .. } | Self::RouteNotFound => StatusCode::NOT_FOUND,
            Self::RevisionConflict { .. }
            | Self::IdempotencyConflict
            | Self::StateConflict(_)
            | Self::ModelNotReady { .. } => StatusCode::CONFLICT,
            Self::ModelServiceFailed {
                failure: ProviderFailure::Timeout { .. },
            } => StatusCode::GATEWAY_TIMEOUT,
            Self::ModelServiceFailed { .. } => StatusCode::BAD_GATEWAY,
            Self::ServiceStopping => StatusCode::SERVICE_UNAVAILABLE,
            Self::MalformedRequest => StatusCode::BAD_REQUEST,
            Self::UntrustedHost | Self::UntrustedOrigin => StatusCode::FORBIDDEN,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// An error a handler returns: the problem to report and, for the log, its cause.
#[derive(Debug)]
pub(crate) struct ApiError {
    problem: ApiProblem,
    cause: Option<String>,
}

impl ApiError {
    pub(crate) fn new(problem: ApiProblem, cause: impl Into<Option<String>>) -> Self {
        Self {
            problem,
            cause: cause.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let request_id = Uuid::now_v7().to_string();
        let status = self.problem.status();
        let cause = self.cause.as_deref().unwrap_or("");
        if status.is_server_error() {
            tracing::error!(%request_id, problem = ?self.problem, cause, "request failed");
        } else {
            tracing::info!(%request_id, problem = ?self.problem, cause, "request rejected");
        }
        let body = ApiErrorBody {
            problem: self.problem,
            request_id,
        };
        (status, Json(body)).into_response()
    }
}

impl From<FieldError> for ApiError {
    fn from(error: FieldError) -> Self {
        Self::new(ApiProblem::FieldInvalid(error), None)
    }
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        let cause = error.to_string();
        let problem = match error {
            StoreError::NotFound(resource) => ApiProblem::NotFound { resource },
            StoreError::RevisionConflict { resource, .. } => {
                ApiProblem::RevisionConflict { resource }
            }
            StoreError::IdempotencyConflict(_) => ApiProblem::IdempotencyConflict,
            StoreError::StateConflict(conflict) => ApiProblem::StateConflict(conflict),
            StoreError::ModelNotReady(problem) => ApiProblem::ModelNotReady { problem },
            StoreError::AnswerInvalid(problem) => ApiProblem::AnswerInvalid { problem },
            StoreError::Database(_) | StoreError::Corrupt(_) => ApiProblem::Internal,
        };
        Self::new(problem, cause)
    }
}

/// The fallback for unknown `/api` routes, so they answer with an error body instead of
/// the web UI.
pub(crate) async fn route_not_found() -> ApiError {
    ApiError::new(ApiProblem::RouteNotFound, None)
}

#[cfg(test)]
mod tests {
    use nekowriter_core::field::{Field, FieldProblem};
    use serde_json::json;

    use super::*;

    fn body(problem: ApiProblem) -> serde_json::Value {
        serde_json::to_value(ApiErrorBody {
            problem,
            request_id: "r".to_owned(),
        })
        .unwrap()
    }

    #[test]
    fn a_problem_serializes_as_code_and_params() {
        assert_eq!(
            body(ApiProblem::FieldInvalid(FieldError {
                field: Field::ProjectName,
                problem: FieldProblem::TooLong { max_chars: 100 },
            })),
            json!({
                "code": "FIELD_INVALID",
                "params": {
                    "field": "project_name",
                    "problem": { "kind": "too_long", "maxChars": 100 }
                },
                "requestId": "r"
            })
        );
        assert_eq!(
            body(ApiProblem::NotFound {
                resource: Resource::Project
            }),
            json!({ "code": "NOT_FOUND", "params": { "resource": "project" }, "requestId": "r" })
        );
        assert_eq!(
            body(ApiProblem::IdempotencyConflict),
            json!({ "code": "IDEMPOTENCY_CONFLICT", "requestId": "r" })
        );
    }
}
