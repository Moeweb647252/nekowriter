//! SQLite 主数据与 `LanceDB` 连接。
//!
//! M0 有意只落地项目、稿本、工作流和命令回执的最小闭环。schema 具有明确
//! 版本；遇到旧格式时拒绝打开，而不是暗中迁移或重建。

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use nekowriter_core::{ProjectId, VersionId, utc_now_millis};
use sea_orm::prelude::TextUuid;
use sea_orm::{ActiveModelTrait, ConnectionTrait, DatabaseConnection, DbErr};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use tokio::sync::broadcast;
use url::Url;

/// 24 新增用户提示词模板；26 把 OS 凭据库引用换成直接保存在本地的 `api_key` 列。
/// 旧格式不能安全反序列化为当前合同，因此按项目约定拒绝打开而不尝试迁移。
pub const SCHEMA_VERSION: u32 = 26;
const CREATE_PROJECT_METHOD: &str = "POST";
const CREATE_PROJECT_TARGET: &str = "/api/projects";
const CREATE_VERSION_METHOD: &str = "POST";
const SELECT_VERSION_METHOD: &str = "POST";
/// 密钥正文的长度上限：足够容纳常见供应商的密钥，同时拒绝明显异常的粘贴内容。
pub(crate) const MAX_API_KEY_CHARS: usize = 4096;
pub(crate) const NOVEL_BRIEF_ROLE: &str = "novel_brief";
pub(crate) const CHAPTER_GUIDANCE_ROLE: &str = "chapter_guidance";
pub(crate) const CHAPTER_OUTLINE_ROLE: &str = "chapter_outline";
pub(crate) const CHAPTER_BODY_ROLE: &str = "chapter_body";

mod agent_messages;
mod budget;
mod chapters;
mod command;
mod dashboard;
pub(crate) mod entity;
mod event_log;
pub mod event_types;
mod export;
mod growth;
mod knowledge;
mod manuscript;
mod model_requests;
mod preparation;
mod project_settings;
mod projects;
mod prompt_templates;
mod prompts;
mod protocol_correction;
mod recycle;
mod schema;
mod settings;
mod system_operations;
mod version_copy;

pub use agent_messages::{AgentMessageRole, NewAgentMessage, PersistedAgentMessage};
pub(crate) use command::{
    Command, Notify, approval_mode_name, db_id, insert_receipt, parse_id, parse_optional_id,
    payload_hash, replay_or_conflict, transaction_error,
};
pub use event_log::{EventStreamBounds, PersistedEvent};
pub use growth::TableGrowth;
pub use knowledge::{
    DEFAULT_SEARCH_TOP_K, INDEX_TASK_FAILED, INDEX_TASK_PENDING, INDEX_TASK_READY,
    INDEX_TASK_RUNNING, KnowledgeIndexTask, KnowledgeSearchQuery, KnowledgeSpaceInput,
    MAX_SEARCH_TOP_K, NewEmbeddingSettings, NewKnowledgeEntry, UpdateKnowledgeEntry,
    chunk_knowledge_content,
};
pub use model_requests::{
    ModelRequestResolution, ModelRequestStatus, NewModelRequest, PersistedModelRequest,
};
pub use nekowriter_core::recycle::{RecycleBin, RecycleItem, RecycleItemKind, RecycleParent};
pub use preparation::{
    AgentQuestionToolCall, AppliedAgentQuestionToolCall, AppliedChapterBodyToolCall,
    AppliedFreeAgentToolCall, CancelReviewDelegationResult, ChapterBodyAgentToolCall,
    CompletedRunContextFold, CompletedRunContextStage, DelegatedReviewKind, DelegatedReviewTask,
    FreeAgentToolCall, InterruptionOutcome, InterruptionSource, MaterializedChapterBodyFeedback,
    QuestionAnswerResult, ReviewDelegationResult, StopWritingRunResult,
};
pub(crate) use projects::{
    insert_document_slots, insert_novel_version, insert_workflow, project_from_model,
    workflow_from_model,
};
pub use prompts::{NewPromptOverride, PromptOverride, PromptOverrideReset, PromptOverrideScope};
pub use protocol_correction::{
    ProtocolCorrectionTarget, RejectedRoundOutcome, RejectedRoundRecord,
};
pub use settings::{NewSystemModelSettings, SYSTEM_WRITING_MODEL_KEY, SystemModelSettings};
pub use system_operations::{
    NewSystemModelOperation, OPERATION_KIND_CONNECTION_TEST,
    OPERATION_KIND_EMBEDDING_CONNECTION_TEST, OPERATION_KIND_EMBEDDING_INDEX,
    OPERATION_KIND_EMBEDDING_QUERY, OPERATION_STATUS_FAILED, OPERATION_STATUS_SUCCEEDED,
    OPERATION_STATUS_UNKNOWN, is_embedding_operation,
};

