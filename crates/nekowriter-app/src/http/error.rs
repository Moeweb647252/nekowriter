//! HTTP 层的错误映射：`ApiError` 与它对外的错误信封。
//!
//! 从 `app/src/main.rs` 整块搬来（`docs/08` 的 P1.1），搬运过程中一行未改。`main.rs`
//! 保留同名的 `pub(crate) use http::error::ApiError;`，因此 `crate::ApiError` 的每个
//! 调用点（`knowledge.rs`、`recycle.rs`、`templates.rs`、`dashboard.rs`、`manuscript.rs`、
//! `prompts.rs`）都不用改。

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use nekowriter_agent::provider::ProviderError;
use nekowriter_core::{
    ChapterBodyEditError, DashboardQueryError, ExportQueryError, ManuscriptQueryError,
    PromptTemplateValidationError, TemplateError, ValidationError, recycle::RecycleQueryError,
};
use nekowriter_store::StoreError;
use serde::Serialize;
use serde_json::{Value, json};
use tracing::warn;
use uuid::Uuid;

use crate::scheduler;

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
    details: Value,
    request_id: String,
}

#[derive(Debug)]
pub(crate) enum ApiError {
    Validation(ValidationError),
    Store(StoreError),
    Embedding(nekowriter_agent::embedding::EmbeddingError),
    Engine(nekowriter_agent::engine::EngineError),
    Provider(ProviderError),
    Scheduler(scheduler::SchedulerError),
    InvalidEventCursor,
    UntrustedHost,
    UntrustedOrigin,
    NotFound,
    PromptTemplate(PromptTemplateValidationError),
    RecycleQuery(RecycleQueryError),
    DashboardQuery(DashboardQueryError),
    ExportQuery(ExportQueryError),
    ManuscriptQuery(ManuscriptQueryError),
    UnknownPromptKey(String),
}

impl From<ValidationError> for ApiError {
    fn from(value: ValidationError) -> Self {
        Self::Validation(value)
    }
}

impl From<TemplateError> for ApiError {
    fn from(value: TemplateError) -> Self {
        Self::PromptTemplate(value.into())
    }
}

impl From<PromptTemplateValidationError> for ApiError {
    fn from(value: PromptTemplateValidationError) -> Self {
        Self::PromptTemplate(value)
    }
}

impl From<DashboardQueryError> for ApiError {
    fn from(value: DashboardQueryError) -> Self {
        Self::DashboardQuery(value)
    }
}

impl From<ExportQueryError> for ApiError {
    fn from(value: ExportQueryError) -> Self {
        Self::ExportQuery(value)
    }
}

impl From<ManuscriptQueryError> for ApiError {
    fn from(value: ManuscriptQueryError) -> Self {
        Self::ManuscriptQuery(value)
    }
}

impl From<RecycleQueryError> for ApiError {
    fn from(value: RecycleQueryError) -> Self {
        Self::RecycleQuery(value)
    }
}

impl From<StoreError> for ApiError {
    fn from(value: StoreError) -> Self {
        Self::Store(value)
    }
}

impl From<nekowriter_agent::embedding::EmbeddingError> for ApiError {
    fn from(value: nekowriter_agent::embedding::EmbeddingError) -> Self {
        Self::Embedding(value)
    }
}

impl From<nekowriter_agent::engine::EngineError> for ApiError {
    fn from(value: nekowriter_agent::engine::EngineError) -> Self {
        Self::Engine(value)
    }
}

impl From<ProviderError> for ApiError {
    fn from(value: ProviderError) -> Self {
        Self::Provider(value)
    }
}

