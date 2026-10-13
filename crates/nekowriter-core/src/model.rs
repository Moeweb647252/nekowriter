//! Model settings: the model service, the model a project uses, and whether a request can
//! be made with them (`docs/product-spec.md` §13.3).

use std::fmt;

use sea_orm::{
    DeriveActiveEnum, DeriveValueType, EnumIter, FromJsonQueryResult, sea_query::StringLen,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use url::Url;

use crate::field::{Field, FieldError, FieldProblem, single_line, single_line_text};

single_line_text!(
    /// The model's identifier at the model service, such as `gpt-4.1`.
    ModelId,
    Field::ModelId,
    200
);

/// The root of a Chat Completions-compatible API, including its version prefix (for
/// example `https://api.example.com/v1/`). Endpoint paths are appended to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS, DeriveValueType)]
#[ts(export)]
pub struct ApiBaseUrl(String);

impl ApiBaseUrl {
    /// Accepts an absolute `http` or `https` URL without query or fragment. The result
    /// always ends with `/`.
    pub fn parse(value: &str) -> Result<Self, FieldError> {
        let invalid = || FieldError {
            field: Field::ApiBaseUrl,
            problem: FieldProblem::InvalidUrl,
        };
        let value = single_line(Field::ApiBaseUrl, value, 2_000)?;
        let mut url = Url::parse(&value).map_err(|_| invalid())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host().is_none()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid());
        }
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        Ok(Self(url.into()))
    }

    /// The URL of `endpoint` (such as `chat/completions`) under this root.
    #[must_use]
    pub fn endpoint(&self, endpoint: &str) -> String {
        format!("{}{endpoint}", self.0)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ApiBaseUrl {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// A secret that authenticates requests to the model service. It is stored locally and
/// never sent to clients, logged or written into request snapshots (§4 invariant 17).
#[derive(Clone, PartialEq, Eq, DeriveValueType)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn parse(value: &str) -> Result<Self, FieldError> {
        single_line(Field::ApiKey, value, 4_096).map(Self)
    }

    /// The secret itself, for the `Authorization` header only.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKey(<redacted>)")
    }
}

/// A model and what it supports. The capability flags are claims the connection test can
/// verify; requests rely on them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, FromJsonQueryResult)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModelProfile {
    pub model_id: ModelId,
    /// The model's context window, in tokens.
    pub context_window: u32,
    /// Tokens kept free in the context window for the model's answer.
    pub max_output_tokens: u32,
    /// Whether the model accepts `reasoning_effort`.
    pub supports_thinking: bool,
    /// Whether the service reports token usage at the end of a stream.
    pub reports_stream_usage: bool,
}

impl ModelProfile {
    pub const MIN_CONTEXT_WINDOW: u32 = 4_096;
    pub const MAX_CONTEXT_WINDOW: u32 = 10_000_000;
    pub const MIN_OUTPUT_TOKENS: u32 = 256;

    /// Checks the numeric limits: the context window is within bounds, and the reserved
    /// output leaves at least half of it for the input.
    pub fn validate(&self) -> Result<(), FieldError> {
        let out_of_range = |field, min, max| FieldError {
            field,
            problem: FieldProblem::OutOfRange { min, max },
        };
        if !(Self::MIN_CONTEXT_WINDOW..=Self::MAX_CONTEXT_WINDOW).contains(&self.context_window) {
            return Err(out_of_range(
                Field::ContextWindow,
                Self::MIN_CONTEXT_WINDOW,
                Self::MAX_CONTEXT_WINDOW,
            ));
        }
        let max_output = self.context_window / 2;
        if !(Self::MIN_OUTPUT_TOKENS..=max_output).contains(&self.max_output_tokens) {
            return Err(out_of_range(
                Field::MaxOutputTokens,
                Self::MIN_OUTPUT_TOKENS,
                max_output,
            ));
        }
        Ok(())
    }

    /// The tokens available for the request itself.
    #[must_use]
    pub fn input_budget(&self) -> u32 {
        self.context_window - self.max_output_tokens
    }
}

/// How much the model should reason before answering.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    TS,
    EnumIter,
    DeriveActiveEnum,
)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ThinkingLevel {
    /// No reasoning parameter is sent.
    #[default]
    Off,
    Low,
    Medium,
    High,
}