use command::CommandReceipt;

#[derive(Clone)]
pub struct Store {
    database: DatabaseConnection,
    lance: Arc<lancedb::Connection>,
    data_dir: PathBuf,
    event_notifier: broadcast::Sender<()>,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("数据库错误：{0}")]
    Database(#[from] DbErr),
    #[error("LanceDB 错误：{0}")]
    Lance(#[from] lancedb::Error),
    #[error("文件系统错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("数据库 schema 缺少 NekoWriter 标识，已拒绝打开")]
    UnknownSchema,
    #[error("数据库 schema 版本为 {found}，当前程序仅支持 {expected}")]
    UnsupportedSchema { found: String, expected: u32 },
    #[error("项目不存在或已删除")]
    ProjectNotFound,
    #[error("稿本不存在或已删除")]
    VersionNotFound,
    #[error("章节不存在或已删除")]
    ChapterNotFound,
    #[error("稿本修订冲突：请求基于 {expected}，当前为 {actual}")]
    VersionRevisionConflict { expected: i64, actual: i64 },
    #[error("当前稿本不允许此操作：{0}")]
    InvalidVersionState(String),
    #[error("Run 不存在")]
    RunNotFound,
    #[error("模型请求不存在")]
    ModelRequestNotFound,
    #[error("问题不存在或已不属于当前工作流")]
    QuestionNotFound,
    #[error("问题已经有答案，不能再次提交")]
    QuestionAlreadyAnswered,
    #[error("问题或答案无效：{0}")]
    InvalidQuestion(String),
    #[error("小说目标尚未填写，不能开始准备阶段")]
    NovelBriefRequired,
    #[error("工作流状态已变化：请求基于版本 {expected}，当前为 {actual}")]
    StateRevisionConflict { expected: i64, actual: i64 },
    #[error("当前工作流不允许此准备操作：{0}")]
    InvalidPreparationState(String),
    #[error("当前工作流不允许此章节操作：{0}")]
    InvalidChapterState(String),
    #[error("正文编辑无效：{0}")]
    InvalidChapterBodyEdit(String),
    #[error("正文编辑无法应用：{0}")]
    ChapterBodyEdit(#[from] nekowriter_core::ChapterBodyEditError),
    #[error("自由意见不能为空")]
    EmptyFeedback,
    #[error("引用无效：{0}")]
    InvalidCitation(String),
    #[error("审核决定无效：{0}")]
    InvalidReviewDecision(String),
    #[error("文档修订不存在或所属文档已删除")]
    DocumentRevisionNotFound,
    #[error("正文自由意见在构建请求期间发生变化，请重新封存上下文")]
    FeedbackSnapshotStale,
    #[error("正文编辑提案不存在或不属于当前章节 Run")]
    ProposalNotFound,
    #[error("正文编辑提案已经有终态，不能再次决定")]
    ProposalAlreadyResolved,
    #[error("正文编辑提案的基础修订已变化：预期 {expected:?}，当前 {actual:?}")]
    ProposalBaseRevisionConflict {
        expected: Option<nekowriter_core::RevisionId>,
        actual: Option<nekowriter_core::RevisionId>,
    },
    #[error("准备产物提交无效：{0}")]
    InvalidPreparationArtifacts(String),
    #[error("审核不存在或已不属于当前稿本")]
    ReviewNotFound,
    #[error("审核已经有终态，不能再次决定")]
    ReviewAlreadyResolved,
    #[error("当前项目已有活跃写作任务：{0}")]
    ResourceBusy(String),
    #[error("模型请求无效：{0}")]
    InvalidModelRequest(String),
    #[error("模型请求状态无效：{0}")]
    InvalidModelRequestState(String),
    #[error("系统模型设置无效：{0}")]
    InvalidSystemModelSettings(String),
    #[error("系统模型设置已变化：请求基于版本 {expected}，当前为 {actual}")]
    SystemSettingsRevisionConflict { expected: i64, actual: i64 },
    #[error("Agent 消息无效：{0}")]
    InvalidAgentMessage(String),
    #[error("Agent 消息状态无效：{0}")]
    InvalidAgentMessageState(String),
    #[error("本轮模型请求额度已用完：已使用 {request_count}，上界 {request_allowance_end}")]
    RequestAllowanceExhausted {
        request_count: i64,
        request_allowance_end: i64,
    },
    #[error("工作流状态转换失败：{0}")]
    Workflow(#[from] nekowriter_core::WorkflowTransitionError),
    #[error("command_id 已用于不同请求")]
    IdempotencyConflict,
    #[error("提示词覆盖无效：{0}")]
    InvalidPromptOverride(String),
    #[error("提示词覆盖修订冲突：请求基于 {expected}，当前为 {actual}")]
    PromptOverrideRevisionConflict { expected: i64, actual: i64 },
    #[error("项目设置无效：{0}")]
    InvalidProjectSettings(String),
    #[error("项目设置修订冲突：请求基于 {expected}，当前为 {actual}")]
    ProjectSettingsRevisionConflict { expected: i64, actual: i64 },
    #[error("持久化数据损坏：{0}")]
    CorruptRecord(String),
    #[error("序列化失败：{0}")]
    Serialization(#[from] serde_json::Error),
    #[error("知识 Space 不存在或已删除")]
    KnowledgeSpaceNotFound,
    #[error("知识条目不存在或已删除")]
    KnowledgeEntryNotFound,
    #[error("知识 Space 仍有条目，不能删除；请先移动或删除条目")]
    KnowledgeSpaceNotEmpty,
    #[error("知识 Space 修订冲突：请求基于 {expected}，当前为 {actual}")]
    KnowledgeSpaceRevisionConflict { expected: i64, actual: i64 },
    #[error("知识条目修订冲突：请求基于 {expected}，当前为 {actual}")]
    KnowledgeEntryRevisionConflict { expected: i64, actual: i64 },
    #[error("知识条目无效：{0}")]
    InvalidKnowledgeEntry(String),
    #[error("Embedding 配置无效：{0}")]
    InvalidEmbeddingSettings(String),
    #[error("Embedding 配置修订冲突：请求基于 {expected}，当前为 {actual}")]
    EmbeddingSettingsRevisionConflict { expected: i64, actual: i64 },
    #[error("尚未配置 Embedding，语义检索不可用")]
    EmbeddingNotConfigured,
    #[error("Embedding 维度不符：期望 {expected}，实际 {actual}")]
    EmbeddingDimensionMismatch { expected: i64, actual: i64 },
    #[error("知识检索输入无效：{0}")]
    InvalidKnowledgeSearch(String),
    #[error("提示词模板不存在或已删除")]
    PromptTemplateNotFound,
    #[error("提示词模板修订冲突：请求基于 {expected}，当前为 {actual}")]
    PromptTemplateRevisionConflict { expected: i64, actual: i64 },
    #[error("提示词模板无效：{0}")]
    InvalidPromptTemplate(String),
    #[error("Dashboard 查询无效：{0}")]
    InvalidDashboardQuery(String),
    #[error("对象不在回收站里：不存在，或已经被彻底删除")]
    RecycleItemNotFound,
    #[error("{0}仍在回收站里，请先恢复它")]
    RecycleAncestorDeleted(String),
    #[error("该对象不能彻底删除：{0}")]
    RecycleNotPurgeable(String),
}

impl Store {
    pub async fn open(data_dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let data_dir = data_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&data_dir)?;

        let database = schema::open_database(&data_dir).await?;

        let lance_dir = data_dir.join("lancedb");
        let lance_uri = lance_dir.to_string_lossy().into_owned();
        let lance = lancedb::connect(&lance_uri).execute().await?;
        // 连通性检查在启动时进行。实际知识表由 M4 的索引任务创建。
        let _ = lance.table_names().execute().await?;
        let (event_notifier, _) = broadcast::channel(256);
        let store = Self {
            database,
            lance: Arc::new(lance),
            data_dir,
            event_notifier,
        };
        store
            .interrupt_active_runs(InterruptionSource::StartupRecovery)
            .await?;
        event_log::discard_expired_events(&store).await?;
        Ok(store)
    }

    /// 退出前的数据收尾：先把 WAL 落回主库文件，再关闭连接池。
    ///
    /// 调用方必须已经取消所有本地执行任务，否则关闭之后仍会有写入尝试。
    ///
    /// # Errors
    ///
    /// WAL 回写或关闭连接池失败时返回数据库错误；此时进程仍应继续退出。
    pub async fn close(&self) -> Result<(), StoreError> {
        self.database
            .execute_unprepared("PRAGMA wal_checkpoint(TRUNCATE)")
            .await?;
        self.database.close_by_ref().await?;
        Ok(())
    }

    #[must_use]
    pub fn schema_version(&self) -> u32 {
        SCHEMA_VERSION
    }

    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    #[must_use]
    pub fn lance(&self) -> &lancedb::Connection {
        &self.lance
    }
}

/// 一次设置保存对密钥列的意图。
///
/// 三态是显式的，而且直接对应 HTTP 合同：`apiKey` 省略 = [`CredentialPatch::Keep`]、
/// `apiKey: null` = [`CredentialPatch::Clear`]、字符串 = [`CredentialPatch::Replace`]。
/// 用枚举而不是 `Option<Option<String>>`，因为「未配置」是被其它逻辑依赖的合法状态
/// （`EMBEDDING_NOT_CONFIGURED` 降级、界面「未配置」提示、`credential_configured`
/// 都由它决定），调用点必须一眼分得清「不动密钥」与「把它清掉」。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CredentialPatch {
    /// 省略：保留库中现有密钥。
    #[default]
    Keep,
    /// 显式 `null`：把密钥列清回未配置。
    Clear,
    /// 字符串：替换为新的密钥正文。
    Replace(String),
}

impl CredentialPatch {
    /// 把这条意图并到库中现有密钥上，得到落库后的列值。
    ///
    /// 写入与「保存前的真实探测该用哪把密钥」共用这一处合并语义，两侧不会分叉。
    #[must_use]
    pub fn merge(self, existing: Option<String>) -> Option<String> {
        match self {
            Self::Keep => existing,
            Self::Clear => None,
            Self::Replace(value) => Some(value),
        }
    }

    /// 校验并归一化密钥正文；`Keep` 与 `Clear` 原样通过。
    pub(crate) fn normalize(self) -> Result<Self, String> {
        match self {
            Self::Replace(value) => normalize_api_key(&value).map(Self::Replace),
            patch => Ok(patch),
        }
    }

    /// 命令指纹里的密钥意图：只区分三态，不含密钥正文。
    pub(crate) const fn intent(&self) -> CredentialIntent {
        match self {
            Self::Keep => CredentialIntent::Keep,
            Self::Clear => CredentialIntent::Clear,
            Self::Replace(_) => CredentialIntent::Replace,
        }
    }
}

/// 命令指纹里的密钥意图；只记「保留 / 替换 / 清除」，不记密钥正文。
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CredentialIntent {
    Keep,
    Replace,
    Clear,
}

/// 省略字段得到 [`CredentialPatch::Keep`]，`null` 得到 [`CredentialPatch::Clear`]。
impl<'de> Deserialize<'de> for CredentialPatch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match Option::<String>::deserialize(deserializer)? {
            Some(value) => Self::Replace(value),
            None => Self::Clear,
        })
    }
}