impl From<scheduler::SchedulerError> for ApiError {
    fn from(value: scheduler::SchedulerError) -> Self {
        Self::Scheduler(value)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(error) => write!(formatter, "{error}"),
            Self::Store(error) => write!(formatter, "{error}"),
            Self::Embedding(error) => write!(formatter, "{error}"),
            Self::Engine(error) => write!(formatter, "{error}"),
            Self::Provider(error) => write!(formatter, "{error}"),
            Self::Scheduler(error) => write!(formatter, "{error}"),
            Self::PromptTemplate(error) => write!(formatter, "{error}"),
            Self::DashboardQuery(error) => write!(formatter, "{error}"),
            Self::ExportQuery(error) => write!(formatter, "{error}"),
            Self::ManuscriptQuery(error) => write!(formatter, "{error}"),
            Self::RecycleQuery(error) => write!(formatter, "{error}"),
            Self::UnknownPromptKey(key) => write!(formatter, "未知提示词：{key}"),
            Self::InvalidEventCursor => formatter.write_str("事件游标无效"),
            Self::UntrustedHost => formatter.write_str("请求主机不是本地回环地址"),
            Self::UntrustedOrigin => formatter.write_str("请求来源不被信任"),
            Self::NotFound => formatter.write_str("请求的资源不存在"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let request_id = Uuid::new_v4().to_string();
        let (status, code, message, details) = match self {
            Self::Validation(error) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "VALIDATION_ERROR",
                error.to_string(),
                json!({ "reason": error.to_string() }),
            ),
            Self::Store(
                StoreError::ProjectNotFound
                | StoreError::VersionNotFound
                | StoreError::ChapterNotFound
                | StoreError::ReviewNotFound
                | StoreError::RunNotFound
                | StoreError::ModelRequestNotFound
                | StoreError::QuestionNotFound
                | StoreError::PromptTemplateNotFound
                | StoreError::DocumentRevisionNotFound
                | StoreError::RecycleItemNotFound,
            ) => (
                StatusCode::NOT_FOUND,
                "RESOURCE_NOT_FOUND",
                "请求的资源不存在或已删除".to_owned(),
                json!({}),
            ),
            Self::Store(StoreError::RecycleAncestorDeleted(message)) => (
                StatusCode::CONFLICT,
                "RECYCLE_ANCESTOR_DELETED",
                format!("{message}仍在回收站里，请先恢复它"),
                json!({ "reason": message }),
            ),
            Self::Store(StoreError::RecycleNotPurgeable(message)) => (
                StatusCode::CONFLICT,
                "RECYCLE_NOT_PURGEABLE",
                message.clone(),
                json!({ "reason": message }),
            ),
            Self::Store(StoreError::IdempotencyConflict) => (
                StatusCode::CONFLICT,
                "IDEMPOTENCY_CONFLICT",
                "command_id 已用于内容不同的请求".to_owned(),
                json!({}),
            ),
            Self::Store(StoreError::ResourceBusy(message)) => (
                StatusCode::CONFLICT,
                "RESOURCE_BUSY",
                message.clone(),
                json!({ "reason": message }),
            ),
            Self::Store(
                StoreError::InvalidPreparationState(message)
                | StoreError::InvalidChapterState(message)
                | StoreError::InvalidVersionState(message)
                | StoreError::InvalidModelRequestState(message)
                | StoreError::InvalidAgentMessageState(message),
            ) => (
                StatusCode::CONFLICT,
                "INVALID_TRANSITION",
                message.clone(),
                json!({ "reason": message }),
            ),
            Self::Store(StoreError::VersionRevisionConflict { expected, actual }) => (
                StatusCode::CONFLICT,
                "REVISION_CONFLICT",
                "稿本已经变化，请刷新后重试".to_owned(),
                json!({
                    "expected_revision": expected.to_string(),
                    "actual_revision": actual.to_string(),
                }),
            ),
            Self::Store(StoreError::StateRevisionConflict { expected, actual }) => (
                StatusCode::CONFLICT,
                "REVISION_CONFLICT",
                "当前工作流状态已经变化，请刷新后重试".to_owned(),
                json!({
                    "expected_state_revision": expected.to_string(),
                    "actual_state_revision": actual.to_string(),
                }),
            ),
            Self::Store(StoreError::PromptTemplateRevisionConflict { expected, actual }) => (
                StatusCode::CONFLICT,
                "REVISION_CONFLICT",
                "提示词模板已经变化，请刷新后重试".to_owned(),
                json!({
                    "expected_revision": expected.to_string(),
                    "actual_revision": actual.to_string(),
                }),
            ),
            Self::Store(StoreError::SystemSettingsRevisionConflict { expected, actual }) => (
                StatusCode::CONFLICT,
                "REVISION_CONFLICT",
                "系统模型设置已经变化，请刷新后重试".to_owned(),
                json!({
                    "expected_revision": expected.to_string(),
                    "actual_revision": actual.to_string(),
                }),
            ),
            Self::Store(StoreError::ReviewAlreadyResolved) => (
                StatusCode::CONFLICT,
                "INVALID_REVIEW_RESULT",
                "该审核已经有终态，请刷新后重试".to_owned(),
                json!({}),
            ),
            Self::Store(StoreError::ProposalNotFound) => (
                StatusCode::NOT_FOUND,
                "PROPOSAL_NOT_FOUND",
                "待审正文编辑不存在或已不属于当前章节".to_owned(),
                json!({}),
            ),
            Self::Store(StoreError::ProposalAlreadyResolved) => (
                StatusCode::CONFLICT,
                "INVALID_PROPOSAL_RESULT",
                "该正文编辑已经有终态，请刷新后重试".to_owned(),
                json!({}),
            ),
            Self::Store(StoreError::ProposalBaseRevisionConflict { expected, actual }) => (
                StatusCode::CONFLICT,
                "REVISION_CONFLICT",
                "正文基础修订已经变化，不能应用待审编辑".to_owned(),
                json!({
                    "expected_base_revision_id": expected.map(|revision_id| revision_id.to_string()),
                    "actual_base_revision_id": actual.map(|revision_id| revision_id.to_string()),
                }),
            ),
            Self::Store(StoreError::EmptyFeedback) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "EMPTY_FEEDBACK",
                "自由意见不能为空".to_owned(),
                json!({}),
            ),
            Self::Store(StoreError::InvalidReviewDecision(message)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_REVIEW_DECISION",
                message.clone(),
                json!({ "reason": message }),
            ),
            Self::Store(StoreError::InvalidCitation(message)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_CITATION",
                "引用无法对应当前可验证文本".to_owned(),
                json!({ "reason": message }),
            ),
            Self::Store(StoreError::EmbeddingNotConfigured) => (
                StatusCode::CONFLICT,
                "EMBEDDING_NOT_CONFIGURED",
                "尚未配置 Embedding，语义检索不可用；知识仍可正常保存".to_owned(),
                json!({}),
            ),
            Self::Engine(error) => (
                match &error {
                    nekowriter_agent::engine::EngineError::NotAvailable { .. } => {
                        StatusCode::CONFLICT
                    }
                    nekowriter_agent::engine::EngineError::Failed { .. } => StatusCode::BAD_GATEWAY,
                },
                error.code(),
                error.to_string(),
                json!({}),
            ),
            Self::Store(StoreError::EmbeddingDimensionMismatch { expected, actual }) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "EMBEDDING_DIMENSION_MISMATCH",
                "Embedding 服务返回的向量维度与配置不符".to_owned(),
                json!({
                    "expected": expected.to_string(),
                    "actual": actual.to_string(),
                }),
            ),
            Self::Store(
                StoreError::KnowledgeSpaceNotFound | StoreError::KnowledgeEntryNotFound,
            ) => (
                StatusCode::NOT_FOUND,
                "RESOURCE_NOT_FOUND",
                "请求的知识资源不存在或已删除".to_owned(),
                json!({}),
            ),
            Self::Store(StoreError::KnowledgeSpaceNotEmpty) => (
                StatusCode::CONFLICT,
                "KNOWLEDGE_SPACE_NOT_EMPTY",
                "该 Space 仍有条目，不能删除；请先移动或删除条目".to_owned(),
                json!({}),
            ),
            Self::Store(StoreError::KnowledgeSpaceRevisionConflict { expected, actual }) => (
                StatusCode::CONFLICT,
                "REVISION_CONFLICT",
                "知识 Space 已经变化，请刷新后重试".to_owned(),
                json!({
                    "expected_revision": expected.to_string(),
                    "actual_revision": actual.to_string(),
                }),
            ),
            Self::Store(StoreError::KnowledgeEntryRevisionConflict { expected, actual }) => (
                StatusCode::CONFLICT,
                "REVISION_CONFLICT",
                "知识条目已经变化，请刷新后重试".to_owned(),
                json!({
                    "expected_revision": expected.to_string(),
                    "actual_revision": actual.to_string(),
                }),
            ),
            Self::Store(StoreError::EmbeddingSettingsRevisionConflict { expected, actual }) => (
                StatusCode::CONFLICT,
                "REVISION_CONFLICT",
                "Embedding 配置已经变化，请刷新后重试".to_owned(),
                json!({
                    "expected_revision": expected.to_string(),
                    "actual_revision": actual.to_string(),
                }),
            ),
            Self::Store(
                StoreError::InvalidKnowledgeEntry(message)
                | StoreError::InvalidEmbeddingSettings(message)
                | StoreError::InvalidKnowledgeSearch(message)
                | StoreError::InvalidDashboardQuery(message)
                | StoreError::InvalidModelRequest(message)
                | StoreError::InvalidAgentMessage(message)
                | StoreError::InvalidSystemModelSettings(message),
            ) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "VALIDATION_ERROR",
                message.clone(),
                json!({ "reason": message }),
            ),
            Self::Store(StoreError::InvalidPromptTemplate(message)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_PROMPT_TEMPLATE",
                message.clone(),
                json!({ "reason": message }),
            ),
            Self::Store(StoreError::InvalidPromptOverride(message)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_PROMPT_OVERRIDE",
                message.clone(),
                json!({ "reason": message }),
            ),
            Self::Store(StoreError::PromptOverrideRevisionConflict { expected, actual }) => (
                StatusCode::CONFLICT,
                "REVISION_CONFLICT",
                "提示词覆盖已经变化，请刷新后重试".to_owned(),
                json!({
                    "expected_revision": expected.to_string(),
                    "actual_revision": actual.to_string(),
                }),
            ),
            Self::Store(StoreError::InvalidProjectSettings(message)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_PROJECT_SETTINGS",
                message.clone(),
                json!({ "reason": message }),
            ),
            Self::Store(StoreError::ProjectSettingsRevisionConflict { expected, actual }) => (
                StatusCode::CONFLICT,
                "REVISION_CONFLICT",
                "项目设置已经变化，请刷新后重试".to_owned(),
                json!({
                    "expected_revision": expected.to_string(),
                    "actual_revision": actual.to_string(),
                }),
            ),
            Self::PromptTemplate(error) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_PROMPT_TEMPLATE",
                error.to_string(),
                json!({ "reason": error.to_string() }),
            ),
            Self::DashboardQuery(error) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_DASHBOARD_QUERY",
                error.to_string(),
                json!({ "reason": error.to_string() }),
            ),
            Self::ExportQuery(error) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_EXPORT_FORMAT",
                error.to_string(),
                json!({ "reason": error.to_string() }),
            ),
            Self::RecycleQuery(error) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_RECYCLE_QUERY",
                error.to_string(),
                json!({ "reason": error.to_string() }),
            ),
            Self::ManuscriptQuery(error) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                match error {
                    ManuscriptQueryError::EmptyQuery | ManuscriptQueryError::QueryTooLong => {
                        "INVALID_SEARCH_QUERY"
                    }
                    ManuscriptQueryError::InvalidOffset { .. }
                    | ManuscriptQueryError::InvalidLimit { .. } => "INVALID_MANUSCRIPT_QUERY",
                },
                error.to_string(),
                json!({ "reason": error.to_string() }),
            ),
            Self::UnknownPromptKey(prompt_key) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "UNKNOWN_PROMPT_KEY",
                "请求的内建提示词键不在当前目录中".to_owned(),
                json!({ "prompt_key": prompt_key }),
            ),
            Self::Store(StoreError::QuestionAlreadyAnswered) => (
                StatusCode::CONFLICT,
                "QUESTION_ALREADY_ANSWERED",
                "该问题已经回答，请刷新后重试".to_owned(),
                json!({}),
            ),
            Self::Store(StoreError::InvalidQuestion(message)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_QUESTION_ANSWER",
                message.clone(),
                json!({ "reason": message }),
            ),
            Self::Store(StoreError::Workflow(error)) => (
                StatusCode::CONFLICT,
                "INVALID_TRANSITION",
                "当前工作流状态已经变化，请刷新后重试".to_owned(),
                json!({ "reason": error.to_string() }),
            ),
            Self::Store(
                StoreError::NovelBriefRequired | StoreError::InvalidPreparationArtifacts(_),
            ) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "VALIDATION_ERROR",
                "准备输入不完整或不符合当前阶段要求".to_owned(),
                json!({}),
            ),
            Self::Provider(error) => {
                warn!(request_id = %request_id, error = %error, "模型服务调用失败");
                (
                    StatusCode::BAD_GATEWAY,
                    "MODEL_TEST_FAILED",
                    "模型服务未通过实际连接测试；请检查地址、模型、密钥和能力配置".to_owned(),
                    json!({}),
                )
            }
            // 失败原因必须点名到具体一项：界面只展示 `message`，不读 `details`，一句
            // 「请检查地址、模型、维度与密钥」会让用户无法判断该改哪一个字段。适配器
            // 已经知道是维度不符、还是 HTTP 状态还是连不上，这里原样透出（`reason`
            // 供机器读取）。密钥从不进入这些文本。
            Self::Embedding(error) => {
                let message = error.to_string();
                let invalid_settings = matches!(
                    error,
                    nekowriter_agent::embedding::EmbeddingError::InvalidApiBaseUrl { .. }
                        | nekowriter_agent::embedding::EmbeddingError::EndpointInsteadOfApiBase
                        | nekowriter_agent::embedding::EmbeddingError::EmptyModelId
                        | nekowriter_agent::embedding::EmbeddingError::EmptyDimension
                );
                if invalid_settings {
                    (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "INVALID_EMBEDDING_SETTINGS",
                        message.clone(),
                        json!({ "reason": message }),
                    )
                } else {
                    warn!(request_id = %request_id, error = %error, "Embedding 服务调用失败");
                    (
                        StatusCode::BAD_GATEWAY,
                        "EMBEDDING_SERVICE_FAILED",
                        message.clone(),
                        json!({ "reason": message }),
                    )
                }
            }
            Self::Scheduler(error) => {
                warn!(request_id = %request_id, error = %error, "本地模型调度器操作失败");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "MODEL_SCHEDULER_UNAVAILABLE",
                    "本地模型调度器暂时无法启动；请检查模型设置后重试".to_owned(),
                    json!({}),
                )
            }
            Self::Store(StoreError::RequestAllowanceExhausted {
                request_count,
                request_allowance_end,
            }) => (
                StatusCode::CONFLICT,
                "REQUEST_LIMIT_EXCEEDED",
                "本轮模型请求额度已用完；请由用户明确继续该 Run".to_owned(),
                json!({
                    "request_count": request_count.to_string(),
                    "request_allowance_end": request_allowance_end.to_string(),
                }),
            ),
            Self::Store(StoreError::ChapterBodyEdit(ChapterBodyEditError::NoMatch)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "EDIT_NO_MATCH",
                "待替换正文在当前章节中没有匹配".to_owned(),
                json!({}),
            ),
            Self::Store(StoreError::ChapterBodyEdit(ChapterBodyEditError::MultipleMatches)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "EDIT_AMBIGUOUS_MATCH",
                "待替换正文在当前章节中匹配多处".to_owned(),
                json!({}),
            ),
            Self::Store(StoreError::ChapterBodyEdit(_)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "VALIDATION_ERROR",
                "正文编辑参数无效".to_owned(),
                json!({}),
            ),
            Self::Store(StoreError::InvalidChapterBodyEdit(_)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "VALIDATION_ERROR",
                "正文编辑参数无效或无法应用到当前内容".to_owned(),
                json!({}),
            ),
            Self::InvalidEventCursor => (
                StatusCode::BAD_REQUEST,
                "INVALID_EVENT_CURSOR",
                "事件游标必须是非负整数".to_owned(),
                json!({}),
            ),
            Self::Store(StoreError::UnsupportedSchema { found, expected }) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "SCHEMA_VERSION_UNSUPPORTED",
                format!("数据版本 {found} 不受当前程序支持（仅支持 {expected}）"),
                json!({ "found": found, "expected": expected.to_string() }),
            ),
            Self::Store(error) => {
                warn!(request_id = %request_id, error = %error, "API storage error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "LOCAL_STORAGE_ERROR",
                    "本地数据操作失败；请查看服务日志后重试".to_owned(),
                    json!({}),
                )
            }
            Self::UntrustedHost => (
                StatusCode::BAD_REQUEST,
                "UNTRUSTED_HOST",
                "只接受回环地址的请求".to_owned(),
                json!({}),
            ),
            Self::UntrustedOrigin => (
                StatusCode::FORBIDDEN,
                "UNTRUSTED_ORIGIN",
                "当前页面来源不被本地服务允许".to_owned(),
                json!({}),
            ),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "NOT_FOUND",
                "接口不存在".to_owned(),
                json!({}),
            ),
        };
        (
            status,
            Json(ErrorEnvelope {
                error: ErrorBody {
                    code,
                    message,
                    details,
                    request_id,
                },
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn conflict_errors_keep_stable_code_details_and_request_id() {
        let response = ApiError::Store(StoreError::StateRevisionConflict {
            expected: 4,
            actual: 5,
        })
        .into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], "REVISION_CONFLICT");
        assert_eq!(value["error"]["details"]["expected_state_revision"], "4");
        assert_eq!(value["error"]["details"]["actual_state_revision"], "5");
        assert!(Uuid::parse_str(value["error"]["request_id"].as_str().unwrap()).is_ok());
    }

    #[tokio::test]
    async fn invalid_review_decision_is_a_client_validation_error() {
        let response = ApiError::Store(StoreError::InvalidReviewDecision(
            "退回时需要理由或引用".to_owned(),
        ))
        .into_response();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], "INVALID_REVIEW_DECISION");
        assert_eq!(value["error"]["details"]["reason"], "退回时需要理由或引用");
    }

    #[tokio::test]
    async fn request_limit_errors_keep_decimal_details() {
        let response = ApiError::Store(StoreError::RequestAllowanceExhausted {
            request_count: i64::MAX - 1,
            request_allowance_end: i64::MAX - 1,
        })
        .into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], "REQUEST_LIMIT_EXCEEDED");
        assert_eq!(
            value["error"]["details"]["request_count"],
            (i64::MAX - 1).to_string()
        );
        assert_eq!(
            value["error"]["details"]["request_allowance_end"],
            (i64::MAX - 1).to_string()
        );
        assert!(Uuid::parse_str(value["error"]["request_id"].as_str().unwrap()).is_ok());
    }
}