impl ThinkingLevel {
    /// The `reasoning_effort` value to send, if any.
    #[must_use]
    pub fn reasoning_effort(self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
        }
    }
}

/// Where requests go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelService {
    pub base_url: ApiBaseUrl,
    /// Absent for services that need no key, such as a local server.
    pub api_key: Option<ApiKey>,
}

/// Why no model request can be made with the current settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ModelProblem {
    /// The model service's address is not set.
    ServiceNotConfigured,
    /// Neither the project nor the system settings name a model.
    ModelNotConfigured,
    /// The project asks for reasoning, but its model does not support it.
    ThinkingNotSupported,
}

/// Everything a request needs to reach the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveModel {
    pub service: ModelService,
    pub profile: ModelProfile,
    pub thinking: ThinkingLevel,
}

/// Resolves the model a project's requests use: the project's own model if it names one,
/// otherwise the system default, checked against the requested thinking level.
pub fn effective_model(
    service: Option<ModelService>,
    system_default: Option<ModelProfile>,
    project_model: Option<ModelProfile>,
    thinking: ThinkingLevel,
) -> Result<EffectiveModel, ModelProblem> {
    let service = service.ok_or(ModelProblem::ServiceNotConfigured)?;
    let profile = project_model
        .or(system_default)
        .ok_or(ModelProblem::ModelNotConfigured)?;
    if thinking != ThinkingLevel::Off && !profile.supports_thinking {
        return Err(ModelProblem::ThinkingNotSupported);
    }
    Ok(EffectiveModel {
        service,
        profile,
        thinking,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_url_is_normalized_to_end_with_a_slash() {
        let url = ApiBaseUrl::parse(" https://api.example.com/v1 ").unwrap();
        assert_eq!(url.as_str(), "https://api.example.com/v1/");
        assert_eq!(
            url.endpoint("chat/completions"),
            "https://api.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn a_base_url_must_be_plain_http() {
        for value in [
            "ftp://example.com/v1",
            "https://example.com/v1?key=1",
            "not a url",
            "http://",
        ] {
            assert_eq!(
                ApiBaseUrl::parse(value).unwrap_err().problem,
                FieldProblem::InvalidUrl,
                "{value}"
            );
        }
    }

    #[test]
    fn the_api_key_never_appears_in_debug_output() {
        let key = ApiKey::parse("sk-secret").unwrap();
        assert!(!format!("{key:?}").contains("secret"));
    }

    fn profile(supports_thinking: bool) -> ModelProfile {
        ModelProfile {
            model_id: ModelId::parse("writer").unwrap(),
            context_window: 128_000,
            max_output_tokens: 8_000,
            supports_thinking,
            reports_stream_usage: true,
        }
    }

    fn service() -> ModelService {
        ModelService {
            base_url: ApiBaseUrl::parse("http://127.0.0.1:8000/v1").unwrap(),
            api_key: None,
        }
    }

    #[test]
    fn the_project_model_overrides_the_system_default() {
        let mut own = profile(false);
        own.model_id = ModelId::parse("own").unwrap();
        let model = effective_model(
            Some(service()),
            Some(profile(false)),
            Some(own),
            ThinkingLevel::Off,
        )
        .unwrap();
        assert_eq!(model.profile.model_id.as_str(), "own");
    }

    #[test]
    fn requests_need_a_service_a_model_and_a_supported_thinking_level() {
        assert_eq!(
            effective_model(None, Some(profile(true)), None, ThinkingLevel::Off).unwrap_err(),
            ModelProblem::ServiceNotConfigured
        );
        assert_eq!(
            effective_model(Some(service()), None, None, ThinkingLevel::Off).unwrap_err(),
            ModelProblem::ModelNotConfigured
        );
        assert_eq!(
            effective_model(
                Some(service()),
                Some(profile(false)),
                None,
                ThinkingLevel::High
            )
            .unwrap_err(),
            ModelProblem::ThinkingNotSupported
        );
    }

    #[test]
    fn the_reserved_output_must_leave_room_for_the_input() {
        let mut too_greedy = profile(false);
        too_greedy.max_output_tokens = 100_000;
        assert_eq!(
            too_greedy.validate().unwrap_err().field,
            Field::MaxOutputTokens
        );
        assert!(profile(false).validate().is_ok());
    }
}