/// 校验并归一化一条 API 根地址。
///
/// 系统写作模型与 Embedding 两条设置路径共用这一处判定，因此「API 根地址合法」
/// 只有一种强度：http(s)、带主机、不带用户名/密码、不带 query/fragment、路径非空、
/// 且不是 `chat/completions` 这类完整端点。判定覆盖了适配器发请求时再检查的那几条
/// （见 `nekowriter_agent::provider`），所以设置能存进去就意味着它发得出去。
///
/// 返回去掉首尾空白与末尾斜杠的地址：两个适配器都会在拼接请求路径时补上分隔符。
pub(crate) fn validate_api_base_url(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("服务地址不能为空".to_owned());
    }
    let url = Url::parse(trimmed).map_err(|error| format!("服务地址无效：{error}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("服务地址必须是带主机的 HTTP(S) URL".to_owned());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("服务地址不能携带用户名或密码".to_owned());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("服务地址不能包含 query 或 fragment".to_owned());
    }
    let path = url.path().trim_matches('/');
    if path.is_empty() {
        return Err("服务地址必须是 API 根路径（例如 https://host/v1）".to_owned());
    }
    if path.ends_with("chat/completions") {
        return Err(
            "服务地址必须是 API 根路径（例如 https://host/v1），不能是 chat/completions 完整端点"
                .to_owned(),
        );
    }
    Ok(trimmed.trim_end_matches('/').to_owned())
}

/// 归一化一条密钥正文：去掉首尾空白，拒绝空值、超长与控制字符。
///
/// 返回 `Err` 时携带面向用户的原因，由调用方包进自己的设置错误变体：写作模型与
/// Embedding 两条设置路径共用同一套密钥规则，不各自维护一份。
pub(crate) fn normalize_api_key(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err("API key 不能为空".to_owned())
    } else if value.chars().count() > MAX_API_KEY_CHARS {
        Err(format!("API key 不能超过 {MAX_API_KEY_CHARS} 个字符"))
    } else if value.chars().any(char::is_control) {
        Err("API key 不能包含控制字符".to_owned())
    } else {
        Ok(value.to_owned())
    }
}

#[cfg(test)]
mod tests;
