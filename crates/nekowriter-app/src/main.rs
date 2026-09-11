use std::{
    collections::{HashSet, VecDeque},
    convert::Infallible,
    fs::{File, OpenOptions},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

mod credentials;
mod scheduler;

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, StatusCode, header},
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
    provider::{
        ChatCompletionsAdapter, ChatCompletionsConfig, ModelCapabilities, ModelRequest,
        ModelRequestMessage, ModelTimeouts, ProviderError,
    },
};
use nekowriter_core::{
    AppendChapterContent, ApprovalMode, ChapterBodyEditError, ChapterId, ChapterPlanDraft,
    ExecutionStatus, FeedbackCitation, HumanReviewDecision, NewChapterGuidance, NewChapterOutline,
    NewCopiedNovelVersion, NewExtraChapter, NewNovelBrief, NewNovelVersion, NewProject,
    PreparationArtifactInput,
    PreparationArtifactKind, PreparationSnapshot, ProjectBundle, ProjectId, ProjectListItem,
    ReplaceChapterContent, ReviewId, UserQuestionAnswers, ValidationError, VersionId,
    VersionSnapshot, WorkflowSnapshot, WorkflowStage, decimal_i64,
};
use nekowriter_store::{EventStreamBounds, PersistedEvent, Store, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
    signal,
    sync::{Mutex, Semaphore, broadcast},
};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
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
    /// 已构建前端资源目录；开发时默认使用当前工作目录的 webui/dist。
    #[arg(long)]
    web_dir: Option<PathBuf>,
}

#[derive(Clone)]
struct AppState {
    store: Store,
    allowed_write_origins: Arc<HashSet<String>>,
    settings_write_lock: Arc<Mutex<()>>,
    model_request_permit: Arc<Semaphore>,
    scheduler: scheduler::WorkflowScheduler,
}

