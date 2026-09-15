use std::{
    collections::{HashSet, VecDeque},
    convert::Infallible,
    fs::{File, OpenOptions},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

mod dashboard;
mod knowledge;
mod lifecycle;
mod manuscript;
mod model_settings;
mod operations;
mod prompts;
mod recycle;
mod scheduler;
mod templates;

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, put},
};
use clap::Parser;
use directories::ProjectDirs;
use fs4::{FileExt, TryLockError};
use futures_util::stream;
use nekowriter_agent::{
    ContextMessageRole, capabilities,
    engine::AccumulationEngine,
    provider::{
        ChatCompletionsAdapter, ChatCompletionsConfig, ModelCompletion, ModelFailureCode,
        ModelRequest, ModelRequestMessage, ModelTimeouts, ProviderError,
    },
    validate_builtin_prompt_pair,
};
use nekowriter_core::{
    AppendChapterContent, ApprovalMode, ChapterBodyEditError, ChapterId, ChapterPlanDraft,
    DashboardQueryError, DocumentId, ExecutionStatus, ExportFormat, ExportQueryError,
    FeedbackCitation, HumanReviewDecision, ManuscriptQueryError, ModelCapabilityProfile,
    NewChapterGuidance, NewChapterOutline, NewCopiedNovelVersion, NewExtraChapter, NewNovelBrief,
    NewNovelVersion, NewProject, NewProjectSettings, PreparationArtifactInput,
    PreparationArtifactKind, PreparationSnapshot, ProjectBundle, ProjectId, ProjectListItem,
    ProjectModelReference, ProjectSettings, PromptTemplateValidationError, ReplaceChapterContent,
    ReviewId, RevisionId, TemplateError, ThinkingLevel, UserQuestionAnswer, UserQuestionAnswers,
    ValidationError, VersionId, VersionSnapshot, WorkflowSnapshot, WorkflowStage, decimal_i64,
    utc_now_millis,
};
use nekowriter_store::{
    EventStreamBounds, InterruptionSource, NewPromptOverride, OPERATION_KIND_CONNECTION_TEST,
    PersistedEvent, PromptOverride, PromptOverrideReset, PromptOverrideScope, Store, StoreError,
};

use crate::lifecycle::Shutdown;
use crate::operations::SystemOperationResult;
use nekowriter_core::recycle::RecycleQueryError;

use crate::model_settings::{
    capability_problem_message, pre_dispatch_capability_problem, resolve_effective_model,
    thinking_probe_level,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::{Mutex, Semaphore, broadcast, watch};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing::{error, info, warn};
use uuid::Uuid;

const DEFAULT_PORT: u16 = 47_821;
const VITE_ORIGINS: [&str; 2] = ["http://127.0.0.1:5173", "http://localhost:5173"];

#[derive(Debug, Parser)]
#[command(name = "nekowriter", version, about = "NekoWriter 本地写作工具")]
struct Cli {
    /// 本地数据库、LanceDB 和日志所在目录。
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// 仅监听回环地址的 HTTP 端口。
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,
    /// 已构建前端资源目录；默认使用可执行文件旁的 webui/dist。
    #[arg(long)]
    web_dir: Option<PathBuf>,
    /// 不自动打开默认浏览器；无界面环境与自动化探针使用。
    #[arg(long)]
    no_browser: bool,
}

#[derive(Clone)]
struct AppState {
    store: Store,
    allowed_write_origins: Arc<HashSet<String>>,
    settings_write_lock: Arc<Mutex<()>>,
    model_request_permit: Arc<Semaphore>,
    scheduler: scheduler::WorkflowScheduler,
    /// 仅供当前 SSE 连接消费的模型字符流；它不进入持久事件日志。
    preview_deltas: broadcast::Sender<scheduler::PreviewDelta>,
    shutdown: Shutdown,
    /// 本次进程实际拥有的知识积累引擎；能力查询与积累入口读同一份。MVP 为空。
    accumulation_engines: &'static [&'static dyn AccumulationEngine],
}

struct EventStreamState {
    store: Store,
    receiver: broadcast::Receiver<()>,
    preview_receiver: broadcast::Receiver<scheduler::PreviewDelta>,
    shutdown: watch::Receiver<bool>,
    cursor: i64,
    pending: VecDeque<PersistedEvent>,
    send_connection_comment: bool,
}

