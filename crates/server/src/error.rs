//! `AppError` and the closed `ErrorCode` enum (spec/01 §8, spec/10 §4).

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// A closed set of error codes. Serialized as dotted strings; the frontend localizes by code
/// (spec/10 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ErrorCode {
    #[serde(rename = "validation.failed")]
    #[ts(rename = "validation.failed")]
    ValidationFailed,
    #[serde(rename = "validation.invalid_fields")]
    #[ts(rename = "validation.invalid_fields")]
    ValidationInvalidFields,
    #[serde(rename = "not_found")]
    #[ts(rename = "not_found")]
    NotFound,
    #[serde(rename = "host.forbidden")]
    #[ts(rename = "host.forbidden")]
    HostForbidden,
    #[serde(rename = "conflict.version")]
    #[ts(rename = "conflict.version")]
    ConflictVersion,
    #[serde(rename = "conflict.exists")]
    #[ts(rename = "conflict.exists")]
    ConflictExists,
    #[serde(rename = "timeline.has_children")]
    #[ts(rename = "timeline.has_children")]
    TimelineHasChildren,
    #[serde(rename = "timeline.status_conflict")]
    #[ts(rename = "timeline.status_conflict")]
    TimelineStatusConflict,
    #[serde(rename = "book.invalid_state")]
    #[ts(rename = "book.invalid_state")]
    BookInvalidState,
    #[serde(rename = "research.invalid_state")]
    #[ts(rename = "research.invalid_state")]
    ResearchInvalidState,
    #[serde(rename = "settings.locked")]
    #[ts(rename = "settings.locked")]
    SettingsLocked,
    #[serde(rename = "chapter.invalid_state")]
    #[ts(rename = "chapter.invalid_state")]
    ChapterInvalidState,
    #[serde(rename = "chapter.session_exists")]
    #[ts(rename = "chapter.session_exists")]
    ChapterSessionExists,
    #[serde(rename = "chapter.previous_not_ready")]
    #[ts(rename = "chapter.previous_not_ready")]
    ChapterPreviousNotReady,
    #[serde(rename = "chapter.is_anchor")]
    #[ts(rename = "chapter.is_anchor")]
    ChapterIsAnchor,
    #[serde(rename = "chapter.outline_missing")]
    #[ts(rename = "chapter.outline_missing")]
    ChapterOutlineMissing,
    #[serde(rename = "chapter.locked")]
    #[ts(rename = "chapter.locked")]
    ChapterLocked,
    #[serde(rename = "session.invalid_state")]
    #[ts(rename = "session.invalid_state")]
    SessionInvalidState,
    #[serde(rename = "session.exists")]
    #[ts(rename = "session.exists")]
    SessionExists,
    #[serde(rename = "model.missing_role")]
    #[ts(rename = "model.missing_role")]
    ModelMissingRole,
    #[serde(rename = "conception.incomplete")]
    #[ts(rename = "conception.incomplete")]
    ConceptionIncomplete,
    #[serde(rename = "track.mode_locked")]
    #[ts(rename = "track.mode_locked")]
    TrackModeLocked,
    #[serde(rename = "change.not_latest")]
    #[ts(rename = "change.not_latest")]
    ChangeNotLatest,
    #[serde(rename = "proposal.invalid_state")]
    #[ts(rename = "proposal.invalid_state")]
    ProposalInvalidState,
    #[serde(rename = "writing_skill.builtin")]
    #[ts(rename = "writing_skill.builtin")]
    WritingSkillBuiltin,
    #[serde(rename = "goal.open_children")]
    #[ts(rename = "goal.open_children")]
    GoalOpenChildren,
    #[serde(rename = "goal.has_children")]
    #[ts(rename = "goal.has_children")]
    GoalHasChildren,
    #[serde(rename = "prompt.template_invalid")]
    #[ts(rename = "prompt.template_invalid")]
    PromptTemplateInvalid,
    #[serde(rename = "provider.unavailable")]
    #[ts(rename = "provider.unavailable")]
    ProviderUnavailable,
    #[serde(rename = "provider.auth")]
    #[ts(rename = "provider.auth")]
    ProviderAuth,
    #[serde(rename = "provider.bad_request")]
    #[ts(rename = "provider.bad_request")]
    ProviderBadRequest,
    #[serde(rename = "search.not_configured")]
    #[ts(rename = "search.not_configured")]
    SearchNotConfigured,
    #[serde(rename = "internal")]
    #[ts(rename = "internal")]
    Internal,
}