struct EventStreamState {
    store: Store,
    receiver: broadcast::Receiver<()>,
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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DelegateReviewRequest {
    command_id: Uuid,
    #[serde(with = "decimal_i64")]
    expected_state_revision: i64,
    #[serde(default)]
    notes: String,
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
    answers: UserQuestionAnswers,
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
    capabilities: nekowriter_store::ModelCapabilityProfile,
    #[serde(with = "decimal_i64")]
    max_context_tokens: i64,
    #[serde(with = "decimal_i64")]
    reserved_output_tokens: i64,
    #[serde(default)]
    api_key: Option<String>,
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
    capabilities: nekowriter_store::ModelCapabilityProfile,
    max_context_tokens: String,
    reserved_output_tokens: String,
    revision: String,
    credential_configured: bool,
    created_at_ms: String,
    updated_at_ms: String,
}

#[derive(Debug, Deserialize)]
struct TestModelRequest {}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TestModelResponse {
    provider_request_id: Option<String>,
    content_received: bool,
    tool_call_count: usize,
    usage: Option<nekowriter_agent::provider::ModelUsage>,
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
    Credential(credentials::CredentialStoreError),
    Provider(ProviderError),
    Scheduler(scheduler::SchedulerError),
    InvalidEventCursor,
    UntrustedHost,
    UntrustedOrigin,
    NotFound,
}

impl From<ValidationError> for ApiError {
    fn from(value: ValidationError) -> Self {
        Self::Validation(value)
    }
}

impl From<StoreError> for ApiError {
    fn from(value: StoreError) -> Self {
        Self::Store(value)
    }
}

impl From<credentials::CredentialStoreError> for ApiError {
    fn from(value: credentials::CredentialStoreError) -> Self {
        Self::Credential(value)
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
                | StoreError::QuestionNotFound,
            ) => (
                StatusCode::NOT_FOUND,
                "RESOURCE_NOT_FOUND",
                "请求的资源不存在或已删除".to_owned(),
                json!({}),
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
                | StoreError::InvalidModelRequestState(message)
                | StoreError::InvalidAgentMessageState(message),
            ) => (
                StatusCode::CONFLICT,
                "INVALID_TRANSITION",
                message.clone(),
                json!({ "reason": message }),
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
            Self::Store(StoreError::InvalidCitation(message)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_CITATION",
                "引用无法对应当前可验证文本".to_owned(),
                json!({ "reason": message }),
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
            Self::Store(StoreError::InvalidModelRequest(message))
            | Self::Store(StoreError::InvalidAgentMessage(message))
            | Self::Store(StoreError::InvalidSystemModelSettings(message)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "VALIDATION_ERROR",
                message.clone(),
                json!({ "reason": message }),
            ),
            Self::Credential(error) => {
                warn!(request_id = %request_id, error = %error, "OS 凭据库操作失败");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "CREDENTIAL_STORE_UNAVAILABLE",
                    "无法访问本机凭据库；请检查 Windows Credential Manager 后重试".to_owned(),
                    json!({}),
                )
            }
            Self::Provider(error) => {
                warn!(request_id = %request_id, error = %error, "模型服务调用失败");
                (
                    StatusCode::BAD_GATEWAY,
                    "MODEL_TEST_FAILED",
                    "模型服务未通过实际连接测试；请检查地址、模型、密钥和能力配置".to_owned(),
                    json!({}),
                )
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

#[tokio::main]
async fn main() -> Result<(), StartupError> {
    init_tracing();
    let cli = Cli::parse();
    let data_dir = resolve_data_dir(cli.data_dir.unwrap_or(default_data_dir()?))?;
    let _lock = ProcessLock::acquire(&data_dir)?;
    let store = Store::open(&data_dir).await?;
    let web_dir = cli.web_dir.unwrap_or(default_web_dir()?);
    if !web_dir.exists() {
        warn!(path = %web_dir.display(), "未找到前端构建产物；可使用 pnpm --dir webui build 后重启，或在开发模式下运行 Vite");
    }

    let model_request_permit = Arc::new(Semaphore::new(1));
    let settings_write_lock = Arc::new(Mutex::new(()));
    let state = AppState {
        scheduler: scheduler::WorkflowScheduler::new(
            store.clone(),
            model_request_permit.clone(),
            settings_write_lock.clone(),
        ),
        store,
        allowed_write_origins: Arc::new(allowed_write_origins(cli.port)),
        settings_write_lock,
        model_request_permit,
    };
    let app = router(state, web_dir);
    let address = SocketAddr::from(([127, 0, 0, 1], cli.port));
    let listener = tokio::net::TcpListener::bind(address).await?;
    info!(address = %address, "NekoWriter 已启动");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn router(state: AppState, web_dir: PathBuf) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/capabilities", get(get_capabilities))
        .route(
            "/settings",
            get(get_system_settings).put(save_system_settings),
        )
        .route(
            "/settings/test-model",
            axum::routing::post(test_model_connection),
        )
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{project_id}", get(get_project))
        .route(
            "/projects/{project_id}/versions",
            get(list_versions).post(create_version),
        )
        .route(
            "/projects/{project_id}/selected-version",
            axum::routing::post(select_version),
        )
        .route("/versions/{version_id}/snapshot", get(get_version_snapshot))
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
        .route("/events", get(stream_events))
        .route("/versions/{version_id}/preparation", get(get_preparation))
        .route("/versions/{version_id}/brief", put(save_novel_brief))
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
        .with_state(state);

    Router::new()
        .nest("/api", api)
        .fallback_service(ServeDir::new(web_dir).append_index_html_on_directories(true))
        .layer(TraceLayer::new_for_http())
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        schema_version: state.store.schema_version(),
    })
}

async fn get_capabilities() -> Json<nekowriter_agent::AgentCapabilities> {
    Json(capabilities())
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
    headers: HeaderMap,
    Json(request): Json<SaveSystemModelSettingsRequest>,
) -> Result<Json<SystemSettingsResponse>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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