#[derive(Debug, Error)]
enum StartupError {
    #[error("无法确定系统本地数据目录")]
    NoLocalDataDirectory,
    #[error("无法启动 NekoWriter：{0}")]
    Store(#[from] StoreError),
    #[error("文件系统错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("数据目录已被另一个 NekoWriter 进程占用：{0}")]
    DataDirectoryLocked(PathBuf),
    #[error("无法在数据目录创建日志文件：{0}")]
    LogFile(#[from] tracing_appender::rolling::InitError),
    #[error("无法定位当前可执行文件所在目录")]
    NoExecutableDirectory,
}

struct ProcessLock {
    _file: File,
}

impl ProcessLock {
    fn acquire(data_dir: &Path) -> Result<Self, StartupError> {
        let lock_path = data_dir.join("nekowriter.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        match FileExt::try_lock(&file) {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(StartupError::DataDirectoryLocked(lock_path)),
            Err(TryLockError::Error(error)) => Err(StartupError::Io(error)),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateProjectRequest {
    command_id: Uuid,
    name: String,
    novel_title: String,
    version_label: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateVersionRequest {
    command_id: Uuid,
    label: String,
    novel_title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelectVersionRequest {
    command_id: Uuid,
    version_id: VersionId,
    #[serde(with = "decimal_i64")]
    expected_project_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CopyVersionRequest {
    command_id: Uuid,
    label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveNovelBriefRequest {
    command_id: Uuid,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartPreparationRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreparationArtifactRequest {
    kind: PreparationArtifactKind,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterPlanDraftRequest {
    title: String,
    goal: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitPreparationArtifactsRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
    artifacts: Vec<PreparationArtifactRequest>,
    chapter_plan: Option<Vec<ChapterPlanDraftRequest>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DecidePreparationReviewRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
    passed: bool,
    issues: Option<String>,
    #[serde(default)]
    citations: Vec<FeedbackCitation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DelegateReviewRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    citations: Vec<FeedbackCitation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelReviewDelegationRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveChapterGuidanceRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartChapterOutlineRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitChapterOutlineRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppendChapterBodyRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplaceChapterBodyRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
    old_str: String,
    new_str: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetApprovalModeRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
    mode: ApprovalMode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DecideChapterBodyProposalRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
    #[serde(default)]
    feedback: String,
    citations: Vec<FeedbackCitation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitRunFeedbackRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
    text: String,
    citations: Vec<FeedbackCitation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnswerQuestionRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
    /// 一次询问的整批答案。
    ///
    /// 这里直接收列表，而不是领域类型 `UserQuestionAnswers`：后者的唯一字段也叫
    /// `answers`，把它当作同名字段会形成线上形状 `answers.answers`，与界面按本字段
    /// 发送的 `answers: [...]` 不一致。
    answers: Vec<UserQuestionAnswer>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunWorkflowControlRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitChapterFinalReviewRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdvanceToNextChapterRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfirmNovelCompletionRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddExtraChapterRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
    title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveSystemModelSettingsRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_revision: i64,
    api_base_url: String,
    model_id: String,
    capabilities: nekowriter_core::ModelCapabilityProfile,
    #[serde(with = "decimal_i64")]
    max_context_tokens: i64,
    #[serde(with = "decimal_i64")]
    reserved_output_tokens: i64,
    /// 三态：省略 = 保留既有密钥，`null` = 清回未配置，字符串 = 替换。
    #[serde(default)]
    api_key: nekowriter_store::CredentialPatch,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SystemSettingsResponse {
    writing_model: Option<PublicSystemModelSettings>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicSystemModelSettings {
    api_base_url: String,
    model_id: String,
    capabilities: nekowriter_core::ModelCapabilityProfile,
    max_context_tokens: String,
    reserved_output_tokens: String,
    revision: String,
    credential_configured: bool,
    created_at_ms: String,
    updated_at_ms: String,
}

/// 连接测试的请求体：明确要测试哪个模型、以及它声明了哪些能力。
///
/// 模型 ID 与能力资料一同给出，因为「是否接受 `reasoning_effort`」是模型的属性，
/// 而不是当前系统默认设置的属性。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestModelRequest {
    model_id: String,
    capabilities: ModelCapabilityProfile,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TestModelResponse {
    provider_request_id: Option<String>,
    content_received: bool,
    tool_call_count: usize,
    usage: Option<nekowriter_agent::provider::ModelUsage>,
    /// 本次探针是否真的带上了 `reasoning_effort`。只有它为真时，
    /// 「该模型支持思考」才是被验证过的声明。
    thinking_probed: bool,
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    after: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    status: &'static str,
    schema_version: u32,
    /// 当前进程实际使用的数据目录，界面用它告诉用户数据与日志在哪里。
    data_dir: String,
}

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
enum ApiError {
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
                | StoreError::InvalidDashboardQuery(message),
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
            Self::Store(
                StoreError::InvalidModelRequest(message)
                | StoreError::InvalidAgentMessage(message)
                | StoreError::InvalidSystemModelSettings(message),
            ) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "VALIDATION_ERROR",
                message.clone(),
                json!({ "reason": message }),
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

/// 启动过程中的一步：说明里点名是哪一步失败，并给出这一步能走的下一步。
///
/// 启动期的几步各自会以不同方式失败（路径不存在、只读、被占用、来自更早版本），用户
/// 需要的不是 Rust 错误行，而是「哪一步、为什么、现在能做什么」。
#[derive(Clone, Copy)]
enum StartupStep {
    DataDirectory,
    Logging,
    ProcessLock,
    Database,
    WebDirectory,
}

impl StartupStep {
    fn label(self) -> &'static str {
        match self {
            Self::DataDirectory => "无法准备数据目录",
            Self::Logging => "无法在该目录下创建日志",
            Self::ProcessLock => "无法锁定数据目录",
            Self::Database => "无法打开该目录里的数据库",
            Self::WebDirectory => "无法定位前端构建产物目录",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::DataDirectory => {
                "请确认这个路径存在且可写，然后重试；也可以改用另一个数据目录启动，例如：nekowriter --data-dir <新目录>"
            }
            Self::Logging => "请确认这个目录可写（应用需要它下面的 logs/ 子目录），然后重试",
            Self::ProcessLock => {
                "同一时间只允许一个进程打开同一个数据目录；请关闭正在运行的那个 NekoWriter，或改用另一个数据目录启动"
            }
            Self::Database => {
                "请确认这个目录可写且属于 NekoWriter；由更早版本创建的目录请用创建它的旧版本打开，或改用另一个数据目录启动"
            }
            Self::WebDirectory => "请用 --web-dir <目录> 明确指定前端构建产物所在目录",
        }
    }
}

/// 启动失败的统一出口：任何一步失败都经这里给出同一种说明，并保留原始错误细节。
///
/// `logs_dir` 只在该目录下的日志确实可写后给出：日志初始化之前没有可写的日志文件，
/// 说明因此不会声称原因已经记录在 `logs/`。返回值原样透传，进程仍以非零状态退出；
/// stderr 上随后出现的 Rust 原始错误行保留完整细节，供事后核对。
fn startup_failure(
    data_dir: Option<&Path>,
    logs_dir: Option<&Path>,
    step: StartupStep,
    error: StartupError,
) -> StartupError {
    error!(data_dir = ?data_dir, step = step.label(), %error, "启动失败");
    eprintln!(
        "{}",
        startup_failure_message(data_dir, logs_dir, step, &error)
    );
    error
}

/// 一次启动失败的原因与它自己能走的下一步。
///
/// 「旧目录」两种情形自带详细说明与前进路径，因此不再叠加步骤提示。
fn startup_failure_reason(
    step: StartupStep,
    error: &StartupError,
) -> (String, Option<&'static str>) {
    match error {
        StartupError::Store(StoreError::UnsupportedSchema { found, expected }) => (
            format!(
                "这个目录由更早的 NekoWriter 创建，数据库 schema 版本为 {found}，本版本只支持 {expected}。\n\
                 本版本不会读取或迁移其中的作品数据；要继续其中已有的作品，请用创建它的旧版本打开。\n\
                 要用本版本开始创作，请指定另一个数据目录启动，例如：nekowriter --data-dir <新目录>"
            ),
            None,
        ),
        StartupError::Store(StoreError::UnknownSchema) => (
            "这个目录里的数据库不是 NekoWriter 的数据（缺少 schema 标识）。\n\
             请确认 --data-dir 指向正确的数据目录，或换一个空目录启动本版本，例如：nekowriter --data-dir <新目录>"
                .to_owned(),
            None,
        ),
        StartupError::NoLocalDataDirectory => (
            "无法确定系统本地数据目录。".to_owned(),
            Some("请用 --data-dir <目录> 明确指定数据目录，例如：nekowriter --data-dir <新目录>"),
        ),
        other => (format!("{}（{other}）", step.label()), Some(step.hint())),
    }
}

/// 启动失败时给用户的说明：点名数据目录与失败的步骤、可诊断的原因与可以走的下一步。
///
/// 只在日志可写时才声明「往这个目录写了什么」，而且声明的内容就是实际写入的内容
/// （启动日志与进程锁文件），不用「不会改动它」这种与行为不符的说法。
fn startup_failure_message(
    data_dir: Option<&Path>,
    logs_dir: Option<&Path>,
    step: StartupStep,
    error: &StartupError,
) -> String {
    let (reason, hint) = startup_failure_reason(step, error);
    let mut lines = vec!["NekoWriter 无法启动。".to_owned()];
    if let Some(data_dir) = data_dir {
        lines.push(format!("数据目录：{}", data_dir.display()));
    }
    lines.push(format!("原因：{reason}"));
    if let Some(hint) = hint {
        lines.push(hint.to_owned());
    }
    if let Some(logs_dir) = logs_dir {
        lines.push(format!(
            "除了该目录原有的文件，本次启动只写入了启动日志与进程锁文件，作品数据（SQLite 与向量）未被修改；日志：{}。",
            logs_dir.display()
        ));
    }
    lines.join("\n")
}

/// 启动期准备好的东西：数据库句柄、目录，以及必须随进程存活的日志写入器与进程锁。
struct Startup {
    store: Store,
    data_dir: PathBuf,
    web_dir: PathBuf,
    /// 非阻塞日志写入器的守卫：drop 它会丢掉尚未落盘的日志行。
    _log_guard: tracing_appender::non_blocking::WorkerGuard,
    /// 进程锁：drop 它会释放 `nekowriter.lock`。
    _lock: ProcessLock,
}

/// 完成启动期的全部步骤：数据目录、日志、进程锁、数据库、前端产物。
///
/// 每一步失败都经 `startup_failure` 给出同一种可诊断说明，因此这里不出现裸 `?`：
/// 恰恰是这几步最容易因为「路径不存在、只读、被占用、来自更早版本」而失败，而那正是
/// 用户最需要看到「哪一步、为什么、现在能做什么」的时候。日志在拿到数据目录后立即
/// 落地，之后的所有诊断都会同时进入该目录的 `logs/`。
///
/// # Errors
///
/// 任意一步失败时返回原始错误；调用方只需要把它交给 `main` 的非零退出即可。
async fn prepare(cli: &Cli) -> Result<Startup, StartupError> {
    let requested_dir = match cli.data_dir.clone() {
        Some(path) => path,
        None => match default_data_dir() {
            Ok(path) => path,
            Err(error) => {
                return Err(startup_failure(
                    None,
                    None,
                    StartupStep::DataDirectory,
                    error,
                ));
            }
        },
    };
    let data_dir = match resolve_data_dir(requested_dir.clone()) {
        Ok(data_dir) => data_dir,
        Err(error) => {
            return Err(startup_failure(
                Some(&requested_dir),
                None,
                StartupStep::DataDirectory,
                error,
            ));
        }
    };
    let log_guard = match lifecycle::init_logging(&data_dir) {
        Ok(guard) => guard,
        // 日志这一层自己失败时没有可写的日志文件，说明只落到 stderr。
        Err(error) => {
            return Err(startup_failure(
                Some(&data_dir),
                None,
                StartupStep::Logging,
                error,
            ));
        }
    };
    let logs_dir = data_dir.join("logs");
    let lock = match ProcessLock::acquire(&data_dir) {
        Ok(lock) => lock,
        Err(error) => {
            return Err(startup_failure(
                Some(&data_dir),
                Some(&logs_dir),
                StartupStep::ProcessLock,
                error,
            ));
        }
    };
    let store = match Store::open(&data_dir).await {
        Ok(store) => store,
        Err(error) => {
            return Err(startup_failure(
                Some(&data_dir),
                Some(&logs_dir),
                StartupStep::Database,
                error.into(),
            ));
        }
    };
    let web_dir = match cli.web_dir.clone() {
        Some(path) => path,
        None => match default_web_dir() {
            Ok(path) => path,
            Err(error) => {
                return Err(startup_failure(
                    Some(&data_dir),
                    Some(&logs_dir),
                    StartupStep::WebDirectory,
                    error,
                ));
            }
        },
    };
    Ok(Startup {
        store,
        data_dir,
        web_dir,
        _log_guard: log_guard,
        _lock: lock,
    })
}

#[tokio::main]
async fn main() -> Result<(), StartupError> {
    let cli = Cli::parse();
    // `startup` 必须存活到进程结束：日志守卫与进程锁都在它里面。
    let startup = prepare(&cli).await?;
    let store = startup.store.clone();
    let data_dir = startup.data_dir.clone();
    let web_dir = startup.web_dir.clone();
    if !web_dir.exists() {
        warn!(path = %web_dir.display(), "未找到前端构建产物；可使用 pnpm --dir webui build 后重启，或在开发模式下运行 Vite");
    }

    let shutdown = Shutdown::new();
    let model_request_permit = Arc::new(Semaphore::new(1));
    let settings_write_lock = Arc::new(Mutex::new(()));
    let (preview_deltas, _) = broadcast::channel(256);
    let state = AppState {
        scheduler: scheduler::WorkflowScheduler::with_preview_deltas(
            store.clone(),
            model_request_permit.clone(),
            settings_write_lock.clone(),
            preview_deltas.clone(),
        ),
        store,
        allowed_write_origins: Arc::new(allowed_write_origins(cli.port)),
        settings_write_lock,
        model_request_permit,
        preview_deltas,
        shutdown: shutdown.clone(),
        accumulation_engines: nekowriter_agent::engine::ACCUMULATION_ENGINES,
    };
    // 上次进程退出时仍在 running 的索引任务重新排队；向量本身可由正文重建。
    let requeued = state.store.requeue_running_index_tasks().await?;
    if requeued > 0 {
        info!(requeued, "已重新排队未完成的知识索引任务");
    }
    let indexer_store = state.store.clone();
    let indexer = tokio::spawn(async move {
        knowledge::KnowledgeIndexer::new(indexer_store).run().await;
    });
    let app = router(state.clone(), web_dir);
    let address = SocketAddr::from(([127, 0, 0, 1], cli.port));
    let listener = tokio::net::TcpListener::bind(address).await?;
    info!(address = %address, data_dir = %data_dir.display(), "NekoWriter 已启动");
    // 长期增长口径：请求快照、命令回执与消息账本是审计事实，不自动回收；把各表当前的
    // 行数与重文本字节数写到日志，长年使用后的体积因此可诊断（见 docs/02 §11.3）。
    match state.store.storage_growth().await {
        Ok(growth) => info!(
            table = %growth
                .iter()
                .map(|table| format!("{}={}行/{}字节", table.table, table.rows, table.text_bytes))
                .collect::<Vec<_>>()
                .join(" "),
            "本地数据规模"
        ),
        Err(error) => warn!(%error, "本地数据规模统计失败"),
    }
    if !cli.no_browser {
        lifecycle::open_browser(cli.port);
    }

    // 退出信号（Ctrl+C / SIGTERM / `POST /api/system/shutdown`）一到达，服务器就
    // 停止接受新连接，因此退出流程不再产生新的生成请求。
    let serve = {
        let shutdown = shutdown.clone();
        async move { shutdown.wait().await }
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(serve)
        .await?;

    info!("正在取消在途请求并收尾状态");
    state.scheduler.cancel_in_flight().await;
    indexer.abort();
    let outcome = state
        .store
        .interrupt_active_runs(InterruptionSource::ProcessShutdown)
        .await?;
    state.store.close().await?;
    info!(
        writing_runs = outcome.writing_runs,
        review_delegations = outcome.review_delegations,
        "NekoWriter 已退出"
    );
    Ok(())
}

fn router(state: AppState, web_dir: PathBuf) -> Router {
    let root_index_html = web_dir.join("index.html");
    let explicit_index_html = root_index_html.clone();
    // 深链接回落用同一份入口 HTML：前端用 path 路由，磁盘上没有 `/works/...` 这样的文件。
    let fallback_index_html = root_index_html.clone();
    let static_files = web_dir.clone();
    let api = Router::new()
        .route("/health", get(health))
        .route("/system/shutdown", axum::routing::post(shutdown_service))
        .route("/capabilities", get(get_capabilities))
        .route(
            "/settings",
            get(get_system_settings).put(save_system_settings),
        )
        .route(
            "/settings/test-model",
            axum::routing::post(test_model_connection),
        )
        .route(
            "/settings/embedding",
            get(knowledge::get_embedding_settings).put(knowledge::save_embedding_settings),
        )
        .route(
            "/settings/test-embedding",
            axum::routing::post(knowledge::test_embedding_connection),
        )
        .route(
            "/knowledge/spaces",
            get(knowledge::list_knowledge_spaces).post(knowledge::create_knowledge_space),
        )
        .route(
            "/knowledge/spaces/{space_id}",
            axum::routing::patch(knowledge::rename_knowledge_space)
                .delete(knowledge::delete_knowledge_space),
        )
        .route(
            "/knowledge/entries",
            get(knowledge::list_knowledge_entries).post(knowledge::create_knowledge_entry),
        )
        .route(
            "/knowledge/entries/{entry_id}",
            get(knowledge::get_knowledge_entry)
                .put(knowledge::update_knowledge_entry)
                .delete(knowledge::delete_knowledge_entry),
        )
        .route(
            "/knowledge/search",
            axum::routing::post(knowledge::search_knowledge),
        )
        .route(
            "/knowledge/reindex",
            axum::routing::post(knowledge::reindex_knowledge),
        )
        .route(
            "/knowledge/accumulate",
            axum::routing::post(knowledge::accumulate_knowledge),
        )
        .route("/projects", get(list_projects).post(create_project))
        .route(
            "/projects/{project_id}",
            get(get_project).delete(recycle::delete_project),
        )
        .route(
            "/projects/{project_id}/settings",
            get(get_project_settings).put(save_project_settings),
        )
        .route(
            "/projects/{project_id}/versions",
            get(list_versions).post(create_version),
        )
        .route(
            "/projects/{project_id}/selected-version",
            axum::routing::post(select_version),
        )
        .route("/versions/{version_id}/snapshot", get(get_version_snapshot))
        .route("/versions/{version_id}/export", get(export_version))
        .route(
            "/versions/{version_id}/copy",
            axum::routing::post(copy_version),
        )
        .route(
            "/versions/{version_id}/approval-mode",
            axum::routing::post(set_approval_mode),
        )
        .route("/versions/{version_id}/chapters", get(list_chapters))
        .route(
            "/versions/{version_id}/manuscript",
            get(manuscript::get_manuscript_window),
        )
        .route(
            "/versions/{version_id}/manuscript/search",
            get(manuscript::search_manuscript),
        )
        .route(
            "/versions/{version_id}/chapters/next",
            axum::routing::post(advance_to_next_chapter),
        )
        .route(
            "/versions/{version_id}/completion",
            axum::routing::post(confirm_novel_completion),
        )
        .route(
            "/versions/{version_id}/extras",
            axum::routing::post(add_extra_chapter),
        )
        .route(
            "/chapters/{chapter_id}/guidance",
            get(get_chapter_guidance).put(save_chapter_guidance),
        )
        .route(
            "/chapters/{chapter_id}/outline/start",
            axum::routing::post(start_chapter_outline),
        )
        .route(
            "/chapters/{chapter_id}/outline",
            get(get_chapter_outline).post(submit_chapter_outline),
        )
        .route("/chapters/{chapter_id}/body", get(get_chapter_body))
        .route(
            "/documents/{document_id}/revisions/{revision_id}",
            get(get_document_revision),
        )
        .route(
            "/chapters/{chapter_id}/body/append",
            axum::routing::post(append_chapter_body),
        )
        .route(
            "/chapters/{chapter_id}/body/replace",
            axum::routing::post(replace_chapter_body),
        )
        .route(
            "/chapters/{chapter_id}/body/proposals/{tool_call_id}/decision",
            axum::routing::post(decide_chapter_body_proposal),
        )
        .route(
            "/runs/{run_id}/feedback",
            axum::routing::post(submit_run_feedback),
        )
        .route(
            "/runs/{run_id}/continue",
            axum::routing::post(continue_writing_run),
        )
        .route("/runs/{run_id}/stop", axum::routing::post(stop_writing_run))
        .route(
            "/questions/{question_id}/answer",
            axum::routing::post(answer_question),
        )
        .route(
            "/runs/{run_id}/pause",
            axum::routing::post(request_chapter_body_pause),
        )
        .route(
            "/runs/{run_id}/resume",
            axum::routing::post(resume_chapter_body_pause),
        )
        .route(
            "/chapters/{chapter_id}/body/review",
            axum::routing::post(submit_chapter_final_review),
        )
        .route("/events/cursor", get(get_event_cursor))
        .route("/events", get(stream_events))
        .route("/versions/{version_id}/preparation", get(get_preparation))
        .route("/versions/{version_id}/brief", put(save_novel_brief))
        .route("/prompts/system", get(get_system_prompts))
        .route(
            "/prompts/system/{prompt_key}",
            put(save_system_prompt_override).delete(reset_system_prompt_override),
        )
        .route("/prompts/preview", axum::routing::post(preview_prompt))
        .route(
            "/prompt-templates",
            get(templates::list_prompt_templates).post(templates::create_prompt_template),
        )
        .route(
            "/prompt-templates/{template_id}",
            get(templates::get_prompt_template)
                .patch(templates::update_prompt_template)
                .delete(templates::delete_prompt_template),
        )
        .route("/dashboard", get(dashboard::get_dashboard))
        .route("/recycle-bin", get(recycle::get_recycle_bin))
        .route(
            "/recycle-bin/{kind}/{id}/restore",
            axum::routing::post(recycle::restore_recycle_item),
        )
        .route(
            "/recycle-bin/{kind}/{id}/purge",
            axum::routing::post(recycle::purge_recycle_item),
        )
        .route(
            "/chapters/{chapter_id}",
            axum::routing::delete(recycle::delete_extra_chapter),
        )
        .route(
            "/versions/{version_id}",
            axum::routing::delete(recycle::delete_version),
        )
        .route("/projects/{project_id}/prompts", get(get_project_prompts))
        .route(
            "/projects/{project_id}/prompts/{prompt_key}",
            put(save_project_prompt_override).delete(reset_project_prompt_override),
        )
        .route(
            "/versions/{version_id}/preparation/start",
            axum::routing::post(start_preparation),
        )
        .route(
            "/versions/{version_id}/preparation/artifacts",
            axum::routing::post(submit_preparation_artifacts),
        )
        .route(
            "/reviews/{review_id}/decision",
            axum::routing::post(decide_review),
        )
        .route(
            "/reviews/{review_id}/delegate",
            axum::routing::post(delegate_review),
        )
        .route(
            "/reviews/{review_id}/cancel-delegation",
            axum::routing::post(cancel_review_delegation),
        )
        .fallback(api_not_found)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_trusted_write,
        ))
        .with_state(state);

    Router::new()
        .nest("/api", api)
        .route("/", get(move || serve_index_html(root_index_html.clone())))
        .route(
            "/index.html",
            get(move || serve_index_html(explicit_index_html.clone())),
        )
        .fallback_service(
            ServeDir::new(static_files)
                .append_index_html_on_directories(true)
                .fallback(get(move |uri: Uri| {
                    let index_html = fallback_index_html.clone();
                    async move { serve_client_route(uri, index_html).await }
                })),
        )
        .layer(TraceLayer::new_for_http())
}

/// 静态目录未命中时的回落：只有前端路由形的路径回落到入口 HTML。
///
/// 前端用 path 路由，磁盘上没有 `/works/<id>` 这样的文件，直接打开或刷新深链接必须由
/// 路由接管，否则用户拿到的是一个空白 404。带扩展名的路径是静态资源（脚本、样式、
/// 图片、字体）：磁盘上没有就如实 404，回落到 HTML 会把真正的缺失掩盖成界面故障——
/// 浏览器把 HTML 当脚本执行（MIME 不匹配）或把坏资源当成加载成功。
async fn serve_client_route(uri: Uri, index_html: PathBuf) -> Response {
    if is_static_asset_path(uri.path()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    serve_index_html(index_html).await
}

/// 末段带 `.` 的路径按静态资源看待；前端路由段（UUID、页面名）里不含点。
fn is_static_asset_path(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|segment| segment.contains('.'))
}

/// 入口 HTML 不能被启发式缓存：每次启动新版本时，它必须引用同一构建产物里的懒加载 chunk。
async fn serve_index_html(path: PathBuf) -> Response {
    match tokio::fs::read(&path).await {
        Ok(content) => (
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                ),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
            ],
            content,
        )
            .into_response(),
        Err(error) => {
            warn!(path = %path.display(), %error, "未找到前端入口文件");
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShutdownResponse {
    status: &'static str,
}

/// 显式退出本地服务：与进程信号走同一条收尾路径。
///
/// 响应先写回客户端，服务随后停止接受连接并取消在途请求；界面据此显示连接已断开。
async fn shutdown_service(
    State(state): State<AppState>,
) -> Result<Json<ShutdownResponse>, ApiError> {
    if state.shutdown.is_requested() {
        return Ok(Json(ShutdownResponse {
            status: "already_shutting_down",
        }));
    }
    state.shutdown.request_after_response();
    Ok(Json(ShutdownResponse {
        status: "shutting_down",
    }))
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        schema_version: state.store.schema_version(),
        data_dir: state.store.data_dir().display().to_string(),
    })
}

async fn get_capabilities(
    State(state): State<AppState>,
) -> Json<nekowriter_agent::AgentCapabilities> {
    Json(capabilities(state.accumulation_engines))
}

async fn get_system_settings(
    State(state): State<AppState>,
) -> Result<Json<SystemSettingsResponse>, ApiError> {
    Ok(Json(system_settings_response(
        state.store.get_system_model_settings().await?,
    )))
}

async fn save_system_settings(
    State(state): State<AppState>,
    Json(request): Json<SaveSystemModelSettingsRequest>,
) -> Result<Json<SystemSettingsResponse>, ApiError> {
    let _guard = state.settings_write_lock.lock().await;
    let current = state.store.get_system_model_settings().await?;
    let SaveSystemModelSettingsRequest {
        command_id,
        expected_revision,
        api_base_url,
        model_id,
        capabilities,
        max_context_tokens,
        reserved_output_tokens,
        api_key,
    } = request;

    let settings = nekowriter_store::NewSystemModelSettings {
        api_base_url,
        api_key,
        model_id,
        capabilities,
        max_context_tokens,
        reserved_output_tokens,
    };

    if let Some(replayed) = state
        .store
        .replay_system_model_settings_if_known(command_id, expected_revision, settings.clone())
        .await?
    {
        return Ok(Json(system_settings_response(Some(replayed))));
    }

    let actual_revision = current.as_ref().map_or(0, |settings| settings.revision);
    if actual_revision != expected_revision {
        return Err(StoreError::SystemSettingsRevisionConflict {
            expected: expected_revision,
            actual: actual_revision,
        }
        .into());
    }

    let saved = state
        .store
        .save_system_model_settings(command_id, expected_revision, settings)
        .await?;
    Ok(Json(system_settings_response(Some(saved))))
}

fn system_settings_response(
    settings: Option<nekowriter_store::SystemModelSettings>,
) -> SystemSettingsResponse {
    SystemSettingsResponse {
        writing_model: settings.map(|settings| PublicSystemModelSettings {
            api_base_url: settings.api_base_url,
            model_id: settings.model_id,
            capabilities: settings.capabilities,
            max_context_tokens: settings.max_context_tokens.to_string(),
            reserved_output_tokens: settings.reserved_output_tokens.to_string(),
            revision: settings.revision.to_string(),
            credential_configured: settings.credential_configured,
            created_at_ms: settings.created_at_ms.to_string(),
            updated_at_ms: settings.updated_at_ms.to_string(),
        }),
    }
}

/// 实际验证一个模型是否可用，并在声明支持思考时真的带上 `reasoning_effort`。
///
/// 服务地址与凭据始终取自系统设置：连接测试测的是「同一服务上的某个模型」，
/// 不允许绕过已配置的服务。
async fn test_model_connection(
    State(state): State<AppState>,
    Json(request): Json<TestModelRequest>,
) -> Result<Json<TestModelResponse>, ApiError> {
    let model_id = request.model_id.trim().to_owned();
    if model_id.is_empty() {
        return Err(StoreError::InvalidSystemModelSettings("模型 ID 不能为空".to_owned()).into());
    }
    let settings_guard = state.settings_write_lock.lock().await;
    let settings = state
        .store
        .get_system_model_settings()
        .await?
        .ok_or_else(|| StoreError::InvalidSystemModelSettings("请先保存写作模型设置".to_owned()))?;
    let api_key = state.store.system_model_api_key().await?;
    let thinking_probed = request.capabilities.thinking;
    let config = ChatCompletionsConfig::new(
        &settings.api_base_url,
        model_id.clone(),
        api_key,
        request.capabilities,
        ModelTimeouts::default(),
    )?;
    let provider = ChatCompletionsAdapter::new(config)?;
    drop(settings_guard);
    let request_payload = ModelRequest {
        messages: vec![
            ModelRequestMessage::Text {
                role: ContextMessageRole::System,
                content: "你正在执行 NekoWriter 的模型连接测试。".to_owned(),
            },
            ModelRequestMessage::Text {
                role: ContextMessageRole::User,
                content: "请简短回复“连接测试成功”。不要调用工具。".to_owned(),
            },
        ],
        tools: Vec::new(),
        // 声明支持思考时用一个代表性等级真的发一次，使这个声明可被验证；
        // 否则探针保持 Off，不做超出声明的断言。
        thinking_level: thinking_probe_level(thinking_probed),
    };

    // 系统设置测试与写作 Agent 共用同一个全局许可，避免用户在测试连接时绕开
    // “全局最多一个模型网络请求”的约束。
    let _permit = state
        .model_request_permit
        .clone()
        .acquire_owned()
        .await
        .expect("AppState 持有模型调度 Semaphore，许可不会永久关闭");
    let started_at_ms = utc_now_millis();
    let outcome = provider.execute_with(&request_payload, |_| {}).await;
    let completed_at_ms = utc_now_millis();
    // 连接测试真的发出了一次模型请求，因此它和对话请求共用同一份用量口径：
    // 成功时留下供应商报告的用量，失败时留下类型化原因，两者都必须落库，
    // 否则这次请求的用量会在统计里凭空消失。
    persist_connection_test(
        &state,
        &model_id,
        &settings.api_base_url,
        started_at_ms,
        completed_at_ms,
        &outcome,
    )
    .await?;
    let completion = match outcome {
        Ok(completion) => completion,
        Err(ProviderError::HttpStatus { status, .. }) if thinking_probed && status == 400 => {
            return Err(StoreError::InvalidSystemModelSettings(format!(
                "模型 {model_id} 拒绝了 reasoning_effort（400）；该模型可能不支持思考参数"
            ))
            .into());
        }
        Err(error) => return Err(error.into()),
    };
    Ok(Json(TestModelResponse {
        provider_request_id: completion.provider_request_id,
        content_received: completion
            .content
            .as_deref()
            .is_some_and(|content| !content.trim().is_empty()),
        tool_call_count: completion.tool_calls.len(),
        usage: completion.usage,
        thinking_probed,
    }))
}

/// 把一次连接测试落库成系统模型操作。
///
/// 这类请求不属于任何稿本，因此不能写进以 Run 为主键的 `model_requests`；它们
/// 单独成表并在 Dashboard 里单列，使「刚才那次测试花了多少、成功没有」有据可查。
async fn persist_connection_test(
    state: &AppState,
    model_id: &str,
    api_base_url: &str,
    started_at_ms: i64,
    completed_at_ms: i64,
    outcome: &Result<ModelCompletion, ProviderError>,
) -> Result<(), ApiError> {
    let result = match outcome {
        Ok(completion) => SystemOperationResult::Succeeded {
            usage: completion.persisted_usage(),
            provider_request_id: completion.provider_request_id.clone(),
        },
        Err(error) => SystemOperationResult::Failed {
            // 连接与超时失败无法断定请求是否已经到达供应商，因此标为未知而非失败。
            reached_provider_unknown: matches!(
                error.failure_code(),
                ModelFailureCode::Connection | ModelFailureCode::Timeout
            ),
            kind: "provider_error",
            message: error.to_string(),
        },
    };
    operations::record_system_operation(
        &state.store,
        OPERATION_KIND_CONNECTION_TEST,
        model_id,
        api_base_url,
        started_at_ms,
        completed_at_ms,
        result,
    )
    .await?;
    Ok(())
}

async fn list_projects(
    State(state): State<AppState>,
) -> Result<Json<Vec<ProjectListItem>>, ApiError> {
    Ok(Json(state.store.list_projects().await?))
}

async fn get_project(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<ProjectId>,
) -> Result<Json<ProjectBundle>, ApiError> {
    Ok(Json(state.store.get_project(project_id).await?))
}

async fn list_versions(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<ProjectId>,
) -> Result<Json<Vec<nekowriter_core::NovelVersion>>, ApiError> {
    Ok(Json(state.store.list_versions(project_id).await?))
}

async fn get_version_snapshot(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
) -> Result<Json<VersionSnapshot>, ApiError> {
    Ok(Json(state.store.get_version_snapshot(version_id).await?))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventCursorResponse {
    event_watermark: String,
}

/// 返回全局可恢复事件流的当前水位，供单页应用在创建 SSE 连接前取得起点。
async fn get_event_cursor(
    State(state): State<AppState>,
) -> Result<Json<EventCursorResponse>, ApiError> {
    let bounds = state.store.event_stream_bounds().await?;
    Ok(Json(EventCursorResponse {
        event_watermark: bounds.watermark.to_string(),
    }))
}

/// 导出查询参数；格式是必填项，不接受默认值。
#[derive(Debug, Deserialize)]
struct ExportQuery {
    format: Option<String>,
}

/// 把稿本已提交正文导出成一个可下载的文件。
///
/// 内容是现读的：每次请求都在同一个读取快照里重新组装，不存在可以过期的导出副本。
async fn export_version(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    let format = ExportFormat::parse(query.format.as_deref().unwrap_or_default())?;
    let export = state.store.build_version_export(version_id).await?;
    let file_name = export.file_name(format);
    let mut response = export.render(format).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(format.media_type()),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&content_disposition(&file_name, format))
            .expect("导出文件名已去除控制字符"),
    );
    Ok(response)
}

/// RFC 6266 下载头：ASCII 回退名加上 UTF-8 百分号编码的完整名。
///
/// 中文标题不能直接写进 `filename=`，因此两者都给出：旧客户端用 ASCII 回退名，
/// 规范客户端用 `filename*` 得到原名。
fn content_disposition(file_name: &str, format: ExportFormat) -> String {
    let mut encoded = String::new();
    for byte in file_name.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(*byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!(
        "attachment; filename=\"nekowriter-export.{}\"; filename*=UTF-8''{encoded}",
        format.extension()
    )
}

async fn list_chapters(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
) -> Result<Json<Vec<nekowriter_core::ChapterSummary>>, ApiError> {
    Ok(Json(state.store.list_chapters(version_id).await?))
}

async fn advance_to_next_chapter(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<AdvanceToNextChapterRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    Ok(Json(
        state
            .store
            .advance_to_next_chapter(
                version_id,
                request.command_id,
                request.expected_state_revision,
            )
            .await?,
    ))
}

async fn confirm_novel_completion(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<ConfirmNovelCompletionRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    Ok(Json(
        state
            .store
            .confirm_novel_completion(
                version_id,
                request.command_id,
                request.expected_state_revision,
            )
            .await?,
    ))
}

async fn add_extra_chapter(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<AddExtraChapterRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    let input = NewExtraChapter::new(request.title)?;
    Ok(Json(
        state
            .store
            .add_extra_chapter(
                version_id,
                request.command_id,
                request.expected_state_revision,
                input,
            )
            .await?,
    ))
}

async fn get_chapter_guidance(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
) -> Result<Json<nekowriter_core::CurrentChapterGuidance>, ApiError> {
    Ok(Json(state.store.get_chapter_guidance(chapter_id).await?))
}

async fn save_chapter_guidance(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
    Json(request): Json<SaveChapterGuidanceRequest>,
) -> Result<Json<nekowriter_core::CurrentChapterGuidance>, ApiError> {
    let input = NewChapterGuidance::new(request.content)?;
    Ok(Json(
        state
            .store
            .save_chapter_guidance(
                chapter_id,
                request.command_id,
                request.expected_state_revision,
                input,
            )
            .await?,
    ))
}

async fn start_chapter_outline(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
    Json(request): Json<StartChapterOutlineRequest>,
) -> Result<Json<nekowriter_core::WorkflowSnapshot>, ApiError> {
    let project_id = state.store.project_id_for_chapter(chapter_id).await?;
    ensure_model_ready(&state, project_id).await?;
    let workflow = state
        .store
        .start_chapter_outline(
            chapter_id,
            request.command_id,
            request.expected_state_revision,
        )
        .await?;
    if workflow.execution == ExecutionStatus::Queued {
        let run_id = workflow.active_run_id.ok_or_else(|| {
            StoreError::InvalidChapterState("章节细纲任务缺少活跃 Run".to_owned())
        })?;
        state
            .scheduler
            .enqueue_chapter_outline(workflow.version_id, chapter_id, run_id)
            .await?;
    }
    Ok(Json(workflow))
}

async fn get_chapter_outline(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
) -> Result<Json<nekowriter_core::ChapterOutlineSnapshot>, ApiError> {
    Ok(Json(state.store.get_chapter_outline(chapter_id).await?))
}

async fn submit_chapter_outline(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
    Json(request): Json<SubmitChapterOutlineRequest>,
) -> Result<Json<nekowriter_core::ChapterOutlineSnapshot>, ApiError> {
    let input = NewChapterOutline::new(request.content)?;
    Ok(Json(
        state
            .store
            .submit_chapter_outline(
                chapter_id,
                request.command_id,
                request.expected_state_revision,
                input,
            )
            .await?,
    ))
}

async fn get_chapter_body(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    Ok(Json(state.store.get_chapter_body(chapter_id).await?))
}

async fn get_document_revision(
    State(state): State<AppState>,
    AxumPath((document_id, revision_id)): AxumPath<(DocumentId, RevisionId)>,
) -> Result<Json<nekowriter_core::DocumentRevisionView>, ApiError> {
    Ok(Json(
        state
            .store
            .get_document_revision(document_id, revision_id)
            .await?,
    ))
}

async fn append_chapter_body(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
    Json(request): Json<AppendChapterBodyRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    let input = AppendChapterContent::new(request.content)?;
    Ok(Json(
        state
            .store
            .append_chapter_body(
                chapter_id,
                request.command_id,
                request.expected_state_revision,
                input,
            )
            .await?,
    ))
}

async fn replace_chapter_body(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
    Json(request): Json<ReplaceChapterBodyRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    let input = ReplaceChapterContent::new(request.old_str, request.new_str)?;
    Ok(Json(
        state
            .store
            .replace_chapter_body(
                chapter_id,
                request.command_id,
                request.expected_state_revision,
                input,
            )
            .await?,
    ))
}

async fn decide_chapter_body_proposal(
    State(state): State<AppState>,
    AxumPath((chapter_id, tool_call_id)): AxumPath<(ChapterId, Uuid)>,
    Json(request): Json<DecideChapterBodyProposalRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    let snapshot = state
        .store
        .decide_chapter_body_proposal(
            chapter_id,
            tool_call_id,
            request.command_id,
            request.expected_state_revision,
            request.feedback,
            request.citations,
        )
        .await?;
    enqueue_queued_writing_run(&state, &snapshot.workflow).await?;
    Ok(Json(snapshot))
}

async fn submit_run_feedback(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<nekowriter_core::RunId>,
    Json(request): Json<SubmitRunFeedbackRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    let snapshot = state
        .store
        .submit_run_feedback(
            run_id,
            request.command_id,
            request.expected_state_revision,
            request.text,
            request.citations,
        )
        .await?;
    enqueue_queued_writing_run(&state, &snapshot.workflow).await?;
    Ok(Json(snapshot))
}

async fn answer_question(
    State(state): State<AppState>,
    AxumPath(question_id): AxumPath<nekowriter_core::QuestionId>,
    Json(request): Json<AnswerQuestionRequest>,
) -> Result<Json<nekowriter_store::QuestionAnswerResult>, ApiError> {
    let response = state
        .store
        .answer_user_question(
            question_id,
            request.command_id,
            request.expected_state_revision,
            UserQuestionAnswers {
                answers: request.answers,
            },
        )
        .await?;
    enqueue_queued_writing_run(&state, &response.workflow).await?;
    Ok(Json(response))
}

async fn continue_writing_run(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<nekowriter_core::RunId>,
    Json(request): Json<RunWorkflowControlRequest>,
) -> Result<Json<WorkflowSnapshot>, ApiError> {
    let project_id = state.store.project_id_for_run(run_id).await?;
    ensure_model_ready(&state, project_id).await?;
    let workflow = state
        .store
        .continue_writing_run(run_id, request.command_id, request.expected_state_revision)
        .await?;
    enqueue_queued_writing_run(&state, &workflow).await?;
    Ok(Json(workflow))
}

async fn stop_writing_run(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<nekowriter_core::RunId>,
    Json(request): Json<RunWorkflowControlRequest>,
) -> Result<Json<WorkflowSnapshot>, ApiError> {
    let stopped = state
        .store
        .stop_writing_run(run_id, request.command_id, request.expected_state_revision)
        .await?;
    state
        .scheduler
        .stop_run(
            stopped.workflow.version_id,
            run_id,
            &stopped.cancelled_request_ids,
        )
        .await;
    Ok(Json(stopped.workflow))
}

async fn request_chapter_body_pause(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<nekowriter_core::RunId>,
    Json(request): Json<RunWorkflowControlRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    Ok(Json(
        state
            .store
            .request_chapter_body_pause(run_id, request.command_id, request.expected_state_revision)
            .await?,
    ))
}

async fn resume_chapter_body_pause(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<nekowriter_core::RunId>,
    Json(request): Json<RunWorkflowControlRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    let snapshot = state
        .store
        .resume_chapter_body_pause(run_id, request.command_id, request.expected_state_revision)
        .await?;
    enqueue_queued_writing_run(&state, &snapshot.workflow).await?;
    Ok(Json(snapshot))
}

async fn set_approval_mode(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<SetApprovalModeRequest>,
) -> Result<Json<WorkflowSnapshot>, ApiError> {
    Ok(Json(
        state
            .store
            .set_approval_mode(
                version_id,
                request.command_id,
                request.expected_state_revision,
                request.mode,
            )
            .await?,
    ))
}

async fn submit_chapter_final_review(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
    Json(request): Json<SubmitChapterFinalReviewRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    Ok(Json(
        state
            .store
            .submit_chapter_final_review(
                chapter_id,
                request.command_id,
                request.expected_state_revision,
            )
            .await?,
    ))
}

async fn stream_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Result<Response, ApiError> {
    let cursor = event_cursor(&headers, query.after.as_deref())?;
    // 先订阅再检查游标边界。持久事件不会在“查历史到订阅实时”之间丢失；临时预览
    // 本来就不可恢复，只从连接建立后开始消费。
    let receiver = state.store.subscribe_events();
    let preview_receiver = state.preview_deltas.subscribe();
    let bounds = state.store.event_stream_bounds().await?;
    if stream_reset_required(cursor, bounds) {
        return Ok(Sse::new(stream::once(async move {
            Ok::<_, Infallible>(stream_reset_event(cursor, bounds))
        }))
        .into_response());
    }

    let stream_state = EventStreamState {
        receiver,
        preview_receiver,
        shutdown: state.shutdown.subscribe(),
        store: state.store,
        cursor,
        pending: VecDeque::new(),
        send_connection_comment: true,
    };
    let stream = stream::unfold(stream_state, next_sse_event);
    Ok(Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response())
}

async fn next_sse_event(
    mut state: EventStreamState,
) -> Option<(Result<Event, Infallible>, EventStreamState)> {
    loop {
        // 退出服务时先结束持久连接，否则优雅退出会一直等 SSE 客户端断开。
        if Shutdown::requested(&state.shutdown) {
            return None;
        }
        if state.send_connection_comment {
            state.send_connection_comment = false;
            return Some((Ok(Event::default().comment("connected")), state));
        }
        if let Some(event) = state.pending.pop_front() {
            state.cursor = event
                .seq
                .parse()
                .expect("持久事件序列号始终由 SQLite 整数转换而来");
            return Some((Ok(to_sse_event(event)), state));
        }

        match state.store.events_after(state.cursor).await {
            Ok(events) if !events.is_empty() => {
                state.pending = events.into();
            }
            Ok(_) => {
                let mut shutdown = state.shutdown.clone();
                tokio::select! {
                    result = state.receiver.recv() => match result {
                        Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => return None,
                    },
                    result = state.preview_receiver.recv() => match result {
                        Ok(delta) => return Some((Ok(to_preview_delta_sse_event(delta)), state)),
                        // 预览是明确可丢弃的临时层；落后时直接等待下一段或持久状态。
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => return None,
                    },
                    _ = shutdown.changed() => return None,
                }
            }
            Err(error) => {
                warn!(cursor = state.cursor, %error, "读取 SSE 持久事件失败，关闭当前连接");
                return None;
            }
        }
    }
}

fn to_sse_event(event: PersistedEvent) -> Event {
    let data =
        serde_json::to_string(&event).expect("持久事件的 JSON 值和 UUID 始终可以序列化为 SSE 数据");
    Event::default()
        .id(event.seq)
        .event(event.event_type)
        .data(data)
}

/// 临时预览故意不设置 SSE `id`。浏览器可能仍在 `lastEventId` 上保留上一条持久
/// 事件的值，前端因此只按 `request/tool/offset` 处理这类事件。
fn to_preview_delta_sse_event(delta: scheduler::PreviewDelta) -> Event {
    let data = serde_json::to_string(&delta).expect("临时预览只包含可序列化的 UUID、标量与字符串");
    Event::default().event("preview.delta").data(data)
}

fn event_cursor(headers: &HeaderMap, query_after: Option<&str>) -> Result<i64, ApiError> {
    let header_cursor = headers
        .get("last-event-id")
        .map(|value| value.to_str().map_err(|_| ApiError::InvalidEventCursor))
        .transpose()?;
    let cursor_text = header_cursor
        .filter(|cursor| !cursor.is_empty())
        .or(query_after)
        .unwrap_or("0");
    if cursor_text.is_empty() || !cursor_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ApiError::InvalidEventCursor);
    }
    let cursor = cursor_text
        .parse::<i64>()
        .map_err(|_| ApiError::InvalidEventCursor)?;
    Ok(cursor)
}

fn stream_reset_required(cursor: i64, bounds: EventStreamBounds) -> bool {
    cursor < bounds.minimum_cursor || cursor > bounds.watermark
}

fn stream_reset_event(cursor: i64, bounds: EventStreamBounds) -> Event {
    let reason = if cursor > bounds.watermark {
        "cursor_ahead"
    } else {
        "cursor_expired"
    };
    Event::default().event("stream.reset_required").data(
        serde_json::json!({
            "reason": reason,
            "watermark": bounds.watermark.to_string(),
        })
        .to_string(),
    )
}

async fn get_preparation(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    Ok(Json(
        state.store.get_preparation_snapshot(version_id).await?,
    ))
}

async fn save_novel_brief(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<SaveNovelBriefRequest>,
) -> Result<Json<nekowriter_core::CurrentNovelBrief>, ApiError> {
    let input = NewNovelBrief::new(request.content)?;
    Ok(Json(
        state
            .store
            .save_novel_brief(version_id, request.command_id, input)
            .await?,
    ))
}

async fn start_preparation(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<StartPreparationRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    let project_id = state.store.project_id_for_version(version_id).await?;
    ensure_model_ready(&state, project_id).await?;
    let snapshot = state
        .store
        .start_preparation(
            version_id,
            request.command_id,
            request.expected_state_revision,
        )
        .await?;
    if snapshot.workflow.execution == ExecutionStatus::Queued {
        let run_id = snapshot.workflow.active_run_id.ok_or_else(|| {
            StoreError::InvalidPreparationState("准备阶段缺少活跃 Run".to_owned())
        })?;
        state
            .scheduler
            .enqueue_preparation(version_id, run_id)
            .await?;
    }
    Ok(Json(snapshot))
}

/// 命令前置检查：按项目的**有效模型**判断能力，而不是系统模型。
///
/// 项目可以引用另一个模型，那个模型（而非系统写作模型）才是请求真正发往的对象；
/// 用系统模型的资料放行会让项目引用在真正 dispatch 时才以指向系统模型的文案失败。
/// 这里与调度器共用 [`pre_dispatch_capability_problem`]，不维护第二套规则。
async fn ensure_model_ready(state: &AppState, project_id: ProjectId) -> Result<(), ApiError> {
    let system = state
        .store
        .get_system_model_settings()
        .await?
        .ok_or_else(|| StoreError::InvalidSystemModelSettings("请先保存写作模型设置".to_owned()))?;
    let project = state.store.get_project_settings(project_id).await?;
    let effective = resolve_effective_model(&system, &project);
    if let Some(problem) =
        pre_dispatch_capability_problem(*effective.capabilities, project.thinking_level)
    {
        return Err(
            StoreError::InvalidSystemModelSettings(capability_problem_message(
                problem,
                &effective.describe(),
            ))
            .into(),
        );
    }
    Ok(())
}

async fn submit_preparation_artifacts(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<SubmitPreparationArtifactsRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    let artifacts = request
        .artifacts
        .into_iter()
        .map(|artifact| PreparationArtifactInput::new(artifact.kind, artifact.content))
        .collect::<Result<Vec<_>, _>>()?;
    let chapter_plan = request
        .chapter_plan
        .map(|drafts| {
            drafts
                .into_iter()
                .map(|draft| ChapterPlanDraft::new(draft.title, draft.goal))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    Ok(Json(
        state
            .store
            .submit_preparation_artifacts(
                version_id,
                request.command_id,
                request.expected_state_revision,
                artifacts,
                chapter_plan,
            )
            .await?,
    ))
}

async fn decide_review(
    State(state): State<AppState>,
    AxumPath(review_id): AxumPath<ReviewId>,
    Json(request): Json<DecidePreparationReviewRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    let decision = HumanReviewDecision::new(request.passed, request.issues.unwrap_or_default())?;
    let snapshot = state
        .store
        .decide_review(
            review_id,
            request.command_id,
            request.expected_state_revision,
            decision,
            request.citations,
        )
        .await?;
    enqueue_queued_writing_run(&state, &snapshot.workflow).await?;
    Ok(Json(snapshot))
}

async fn delegate_review(
    State(state): State<AppState>,
    AxumPath(review_id): AxumPath<ReviewId>,
    Json(request): Json<DelegateReviewRequest>,
) -> Result<(StatusCode, Json<nekowriter_store::ReviewDelegationResult>), ApiError> {
    let project_id = state.store.project_id_for_review(review_id).await?;
    ensure_model_ready(&state, project_id).await?;
    let delegation = state
        .store
        .delegate_review(
            review_id,
            request.command_id,
            request.expected_state_revision,
            request.notes,
            request.citations,
        )
        .await?;
    if let Err(error) = state
        .scheduler
        .enqueue_review(
            delegation.snapshot.workflow.version_id,
            delegation.review_id,
            delegation.run_id,
        )
        .await
    {
        let failure = json!({
            "kind": "scheduler_enqueue",
            "message": error.to_string(),
        });
        if let Err(recovery_error) = state
            .store
            .fail_delegated_review_attempt(
                delegation.review_id,
                delegation.run_id,
                None,
                None,
                failure,
                None,
                false,
            )
            .await
        {
            warn!(
                review_id = %delegation.review_id,
                run_id = %delegation.run_id,
                error = %recovery_error,
                "审核 Agent 调度失败后未能重新开放审核"
            );
        }
        return Err(error.into());
    }
    Ok((StatusCode::ACCEPTED, Json(delegation)))
}

async fn cancel_review_delegation(
    State(state): State<AppState>,
    AxumPath(review_id): AxumPath<ReviewId>,
    Json(request): Json<CancelReviewDelegationRequest>,
) -> Result<Json<nekowriter_store::CancelReviewDelegationResult>, ApiError> {
    let cancellation = state
        .store
        .cancel_review_delegation(
            review_id,
            request.command_id,
            request.expected_state_revision,
        )
        .await?;
    state
        .scheduler
        .stop_run(
            cancellation.snapshot.workflow.version_id,
            cancellation.run_id,
            &cancellation.cancelled_request_ids,
        )
        .await;
    Ok(Json(cancellation))
}

/// 审核拒绝或章节细纲通过后，Store 会在同一事务中把原 Run（或新正文 Run）恢复
/// 为 queued。HTTP 命令返回前把它交还给每稿本调度队列，避免工作流停在没有消费者
/// 的队列状态；调度器仍会在实际发送前重新读取最新模型设置和上下文。
async fn enqueue_queued_writing_run(
    state: &AppState,
    workflow: &WorkflowSnapshot,
) -> Result<(), ApiError> {
    if workflow.execution != ExecutionStatus::Queued {
        return Ok(());
    }
    let run_id = workflow
        .active_run_id
        .ok_or_else(|| StoreError::InvalidPreparationState("排队工作流缺少活跃 Run".to_owned()))?;
    match workflow.stage {
        WorkflowStage::PreparingRevising | WorkflowStage::PreparingGenerating => {
            state
                .scheduler
                .enqueue_preparation(workflow.version_id, run_id)
                .await?;
        }
        WorkflowStage::WritingOutlineGenerating => {
            let chapter_id = workflow.active_chapter_id.ok_or_else(|| {
                StoreError::InvalidChapterState("排队细纲任务缺少当前章节".to_owned())
            })?;
            state
                .scheduler
                .enqueue_chapter_outline(workflow.version_id, chapter_id, run_id)
                .await?;
        }
        WorkflowStage::WritingDrafting => {
            let chapter_id = workflow.active_chapter_id.ok_or_else(|| {
                StoreError::InvalidChapterState("排队正文任务缺少当前章节".to_owned())
            })?;
            state
                .scheduler
                .enqueue_chapter_body(workflow.version_id, chapter_id, run_id)
                .await?;
        }
        _ => {}
    }
    Ok(())
}

async fn create_project(
    State(state): State<AppState>,
    Json(request): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectBundle>), ApiError> {
    let input = NewProject::new(
        request.name,
        request.novel_title,
        request.version_label.as_deref(),
    )?;
    let bundle = state
        .store
        .create_project(request.command_id, input)
        .await?;
    Ok((StatusCode::CREATED, Json(bundle)))
}

async fn create_version(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<ProjectId>,
    Json(request): Json<CreateVersionRequest>,
) -> Result<(StatusCode, Json<ProjectBundle>), ApiError> {
    let input = NewNovelVersion::new(request.label, request.novel_title)?;
    let bundle = state
        .store
        .create_version(project_id, request.command_id, input)
        .await?;
    Ok((StatusCode::CREATED, Json(bundle)))
}

async fn select_version(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<ProjectId>,
    Json(request): Json<SelectVersionRequest>,
) -> Result<Json<ProjectBundle>, ApiError> {
    Ok(Json(
        state
            .store
            .select_version(
                project_id,
                request.version_id,
                request.command_id,
                request.expected_project_revision,
            )
            .await?,
    ))
}

async fn copy_version(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    Json(request): Json<CopyVersionRequest>,
) -> Result<(StatusCode, Json<ProjectBundle>), ApiError> {
    let input = NewCopiedNovelVersion::new(request.label)?;
    let bundle = state
        .store
        .copy_version(version_id, request.command_id, input)
        .await?;
    Ok((StatusCode::CREATED, Json(bundle)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveProjectSettingsRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_revision: i64,
    /// 模型引用必须显式给出（`null` 表示清除覆盖）。缺少该字段会被拒绝，
    /// 而不是当成「未覆盖」静默清空项目设置。
    model: Option<ProjectModelReference>,
    thinking_level: ThinkingLevel,
    #[serde(with = "decimal_i64")]
    auxiliary_tool_budget: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavePromptOverrideRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_revision: i64,
    system: String,
    user: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResetPromptOverrideRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_revision: i64,
}

async fn get_system_prompts(
    State(state): State<AppState>,
) -> Result<Json<Vec<prompts::PromptCatalogEntry>>, ApiError> {
    Ok(Json(prompts::prompt_catalog(&state.store, None).await?))
}

async fn get_project_prompts(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<ProjectId>,
) -> Result<Json<Vec<prompts::PromptCatalogEntry>>, ApiError> {
    Ok(Json(
        prompts::prompt_catalog(&state.store, Some(project_id)).await?,
    ))
}

/// 保存覆盖前先按受限模板语言校验，拒绝未知变量与缺失的必需占位符。
async fn save_prompt_override_command(
    state: &AppState,
    scope: PromptOverrideScope,
    project_id: Option<ProjectId>,
    prompt_key: &str,
    request: SavePromptOverrideRequest,
) -> Result<Json<PromptOverride>, ApiError> {
    let key = prompts::parse_prompt_key(prompt_key)?;
    validate_builtin_prompt_pair(key, &request.system, &request.user)?;
    Ok(Json(
        state
            .store
            .save_prompt_override(
                request.command_id,
                request.expected_revision,
                NewPromptOverride {
                    prompt_key: key.as_str().to_owned(),
                    scope,
                    project_id,
                    system: request.system,
                    user: request.user,
                },
            )
            .await?,
    ))
}

async fn reset_prompt_override_command(
    state: &AppState,
    scope: PromptOverrideScope,
    project_id: Option<ProjectId>,
    prompt_key: &str,
    request: ResetPromptOverrideRequest,
) -> Result<Json<PromptOverrideReset>, ApiError> {
    let key = prompts::parse_prompt_key(prompt_key)?;
    Ok(Json(
        state
            .store
            .reset_prompt_override(
                request.command_id,
                request.expected_revision,
                scope,
                project_id,
                key.as_str(),
            )
            .await?,
    ))
}

async fn save_system_prompt_override(
    State(state): State<AppState>,
    AxumPath(prompt_key): AxumPath<String>,
    Json(request): Json<SavePromptOverrideRequest>,
) -> Result<Json<PromptOverride>, ApiError> {
    save_prompt_override_command(
        &state,
        PromptOverrideScope::System,
        None,
        &prompt_key,
        request,
    )
    .await
}

async fn reset_system_prompt_override(
    State(state): State<AppState>,
    AxumPath(prompt_key): AxumPath<String>,
    Json(request): Json<ResetPromptOverrideRequest>,
) -> Result<Json<PromptOverrideReset>, ApiError> {
    reset_prompt_override_command(
        &state,
        PromptOverrideScope::System,
        None,
        &prompt_key,
        request,
    )
    .await
}

async fn save_project_prompt_override(
    State(state): State<AppState>,
    AxumPath((project_id, prompt_key)): AxumPath<(ProjectId, String)>,
    Json(request): Json<SavePromptOverrideRequest>,
) -> Result<Json<PromptOverride>, ApiError> {
    save_prompt_override_command(
        &state,
        PromptOverrideScope::Project,
        Some(project_id),
        &prompt_key,
        request,
    )
    .await
}

async fn reset_project_prompt_override(
    State(state): State<AppState>,
    AxumPath((project_id, prompt_key)): AxumPath<(ProjectId, String)>,
    Json(request): Json<ResetPromptOverrideRequest>,
) -> Result<Json<PromptOverrideReset>, ApiError> {
    reset_prompt_override_command(
        &state,
        PromptOverrideScope::Project,
        Some(project_id),
        &prompt_key,
        request,
    )
    .await
}

async fn preview_prompt(
    State(state): State<AppState>,
    Json(request): Json<prompts::PreviewPromptRequest>,
) -> Result<Json<prompts::PreviewPromptResponse>, ApiError> {
    Ok(Json(prompts::preview_prompt(&state.store, request).await?))
}

async fn get_project_settings(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<ProjectId>,
) -> Result<Json<ProjectSettings>, ApiError> {
    Ok(Json(state.store.get_project_settings(project_id).await?))
}

/// 项目设置写入与系统设置共用同一把短锁：调度器在读取有效设置到封存请求快照之间
/// 持有它，保存命令不会夹在中间，已 dispatch 的快照也不会被原地改写。
async fn save_project_settings(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<ProjectId>,
    Json(request): Json<SaveProjectSettingsRequest>,
) -> Result<Json<ProjectSettings>, ApiError> {
    let _guard = state.settings_write_lock.lock().await;
    Ok(Json(
        state
            .store
            .save_project_settings(
                project_id,
                request.command_id,
                request.expected_revision,
                NewProjectSettings {
                    model: request.model,
                    thinking_level: request.thinking_level,
                    auxiliary_tool_budget: request.auxiliary_tool_budget,
                },
            )
            .await?,
    ))
}

async fn api_not_found() -> ApiError {
    ApiError::NotFound
}

/// 写请求的信任边界：`Host` 必须是本机回环地址，带 `Origin` 时必须来自本机允许的来源。
///
/// 它挂成一层中间件，而不是由每个 handler 记得调用：新增写端点时不会漏掉这条校验。
async fn require_trusted_write(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let is_write = !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if is_write && let Err(error) = ensure_trusted_write_request(request.headers(), &state) {
        return error.into_response();
    }
    next.run(request).await
}

fn ensure_trusted_write_request(headers: &HeaderMap, state: &AppState) -> Result<(), ApiError> {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::UntrustedHost)?;
    if !is_loopback_host(host) {
        return Err(ApiError::UntrustedHost);
    }

    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        && !state.allowed_write_origins.contains(origin)
    {
        return Err(ApiError::UntrustedOrigin);
    }
    Ok(())
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "localhost"
        || host.starts_with("localhost:")
        || host == "127.0.0.1"
        || host.starts_with("127.0.0.1:")
        || host == "[::1]"
        || host.starts_with("[::1]:")
}

fn allowed_write_origins(port: u16) -> HashSet<String> {
    let mut origins = HashSet::from([
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ]);
    origins.extend(VITE_ORIGINS.map(str::to_owned));
    origins
}

fn default_data_dir() -> Result<PathBuf, StartupError> {
    ProjectDirs::from("com", "NekoWriter", "NekoWriter")
        .map(|directories| directories.data_local_dir().to_path_buf())
        .ok_or(StartupError::NoLocalDataDirectory)
}

/// 在取得进程锁、SQLite 和 `LanceDB` 连接之前固定数据目录的物理位置。
///
/// 这样相同的相对 `--data-dir` 不会因启动工作目录不同而指向不同的本地作品库。
fn resolve_data_dir(path: PathBuf) -> Result<PathBuf, StartupError> {
    std::fs::create_dir_all(&path)?;
    Ok(path.canonicalize()?)
}

/// 前端构建产物默认放在可执行文件旁的 `webui/dist`：发行目录整体拷走后从任何
/// 工作目录启动都能打开界面，不依赖源码树。
fn default_web_dir() -> Result<PathBuf, StartupError> {
    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or(StartupError::NoExecutableDirectory)?;
    Ok(directory.join("webui").join("dist"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures_util::{StreamExt, future::BoxFuture};
    use nekowriter_agent::engine::{
        ACCUMULATION_ENGINES, AccumulatedEntry, AccumulationEngine, AccumulationOutcome,
        AccumulationRequest, EngineDescriptor, EngineError,
    };
    use nekowriter_store::{CredentialPatch, KnowledgeSpaceInput, NewEmbeddingSettings};
    use tempfile::TempDir;

    use super::*;

    /// 启动失败时，用户看到的必须是一句点明目录、失败步骤、原因与前进路径的说明，
    /// 而不是一个只能看懂的 Rust 错误。
    #[test]
    fn startup_failure_messages_name_the_directory_step_cause_and_way_forward() {
        let data_dir = Path::new("/tmp/nw-schema-probe");
        let logs_dir = data_dir.join("logs");

        let stale = startup_failure_message(
            Some(data_dir),
            Some(&logs_dir),
            StartupStep::Database,
            &StartupError::Store(StoreError::UnsupportedSchema {
                found: "25".to_owned(),
                expected: 26,
            }),
        );
        assert!(stale.contains("数据目录：/tmp/nw-schema-probe"));
        assert!(stale.contains("25") && stale.contains("26"));
        assert!(stale.contains("旧版本"));
        assert!(stale.contains("--data-dir"));
        assert!(stale.contains("/tmp/nw-schema-probe/logs"));
        // 只声明实际写下的东西：日志与进程锁，而不是「不会改动它」。
        assert!(stale.contains("不会读取或迁移其中的作品数据"));
        assert!(!stale.contains("不会改动它"));

        let unknown = startup_failure_message(
            Some(data_dir),
            None,
            StartupStep::Database,
            &StartupError::Store(StoreError::UnknownSchema),
        );
        assert!(unknown.contains("数据目录：/tmp/nw-schema-probe"));
        assert!(unknown.contains("schema"));
        assert!(unknown.contains("--data-dir"));
        // 日志没落地时不能声称原因已记录在 logs/。
        assert!(!unknown.contains("只写入了启动日志"));

        // 早期步骤（数据目录、日志）的 IO 失败：原因用中文点名是哪一步。
        let unusable = startup_failure_message(
            Some(data_dir),
            None,
            StartupStep::DataDirectory,
            &StartupError::Io(std::io::Error::from_raw_os_error(20)),
        );
        assert!(unusable.contains("原因：无法准备数据目录（"));
        assert!(unusable.contains("--data-dir"));

        let locked = startup_failure_message(
            Some(data_dir),
            Some(&logs_dir),
            StartupStep::ProcessLock,
            &StartupError::DataDirectoryLocked(data_dir.join("nekowriter.lock")),
        );
        assert!(locked.contains("原因：无法锁定数据目录（"));
        assert!(locked.contains("关闭正在运行的那个"));

        let no_directory = startup_failure_message(
            None,
            None,
            StartupStep::DataDirectory,
            &StartupError::NoLocalDataDirectory,
        );
        assert!(no_directory.contains("NekoWriter 无法启动。"));
        assert!(!no_directory.contains("数据目录："));
        assert!(no_directory.contains("请用 --data-dir <目录> 明确指定"));

        let web = startup_failure_message(
            Some(data_dir),
            Some(&logs_dir),
            StartupStep::WebDirectory,
            &StartupError::NoExecutableDirectory,
        );
        assert!(web.contains("无法定位前端构建产物目录"));
        assert!(web.contains("--web-dir"));
    }

    /// 契约测试用的积累引擎：返回两条候选，并记录被调用的次数。
    struct StubAccumulationEngine;
    /// 引擎调用失败时必须什么都不写。
    struct FailingAccumulationEngine;

    static STUB_ACCUMULATION_ENGINES: &[&dyn AccumulationEngine] = &[&StubAccumulationEngine];
    static FAILING_ACCUMULATION_ENGINES: &[&dyn AccumulationEngine] = &[&FailingAccumulationEngine];
    static STUB_CALLS: AtomicUsize = AtomicUsize::new(0);

    impl AccumulationEngine for StubAccumulationEngine {
        fn descriptor(&self) -> EngineDescriptor {
            EngineDescriptor {
                id: "stub-accumulation",
                display_name: "探针积累引擎",
            }
        }

        fn accumulate<'a>(
            &'a self,
            request: &'a AccumulationRequest,
        ) -> BoxFuture<'a, Result<AccumulationOutcome, EngineError>> {
            STUB_CALLS.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                Ok(AccumulationOutcome {
                    entries: vec![
                        AccumulatedEntry {
                            title: "灯塔线索".to_owned(),
                            keywords: vec!["线索".to_owned()],
                            content: format!("{}：灯塔在雨夜里反复出现。", request.clues),
                            source: Some("探针引擎".to_owned()),
                        },
                        AccumulatedEntry {
                            title: "码头线索".to_owned(),
                            keywords: Vec::new(),
                            content: "码头在第三章被烧毁。".to_owned(),
                            source: None,
                        },
                    ],
                })
            })
        }
    }

    impl AccumulationEngine for FailingAccumulationEngine {
        fn descriptor(&self) -> EngineDescriptor {
            EngineDescriptor {
                id: "failing-accumulation",
                display_name: "总是失败的探针引擎",
            }
        }

        fn accumulate<'a>(
            &'a self,
            _request: &'a AccumulationRequest,
        ) -> BoxFuture<'a, Result<AccumulationOutcome, EngineError>> {
            Box::pin(async move {
                Err(EngineError::Failed {
                    message: "探针引擎无法访问上游资料".to_owned(),
                })
            })
        }
    }

    /// 一个测试应用：真实临时数据目录、指定注册表的应用状态。
    async fn accumulation_app(
        engines: &'static [&'static dyn AccumulationEngine],
    ) -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        let settings_write_lock = Arc::new(Mutex::new(()));
        let model_request_permit = Arc::new(Semaphore::new(1));
        let (preview_deltas, _) = broadcast::channel(256);
        let state = AppState {
            scheduler: scheduler::WorkflowScheduler::with_preview_deltas(
                store.clone(),
                model_request_permit.clone(),
                settings_write_lock.clone(),
                preview_deltas.clone(),
            ),
            store,
            allowed_write_origins: Arc::new(HashSet::from(["http://127.0.0.1:47822".to_owned()])),
            settings_write_lock,
            model_request_permit,
            preview_deltas,
            shutdown: Shutdown::new(),
            accumulation_engines: engines,
        };
        (dir, state)
    }

    /// 在回环地址的真实端口上启动真实路由，返回基地址。
    fn spawn_app(state: AppState) -> String {
        spawn_app_with_web_dir(state, std::env::temp_dir())
    }

    fn spawn_app_with_web_dir(state: AppState, web_dir: std::path::PathBuf) -> String {
        let app = router(state, web_dir);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{address}")
    }

    async fn new_accumulation_space(state: &AppState) -> Uuid {
        state
            .store
            .create_knowledge_space(
                Uuid::new_v4(),
                KnowledgeSpaceInput {
                    title: "积累".to_owned(),
                },
            )
            .await
            .unwrap()
            .id
    }

    /// 有注册表的引擎时，积累命令真的写入条目；重放不再次调用引擎。
    #[tokio::test]
    async fn accumulation_command_writes_candidates_and_replays_without_a_second_engine_call() {
        let (_dir, state) = accumulation_app(STUB_ACCUMULATION_ENGINES).await;
        let space_id = new_accumulation_space(&state).await;
        state
            .store
            .save_embedding_settings(
                Uuid::new_v4(),
                0,
                NewEmbeddingSettings {
                    api_base_url: "https://embed.example/v1".to_owned(),
                    model_id: "test-embed".to_owned(),
                    dimension: 2,
                    api_key: CredentialPatch::Keep,
                },
            )
            .await
            .unwrap();
        let base = spawn_app(state.clone());
        let client = reqwest::Client::new();
        let command_id = Uuid::new_v4();
        let body = json!({
            "commandId": command_id,
            "spaceId": space_id,
            "clues": "雨夜灯塔",
        });

        let response = client
            .post(format!("{base}/api/knowledge/accumulate"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let created: Value = response.json().await.unwrap();
        assert_eq!(created.as_array().unwrap().len(), 2);
        assert_eq!(STUB_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(
            state
                .store
                .list_knowledge_entries(Some(space_id), None)
                .await
                .unwrap()
                .len(),
            2
        );
        // 每条候选都在同一事务里建立了索引任务。
        assert_eq!(
            state
                .store
                .knowledge_index_coverage(&[space_id])
                .await
                .unwrap()
                .pending_entries,
            2
        );

        // 同一条命令重试：返回原结果，不再次调用引擎，也不重复写入。
        let replay = client
            .post(format!("{base}/api/knowledge/accumulate"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::OK);
        let replayed: Value = replay.json().await.unwrap();
        assert_eq!(replayed, created);
        assert_eq!(STUB_CALLS.load(Ordering::SeqCst), 1, "重放不再次调用引擎");
        assert_eq!(
            state
                .store
                .list_knowledge_entries(Some(space_id), None)
                .await
                .unwrap()
                .len(),
            2
        );

        // 同一个 command_id 换一组线索是幂等冲突，不写入。
        let conflict = client
            .post(format!("{base}/api/knowledge/accumulate"))
            .json(&json!({
                "commandId": command_id,
                "spaceId": space_id,
                "clues": "另一次积累",
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        let conflict_body: Value = conflict.json().await.unwrap();
        assert_eq!(conflict_body["error"]["code"], "IDEMPOTENCY_CONFLICT");
    }

    /// 引擎执行失败时不得写入任何条目，也不留下可重放的回执。
    #[tokio::test]
    async fn accumulation_engine_failure_writes_nothing() {
        let (_dir, state) = accumulation_app(FAILING_ACCUMULATION_ENGINES).await;
        let space_id = new_accumulation_space(&state).await;
        let base = spawn_app(state.clone());
        let client = reqwest::Client::new();
        let command_id = Uuid::new_v4();

        let response = client
            .post(format!("{base}/api/knowledge/accumulate"))
            .json(&json!({
                "commandId": command_id,
                "spaceId": space_id,
                "clues": "雨夜灯塔",
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], "ENGINE_FAILED");
        assert_eq!(
            state
                .store
                .list_knowledge_entries(Some(space_id), None)
                .await
                .unwrap(),
            [] as [nekowriter_core::KnowledgeEntrySummary; 0]
        );
        assert!(
            state
                .store
                .find_accumulation_result(command_id, space_id, "雨夜灯塔")
                .await
                .unwrap()
                .is_none()
        );
    }

    /// MVP 的生产注册表为空：能力查询与积累入口读同一份，都不产生写入。
    #[tokio::test]
    async fn accumulation_without_a_production_engine_reports_not_available_and_writes_nothing() {
        let (_dir, state) = accumulation_app(ACCUMULATION_ENGINES).await;
        let space_id = new_accumulation_space(&state).await;
        let base = spawn_app(state.clone());
        let client = reqwest::Client::new();

        let capabilities: Value = client
            .get(format!("{base}/api/capabilities"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(capabilities["knowledgeAccumulationEngines"], json!([]));
        assert!(capabilities["knowledgeAccumulationUnavailableReason"].is_string());

        let response = client
            .post(format!("{base}/api/knowledge/accumulate"))
            .json(&json!({
                "commandId": Uuid::new_v4(),
                "spaceId": space_id,
                "clues": "雨夜灯塔",
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], "ENGINE_NOT_AVAILABLE");
        assert_eq!(
            state
                .store
                .list_knowledge_entries(Some(space_id), None)
                .await
                .unwrap(),
            [] as [nekowriter_core::KnowledgeEntrySummary; 0]
        );
    }

    #[test]
    fn default_web_dir_lives_next_to_the_executable() {
        let web_dir = default_web_dir().unwrap();
        assert!(web_dir.ends_with(Path::new("webui").join("dist")));
        let executable = std::env::current_exe().unwrap();
        assert_eq!(
            web_dir.parent().unwrap().parent().unwrap(),
            executable.parent().unwrap()
        );
    }

    #[tokio::test]
    async fn index_html_is_revalidated_before_loading_route_chunks() {
        let web_dir = TempDir::new().unwrap();
        let index_html = web_dir.path().join("index.html");
        std::fs::write(&index_html, "<!doctype html><title>NekoWriter</title>").unwrap();

        let response = serve_index_html(index_html).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-cache"))
        );
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/html; charset=utf-8"))
        );
    }

    #[test]
    fn loopback_host_validation_rejects_external_hosts() {
        assert!(is_loopback_host("127.0.0.1:47821"));
        assert!(is_loopback_host("[::1]:47821"));
        assert!(!is_loopback_host("nekowriter.example"));
        assert!(!is_loopback_host("127.0.0.10"));
    }

    #[test]
    fn event_cursor_prefers_last_event_id_and_rejects_invalid_values() {
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", "42".parse().unwrap());
        assert_eq!(event_cursor(&headers, Some("7")).unwrap(), 42);
        assert_eq!(event_cursor(&HeaderMap::new(), Some("7")).unwrap(), 7);

        assert!(matches!(
            event_cursor(&HeaderMap::new(), Some("+7")),
            Err(ApiError::InvalidEventCursor)
        ));
        assert!(matches!(
            event_cursor(&HeaderMap::new(), Some("")),
            Err(ApiError::InvalidEventCursor)
        ));

        headers.insert("last-event-id", "-1".parse().unwrap());
        assert!(matches!(
            event_cursor(&headers, None),
            Err(ApiError::InvalidEventCursor)
        ));
    }

    #[tokio::test]
    async fn event_cursor_endpoint_reports_the_retained_global_watermark() {
        let (_dir, state) = accumulation_app(ACCUMULATION_ENGINES).await;
        state
            .store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("事件游标", "雨夜的来客", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let expected = state.store.event_stream_bounds().await.unwrap().watermark;
        let base = spawn_app(state);
        let response: Value = reqwest::Client::new()
            .get(format!("{base}/api/events/cursor"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(response["eventWatermark"], expected.to_string());
    }

    #[test]
    fn event_stream_requires_a_snapshot_reset_outside_its_retained_window() {
        let bounds = EventStreamBounds {
            minimum_cursor: 12,
            watermark: 21,
        };
        assert!(stream_reset_required(11, bounds));
        assert!(!stream_reset_required(12, bounds));
        assert!(!stream_reset_required(21, bounds));
        assert!(stream_reset_required(22, bounds));
    }

    #[tokio::test]
    async fn preview_delta_shares_sse_without_a_persistent_id() {
        let (_dir, state) = accumulation_app(ACCUMULATION_ENGINES).await;
        let version_id = Uuid::new_v4();
        let request_id = Uuid::new_v4();
        let base = spawn_app(state.clone());
        let response = reqwest::Client::new()
            .get(format!("{base}/api/events?after=0"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = response.bytes_stream();
        // 先消费连接注释，确保服务端已经订阅临时广播，再发出预览片段。
        let first = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("SSE 应先发送连接注释")
            .expect("SSE 不应在连接后立即关闭")
            .expect("SSE 连接注释不应写入错误");
        let _ = state.preview_deltas.send(scheduler::PreviewDelta {
            version_id,
            request_id,
            tool: Some(scheduler::PreviewTool {
                index: 0,
                id: Some("preview-tool-1".to_owned()),
                name: Some("append_content".to_owned()),
            }),
            offset: 0,
            content: "{\"content\":\"半截".to_owned(),
        });

        let mut wire = String::from_utf8(first.to_vec()).unwrap();
        for _ in 0..4 {
            if wire.contains("event: preview.delta") {
                break;
            }
            let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
                .await
                .expect("预览片段应在同一 SSE 连接中到达")
                .expect("SSE 不应在预览前关闭")
                .expect("SSE 预览不应写入错误");
            wire.push_str(core::str::from_utf8(&next).unwrap());
        }
        let preview_block = wire
            .split("\n\n")
            .find(|block| block.contains("event: preview.delta"))
            .expect("应该收到 preview.delta");
        assert!(preview_block.contains(&format!("\"requestId\":\"{request_id}\"")));
        assert!(preview_block.contains("\"offset\":\"0\""));
        assert!(preview_block.contains("\"append_content\""));
        assert!(
            !preview_block.lines().any(|line| line.starts_with("id:")),
            "临时预览不能携带可恢复的 SSE id：{preview_block}"
        );
        assert!(state.store.events_after(0).await.unwrap().is_empty());
    }

    #[test]
    fn state_revision_requests_require_decimal_strings() {
        let command_id = Uuid::new_v4();
        let parsed: StartPreparationRequest = serde_json::from_value(json!({
            "commandId": command_id,
            "expectedStateRevision": i64::MAX.to_string(),
        }))
        .unwrap();
        assert_eq!(parsed.command_id, command_id);
        assert_eq!(parsed.expected_state_revision, i64::MAX);
        assert!(
            serde_json::from_value::<StartPreparationRequest>(json!({
                "commandId": Uuid::new_v4(),
                "expectedStateRevision": 1,
            }))
            .is_err()
        );
    }

    /// 审核决定的请求体形状必须与界面 `decideChapterFinalReview` 真正发出的 JSON 一致。
    ///
    /// 「退回修订」写下的意见是正文 Agent 的修订要求：服务端把它排进正文 Run 的自由
    /// 意见批次（store 一侧见 `chapter_final_rejection_hands_its_issues_to_the_body_agent`）。
    /// 这里钉住界面真实发出的报文，避免退回意见因字段名或取值形状不符而根本没到服务端。
    #[test]
    fn review_decision_accepts_the_payload_the_ui_sends() {
        let command_id = Uuid::new_v4();
        let issues = "让沈宁主动留下查清寄信人；第二段的雨声描写与结尾重复。";
        let rejected: DecidePreparationReviewRequest = serde_json::from_value(json!({
            "commandId": command_id,
            "expectedStateRevision": "27",
            "passed": false,
            "issues": issues,
            "citations": [
                {
                    "kind": "document",
                    "documentId": Uuid::new_v4(),
                    "revisionId": Uuid::new_v4(),
                    "startChar": "4",
                    "endChar": "6",
                    "quote": "胶卷",
                    "chapterId": null,
                }
            ],
        }))
        .unwrap();
        assert_eq!(rejected.command_id, command_id);
        assert_eq!(rejected.expected_state_revision, 27);
        assert!(!rejected.passed);
        assert_eq!(rejected.issues.as_deref(), Some(issues));
        assert_eq!(rejected.citations.len(), 1);

        // 通过时界面把 issues 写成空字符串、citations 写成空数组。
        let approved: DecidePreparationReviewRequest = serde_json::from_value(json!({
            "commandId": Uuid::new_v4(),
            "expectedStateRevision": "28",
            "passed": true,
            "issues": "",
            "citations": [],
        }))
        .unwrap();
        assert!(approved.passed);
        assert_eq!(approved.issues.as_deref(), Some(""));
        assert!(approved.citations.is_empty());
    }

    /// 问答提交的请求体形状必须与界面 `answerQuestion` 真正发出的 JSON 一致。
    ///
    /// 实战里发现过反例：接口把领域类型 `UserQuestionAnswers` 当作同名字段，线上
    /// 形状变成 `answers.answers`，而界面按 `answers: [...]` 提交，于是每次提交都是
    /// 422——模型问一次问题就把整章卡在 `waiting_answer`，界面只给出一个永远不会
    /// 成功的提交按钮。
    #[test]
    fn answer_question_request_accepts_the_payload_the_ui_sends() {
        let command_id = Uuid::new_v4();
        let parsed: AnswerQuestionRequest = serde_json::from_value(json!({
            "commandId": command_id,
            "expectedStateRevision": "38",
            "answers": [
                {
                    "questionId": "q1",
                    "selectedOptionIds": ["night"],
                    "customText": "当夜约 2:10，巡道工因暴雨上线检查道岔。",
                },
                {
                    "questionId": "q2",
                    "selectedOptionIds": ["dialogue_only"],
                    "customText": null,
                },
            ],
        }))
        .unwrap();
        assert_eq!(parsed.command_id, command_id);
        assert_eq!(parsed.expected_state_revision, 38);
        assert_eq!(parsed.answers.len(), 2);
        assert_eq!(parsed.answers[0].question_id, "q1");
        assert_eq!(parsed.answers[0].selected_option_ids, ["night"]);
        assert_eq!(parsed.answers[1].custom_text, None);
        // 逐题答案仍然拒绝未知字段：多出来的一层 `answers` 不能静默通过。
        assert!(
            serde_json::from_value::<AnswerQuestionRequest>(json!({
                "commandId": command_id,
                "expectedStateRevision": "38",
                "answers": {
                    "answers": [
                        { "questionId": "q1", "selectedOptionIds": [], "customText": null },
                    ],
                },
            }))
            .is_err()
        );
    }

    /// Embedding 设置与连接测试的请求体形状必须与界面真正发出的 JSON 一致。
    ///
    /// 这条路径此前只有 store 级单测：`/api/questions/{id}/answer` 那个「服务端期望
    /// `answers.answers`、界面发 `answers: [...]`」的缺陷正是同一种漏检。这里用界面
    /// 真实抓到的报文（docs/03 §40）钉住两侧形状，而不只是 Rust 字段。
    #[test]
    fn embedding_requests_accept_the_payload_the_ui_sends() {
        let command_id = Uuid::new_v4();
        let save: knowledge::SaveEmbeddingSettingsRequest = serde_json::from_value(json!({
            "commandId": command_id,
            "expectedRevision": "2",
            "apiBaseUrl": "http://127.0.0.1:8899/v1",
            "modelId": "stub-embed",
            "dimension": "8",
            "apiKey": null,
        }))
        .unwrap();
        assert_eq!(save.command_id, command_id);
        assert_eq!(save.expected_revision, 2);
        assert_eq!(save.api_base_url, "http://127.0.0.1:8899/v1");
        assert_eq!(save.model_id, "stub-embed");
        assert_eq!(save.dimension, 8);
        // 保存命令里的 `apiKey` 是三态：`null` = 清除，省略 = 保留，字符串 = 替换。
        assert_eq!(save.api_key, CredentialPatch::Clear);

        let omitted: knowledge::SaveEmbeddingSettingsRequest = serde_json::from_value(json!({
            "commandId": command_id,
            "expectedRevision": "1",
            "apiBaseUrl": "http://127.0.0.1:8899/v1",
            "modelId": "stub-embed",
            "dimension": "8",
        }))
        .unwrap();
        assert_eq!(omitted.api_key, CredentialPatch::Keep);

        let with_key: knowledge::SaveEmbeddingSettingsRequest = serde_json::from_value(json!({
            "commandId": command_id,
            "expectedRevision": "0",
            "apiBaseUrl": "http://127.0.0.1:8899/v1",
            "modelId": "stub-embed",
            "dimension": "8",
            "apiKey": "probe-embed-key-001",
        }))
        .unwrap();
        assert_eq!(
            with_key.api_key,
            CredentialPatch::Replace("probe-embed-key-001".to_owned())
        );

        // 连接测试不是命令、不落库，它的 `apiKey` 只有两种含义（给一把临时的 / 用库里
        // 那把），所以刻意不用三态：省略与 `null` 都表示「用库中现有密钥」。
        let probe_with_key: knowledge::TestEmbeddingRequest = serde_json::from_value(json!({
            "apiBaseUrl": "http://127.0.0.1:8899/v1",
            "modelId": "stub-embed",
            "dimension": "8",
            "apiKey": "probe-embed-key-001",
        }))
        .unwrap();
        assert_eq!(
            probe_with_key.api_key.as_deref(),
            Some("probe-embed-key-001")
        );

        let test: knowledge::TestEmbeddingRequest = serde_json::from_value(json!({
            "apiBaseUrl": "http://127.0.0.1:8899/v1",
            "modelId": "stub-embed",
            "dimension": "8",
            "apiKey": null,
        }))
        .unwrap();
        assert_eq!(test.dimension, 8);
        assert_eq!(test.api_key, None);

        // 长整数只认十进制字符串；界面把 `dimension` 当字符串发，服务端不能顺手把
        // 数字也收下，否则两侧会在超过 JS 安全整数时悄悄分叉。
        assert!(
            serde_json::from_value::<knowledge::TestEmbeddingRequest>(json!({
                "apiBaseUrl": "http://127.0.0.1:8899/v1",
                "modelId": "stub-embed",
                "dimension": 8,
                "apiKey": null,
            }))
            .is_err()
        );
    }

    /// 路由层的三条边界：深链接回落、静态资源、以及 `/api` 不被回落吞掉。
    ///
    /// 前端用 path 路由，而磁盘上没有 `/works/...` 这样的文件：发行目录（而不是 Vite
    /// 开发服务器）里必须回落到入口 HTML，否则刷新或直接打开一个深链接只会看到 404。
    /// 反过来，未知的 `/api/*` 必须仍然是 JSON 错误信封，不能被回落成一份 HTML。
    #[tokio::test]
    async fn deep_links_fall_back_to_the_entry_html_without_swallowing_api_errors() {
        let dir = TempDir::new().unwrap();
        let web_dir = dir.path().join("dist");
        tokio::fs::create_dir_all(web_dir.join("assets"))
            .await
            .unwrap();
        tokio::fs::write(web_dir.join("index.html"), "<!doctype html>入口")
            .await
            .unwrap();
        tokio::fs::write(web_dir.join("assets").join("app.js"), "console.log(1)")
            .await
            .unwrap();

        let (_data_dir, state) = accumulation_app(ACCUMULATION_ENGINES).await;
        let base = spawn_app_with_web_dir(state, web_dir);
        let client = reqwest::Client::new();

        let deep_link = client
            .get(format!("{base}/works/abc/versions/def"))
            .send()
            .await
            .unwrap();
        assert_eq!(deep_link.status(), StatusCode::OK);
        assert!(deep_link.text().await.unwrap().contains("入口"));

        let asset = client
            .get(format!("{base}/assets/app.js"))
            .send()
            .await
            .unwrap();
        assert_eq!(asset.status(), StatusCode::OK);
        assert_eq!(asset.text().await.unwrap(), "console.log(1)");

        let unknown_api = client.get(format!("{base}/api/nope")).send().await.unwrap();
        assert_eq!(unknown_api.status(), StatusCode::NOT_FOUND);
        let body: Value = unknown_api.json().await.unwrap();
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }

    /// 界面能力必须真的可达：组件文件存在、却没有任何挂载点时，界面只会在点击那一刻
    /// 表现为「无反应」，既不会有编译错误，也不会让任何请求失败。
    ///
    /// 它只能在源码层对账——手法与事件清单相同。本轮之前 `ShortcutHelpPanel` 与
    /// `ServicePanel` 就是这样丢掉了挂载点。
    #[test]
    fn restored_ui_capabilities_are_mounted_in_the_app_shell() {
        let frontend = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../webui/src");
        for (file, marker) in [
            ("App.tsx", "<ShortcutHelpPanel"),
            ("pages/SettingsPage.tsx", "<ServicePanel"),
            ("components/AppFrame.tsx", "<ServiceStoppedNotice"),
            ("pages/WorkspacePage.tsx", "<ServiceStoppedNotice"),
        ] {
            let path = frontend.join(file);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()));
            assert!(
                source.contains(marker),
                "{} 不再挂载 {marker}：界面能力会静默消失",
                path.display()
            );
        }
    }

    /// 写请求的信任校验挂在路由层，而不是由每个 handler 自己记得调用。
    ///
    /// 这里用一个 handler 自己不做校验的路由：它只能被中间件拦下。
    #[tokio::test]
    async fn write_trust_is_enforced_by_the_router_not_by_handlers() {
        let (_dir, state) = accumulation_app(ACCUMULATION_ENGINES).await;
        let base = spawn_app(state);
        let client = reqwest::Client::new();

        let blocked = client
            .post(format!(
                "{base}/api/recycle-bin/knowledge_entry/{}/purge",
                Uuid::new_v4()
            ))
            .header("origin", "http://evil.example")
            .json(&json!({ "commandId": Uuid::new_v4() }))
            .send()
            .await
            .unwrap();
        assert_eq!(blocked.status(), StatusCode::FORBIDDEN);
        let body: Value = blocked.json().await.unwrap();
        assert_eq!(body["error"]["code"], "UNTRUSTED_ORIGIN");

        // `Host` 不是回环地址时也拦下：DNS 重绑定式的请求到不了 handler。
        let rebound = client
            .post(format!("{base}/api/settings"))
            .header("host", "evil.example")
            .json(&json!({ "commandId": Uuid::new_v4() }))
            .send()
            .await
            .unwrap();
        // 400 是这个错误码既有的状态码映射（`ApiError::UntrustedHost`），这里只钉住拦截本身。
        assert_eq!(rebound.status(), StatusCode::BAD_REQUEST);
        let body: Value = rebound.json().await.unwrap();
        assert_eq!(body["error"]["code"], "UNTRUSTED_HOST");

        // 读方法不经过这层：同一个不被信任的来源上 GET 仍然可用。
        let read = client
            .get(format!("{base}/api/settings"))
            .header("origin", "http://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(read.status(), StatusCode::OK);
    }

    /// 静态目录与 `/api` 的边界：前端深链接回落，静态资源与未知接口如实报错。
    ///
    /// 走真实 HTTP 而不是读代码：`/api/*` 必须拿到 JSON 错误信封（不被 SPA 回落吞掉），
    /// 缺失的静态资源必须 404（不被回落成 200 + HTML）。
    #[tokio::test]
    async fn static_dir_serves_client_routes_and_reports_missing_assets_honestly() {
        let web_dir = TempDir::new().unwrap();
        std::fs::write(
            web_dir.path().join("index.html"),
            "<!doctype html><div id=\"root\">NEKOWRITER_INDEX</div>",
        )
        .unwrap();
        std::fs::create_dir_all(web_dir.path().join("assets")).unwrap();
        std::fs::write(
            web_dir.path().join("assets/app.js"),
            "console.log('real-asset');",
        )
        .unwrap();

        let (_dir, state) = accumulation_app(ACCUMULATION_ENGINES).await;
        let base = spawn_app_with_web_dir(state, web_dir.path().to_path_buf());
        let client = reqwest::Client::new();

        // 前端路由（深链接）回落到入口 HTML：直接打开或刷新这些地址必须是界面而不是 404。
        for path in ["/", "/settings", "/works/abc/versions/def"] {
            let response = client.get(format!("{base}{path}")).send().await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "{path} 未回落到入口 HTML"
            );
            let content_type = content_type(&response);
            assert!(
                content_type.starts_with("text/html"),
                "{path} 的内容类型是 {content_type}"
            );
            let body = response.text().await.unwrap();
            assert!(
                body.contains("NEKOWRITER_INDEX"),
                "{path} 拿到的不是入口 HTML"
            );
        }

        // 磁盘上存在的静态资源按真实路径返回。
        let asset = client
            .get(format!("{base}/assets/app.js"))
            .send()
            .await
            .unwrap();
        assert_eq!(asset.status(), StatusCode::OK);
        assert!(asset.text().await.unwrap().contains("real-asset"));

        // 不存在的静态资源如实 404。（存量检查：回落成 200 + HTML 会让浏览器把它当脚本执行。）
        let missing = client
            .get(format!("{base}/assets/removed-chunk.js"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            missing.status(),
            StatusCode::NOT_FOUND,
            "缺失的静态资源被回落成了 HTML"
        );

        // 未知的 `/api` 路径是 JSON 错误信封，而不是入口 HTML。
        let unknown = client
            .get(format!("{base}/api/definitely-not-a-route"))
            .send()
            .await
            .unwrap();
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        let content_type = content_type(&unknown);
        assert!(
            content_type.starts_with("application/json"),
            "未知接口的内容类型是 {content_type}"
        );
        let body: Value = unknown.json().await.unwrap();
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }

    fn content_type(response: &reqwest::Response) -> String {
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    }

    /// 密钥三态在 HTTP 上真的成立：保存 → 省略 → 清除，且响应体从不带回密钥正文。
    ///
    /// 「未配置」是被降级逻辑依赖的合法状态，因此它必须能被写回，而不只是能读出。
    #[tokio::test]
    async fn system_model_credential_can_be_cleared_over_http() {
        let (_dir, state) = accumulation_app(ACCUMULATION_ENGINES).await;
        let base = spawn_app(state.clone());
        let client = reqwest::Client::new();
        let save = |revision: &str, api_key: Option<Value>| {
            let mut body = json!({
                "commandId": Uuid::new_v4(),
                "expectedRevision": revision,
                "apiBaseUrl": "https://model.example/v1",
                "modelId": "writer-1",
                "capabilities": {
                    "streaming": true,
                    "tools": true,
                    "streamUsage": true,
                    "thinking": true,
                },
                "maxContextTokens": "32768",
                "reservedOutputTokens": "4096",
            });
            if let Some(api_key) = api_key {
                body["apiKey"] = api_key;
            }
            body
        };
        let put = |body: Value| {
            client
                .put(format!("{base}/api/settings"))
                .json(&body)
                .send()
        };

        let created = put(save("0", Some(json!("secret-key-1")))).await.unwrap();
        assert_eq!(created.status(), StatusCode::OK);
        let created_body = created.text().await.unwrap();
        assert!(created_body.contains("\"credentialConfigured\":true"));
        // 响应体只公开「是否已配置」，密钥正文绝不出现在任何对外视图里。
        assert!(!created_body.contains("secret-key-1"));

        // 省略密钥：保留既有凭据。
        let kept = put(save("1", None)).await.unwrap();
        assert_eq!(kept.status(), StatusCode::OK);
        assert!(
            kept.text()
                .await
                .unwrap()
                .contains("\"credentialConfigured\":true")
        );
        assert_eq!(
            state.store.system_model_api_key().await.unwrap().as_deref(),
            Some("secret-key-1")
        );

        // 显式 `null`：清回未配置。
        let cleared = put(save("2", Some(Value::Null))).await.unwrap();
        assert_eq!(cleared.status(), StatusCode::OK);
        assert!(
            cleared
                .text()
                .await
                .unwrap()
                .contains("\"credentialConfigured\":false")
        );
        assert_eq!(state.store.system_model_api_key().await.unwrap(), None);
        assert_eq!(
            state
                .store
                .get_system_model_settings()
                .await
                .unwrap()
                .unwrap()
                .revision,
            3
        );
    }

    /// Embedding 设置响应必须与界面 `EmbeddingSettings` 接口同形。
    #[test]
    fn embedding_settings_response_matches_the_interface_shape() {
        let empty =
            serde_json::to_value(knowledge::EmbeddingSettingsResponse { embedding: None }).unwrap();
        assert_eq!(empty, json!({ "embedding": null }));

        let response = knowledge::EmbeddingSettingsResponse {
            embedding: Some(nekowriter_core::EmbeddingSettings {
                profile: nekowriter_core::EmbeddingProfile {
                    api_base_url: "http://127.0.0.1:8899/v1".to_owned(),
                    model_id: "stub-embed".to_owned(),
                    dimension: 8,
                },
                credential_configured: true,
                revision: 2,
                updated_at_ms: 1_789_399_707_544,
            }),
        };
        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "embedding": {
                    "profile": {
                        "apiBaseUrl": "http://127.0.0.1:8899/v1",
                        "modelId": "stub-embed",
                        "dimension": "8",
                    },
                    "credentialConfigured": true,
                    "revision": "2",
                    "updatedAtMs": "1789399707544",
                },
            })
        );
    }

    /// Embedding 失败必须点名到具体原因：界面只展示 `message`，一句「请检查地址、
    /// 模型、维度与密钥」会让用户在四个字段之间猜。适配器已经知道原因（维不符、
    /// HTTP 状态、连不上），这里要求它原样透出。
    #[tokio::test]
    async fn embedding_failures_name_the_concrete_reason() {
        let response = ApiError::Embedding(
            nekowriter_agent::embedding::EmbeddingError::DimensionMismatch {
                expected: 4,
                actual: 8,
            },
        )
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], "EMBEDDING_SERVICE_FAILED");
        assert_eq!(
            value["error"]["message"],
            "Embedding 维度不符：期望 4，实际 8"
        );
        assert_eq!(
            value["error"]["details"]["reason"],
            value["error"]["message"]
        );

        // 地址与字段本身不合规仍然是客户端错误，而且同样点名是哪一项。
        let response =
            ApiError::Embedding(nekowriter_agent::embedding::EmbeddingError::EmptyModelId)
                .into_response();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], "INVALID_EMBEDDING_SETTINGS");
        assert_eq!(value["error"]["message"], "Embedding 模型 ID 不能为空");
    }

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