impl ErrorCode {
    /// The dotted string form. Kept next to the serde attributes because tool results and logs
    /// need it without serializing.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::ValidationFailed => "validation.failed",
            ErrorCode::ValidationInvalidFields => "validation.invalid_fields",
            ErrorCode::NotFound => "not_found",
            ErrorCode::HostForbidden => "host.forbidden",
            ErrorCode::ConflictVersion => "conflict.version",
            ErrorCode::ConflictExists => "conflict.exists",
            ErrorCode::TimelineHasChildren => "timeline.has_children",
            ErrorCode::TimelineStatusConflict => "timeline.status_conflict",
            ErrorCode::BookInvalidState => "book.invalid_state",
            ErrorCode::ResearchInvalidState => "research.invalid_state",
            ErrorCode::SettingsLocked => "settings.locked",
            ErrorCode::ChapterInvalidState => "chapter.invalid_state",
            ErrorCode::ChapterSessionExists => "chapter.session_exists",
            ErrorCode::ChapterPreviousNotReady => "chapter.previous_not_ready",
            ErrorCode::ChapterIsAnchor => "chapter.is_anchor",
            ErrorCode::ChapterOutlineMissing => "chapter.outline_missing",
            ErrorCode::ChapterLocked => "chapter.locked",
            ErrorCode::SessionInvalidState => "session.invalid_state",
            ErrorCode::SessionExists => "session.exists",
            ErrorCode::ModelMissingRole => "model.missing_role",
            ErrorCode::ConceptionIncomplete => "conception.incomplete",
            ErrorCode::TrackModeLocked => "track.mode_locked",
            ErrorCode::ChangeNotLatest => "change.not_latest",
            ErrorCode::ProposalInvalidState => "proposal.invalid_state",
            ErrorCode::WritingSkillBuiltin => "writing_skill.builtin",
            ErrorCode::GoalOpenChildren => "goal.open_children",
            ErrorCode::GoalHasChildren => "goal.has_children",
            ErrorCode::PromptTemplateInvalid => "prompt.template_invalid",
            ErrorCode::ProviderUnavailable => "provider.unavailable",
            ErrorCode::ProviderAuth => "provider.auth",
            ErrorCode::ProviderBadRequest => "provider.bad_request",
            ErrorCode::SearchNotConfigured => "search.not_configured",
            ErrorCode::Internal => "internal",
        }
    }

    /// The HTTP status this code maps to (spec/01 §8).
    pub fn status(self) -> StatusCode {
        match self {
            ErrorCode::ValidationFailed | ErrorCode::PromptTemplateInvalid => {
                StatusCode::BAD_REQUEST
            }
            ErrorCode::ValidationInvalidFields => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::HostForbidden => StatusCode::FORBIDDEN,
            ErrorCode::ConflictVersion
            | ErrorCode::ConflictExists
            | ErrorCode::BookInvalidState
            | ErrorCode::ResearchInvalidState
            | ErrorCode::ChapterInvalidState
            | ErrorCode::ChapterSessionExists
            | ErrorCode::ChapterPreviousNotReady
            | ErrorCode::ChapterIsAnchor
            | ErrorCode::ChapterOutlineMissing
            | ErrorCode::SessionInvalidState
            | ErrorCode::SessionExists
            | ErrorCode::ModelMissingRole
            | ErrorCode::ConceptionIncomplete
            | ErrorCode::TrackModeLocked
            | ErrorCode::ChangeNotLatest
            | ErrorCode::ProposalInvalidState
            | ErrorCode::WritingSkillBuiltin
            | ErrorCode::GoalOpenChildren
            | ErrorCode::GoalHasChildren
            | ErrorCode::SearchNotConfigured
            | ErrorCode::TimelineHasChildren
            | ErrorCode::TimelineStatusConflict => StatusCode::CONFLICT,
            ErrorCode::SettingsLocked | ErrorCode::ChapterLocked => StatusCode::LOCKED,
            ErrorCode::ProviderUnavailable
            | ErrorCode::ProviderAuth
            | ErrorCode::ProviderBadRequest => StatusCode::BAD_GATEWAY,
            ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// The HTTP error type. `message` is English (internal detail for logs and fallback display);
/// the frontend localizes by `code` and falls back to `message`.
#[derive(Debug, Clone)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<Value>,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Replaces the (English, log-facing) message the fallback display shows.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ValidationFailed, message)
    }

    pub fn not_found(what: impl std::fmt::Display) -> Self {
        Self::new(ErrorCode::NotFound, format!("{what} not found"))
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    pub fn conflict(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::new(code, message)
    }

    /// A `validation.failed` error carrying per-field messages in `details.fields`.
    pub fn invalid_fields(fields: serde_json::Map<String, Value>) -> Self {
        Self::validation("invalid input")
            .with_details(serde_json::json!({ "fields": Value::Object(fields) }))
    }
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ErrorBody {
    pub error: ErrorPayload,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<Value>,
}

/// Set on responses built from an [`AppError`], so a later middleware can tell the documented
/// envelope apart from an extractor's bare rejection (see `api::rewrite_extractor_rejection`).
#[derive(Debug, Clone, Copy)]
pub struct ErrorEnvelope;

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.code.status();
        if status.is_server_error() {
            tracing::error!(code = self.code.as_str(), message = %self.message, "request failed");
        } else {
            tracing::debug!(code = self.code.as_str(), message = %self.message, "request rejected");
        }
        let body = ErrorBody {
            error: ErrorPayload {
                code: self.code,
                message: self.message,
                details: self.details,
            },
        };
        let mut response = (status, Json(body)).into_response();
        response.extensions_mut().insert(ErrorEnvelope);
        response
    }
}

impl From<sea_orm::DbErr> for AppError {
    fn from(err: sea_orm::DbErr) -> Self {
        AppError::internal(format!("database error: {err}"))
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for AppError {}

/// A request rejected before reaching a handler (host/origin guard, body parse).
pub type AppResult<T> = Result<T, AppError>;