    let credential_ref = match (&api_key, &current) {
        (Some(_), Some(current)) => current
            .credential_ref
            .clone()
            .or_else(|| Some(format!("writing-model-{}", Uuid::new_v4()))),
        (Some(_), None) => Some(format!("writing-model-{}", Uuid::new_v4())),
        (None, Some(current)) => current.credential_ref.clone(),
        (None, None) => None,
    };
    let settings = nekowriter_store::NewSystemModelSettings {
        api_base_url,
        credential_ref: credential_ref.clone(),
        credential_fingerprint: api_key.as_deref().map(credential_fingerprint),
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

    let previous_api_key = if let (Some(_), Some(credential_ref)) = (&api_key, &credential_ref) {
        match credentials::read_api_key(credential_ref.clone()).await {
            Ok(api_key) => Some(api_key),
            Err(credentials::CredentialStoreError::Missing) => None,
            Err(error) => return Err(error.into()),
        }
    } else {
        None
    };
    let created_credential = api_key.is_some()
        && current
            .as_ref()
            .and_then(|settings| settings.credential_ref.as_ref())
            != credential_ref.as_ref();
    if let (Some(api_key), Some(credential_ref)) = (api_key, credential_ref.clone()) {
        credentials::write_api_key(credential_ref, api_key).await?;
    }

    match state
        .store
        .save_system_model_settings(command_id, expected_revision, settings)
        .await
    {
        Ok(saved) => Ok(Json(system_settings_response(Some(saved)))),
        Err(error) => {
            if let Some(credential_ref) = credential_ref {
                if let Some(previous_api_key) = previous_api_key {
                    let _ = credentials::write_api_key(credential_ref, previous_api_key).await;
                } else if created_credential {
                    let _ = credentials::delete_api_key(credential_ref).await;
                }
            }
            Err(error.into())
        }
    }
}

fn credential_fingerprint(api_key: &str) -> String {
    format!("{:x}", Sha256::digest(api_key.as_bytes()))
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
            credential_configured: settings.credential_ref.is_some(),
            created_at_ms: settings.created_at_ms.to_string(),
            updated_at_ms: settings.updated_at_ms.to_string(),
        }),
    }
}

async fn test_model_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(_request): Json<TestModelRequest>,
) -> Result<Json<TestModelResponse>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let settings_guard = state.settings_write_lock.lock().await;
    let settings = state
        .store
        .get_system_model_settings()
        .await?
        .ok_or_else(|| StoreError::InvalidSystemModelSettings("请先保存写作模型设置".to_owned()))?;
    let api_key = match settings.credential_ref.clone() {
        Some(credential_ref) => Some(credentials::read_api_key(credential_ref).await?),
        None => None,
    };
    let config = ChatCompletionsConfig::new(
        &settings.api_base_url,
        settings.model_id,
        api_key,
        ModelCapabilities {
            streaming: settings.capabilities.streaming,
            tools: settings.capabilities.tools,
            structured_output: settings.capabilities.structured_output,
            stream_usage: settings.capabilities.stream_usage,
        },
        ModelTimeouts::default(),
    )?;
    let provider = ChatCompletionsAdapter::new(config)?;
    drop(settings_guard);
    let request = ModelRequest {
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
        response_format: None,
    };
    // 系统设置测试与写作 Agent 共用同一个全局许可，避免用户在测试连接时绕开
    // “全局最多一个模型网络请求”的约束。
    let _permit = state
        .model_request_permit
        .clone()
        .acquire_owned()
        .await
        .expect("AppState 持有模型调度 Semaphore，许可不会永久关闭");
    let completion = provider.execute_with(&request, |_| {}).await?;
    Ok(Json(TestModelResponse {
        provider_request_id: completion.provider_request_id,
        content_received: completion
            .content
            .as_deref()
            .is_some_and(|content| !content.trim().is_empty()),
        tool_call_count: completion.tool_calls.len(),
        usage: completion.usage,
    }))
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

async fn list_chapters(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
) -> Result<Json<Vec<nekowriter_core::ChapterSummary>>, ApiError> {
    Ok(Json(state.store.list_chapters(version_id).await?))
}

async fn advance_to_next_chapter(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    headers: HeaderMap,
    Json(request): Json<AdvanceToNextChapterRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<ConfirmNovelCompletionRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<AddExtraChapterRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<SaveChapterGuidanceRequest>,
) -> Result<Json<nekowriter_core::CurrentChapterGuidance>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<StartChapterOutlineRequest>,
) -> Result<Json<nekowriter_core::WorkflowSnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    ensure_writing_model_ready(&state).await?;
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
    headers: HeaderMap,
    Json(request): Json<SubmitChapterOutlineRequest>,
) -> Result<Json<nekowriter_core::ChapterOutlineSnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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

async fn append_chapter_body(
    State(state): State<AppState>,
    AxumPath(chapter_id): AxumPath<ChapterId>,
    headers: HeaderMap,
    Json(request): Json<AppendChapterBodyRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<ReplaceChapterBodyRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<DecideChapterBodyProposalRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<SubmitRunFeedbackRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<AnswerQuestionRequest>,
) -> Result<Json<nekowriter_store::QuestionAnswerResult>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let response = state
        .store
        .answer_user_question(
            question_id,
            request.command_id,
            request.expected_state_revision,
            request.answers,
        )
        .await?;
    enqueue_queued_writing_run(&state, &response.workflow).await?;
    Ok(Json(response))
}

async fn continue_writing_run(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<nekowriter_core::RunId>,
    headers: HeaderMap,
    Json(request): Json<RunWorkflowControlRequest>,
) -> Result<Json<WorkflowSnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    ensure_writing_model_ready(&state).await?;
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
    headers: HeaderMap,
    Json(request): Json<RunWorkflowControlRequest>,
) -> Result<Json<WorkflowSnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<RunWorkflowControlRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<RunWorkflowControlRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<SetApprovalModeRequest>,
) -> Result<Json<WorkflowSnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<SubmitChapterFinalReviewRequest>,
) -> Result<Json<nekowriter_core::ChapterBodySnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    // 先订阅再检查游标边界。此后即使有提交发生，events 表查询和广播唤醒都不会
    // 留下“查历史到订阅实时”之间的丢失窗口。
    let receiver = state.store.subscribe_events();
    let bounds = state.store.event_stream_bounds().await?;
    if stream_reset_required(cursor, bounds) {
        return Ok(Sse::new(stream::once(async move {
            Ok::<_, Infallible>(stream_reset_event(cursor, bounds))
        }))
        .into_response());
    }

    let stream_state = EventStreamState {
        receiver,
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
            Ok(_) => match state.receiver.recv().await {
                Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return None,
            },
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
    headers: HeaderMap,
    Json(request): Json<SaveNovelBriefRequest>,
) -> Result<Json<nekowriter_core::CurrentNovelBrief>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<StartPreparationRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    ensure_writing_model_ready(&state).await?;
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

async fn ensure_writing_model_ready(state: &AppState) -> Result<(), ApiError> {
    let settings = state
        .store
        .get_system_model_settings()
        .await?
        .ok_or_else(|| StoreError::InvalidSystemModelSettings("请先保存写作模型设置".to_owned()))?;
    if !settings.capabilities.structured_output {
        return Err(StoreError::InvalidSystemModelSettings(
            "当前写作阶段需要支持结构化提交的写作模型".to_owned(),
        )
        .into());
    }
    if !settings.capabilities.tools {
        return Err(StoreError::InvalidSystemModelSettings(
            "当前写作阶段需要支持 ask_user 的工具调用模型".to_owned(),
        )
        .into());
    }
    if let Some(credential_ref) = settings.credential_ref {
        // 仅验证凭据库中的引用仍可读取；真实请求在取得全局许可后重新读取，绝不把
        // 密钥封存进请求快照或跨异步边界传递。
        let _ = credentials::read_api_key(credential_ref).await?;
    }
    Ok(())
}

/// 审核 Agent 不调用工具，但和写作阶段一样必须能稳定返回结构化结论。
async fn ensure_review_model_ready(state: &AppState) -> Result<(), ApiError> {
    let settings = state
        .store
        .get_system_model_settings()
        .await?
        .ok_or_else(|| StoreError::InvalidSystemModelSettings("请先保存写作模型设置".to_owned()))?;
    if !settings.capabilities.structured_output {
        return Err(StoreError::InvalidSystemModelSettings(
            "审核 Agent 需要支持结构化提交的模型".to_owned(),
        )
        .into());
    }
    if let Some(credential_ref) = settings.credential_ref {
        let _ = credentials::read_api_key(credential_ref).await?;
    }
    Ok(())
}

async fn submit_preparation_artifacts(
    State(state): State<AppState>,
    AxumPath(version_id): AxumPath<VersionId>,
    headers: HeaderMap,
    Json(request): Json<SubmitPreparationArtifactsRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<DecidePreparationReviewRequest>,
) -> Result<Json<PreparationSnapshot>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let decision = HumanReviewDecision::new(request.passed, request.issues.unwrap_or_default())?;
    let snapshot = state
        .store
        .decide_review(
            review_id,
            request.command_id,
            request.expected_state_revision,
            decision,
        )
        .await?;
    enqueue_queued_writing_run(&state, &snapshot.workflow).await?;
    Ok(Json(snapshot))
}

async fn delegate_review(
    State(state): State<AppState>,
    AxumPath(review_id): AxumPath<ReviewId>,
    headers: HeaderMap,
    Json(request): Json<DelegateReviewRequest>,
) -> Result<(StatusCode, Json<nekowriter_store::ReviewDelegationResult>), ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    ensure_review_model_ready(&state).await?;
    let delegation = state
        .store
        .delegate_review(
            review_id,
            request.command_id,
            request.expected_state_revision,
            request.notes,
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
    headers: HeaderMap,
    Json(request): Json<CancelReviewDelegationRequest>,
) -> Result<Json<nekowriter_store::CancelReviewDelegationResult>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectBundle>), ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<CreateVersionRequest>,
) -> Result<(StatusCode, Json<ProjectBundle>), ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<SelectVersionRequest>,
) -> Result<Json<ProjectBundle>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
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
    headers: HeaderMap,
    Json(request): Json<CopyVersionRequest>,
) -> Result<(StatusCode, Json<ProjectBundle>), ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let input = NewCopiedNovelVersion::new(request.label)?;
    let bundle = state
        .store
        .copy_version(version_id, request.command_id, input)
        .await?;
    Ok((StatusCode::CREATED, Json(bundle)))
}

async fn api_not_found() -> ApiError {
    ApiError::NotFound
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

/// 在取得进程锁、SQLite 和 LanceDB 连接之前固定数据目录的物理位置。
///
/// 这样相同的相对 `--data-dir` 不会因启动工作目录不同而指向不同的本地作品库。
fn resolve_data_dir(path: PathBuf) -> Result<PathBuf, StartupError> {
    std::fs::create_dir_all(&path)?;
    Ok(path.canonicalize()?)
}

fn default_web_dir() -> Result<PathBuf, StartupError> {
    Ok(std::env::current_dir()?.join("webui").join("dist"))
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "nekowriter=info,tower_http=info".into());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

async fn shutdown_signal() {
    if let Err(error) = signal::ctrl_c().await {
        warn!(error = %error, "无法监听 Ctrl+C；服务将由进程终止");
        std::future::pending::<()>().await;
    }
    info!("收到退出信号，正在停止服务");
}

#[cfg(test)]
mod tests {
    use super::*;

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
