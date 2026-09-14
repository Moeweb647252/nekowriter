//! SQLite 主数据与 `LanceDB` 连接。
//!
//! M0 有意只落地项目、稿本、工作流和命令回执的最小闭环。schema 具有明确
//! 版本；遇到旧格式时拒绝打开，而不是暗中迁移或重建。

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use nekowriter_core::{
    ApprovalMode, ChapterId, DocumentId, NewNovelVersion, NewProject, NovelVersion,
    PreparationArtifactKind, Project, ProjectBundle, ProjectId, ProjectListItem, ProjectProgress,
    ReviewId, RunId, VersionId, VersionSnapshot, WorkflowSnapshot, utc_now_millis,
};
use sea_orm::prelude::TextUuid;
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, Set, TransactionError, TransactionTrait,
};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::broadcast;
use uuid::Uuid;

/// 24 新增用户提示词模板。旧格式不能安全反序列化为当前合同，因此按项目约定
/// 拒绝打开而不尝试迁移。
pub const SCHEMA_VERSION: u32 = 25;
const CREATE_PROJECT_METHOD: &str = "POST";
const CREATE_PROJECT_TARGET: &str = "/api/projects";
const CREATE_VERSION_METHOD: &str = "POST";
const SELECT_VERSION_METHOD: &str = "POST";
pub(crate) const NOVEL_BRIEF_ROLE: &str = "novel_brief";
pub(crate) const CHAPTER_GUIDANCE_ROLE: &str = "chapter_guidance";
pub(crate) const CHAPTER_OUTLINE_ROLE: &str = "chapter_outline";
pub(crate) const CHAPTER_BODY_ROLE: &str = "chapter_body";

mod agent_messages;
mod budget;
mod chapters;
mod dashboard;
pub(crate) mod entity;
mod event_log;
mod export;
mod knowledge;
mod manuscript;
mod model_requests;
mod preparation;
mod project_settings;
mod prompt_templates;
mod prompts;
mod protocol_correction;
mod recycle;
mod schema;
mod settings;
mod system_operations;
mod version_copy;

pub use agent_messages::{AgentMessageRole, NewAgentMessage, PersistedAgentMessage};
pub use event_log::{EventStreamBounds, PersistedEvent};
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
pub use prompts::{NewPromptOverride, PromptOverride, PromptOverrideReset, PromptOverrideScope};
pub use protocol_correction::{
    ModelProtocolCorrection, ProtocolCorrectionOutcome, ProtocolCorrectionTarget,
};
pub use settings::{NewSystemModelSettings, SYSTEM_WRITING_MODEL_KEY, SystemModelSettings};
pub use system_operations::{
    NewSystemModelOperation, OPERATION_KIND_CONNECTION_TEST,
    OPERATION_KIND_EMBEDDING_CONNECTION_TEST, OPERATION_KIND_EMBEDDING_INDEX,
    OPERATION_KIND_EMBEDDING_QUERY, OPERATION_STATUS_FAILED, OPERATION_STATUS_SUCCEEDED,
    OPERATION_STATUS_UNKNOWN, is_embedding_operation,
};

use entity::{
    chapter, command_receipt, document, document_revision, novel_version, project,
    workflow_instance,
};

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

#[derive(Clone, Debug)]
struct CommandReceipt {
    method: String,
    target: String,
    payload_hash: String,
    response_json: String,
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

    /// 返回某个稿本所属的项目。
    ///
    /// 项目级提示词覆盖以项目为作用域；调度器在构建请求时必须能由稿本解析出它
    /// 所属的项目，才能应用项目覆盖。
    ///
    /// # Errors
    ///
    /// 稿本不存在或已删除时返回 [`StoreError::VersionNotFound`]。
    pub async fn project_id_for_version(
        &self,
        version_id: VersionId,
    ) -> Result<ProjectId, StoreError> {
        novel_version::Entity::find_by_id(db_id(version_id))
            .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
            .one(&self.database)
            .await?
            .map(|model| model.project_id.into())
            .ok_or(StoreError::VersionNotFound)
    }

    /// 返回某个运行所属的项目。
    ///
    /// 命令前置检查需要按项目解析有效模型，而运行行直接记录 `project_id`。
    ///
    /// # Errors
    ///
    /// 运行不存在时返回 [`StoreError::RunNotFound`]。
    pub async fn project_id_for_run(&self, run_id: RunId) -> Result<ProjectId, StoreError> {
        entity::run::Entity::find_by_id(db_id(run_id))
            .one(&self.database)
            .await?
            .map(|model| model.project_id.into())
            .ok_or(StoreError::RunNotFound)
    }

    /// 返回某章所属的项目：先由章节解析稿本，再由稿本解析项目。
    ///
    /// # Errors
    ///
    /// 章节不存在或已删除时返回 [`StoreError::ChapterNotFound`]，稿本不存在时返回
    /// [`StoreError::VersionNotFound`]。
    pub async fn project_id_for_chapter(
        &self,
        chapter_id: ChapterId,
    ) -> Result<ProjectId, StoreError> {
        let version_id = entity::chapter::Entity::find_by_id(db_id(chapter_id))
            .filter(entity::chapter::Entity::COLUMN.deleted_at.is_null())
            .one(&self.database)
            .await?
            .map(|model| model.version_id.into())
            .ok_or(StoreError::ChapterNotFound)?;
        self.project_id_for_version(version_id).await
    }

    /// 返回某次审核所属的项目。
    ///
    /// # Errors
    ///
    /// 审核不存在时返回 [`StoreError::ReviewNotFound`]，稿本不存在时返回
    /// [`StoreError::VersionNotFound`]。
    pub async fn project_id_for_review(
        &self,
        review_id: ReviewId,
    ) -> Result<ProjectId, StoreError> {
        let version_id = entity::review::Entity::find_by_id(db_id(review_id))
            .one(&self.database)
            .await?
            .map(|model| model.version_id.into())
            .ok_or(StoreError::ReviewNotFound)?;
        self.project_id_for_version(version_id).await
    }

    pub async fn list_projects(&self) -> Result<Vec<ProjectListItem>, StoreError> {
        let projects = project::Entity::find()
            .filter(project::Entity::COLUMN.deleted_at.is_null())
            .order_by_desc(project::Entity::COLUMN.updated_at)
            .order_by_asc(project::Entity::COLUMN.id)
            .all(&self.database)
            .await?;
        if projects.is_empty() {
            return Ok(Vec::new());
        }

        let selected_version_ids = projects
            .iter()
            .map(|project| project.selected_version_id)
            .collect::<Vec<_>>();
        let versions = novel_version::Entity::find()
            .filter(novel_version::Entity::COLUMN.id.is_in(selected_version_ids))
            .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
            .all(&self.database)
            .await?;
        let version_ids = versions
            .iter()
            .map(|version| version.id)
            .collect::<Vec<_>>();
        let workflows = workflow_instance::Entity::find()
            .filter(
                workflow_instance::Entity::COLUMN
                    .version_id
                    .is_in(version_ids.clone()),
            )
            .all(&self.database)
            .await?;

        let versions_by_id = versions
            .into_iter()
            .map(|version| (version.id, version))
            .collect::<std::collections::HashMap<_, _>>();
        let workflows_by_version_id = workflows
            .into_iter()
            .map(|workflow| (workflow.version_id, workflow))
            .collect::<std::collections::HashMap<_, _>>();
        let progress_by_version_id = self.project_progress_by_version(&version_ids).await?;

        let mut items = Vec::with_capacity(projects.len());
        for project_model in projects {
            let Some(version_model) = versions_by_id.get(&project_model.selected_version_id) else {
                continue;
            };
            let Some(workflow_model) = workflows_by_version_id.get(&version_model.id) else {
                continue;
            };
            let bundle = project_bundle_from_models(&project_model, version_model, workflow_model)?;
            items.push(ProjectListItem {
                project: bundle.project,
                selected_version: bundle.version,
                workflow: bundle.workflow,
                progress: progress_by_version_id
                    .get(&version_model.id)
                    .cloned()
                    .unwrap_or_default(),
            });
        }
        Ok(items)
    }

    /// 批量回算作品库所需的当前稿本进度；不读取正文内容，也不会为每张卡片单独查库。
    async fn project_progress_by_version(
        &self,
        version_ids: &[TextUuid],
    ) -> Result<std::collections::HashMap<TextUuid, ProjectProgress>, StoreError> {
        let mut progress_by_version_id = version_ids
            .iter()
            .copied()
            .map(|version_id| (version_id, ProjectProgress::default()))
            .collect::<std::collections::HashMap<_, _>>();
        if version_ids.is_empty() {
            return Ok(progress_by_version_id);
        }

        let chapters = chapter::Entity::find()
            .filter(
                chapter::Entity::COLUMN
                    .version_id
                    .is_in(version_ids.to_vec()),
            )
            .filter(chapter::Entity::COLUMN.deleted_at.is_null())
            .all(&self.database)
            .await?;
        for chapter in chapters {
            let progress = progress_by_version_id
                .get_mut(&chapter.version_id)
                .expect("章节查询只会返回当前选中稿本");
            match chapter.kind.as_str() {
                "main" => {
                    progress.main_chapter_total += 1;
                    match chapter.status.as_str() {
                        "approved" => progress.main_chapter_approved += 1,
                        "planned" | "in_progress" => {}
                        other => {
                            return Err(StoreError::CorruptRecord(format!(
                                "未知章节状态：{other}"
                            )));
                        }
                    }
                }
                "extra" => {
                    progress.extra_chapter_total += 1;
                    if !matches!(
                        chapter.status.as_str(),
                        "planned" | "in_progress" | "approved"
                    ) {
                        return Err(StoreError::CorruptRecord(format!(
                            "未知章节状态：{}",
                            chapter.status
                        )));
                    }
                }
                other => {
                    return Err(StoreError::CorruptRecord(format!("未知章节类型：{other}")));
                }
            }
        }

        let body_documents = document::Entity::find()
            .filter(
                document::Entity::COLUMN
                    .version_id
                    .is_in(version_ids.to_vec()),
            )
            .filter(document::Entity::COLUMN.deleted_at.is_null())
            .filter(document::Entity::COLUMN.role.eq(CHAPTER_BODY_ROLE))
            .all(&self.database)
            .await?;
        let current_revision_ids = body_documents
            .iter()
            .filter_map(|document| document.current_revision_id.as_deref())
            .map(|revision_id| parse_id(revision_id, "document.current_revision_id"))
            .collect::<Result<Vec<_>, _>>()?;
        let revisions = document_revision::Entity::find()
            .filter(
                document_revision::Entity::COLUMN.id.is_in(
                    current_revision_ids
                        .into_iter()
                        .map(db_id)
                        .collect::<Vec<_>>(),
                ),
            )
            .all(&self.database)
            .await?;
        let word_count_by_revision_id = revisions
            .into_iter()
            .map(|revision| (revision.id, revision.word_count))
            .collect::<std::collections::HashMap<_, _>>();
        for document in body_documents {
            let Some(revision_id) = document.current_revision_id.as_deref() else {
                continue;
            };
            let revision_id = db_id(parse_id(revision_id, "document.current_revision_id")?);
            let word_count = word_count_by_revision_id
                .get(&revision_id)
                .copied()
                .ok_or_else(|| {
                    StoreError::CorruptRecord("正文槽指向不存在的当前修订".to_owned())
                })?;
            let progress = progress_by_version_id
                .get_mut(&document.version_id)
                .expect("正文槽查询只会返回当前选中稿本");
            progress.current_word_count += word_count;
        }

        Ok(progress_by_version_id)
    }

    pub async fn get_project(&self, project_id: ProjectId) -> Result<ProjectBundle, StoreError> {
        let project_model = project::Entity::find_by_id(db_id(project_id))
            .filter(project::Entity::COLUMN.deleted_at.is_null())
            .one(&self.database)
            .await?
            .ok_or(StoreError::ProjectNotFound)?;
        let version_model = novel_version::Entity::find_by_id(project_model.selected_version_id)
            .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
            .one(&self.database)
            .await?
            .ok_or(StoreError::ProjectNotFound)?;
        let workflow_model = workflow_instance::Entity::find_by_id(version_model.id)
            .one(&self.database)
            .await?
            .ok_or(StoreError::ProjectNotFound)?;
        project_bundle_from_models(&project_model, &version_model, &workflow_model)
    }

    pub async fn list_versions(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<NovelVersion>, StoreError> {
        if !self.project_exists(project_id).await? {
            return Err(StoreError::ProjectNotFound);
        }
        novel_version::Entity::find()
            .filter(
                novel_version::Entity::COLUMN
                    .project_id
                    .eq(db_id(project_id)),
            )
            .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
            .order_by_asc(novel_version::Entity::COLUMN.created_at)
            .order_by_asc(novel_version::Entity::COLUMN.id)
            .all(&self.database)
            .await?
            .iter()
            .map(version_from_model)
            .collect::<Result<Vec<_>, _>>()
    }

    pub async fn get_version_snapshot(
        &self,
        version_id: VersionId,
    ) -> Result<VersionSnapshot, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move { version_snapshot(transaction, version_id).await })
            })
            .await
            .map_err(transaction_error)
    }

    pub async fn create_project(
        &self,
        command_id: Uuid,
        input: NewProject,
    ) -> Result<ProjectBundle, StoreError> {
        let payload_hash = payload_hash(&CreateProjectPayload::from(&input))?;
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(
                receipt,
                CREATE_PROJECT_METHOD,
                CREATE_PROJECT_TARGET,
                &payload_hash,
            );
        }

        let now = utc_now_millis();
        let project_id = Uuid::new_v4();
        let version_id = Uuid::new_v4();
        let workflow = WorkflowSnapshot::initial(version_id);
        let project = Project {
            id: project_id,
            name: input.name,
            selected_version_id: version_id,
            revision: 1,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let version = NovelVersion {
            id: version_id,
            project_id,
            label: input.version_label,
            novel_title: input.novel_title,
            source_version_id: None,
            body_completed_at_ms: None,
            revision: 1,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let bundle = ProjectBundle {
            project,
            version,
            workflow,
        };

        let bundle_for_transaction = bundle.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    insert_project_bundle(transaction, &bundle_for_transaction).await?;
                    preparation::insert_app_event(
                        transaction,
                        "project.changed",
                        bundle_for_transaction.project.id,
                        bundle_for_transaction.project.revision,
                        serde_json::json!({
                            "action": "created",
                            "project_id": bundle_for_transaction.project.id,
                            "version_id": bundle_for_transaction.version.id,
                        }),
                        now,
                    )
                    .await?;
                    let response_json = serde_json::to_string(&bundle_for_transaction)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        CREATE_PROJECT_METHOD,
                        CREATE_PROJECT_TARGET,
                        &payload_hash_for_transaction,
                        &response_json,
                        201,
                        now,
                    )
                    .await?;
                    Ok(bundle_for_transaction)
                })
            })
            .await
            .map_err(transaction_error);

        match result {
            Ok(bundle) => {
                self.notify_event_listeners();
                Ok(bundle)
            }
            Err(StoreError::Database(error)) => {
                self.replay_after_receipt_conflict(
                    command_id,
                    CREATE_PROJECT_METHOD,
                    CREATE_PROJECT_TARGET,
                    &payload_hash,
                    StoreError::Database(error),
                )
                .await
            }
            Err(error) => Err(error),
        }
    }

    pub async fn create_version(
        &self,
        project_id: ProjectId,
        command_id: Uuid,
        input: NewNovelVersion,
    ) -> Result<ProjectBundle, StoreError> {
        let target = format!("/api/projects/{project_id}/versions");
        let payload_hash = payload_hash(&CreateVersionPayload::from(&input))?;
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(receipt, CREATE_VERSION_METHOD, &target, &payload_hash);
        }

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let project_model = project::Entity::find_by_id(db_id(project_id))
                        .filter(project::Entity::COLUMN.deleted_at.is_null())
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ProjectNotFound)?;
                    let mut project = project_from_model(&project_model);
                    let now = utc_now_millis();
                    let version_id = Uuid::new_v4();
                    let workflow = WorkflowSnapshot::initial(version_id);
                    let version = NovelVersion {
                        id: version_id,
                        project_id,
                        label: input.label,
                        novel_title: input.novel_title,
                        source_version_id: None,
                        body_completed_at_ms: None,
                        revision: 1,
                        created_at_ms: now,
                        updated_at_ms: now,
                    };
                    project.selected_version_id = version_id;
                    project.revision += 1;
                    project.updated_at_ms = now;

                    insert_novel_version(transaction, &version).await?;
                    insert_workflow(transaction, &workflow).await?;
                    insert_document_slots(transaction, version_id, now).await?;
                    let mut active_project = project_model.into_active_model();
                    active_project.selected_version_id = Set(db_id(project.selected_version_id));
                    active_project.revision = Set(project.revision);
                    active_project.updated_at = Set(project.updated_at_ms);
                    active_project.update(transaction).await?;

                    let bundle = ProjectBundle {
                        project,
                        version,
                        workflow,
                    };
                    preparation::insert_app_event(
                        transaction,
                        "project.changed",
                        bundle.project.id,
                        bundle.project.revision,
                        serde_json::json!({
                            "action": "version_created",
                            "project_id": bundle.project.id,
                            "version_id": bundle.version.id,
                        }),
                        now,
                    )
                    .await?;
                    let response_json = serde_json::to_string(&bundle)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        CREATE_VERSION_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        201,
                        now,
                    )
                    .await?;
                    Ok(bundle)
                })
            })
            .await
            .map_err(transaction_error);

        match result {
            Ok(bundle) => {
                self.notify_event_listeners();
                Ok(bundle)
            }
            Err(StoreError::Database(error)) => {
                self.replay_after_receipt_conflict(
                    command_id,
                    CREATE_VERSION_METHOD,
                    &target,
                    &payload_hash,
                    StoreError::Database(error),
                )
                .await
            }
            Err(error) => Err(error),
        }
    }

    /// 切换项目工作台当前选中的稿本。
    pub async fn select_version(
        &self,
        project_id: ProjectId,
        version_id: VersionId,
        command_id: Uuid,
        expected_project_revision: i64,
    ) -> Result<ProjectBundle, StoreError> {
        let target = format!("/api/projects/{project_id}/selected-version");
        let payload_hash = payload_hash(&SelectVersionPayload {
            version_id,
            expected_project_revision,
        })?;
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(receipt, SELECT_VERSION_METHOD, &target, &payload_hash);
        }

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let project_model = project::Entity::find_by_id(db_id(project_id))
                        .filter(project::Entity::COLUMN.deleted_at.is_null())
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ProjectNotFound)?;
                    if project_model.revision != expected_project_revision {
                        return Err(StoreError::StateRevisionConflict {
                            expected: expected_project_revision,
                            actual: project_model.revision,
                        });
                    }
                    let version_model = novel_version::Entity::find_by_id(db_id(version_id))
                        .filter(
                            novel_version::Entity::COLUMN
                                .project_id
                                .eq(db_id(project_id)),
                        )
                        .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::VersionNotFound)?;
                    let workflow_model = workflow_instance::Entity::find_by_id(db_id(version_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::VersionNotFound)?;
                    let now = utc_now_millis();
                    let mut active_project = project_model.into_active_model();
                    active_project.selected_version_id = Set(db_id(version_id));
                    active_project.revision = Set(expected_project_revision + 1);
                    active_project.updated_at = Set(now);
                    let updated_project = active_project.update(transaction).await?;
                    let bundle = project_bundle_from_models(
                        &updated_project,
                        &version_model,
                        &workflow_model,
                    )?;
                    preparation::insert_app_event(
                        transaction,
                        "project.changed",
                        bundle.project.id,
                        bundle.project.revision,
                        serde_json::json!({
                            "action": "selected_version_changed",
                            "project_id": bundle.project.id,
                            "version_id": bundle.version.id,
                        }),
                        now,
                    )
                    .await?;
                    let response_json = serde_json::to_string(&bundle)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SELECT_VERSION_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(bundle)
                })
            })
            .await
            .map_err(transaction_error);

        match result {
            Ok(bundle) => {
                self.notify_event_listeners();
                Ok(bundle)
            }
            Err(error) => {
                self.replay_after_receipt_conflict(
                    command_id,
                    SELECT_VERSION_METHOD,
                    &target,
                    &payload_hash,
                    error,
                )
                .await
            }
        }
    }

    async fn replay_after_receipt_conflict<T: DeserializeOwned>(
        &self,
        command_id: Uuid,
        method: &str,
        target: &str,
        expected_hash: &str,
        original_error: StoreError,
    ) -> Result<T, StoreError> {
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(receipt, method, target, expected_hash);
        }
        Err(original_error)
    }

    async fn project_exists(&self, project_id: ProjectId) -> Result<bool, StoreError> {
        Ok(project::Entity::find_by_id(db_id(project_id))
            .filter(project::Entity::COLUMN.deleted_at.is_null())
            .one(&self.database)
            .await?
            .is_some())
    }

    async fn find_receipt(&self, command_id: Uuid) -> Result<Option<CommandReceipt>, StoreError> {
        let receipt = command_receipt::Entity::find_by_id(db_id(command_id))
            .one(&self.database)
            .await?;
        Ok(receipt.map(|receipt| CommandReceipt {
            method: receipt.method,
            target: receipt.target,
            payload_hash: receipt.payload_hash,
            response_json: receipt.response_json,
        }))
    }
}

async fn version_snapshot<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<VersionSnapshot, StoreError> {
    let version_model = novel_version::Entity::find_by_id(db_id(version_id))
        .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .ok_or(StoreError::VersionNotFound)?;
    let project_model = project::Entity::find_by_id(version_model.project_id)
        .filter(project::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .ok_or(StoreError::VersionNotFound)?;
    let workflow_model = workflow_instance::Entity::find_by_id(version_model.id)
        .one(connection)
        .await?
        .ok_or(StoreError::VersionNotFound)?;
    let bundle = project_bundle_from_models(&project_model, &version_model, &workflow_model)?;
    let watermark = event_log::retained_event_watermark(connection).await?;

    Ok(VersionSnapshot {
        project: bundle.project,
        version: bundle.version,
        workflow: bundle.workflow,
        event_watermark: watermark.to_string(),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateProjectPayload<'a> {
    name: &'a str,
    novel_title: &'a str,
    version_label: &'a str,
}

impl<'a> From<&'a NewProject> for CreateProjectPayload<'a> {
    fn from(value: &'a NewProject) -> Self {
        Self {
            name: &value.name,
            novel_title: &value.novel_title,
            version_label: &value.version_label,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateVersionPayload<'a> {
    label: &'a str,
    novel_title: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SelectVersionPayload {
    version_id: VersionId,
    expected_project_revision: i64,
}

impl<'a> From<&'a NewNovelVersion> for CreateVersionPayload<'a> {
    fn from(value: &'a NewNovelVersion) -> Self {
        Self {
            label: &value.label,
            novel_title: &value.novel_title,
        }
    }
}

fn payload_hash<T: Serialize>(payload: &T) -> Result<String, StoreError> {
    let bytes = serde_json::to_vec(payload)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn replay_or_conflict<T: DeserializeOwned>(
    receipt: CommandReceipt,
    expected_method: &str,
    expected_target: &str,
    expected_hash: &str,
) -> Result<T, StoreError> {
    if receipt.method != expected_method
        || receipt.target != expected_target
        || receipt.payload_hash != expected_hash
    {
        return Err(StoreError::IdempotencyConflict);
    }
    Ok(serde_json::from_str(&receipt.response_json)?)
}

async fn insert_project_bundle<C: ConnectionTrait>(
    connection: &C,
    bundle: &ProjectBundle,
) -> Result<(), StoreError> {
    project::ActiveModel {
        id: Set(db_id(bundle.project.id)),
        name: Set(bundle.project.name.clone()),
        selected_version_id: Set(db_id(bundle.project.selected_version_id)),
        settings_json: Set("{}".to_owned()),
        settings_revision: Set(0),
        revision: Set(bundle.project.revision),
        created_at: Set(bundle.project.created_at_ms),
        updated_at: Set(bundle.project.updated_at_ms),
        deleted_at: Set(None),
        ..Default::default()
    }
    .insert(connection)
    .await?;
    insert_novel_version(connection, &bundle.version).await?;
    insert_workflow(connection, &bundle.workflow).await?;
    insert_document_slots(connection, bundle.version.id, bundle.version.created_at_ms).await
}

async fn insert_novel_version<C: ConnectionTrait>(
    connection: &C,
    version: &NovelVersion,
) -> Result<(), StoreError> {
    novel_version::ActiveModel {
        id: Set(db_id(version.id)),
        project_id: Set(db_id(version.project_id)),
        label: Set(version.label.clone()),
        novel_title: Set(version.novel_title.clone()),
        source_version_id: Set(version.source_version_id.map(|id| id.to_string())),
        body_completed_at: Set(version.body_completed_at_ms),
        revision: Set(version.revision),
        created_at: Set(version.created_at_ms),
        updated_at: Set(version.updated_at_ms),
        deleted_at: Set(None),
        ..Default::default()
    }
    .insert(connection)
    .await?;
    Ok(())
}

async fn insert_document_slots<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
    created_at_ms: i64,
) -> Result<(), StoreError> {
    let roles = std::iter::once(NOVEL_BRIEF_ROLE).chain(
        PreparationArtifactKind::ALL
            .into_iter()
            .map(PreparationArtifactKind::stable_key),
    );
    for role in roles {
        document::ActiveModel {
            id: Set(db_id(DocumentId::new_v4())),
            version_id: Set(db_id(version_id)),
            chapter_id: Set(None),
            role: Set(role.to_owned()),
            current_revision_id: Set(None),
            revision: Set(0),
            created_at: Set(created_at_ms),
            updated_at: Set(created_at_ms),
            deleted_at: Set(None),
            ..Default::default()
        }
        .insert(connection)
        .await?;
    }
    Ok(())
}

async fn insert_workflow<C: ConnectionTrait>(
    connection: &C,
    workflow: &WorkflowSnapshot,
) -> Result<(), StoreError> {
    workflow_instance::ActiveModel {
        version_id: Set(db_id(workflow.version_id)),
        state_json: Set(serde_json::to_string(workflow)?),
        state_revision: Set(workflow.state_revision),
        active_run_id: Set(workflow.active_run_id.map(|run_id| run_id.to_string())),
        approval_mode: Set(approval_mode_name(&workflow.approval_mode).to_owned()),
        pause_requested: Set(workflow.pause_requested),
        ..Default::default()
    }
    .insert(connection)
    .await?;
    Ok(())
}

async fn insert_receipt<C: ConnectionTrait>(
    connection: &C,
    command_id: Uuid,
    method: &str,
    target: &str,
    payload_hash: &str,
    response_json: &str,
    response_status: i64,
    created_at: i64,
) -> Result<(), StoreError> {
    command_receipt::ActiveModel {
        command_id: Set(db_id(command_id)),
        method: Set(method.to_owned()),
        target: Set(target.to_owned()),
        payload_hash: Set(payload_hash.to_owned()),
        response_status: Set(response_status),
        response_json: Set(response_json.to_owned()),
        created_at: Set(created_at),
        ..Default::default()
    }
    .insert(connection)
    .await?;
    Ok(())
}

fn project_bundle_from_models(
    project_model: &project::Model,
    version_model: &novel_version::Model,
    workflow_model: &workflow_instance::Model,
) -> Result<ProjectBundle, StoreError> {
    let project = project_from_model(project_model);
    let version = version_from_model(version_model)?;
    let workflow = workflow_from_model(workflow_model)?;
    // `selected_version_id` 只是项目工作台当前展示的稿本，并不限定可读取的
    // 稿本；运行中的旧稿必须仍能单独取得其工作流快照。
    if workflow.version_id != version.id || version.project_id != project.id {
        return Err(StoreError::CorruptRecord(
            "项目、稿本和工作流的归属不一致".to_owned(),
        ));
    }
    Ok(ProjectBundle {
        project,
        version,
        workflow,
    })
}

fn version_from_model(model: &novel_version::Model) -> Result<NovelVersion, StoreError> {
    Ok(NovelVersion {
        id: model.id.into(),
        project_id: model.project_id.into(),
        label: model.label.clone(),
        novel_title: model.novel_title.clone(),
        source_version_id: parse_optional_id(model.source_version_id.clone(), "source_version_id")?,
        body_completed_at_ms: model.body_completed_at,
        revision: model.revision,
        created_at_ms: model.created_at,
        updated_at_ms: model.updated_at,
    })
}

fn project_from_model(model: &project::Model) -> Project {
    Project {
        id: model.id.into(),
        name: model.name.clone(),
        selected_version_id: model.selected_version_id.into(),
        revision: model.revision,
        created_at_ms: model.created_at,
        updated_at_ms: model.updated_at,
    }
}

fn workflow_from_model(model: &workflow_instance::Model) -> Result<WorkflowSnapshot, StoreError> {
    let workflow = serde_json::from_str::<WorkflowSnapshot>(&model.state_json)?;
    let version_id: Uuid = model.version_id.into();
    let active_run_id = parse_optional_id(model.active_run_id.clone(), "active_run_id")?;
    if workflow.version_id != version_id || workflow.active_run_id != active_run_id {
        return Err(StoreError::CorruptRecord(
            "工作流与稿本的归属不一致".to_owned(),
        ));
    }
    Ok(workflow)
}

fn db_id(id: Uuid) -> TextUuid {
    id.into()
}

fn parse_id(value: &str, field: &str) -> Result<Uuid, StoreError> {
    Uuid::parse_str(value)
        .map_err(|error| StoreError::CorruptRecord(format!("{field} 不是有效 UUID：{error}")))
}

fn parse_optional_id(value: Option<String>, field: &str) -> Result<Option<Uuid>, StoreError> {
    value
        .as_deref()
        .map(|value| parse_id(value, field))
        .transpose()
}

fn transaction_error(error: TransactionError<StoreError>) -> StoreError {
    match error {
        TransactionError::Connection(error) => StoreError::Database(error),
        TransactionError::Transaction(error) => error,
    }
}

/// 命令事务提交后的统一收尾：通知事件监听者，并在唯一键竞争时按回执重放。
///
/// 所有带 `command_id` 的写入共用它，因此幂等重放规则只有一处。
async fn finish_write<T: DeserializeOwned + Serialize>(
    store: &Store,
    command_id: Uuid,
    method: &str,
    target: &str,
    payload_hash: &str,
    result: Result<T, StoreError>,
) -> Result<T, StoreError> {
    match result {
        Ok(response) => {
            store.notify_event_listeners();
            Ok(response)
        }
        Err(StoreError::Database(error)) => {
            store
                .replay_after_receipt_conflict(
                    command_id,
                    method,
                    target,
                    payload_hash,
                    StoreError::Database(error),
                )
                .await
        }
        Err(error) => Err(error),
    }
}

const fn approval_mode_name(mode: &ApprovalMode) -> &'static str {
    match mode {
        ApprovalMode::AutoApprove => "auto_approve",
        ApprovalMode::ApproveRequired => "approve_required",
    }
}

#[cfg(test)]
mod tests {
    use arrow_array::record_batch;
    use futures_util::TryStreamExt;
    use lancedb::query::{ExecutableQuery, QueryBase};
    use nekowriter_core::{
        AppendChapterContent, ChapterPlanDraft, ExecutionStatus, HumanReviewDecision,
        MAX_CONSECUTIVE_PROTOCOL_ERRORS, ModelCapabilityProfile, NewChapterGuidance,
        NewChapterOutline, NewCopiedNovelVersion, NewExtraChapter, NewNovelBrief, NewNovelVersion,
        NewProject, NewPromptTemplate, PreparationArtifactInput, PreparationArtifactKind,
        ReplaceChapterContent, ReviewStatus, RevisionId, RunWaitReason, WorkflowStage,
        non_whitespace_char_count,
    };
    use sea_orm::{EntityTrait, ModelTrait};
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn projects_persist_and_command_retries_do_not_duplicate_them() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let command_id = Uuid::new_v4();
        let project = NewProject::new("雨夜计划", "雨夜的来客", None::<&str>).unwrap();

        let created = store
            .create_project(command_id, project.clone())
            .await
            .unwrap();
        let replayed = store.create_project(command_id, project).await.unwrap();
        assert_eq!(created, replayed);
        assert_eq!(store.list_projects().await.unwrap().len(), 1);
        let project_events = store
            .events_after(0)
            .await
            .unwrap()
            .into_iter()
            .filter(|event| event.event_type == "project.changed")
            .collect::<Vec<_>>();
        assert_eq!(project_events.len(), 1, "重放命令不能重复写入项目事件");
        assert_eq!(project_events[0].payload["action"], "created");
        assert_eq!(
            project_events[0].payload["project_id"],
            created.project.id.to_string()
        );

        drop(store);
        let reopened = Store::open(data_dir.path()).await.unwrap();
        assert_eq!(
            reopened.get_project(created.project.id).await.unwrap(),
            created
        );
    }

    #[tokio::test]
    async fn project_list_progress_is_batched_from_current_chapters_and_revisions() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("进度摘要", "雨夜的来客", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let now = utc_now_millis();
        let main_approved_id = ChapterId::new_v4();
        let main_planned_id = ChapterId::new_v4();
        let extra_id = ChapterId::new_v4();
        for (id, kind, status, order_index) in [
            (main_approved_id, "main", "approved", 1),
            (main_planned_id, "main", "planned", 2),
            (extra_id, "extra", "in_progress", 1),
        ] {
            chapter::ActiveModel {
                id: Set(db_id(id)),
                version_id: Set(db_id(created.version.id)),
                kind: Set(kind.to_owned()),
                plan_item_id: Set(None),
                order_index: Set(order_index),
                title: Set(format!("章节 {order_index}")),
                status: Set(status.to_owned()),
                approved_revision_id: Set(None),
                revision: Set(0),
                created_at: Set(now),
                updated_at: Set(now),
                deleted_at: Set(None),
            }
            .insert(&store.database)
            .await
            .unwrap();
        }

        for (chapter_id, content) in [(main_approved_id, "雨落码头"), (extra_id, "尾声仍在继续")]
        {
            let document_id = DocumentId::new_v4();
            let revision_id = RevisionId::new_v4();
            document::ActiveModel {
                id: Set(db_id(document_id)),
                version_id: Set(db_id(created.version.id)),
                chapter_id: Set(Some(chapter_id.to_string())),
                role: Set(CHAPTER_BODY_ROLE.to_owned()),
                current_revision_id: Set(None),
                revision: Set(0),
                created_at: Set(now),
                updated_at: Set(now),
                deleted_at: Set(None),
            }
            .insert(&store.database)
            .await
            .unwrap();
            document_revision::ActiveModel {
                id: Set(db_id(revision_id)),
                document_id: Set(db_id(document_id)),
                revision_no: Set(1),
                content: Set(content.to_owned()),
                structured_payload: Set(None),
                word_count: Set(non_whitespace_char_count(content) as i64),
                created_by: Set("test".to_owned()),
                source_revision_id: Set(None),
                created_at: Set(now),
            }
            .insert(&store.database)
            .await
            .unwrap();
            document::ActiveModel {
                id: Set(db_id(document_id)),
                current_revision_id: Set(Some(revision_id.to_string())),
                revision: Set(1),
                updated_at: Set(now),
                ..Default::default()
            }
            .update(&store.database)
            .await
            .unwrap();
        }

        let item = store
            .list_projects()
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(
            item.progress,
            ProjectProgress {
                main_chapter_total: 2,
                main_chapter_approved: 1,
                extra_chapter_total: 1,
                current_word_count: 10,
            }
        );
    }

    #[tokio::test]
    async fn deleting_a_project_hides_its_versions_and_restoring_it_brings_them_back() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("保留", "留下的作品", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let doomed = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("待删", "要删的作品", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(store.list_projects().await.unwrap().len(), 2);

        // 项目下先单独删掉一个稿本，用来验证恢复项目不会把它一并复活。
        let second = store
            .create_version(
                doomed.project.id,
                Uuid::new_v4(),
                NewNovelVersion::new("第二稿", "要删的作品").unwrap(),
            )
            .await
            .unwrap();
        let removed = doomed.version.id;
        store
            .delete_version(removed, Uuid::new_v4(), doomed.version.revision)
            .await
            .unwrap();

        let command_id = Uuid::new_v4();
        let current = store.get_project(doomed.project.id).await.unwrap();
        let bin = store
            .delete_project(doomed.project.id, command_id, current.project.revision)
            .await
            .unwrap();
        // 同一条命令重放返回原结果，不会重复删除。
        assert_eq!(
            bin,
            store
                .delete_project(doomed.project.id, command_id, current.project.revision)
                .await
                .unwrap()
        );
        assert_eq!(store.list_projects().await.unwrap().len(), 1);
        assert!(matches!(
            store.get_project(doomed.project.id).await.unwrap_err(),
            StoreError::ProjectNotFound
        ));
        assert!(matches!(
            store
                .get_version_snapshot(second.version.id)
                .await
                .unwrap_err(),
            StoreError::VersionNotFound
        ));

        let project_item = bin
            .items
            .iter()
            .find(|item| item.id == doomed.project.id)
            .expect("项目应该出现在回收站里");
        assert_eq!(project_item.kind, RecycleItemKind::Project);
        assert!(project_item.restorable);
        assert!(!project_item.purgeable);
        assert_eq!(project_item.parent, None);
        assert_eq!(project_item.detail.as_deref(), Some("2 份稿本 · 0 章"));

        // 祖先不可见时，被删的稿本不能单独恢复。
        let version_item = bin
            .items
            .iter()
            .find(|item| item.id == removed)
            .expect("被删的稿本应该还在回收站里");
        assert!(!version_item.restorable);
        assert!(version_item.restore_blocked_reason.is_some());
        assert!(matches!(
            store
                .restore_recycle_item(RecycleItemKind::Version, removed, Uuid::new_v4())
                .await
                .unwrap_err(),
            StoreError::RecycleAncestorDeleted(_)
        ));

        // 项目与正文一样构成审计链，不提供物理删除。
        assert!(matches!(
            store
                .purge_recycle_item(RecycleItemKind::Project, doomed.project.id, Uuid::new_v4())
                .await
                .unwrap_err(),
            StoreError::RecycleNotPurgeable(_)
        ));

        // 恢复项目：稿本原样回来，但此前单独删掉的稿本仍留在回收站里。
        let restored = store
            .restore_recycle_item(RecycleItemKind::Project, doomed.project.id, Uuid::new_v4())
            .await
            .unwrap();
        assert_eq!(store.list_projects().await.unwrap().len(), 2);
        assert!(restored.items.iter().any(|item| item.id == removed));
        assert_eq!(
            store
                .get_version_snapshot(second.version.id)
                .await
                .unwrap()
                .version,
            second.version
        );

        // 恢复不存在的项目：明确报错，而不是假装成功。
        assert!(matches!(
            store
                .restore_recycle_item(RecycleItemKind::Project, doomed.project.id, Uuid::new_v4())
                .await
                .unwrap_err(),
            StoreError::RecycleItemNotFound
        ));
    }

    #[tokio::test]
    async fn seaorm_entities_decode_nullable_text_foreign_keys() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("实体映射", "SQLite 空外键", None::<&str>).unwrap(),
            )
            .await
            .unwrap();

        let version = novel_version::Entity::find_by_id(db_id(created.version.id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(version.source_version_id, None);

        let documents = document::Entity::find()
            .filter(
                document::Entity::COLUMN
                    .version_id
                    .eq(db_id(created.version.id)),
            )
            .all(&store.database)
            .await
            .unwrap();
        assert_eq!(documents.len(), 5);
        assert!(documents.iter().all(|document| {
            document.chapter_id.is_none() && document.current_revision_id.is_none()
        }));
        let related_version = documents[0]
            .find_related(novel_version::Entity)
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(related_version.id, db_id(created.version.id));

        let workflow = workflow_instance::Entity::find_by_id(db_id(created.version.id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(workflow.active_run_id, None);
    }

    #[tokio::test]
    async fn entity_registry_initializes_every_store_table() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();

        let metadata =
            entity::application_metadata::Entity::find_by_id("schema_version".to_owned())
                .one(&store.database)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(metadata.value, SCHEMA_VERSION.to_string());

        entity::project::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
        entity::novel_version::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
        entity::chapter::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
        entity::document::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
        entity::document_revision::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
        entity::revision_dependency::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
        entity::workflow_instance::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
        entity::run::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
        entity::agent_message::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
        entity::model_request::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
        entity::tool_call::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
        entity::system_model_setting::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
        entity::review::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
        entity::command_receipt::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
        entity::event::Entity::find()
            .one(&store.database)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn system_model_settings_are_revisioned_idempotent_and_emit_sync_events() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let command_id = Uuid::new_v4();
        let input = NewSystemModelSettings {
            api_base_url: "https://model.example/v1".to_owned(),
            credential_ref: Some("credential-ref-1".to_owned()),
            credential_fingerprint: Some("a".repeat(64)),
            model_id: "writer-1".to_owned(),
            capabilities: ModelCapabilityProfile {
                streaming: true,
                tools: true,
                stream_usage: true,
                thinking: true,
            },
            max_context_tokens: 32_768,
            reserved_output_tokens: 4_096,
        };

        let saved = store
            .save_system_model_settings(command_id, 0, input.clone())
            .await
            .unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(
            store
                .save_system_model_settings(command_id, 0, input.clone())
                .await
                .unwrap(),
            saved
        );
        assert_eq!(
            store.get_system_model_settings().await.unwrap(),
            Some(saved)
        );
        assert!(matches!(
            store
                .save_system_model_settings(Uuid::new_v4(), 0, input)
                .await,
            Err(StoreError::SystemSettingsRevisionConflict {
                expected: 0,
                actual: 1,
            })
        ));
        assert!(
            store
                .events_after(0)
                .await
                .unwrap()
                .iter()
                .any(|event| event.event_type == "settings.changed")
        );
    }

    #[tokio::test]
    async fn model_request_checkpoints_are_immutable_and_charge_only_on_dispatch() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("请求检查点", "模型调用不能重复计费", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个有明确结局的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let workflow = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(created.version.id, Uuid::new_v4(), workflow.state_revision)
            .await
            .unwrap();
        let run_id = started.workflow.active_run_id.unwrap();
        let input = NewModelRequest {
            run_id,
            settings_snapshot: json!({ "model": "test-model" }),
            prompt_snapshot: json!({ "contentHash": "prompt-hash" }),
            context_snapshot: json!({ "messages": [] }),
        };

        let prepared = store.prepare_model_request(input.clone()).await.unwrap();
        assert_eq!(prepared.attempt_no, 1);
        assert_eq!(prepared.status, ModelRequestStatus::Prepared);
        assert!(matches!(
            store.prepare_model_request(input.clone()).await,
            Err(StoreError::ResourceBusy(_))
        ));

        let dispatched = store.dispatch_model_request(prepared.id).await.unwrap();
        assert_eq!(dispatched.status, ModelRequestStatus::Dispatched);
        assert_eq!(
            store.dispatch_model_request(prepared.id).await.unwrap(),
            dispatched
        );
        let in_flight = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        assert_eq!(
            in_flight.execution,
            nekowriter_core::ExecutionStatus::RequestInFlight
        );
        let run = entity::run::Entity::find_by_id(db_id(run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.request_count, 1);
        assert_eq!(run.status, "running");

        let resolution = ModelRequestResolution::Succeeded {
            provider_request_id: Some("provider-request-1".to_owned()),
            usage: json!({ "inputTokens": "12", "outputTokens": "4", "totalTokens": "16" }),
        };
        let succeeded = store
            .resolve_model_request_with_assistant_message(
                prepared.id,
                resolution.clone(),
                NewAgentMessage {
                    role: AgentMessageRole::Assistant,
                    content: json!({
                        "kind": "text",
                        "role": "assistant",
                        "content": "{\"artifacts\":[...]}"
                    }),
                    request_id: Some(prepared.id),
                    tool_call_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(succeeded.status, ModelRequestStatus::Succeeded);
        assert_eq!(
            store
                .resolve_model_request(prepared.id, resolution)
                .await
                .unwrap(),
            succeeded
        );
        assert_eq!(store.list_agent_messages(run_id).await.unwrap().len(), 1);
        let at_boundary = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        assert_eq!(
            at_boundary.execution,
            nekowriter_core::ExecutionStatus::ToolBoundary
        );

        let second = store.prepare_model_request(input).await.unwrap();
        assert_eq!(second.attempt_no, 2);
        let cancelled = store
            .cancel_prepared_model_request(second.id)
            .await
            .unwrap();
        assert_eq!(cancelled.status, ModelRequestStatus::Cancelled);
        let run = entity::run::Entity::find_by_id(db_id(run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.request_count, 1);
    }

    #[tokio::test]
    async fn captured_model_request_commits_its_context_and_dispatch_checkpoint_together() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("原子请求", "请求上下文必须与发送检查点一致", None::<&str>)
                    .unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个在雨夜相遇又告别的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let workflow = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(created.version.id, Uuid::new_v4(), workflow.state_revision)
            .await
            .unwrap();
        let run_id = started.workflow.active_run_id.unwrap();

        let request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id,
                    settings_snapshot: json!({ "model": "test-model" }),
                    prompt_snapshot: json!({ "contentHash": "prompt-hash" }),
                    context_snapshot: json!({ "messages": ["system", "user"] }),
                },
                vec![
                    NewAgentMessage {
                        role: AgentMessageRole::System,
                        content: json!({
                            "kind": "text",
                            "role": "system",
                            "content": "你是严谨的小说写作助手。"
                        }),
                        request_id: None,
                        tool_call_id: None,
                    },
                    NewAgentMessage {
                        role: AgentMessageRole::User,
                        content: json!({
                            "kind": "text",
                            "role": "user",
                            "content": "开始生成准备产物。"
                        }),
                        request_id: None,
                        tool_call_id: None,
                    },
                ],
            )
            .await
            .unwrap();

        assert_eq!(request.status, ModelRequestStatus::Dispatched);
        assert_eq!(request.attempt_no, 1);
        assert_eq!(
            store
                .list_agent_messages(run_id)
                .await
                .unwrap()
                .iter()
                .map(|message| message.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        let snapshot = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap();
        assert_eq!(
            snapshot.workflow.execution,
            nekowriter_core::ExecutionStatus::RequestInFlight
        );
        let run = entity::run::Entity::find_by_id(db_id(run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.status, "running");
        assert_eq!(run.request_count, 1);

        let events = store.events_after(0).await.unwrap();
        assert!(events.iter().any(|event| event.event_type == "run.changed"));
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "workflow.changed")
        );
    }

    #[tokio::test]
    async fn queued_model_setup_failure_becomes_a_persisted_failed_run_without_a_fake_request() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("预发失败", "请求封存前失败也必须结束排队", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个不会永久转圈的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let before_start = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(
                created.version.id,
                Uuid::new_v4(),
                before_start.state_revision,
            )
            .await
            .unwrap();
        let run_id = started.workflow.active_run_id.unwrap();

        assert!(
            store
                .fail_queued_model_run(
                    run_id,
                    json!({
                        "kind": "pre_dispatch",
                        "message": "上下文超过模型窗口",
                    }),
                )
                .await
                .unwrap()
        );
        assert!(
            !store
                .fail_queued_model_run(
                    run_id,
                    json!({
                        "kind": "pre_dispatch",
                        "message": "重复回调不应改写失败记录",
                    }),
                )
                .await
                .unwrap()
        );
        let snapshot = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap();
        assert_eq!(
            snapshot.workflow.execution,
            nekowriter_core::ExecutionStatus::Failed
        );
        let run = entity::run::Entity::find_by_id(db_id(run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.status, "failed");
        assert!(run.checkpoint_json.contains("scheduler_error"));
        assert_eq!(
            store.list_model_requests(run_id).await.unwrap(),
            [] as [model_requests::PersistedModelRequest; 0]
        );
    }

    #[tokio::test]
    async fn free_agent_tool_call_requeues_and_replays_its_stored_result() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("知识检索工具", "自由工具结果落库", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一部关于灯塔与雾港的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let workflow = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(created.version.id, Uuid::new_v4(), workflow.state_revision)
            .await
            .unwrap();
        let run_id = started.workflow.active_run_id.unwrap();
        let request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id,
                    settings_snapshot: json!({ "model": "test-model" }),
                    prompt_snapshot: json!({ "contentHash": "prompt-hash" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                vec![NewAgentMessage {
                    role: AgentMessageRole::User,
                    content: json!({
                        "kind": "text",
                        "role": "user",
                        "content": "请生成四项准备产物。"
                    }),
                    request_id: None,
                    tool_call_id: None,
                }],
            )
            .await
            .unwrap();
        let tool_result = json!({
            "hits": [{
                "chunkId": "chunk-1",
                "entryId": "entry-1",
                "entryRevision": "1",
                "spaceId": "space-1",
                "chunkIndex": "0",
                "title": "雾港城",
                "text": "灯塔用蓝鲸油点亮。",
                "score": 0.5,
            }],
            "coverage": {
                "readyEntries": "1",
                "pendingEntries": "0",
                "failedEntries": "0",
                "notConfiguredEntries": "0",
            },
        });
        let call = FreeAgentToolCall {
            provider_call_id: "call-1".to_owned(),
            name: nekowriter_core::TOOL_QUERY_KNOWLEDGE.to_owned(),
            arguments: json!({ "query": "灯塔", "topK": 5 }),
            result: tool_result.clone(),
            failed: false,
            error_code: None,
        };

        let committed = store
            .execute_free_agent_tool_call(
                request.id,
                Some("provider-request-1".to_owned()),
                json!({ "totalTokens": "5" }),
                call.clone(),
            )
            .await
            .unwrap();
        assert_eq!(committed.result, tool_result);
        assert!(committed.newly_resolved);
        assert!(committed.requeue);
        assert_eq!(
            store
                .free_agent_tool_result(request.id, "call-1")
                .await
                .unwrap(),
            Some(tool_result.clone())
        );
        // 一轮工具结果落库后：assistant 工具调用、tool 结果、Run 重新排队、工作流回到生成中。
        assert_eq!(store.list_agent_messages(run_id).await.unwrap().len(), 3);
        let queued = store
            .get_preparation_snapshot(created.version.id)
            .await
            .unwrap();
        assert_eq!(queued.workflow.execution, ExecutionStatus::Queued);

        // 同一 provider_call_id 的重放只返回已提交结果，不再追加消息或工具调用记录。
        let replayed = store
            .execute_free_agent_tool_call(
                request.id,
                Some("provider-request-1".to_owned()),
                json!({ "totalTokens": "5" }),
                call.clone(),
            )
            .await
            .unwrap();
        assert_eq!(replayed.result, tool_result);
        assert!(!replayed.newly_resolved);
        assert!(!replayed.requeue);
        assert_eq!(store.list_agent_messages(run_id).await.unwrap().len(), 3);

        // 同一调用 ID 换了参数属于协议错误，不能静默改写已落库的调用。
        let conflict = store
            .execute_free_agent_tool_call(
                request.id,
                Some("provider-request-1".to_owned()),
                json!({ "totalTokens": "5" }),
                FreeAgentToolCall {
                    arguments: json!({ "query": "另一个问题", "topK": 5 }),
                    ..call
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(conflict, StoreError::InvalidModelRequestState(_)));
    }

    #[tokio::test]
    async fn invalid_agent_question_tool_calls_stop_at_the_protocol_limit() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("问题工具上限", "无效问题不能无限消耗请求", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个有明确结局的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let workflow = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(created.version.id, Uuid::new_v4(), workflow.state_revision)
            .await
            .unwrap();
        let run_id = started.workflow.active_run_id.unwrap();

        for attempt in 1..=MAX_CONSECUTIVE_PROTOCOL_ERRORS {
            let request = store
                .capture_and_dispatch_model_request(
                    NewModelRequest {
                        run_id,
                        settings_snapshot: json!({ "model": "invalid-question-test" }),
                        prompt_snapshot: json!({ "contentHash": format!("invalid-question-{attempt}") }),
                        context_snapshot: json!({ "messages": [] }),
                    },
                    Vec::new(),
                )
                .await
                .unwrap();
            let result = store
                .execute_agent_question_tool_call(
                    request.id,
                    Some(format!("invalid-question-provider-{attempt}")),
                    json!({ "totalTokens": "1" }),
                    AgentQuestionToolCall {
                        provider_call_id: format!("invalid-question-call-{attempt}"),
                        // `questions` 为空是完整 JSON，但不符合 ask_user 的领域合同。
                        arguments: json!({ "questions": [] }),
                    },
                )
                .await
                .unwrap();
            let AppliedAgentQuestionToolCall::Resolved {
                result,
                newly_resolved,
                requeue,
                ..
            } = result
            else {
                panic!("无效 ask_user 应生成可回送的工具错误");
            };
            assert!(newly_resolved);
            assert_eq!(result["status"], "error");
            assert_eq!(result["code"], "TOOL_ARGUMENTS_INVALID");
            assert_eq!(requeue, attempt < MAX_CONSECUTIVE_PROTOCOL_ERRORS);

            let snapshot = store
                .get_preparation_snapshot(created.version.id)
                .await
                .unwrap();
            assert_eq!(
                snapshot.workflow.execution,
                if attempt < MAX_CONSECUTIVE_PROTOCOL_ERRORS {
                    ExecutionStatus::Queued
                } else {
                    ExecutionStatus::Stopped
                }
            );
        }

        let run = entity::run::Entity::find_by_id(db_id(run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.status, "stopped");
        assert_eq!(
            run.consecutive_protocol_errors,
            MAX_CONSECUTIVE_PROTOCOL_ERRORS
        );
        let last_run_event = store
            .events_after(0)
            .await
            .unwrap()
            .into_iter()
            .rfind(|event| event.event_type == "run.changed")
            .unwrap();
        assert_eq!(last_run_event.payload["reason"], "tool_error");
        assert_eq!(
            last_run_event.payload["wait_reason"],
            "protocol_error_limit"
        );
        assert_eq!(
            last_run_event.payload["consecutive_protocol_errors"],
            MAX_CONSECUTIVE_PROTOCOL_ERRORS.to_string()
        );
    }

    #[tokio::test]
    async fn preparation_model_submission_is_committed_with_its_response_and_reviews() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("自动准备", "模型完成后必须原子进入审核", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一部关于旧车站与重逢的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let workflow = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(created.version.id, Uuid::new_v4(), workflow.state_revision)
            .await
            .unwrap();
        let run_id = started.workflow.active_run_id.unwrap();
        let request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id,
                    settings_snapshot: json!({ "model": "test-model" }),
                    prompt_snapshot: json!({ "contentHash": "prompt-hash" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                vec![NewAgentMessage {
                    role: AgentMessageRole::User,
                    content: json!({
                        "kind": "text",
                        "role": "user",
                        "content": "请生成四项准备产物。"
                    }),
                    request_id: None,
                    tool_call_id: None,
                }],
            )
            .await
            .unwrap();
        let artifacts = PreparationArtifactKind::ALL
            .into_iter()
            .map(|kind| {
                PreparationArtifactInput::new(kind, format!("{} 的可审核内容", kind.label()))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let chapter_plan = vec![ChapterPlanDraft::new("第一章：重逢", "在旧车站相遇").unwrap()];

        let submitted = store
            .commit_preparation_model_submission(
                request.id,
                Some("provider-request-1".to_owned()),
                json!({ "inputTokens": "21", "outputTokens": "8", "totalTokens": "29" }),
                crate::agent_messages::phase_submission_test_message(
                    request.id,
                    "submit_preparation",
                    r#"{"artifacts":[],"chapter_plan":[]}"#,
                ),
                artifacts,
                Some(chapter_plan),
            )
            .await
            .unwrap();
        assert_eq!(
            submitted.workflow.stage,
            nekowriter_core::WorkflowStage::PreparingReviewing
        );
        assert_eq!(
            submitted.workflow.execution,
            nekowriter_core::ExecutionStatus::WaitingReview
        );
        assert!(
            submitted
                .artifacts
                .iter()
                .all(|artifact| artifact.revision_id.is_some() && artifact.review.is_some())
        );
        // 阶段提交在账本里是配对的两条：提交工具的调用与它被接受的结果。
        assert_eq!(store.list_agent_messages(run_id).await.unwrap().len(), 3);
        assert_eq!(
            store.list_model_requests(run_id).await.unwrap()[0].status,
            ModelRequestStatus::Succeeded
        );
        let run = entity::run::Entity::find_by_id(db_id(run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.status, "waiting");
        assert_eq!(run.request_count, 1);

        let replayed = store
            .commit_preparation_model_submission(
                request.id,
                Some("provider-request-1".to_owned()),
                json!({ "inputTokens": "21", "outputTokens": "8", "totalTokens": "29" }),
                crate::agent_messages::phase_submission_test_message(
                    request.id,
                    "submit_preparation",
                    r#"{"artifacts":[]}"#,
                ),
                Vec::new(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(replayed.workflow, submitted.workflow);
        assert_eq!(store.list_agent_messages(run_id).await.unwrap().len(), 3);
    }

    /// 准备修订轮：模型一轮只提交「需要修订」的那些产物，而且只有粗纲也在待修订
    /// 之列时才允许附带 `chapter_plan`。系统消息里的合同与解析器按同一份清单生成，
    /// 所以「照合同提交」必须被 Store 接受，提交其它项必须被明确拒绝且不留痕迹。
    #[tokio::test]
    async fn preparation_revision_round_accepts_only_the_rejected_artifacts() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("准备修订", "只有被拒绝的产物需要重写", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let version_id = created.version.id;
        store
            .save_novel_brief(
                version_id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一部关于旧车站与重逢的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let workflow = store
            .get_version_snapshot(version_id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(version_id, Uuid::new_v4(), workflow.state_revision)
            .await
            .unwrap();
        let run_id = started.workflow.active_run_id.unwrap();
        let first_request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id,
                    settings_snapshot: json!({ "model": "test-model" }),
                    prompt_snapshot: json!({ "contentHash": "prompt-hash" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let artifacts = PreparationArtifactKind::ALL
            .into_iter()
            .map(|kind| {
                PreparationArtifactInput::new(kind, format!("{} 的首版内容", kind.label())).unwrap()
            })
            .collect::<Vec<_>>();
        let mut snapshot = store
            .commit_preparation_model_submission(
                first_request.id,
                Some("provider-request-1".to_owned()),
                json!({ "inputTokens": "21", "outputTokens": "8", "totalTokens": "29" }),
                crate::agent_messages::phase_submission_test_message(
                    first_request.id,
                    "submit_preparation",
                    r#"{"artifacts":[],"chapter_plan":[]}"#,
                ),
                artifacts,
                Some(vec![
                    ChapterPlanDraft::new("第一章：重逢", "旧车站的相遇").unwrap(),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(snapshot.workflow.stage, WorkflowStage::PreparingReviewing);

        // 三项通过、资料被拒绝：工作流进入准备修订，同一个 Run 重新排队。
        for kind in [
            PreparationArtifactKind::Background,
            PreparationArtifactKind::Characters,
            PreparationArtifactKind::Outline,
        ] {
            let review_id = snapshot
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == kind)
                .and_then(|artifact| artifact.review.as_ref())
                .map(|review| review.id)
                .unwrap();
            snapshot = store
                .decide_preparation_review(
                    review_id,
                    Uuid::new_v4(),
                    snapshot.workflow.state_revision,
                    HumanReviewDecision::new(true, "").unwrap(),
                    Vec::new(),
                )
                .await
                .unwrap();
        }
        let research_review_id = snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Research)
            .and_then(|artifact| artifact.review.as_ref())
            .map(|review| review.id)
            .unwrap();
        snapshot = store
            .decide_preparation_review(
                research_review_id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                HumanReviewDecision::new(false, "资料需要补充 1998 年港城晚报的原件出处。")
                    .unwrap(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(snapshot.workflow.stage, WorkflowStage::PreparingRevising);
        assert_eq!(snapshot.workflow.active_run_id, Some(run_id));

        let revision_request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id,
                    settings_snapshot: json!({ "model": "test-model" }),
                    prompt_snapshot: json!({ "contentHash": "prompt-hash" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();

        // 负向：修订轮不能覆盖不需要修订的产物，失败不留痕。
        assert!(matches!(
            store
                .commit_preparation_model_submission(
                    revision_request.id,
                    Some("provider-request-2".to_owned()),
                    json!({ "inputTokens": "21", "outputTokens": "8", "totalTokens": "29" }),
                    crate::agent_messages::phase_submission_test_message(
                        revision_request.id,
                        "submit_preparation",
                        "{\"artifacts\":[{\"kind\":\"background\",\"content\":\"越权重写\"}]}",
                    ),
                    vec![
                        PreparationArtifactInput::new(
                            PreparationArtifactKind::Background,
                            "越权重写的背景",
                        )
                        .unwrap()
                    ],
                    None,
                )
                .await,
            Err(StoreError::InvalidPreparationArtifacts(_))
        ));

        // 正向：只提交被拒绝的资料项，不带 chapter_plan，照合同提交即可通过。
        snapshot = store
            .commit_preparation_model_submission(
                revision_request.id,
                Some("provider-request-2".to_owned()),
                json!({ "inputTokens": "21", "outputTokens": "8", "totalTokens": "29" }),
                crate::agent_messages::phase_submission_test_message(
                    revision_request.id,
                    "submit_preparation",
                    "{\"artifacts\":[{\"kind\":\"research\",\"content\":\"补充后的资料\"}]}",
                ),
                vec![
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Research,
                        "资料：1998 年港城晚报原件第 3 版。",
                    )
                    .unwrap(),
                ],
                None,
            )
            .await
            .unwrap();
        assert_eq!(snapshot.workflow.stage, WorkflowStage::PreparingReviewing);
        let research = snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Research)
            .unwrap();
        assert_eq!(
            research.content.as_deref(),
            Some("资料：1998 年港城晚报原件第 3 版。")
        );
        assert_eq!(
            research.review.as_ref().map(|review| review.status.clone()),
            Some(ReviewStatus::Open)
        );
    }

    #[tokio::test]
    async fn agent_messages_are_immutable_ordered_run_scoped_records() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("消息账本", "Agent 历史不能只保留在内存中", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一段需要保留上下文的故事").unwrap(),
            )
            .await
            .unwrap();
        let workflow = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(created.version.id, Uuid::new_v4(), workflow.state_revision)
            .await
            .unwrap();
        let run_id = started.workflow.active_run_id.unwrap();
        let request_id = Uuid::new_v4();

        let appended = store
            .append_agent_messages(
                run_id,
                vec![
                    NewAgentMessage {
                        role: AgentMessageRole::System,
                        content: json!({
                            "kind": "text",
                            "role": "system",
                            "content": "你是小说写作助手。"
                        }),
                        request_id: None,
                        tool_call_id: None,
                    },
                    NewAgentMessage {
                        role: AgentMessageRole::User,
                        content: json!({
                            "kind": "text",
                            "role": "user",
                            "content": "请开始准备。"
                        }),
                        request_id: Some(request_id),
                        tool_call_id: None,
                    },
                ],
            )
            .await
            .unwrap();
        assert_eq!(
            appended
                .iter()
                .map(|message| message.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(appended[1].request_id, Some(request_id));
        assert_eq!(store.list_agent_messages(run_id).await.unwrap(), appended);

        assert!(matches!(
            store
                .append_agent_messages(
                    run_id,
                    vec![NewAgentMessage {
                        role: AgentMessageRole::Tool,
                        content: json!({ "kind": "tool_result", "content": "结果" }),
                        request_id: Some(request_id),
                        tool_call_id: None,
                    }],
                )
                .await,
            Err(StoreError::InvalidAgentMessage(_))
        ));
        assert_eq!(store.list_agent_messages(run_id).await.unwrap(), appended);
    }

    #[tokio::test]
    async fn model_request_dispatch_never_crosses_the_persisted_allowance_boundary() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("请求额度", "达到额度后不能再发出请求", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个结局明确的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let workflow = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(created.version.id, Uuid::new_v4(), workflow.state_revision)
            .await
            .unwrap();
        let run_id = started.workflow.active_run_id.unwrap();
        let run_model = entity::run::Entity::find_by_id(db_id(run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        let allowance = run_model.request_allowance_end;
        let mut active_run = run_model.into_active_model();
        active_run.request_count = Set(allowance);
        active_run.update(&store.database).await.unwrap();

        let prepared = store
            .prepare_model_request(NewModelRequest {
                run_id,
                settings_snapshot: json!({ "model": "test-model" }),
                prompt_snapshot: json!({ "contentHash": "prompt-hash" }),
                context_snapshot: json!({ "messages": [] }),
            })
            .await
            .unwrap();
        assert!(matches!(
            store.dispatch_model_request(prepared.id).await,
            Err(StoreError::RequestAllowanceExhausted {
                request_count,
                request_allowance_end,
            }) if request_count == allowance && request_allowance_end == allowance
        ));
        let requests = store.list_model_requests(run_id).await.unwrap();
        assert_eq!(requests, vec![prepared]);
    }

    #[tokio::test]
    async fn startup_recovery_marks_dispatched_runs_interrupted_and_explicit_continue_regrants_allowance()
     {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("中断恢复", "重开后不自动重新计费", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个雨夜车站重逢的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let before_start = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(
                created.version.id,
                Uuid::new_v4(),
                before_start.state_revision,
            )
            .await
            .unwrap();
        let run_id = started.workflow.active_run_id.unwrap();
        let dispatched = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id,
                    settings_snapshot: json!({ "model": "recovery-test" }),
                    prompt_snapshot: json!({ "contentHash": "recovery-test" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(dispatched.status, ModelRequestStatus::Dispatched);
        drop(store);

        let reopened = Store::open(data_dir.path()).await.unwrap();
        let interrupted = reopened
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        assert_eq!(interrupted.execution, ExecutionStatus::Interrupted);
        let requests = reopened.list_model_requests(run_id).await.unwrap();
        assert_eq!(requests[0].status, ModelRequestStatus::Unknown);
        let interrupted_run = entity::run::Entity::find_by_id(db_id(run_id))
            .one(&reopened.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(interrupted_run.status, "interrupted");
        assert_eq!(interrupted_run.request_count, 1);

        let continue_command_id = Uuid::new_v4();
        let resumed = reopened
            .continue_writing_run(run_id, continue_command_id, interrupted.state_revision)
            .await
            .unwrap();
        assert_eq!(resumed.execution, ExecutionStatus::Queued);
        assert_eq!(
            reopened
                .continue_writing_run(run_id, continue_command_id, interrupted.state_revision,)
                .await
                .unwrap(),
            resumed
        );
        let resumed_run = entity::run::Entity::find_by_id(db_id(run_id))
            .one(&reopened.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resumed_run.status, "queued");
        assert_eq!(resumed_run.resume_count, 1);
        assert_eq!(resumed_run.request_allowance_end, 101);
        assert_eq!(resumed_run.consecutive_protocol_errors, 0);

        let next = reopened
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id,
                    settings_snapshot: json!({ "model": "recovery-test" }),
                    prompt_snapshot: json!({ "contentHash": "recovery-test-next" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(next.attempt_no, 2);
        assert_eq!(next.status, ModelRequestStatus::Dispatched);
    }

    #[tokio::test]
    async fn request_limit_stop_requires_explicit_continue_before_another_request() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("额度停止", "达到上限后等待显式继续", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个有明确收束的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let initial = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(created.version.id, Uuid::new_v4(), initial.state_revision)
            .await
            .unwrap();
        let run_id = started.workflow.active_run_id.unwrap();
        let run_model = entity::run::Entity::find_by_id(db_id(run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        let allowance = run_model.request_allowance_end;
        let mut exhausted_run = run_model.into_active_model();
        exhausted_run.request_count = Set(allowance);
        exhausted_run.update(&store.database).await.unwrap();

        assert!(
            store
                .stop_queued_model_run(
                    run_id,
                    json!({ "kind": "request_limit", "message": "额度已用完" }),
                )
                .await
                .unwrap()
        );
        let stopped = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        assert_eq!(stopped.execution, ExecutionStatus::Stopped);
        let resumed = store
            .continue_writing_run(run_id, Uuid::new_v4(), stopped.state_revision)
            .await
            .unwrap();
        assert_eq!(resumed.execution, ExecutionStatus::Queued);
        let resumed_run = entity::run::Entity::find_by_id(db_id(run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resumed_run.request_count, allowance);
        assert_eq!(resumed_run.request_allowance_end, allowance + 100);
        assert_eq!(resumed_run.resume_count, 1);
    }

    #[tokio::test]
    async fn explicit_stop_cancels_the_dispatched_attempt_and_rejects_its_late_callback() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("用户停止", "停止不能让迟到回调继续写作", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个雨夜重逢的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let initial = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(created.version.id, Uuid::new_v4(), initial.state_revision)
            .await
            .unwrap();
        let run_id = started.workflow.active_run_id.unwrap();
        let dispatched = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id,
                    settings_snapshot: json!({ "model": "stop-test" }),
                    prompt_snapshot: json!({ "contentHash": "stop-test" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let in_flight = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        assert_eq!(in_flight.execution, ExecutionStatus::RequestInFlight);

        let stop_command_id = Uuid::new_v4();
        let stopped = store
            .stop_writing_run(run_id, stop_command_id, in_flight.state_revision)
            .await
            .unwrap();
        assert_eq!(stopped.workflow.execution, ExecutionStatus::Stopped);
        assert_eq!(
            stopped.workflow.wait_reason,
            Some(RunWaitReason::UserStopped)
        );
        assert_eq!(
            stopped.workflow.resume_execution,
            Some(ExecutionStatus::RequestInFlight)
        );
        assert_eq!(stopped.cancelled_request_ids, vec![dispatched.id]);
        assert_eq!(
            store
                .stop_writing_run(run_id, stop_command_id, in_flight.state_revision)
                .await
                .unwrap(),
            stopped
        );

        let requests = store.list_model_requests(run_id).await.unwrap();
        assert_eq!(requests[0].status, ModelRequestStatus::Cancelled);
        assert_eq!(requests[0].error.as_ref().unwrap()["kind"], "user_stopped");
        assert!(matches!(
            store
                .resolve_model_request(
                    dispatched.id,
                    ModelRequestResolution::Failed {
                        provider_request_id: None,
                        error: json!({ "kind": "late_callback" }),
                    },
                )
                .await,
            Err(StoreError::InvalidModelRequestState(_))
        ));
        let after_late_callback = store
            .get_preparation_snapshot(created.version.id)
            .await
            .unwrap();
        assert_eq!(after_late_callback.workflow, stopped.workflow);
        assert!(
            after_late_callback
                .artifacts
                .iter()
                .all(|artifact| artifact.revision_id.is_none())
        );

        let resumed = store
            .continue_writing_run(run_id, Uuid::new_v4(), stopped.workflow.state_revision)
            .await
            .unwrap();
        assert_eq!(resumed.execution, ExecutionStatus::Queued);
        let next = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id,
                    settings_snapshot: json!({ "model": "stop-test" }),
                    prompt_snapshot: json!({ "contentHash": "after-stop" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(next.attempt_no, 2);
        assert_eq!(next.status, ModelRequestStatus::Dispatched);
    }

    #[tokio::test]
    async fn protocol_errors_requeue_with_paired_results_then_stop_at_the_limit() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("协议纠正", "连续格式错误必须有限重试", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个结局清晰的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let initial = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(created.version.id, Uuid::new_v4(), initial.state_revision)
            .await
            .unwrap();
        let run_id = started.workflow.active_run_id.unwrap();

        for attempt in 1..=MAX_CONSECUTIVE_PROTOCOL_ERRORS {
            let dispatched = store
                .capture_and_dispatch_model_request(
                    NewModelRequest {
                        run_id,
                        settings_snapshot: json!({ "model": "protocol-test" }),
                        prompt_snapshot: json!({ "contentHash": format!("protocol-{attempt}") }),
                        context_snapshot: json!({ "messages": [] }),
                    },
                    Vec::new(),
                )
                .await
                .unwrap();
            let assistant_message = if attempt == 1 {
                NewAgentMessage {
                    role: AgentMessageRole::Assistant,
                    content: json!({
                        "kind": "assistant_tool_calls",
                        "content": null,
                        "toolCalls": [
                            { "id": "bad-call-1", "name": "append_content", "arguments": {} },
                            { "id": "bad-call-2", "name": "replace_content", "arguments": {} },
                        ],
                    }),
                    request_id: Some(dispatched.id),
                    tool_call_id: None,
                }
            } else {
                NewAgentMessage {
                    role: AgentMessageRole::Assistant,
                    content: json!({
                        "kind": "text",
                        "role": "assistant",
                        "content": "这不是合法的阶段 JSON",
                    }),
                    request_id: Some(dispatched.id),
                    tool_call_id: None,
                }
            };
            let result = store
                .record_model_protocol_correction(
                    ProtocolCorrectionTarget::WritingRun,
                    ModelProtocolCorrection {
                        provider_request_id: Some(format!("provider-{attempt}")),
                        usage: json!({ "totalTokens": "1" }),
                        assistant_message,
                        error: json!({
                            "kind": "protocol_error",
                            "message": "终态提交不是合法 JSON",
                        }),
                        correction: "请重新返回一个合法的阶段结果。".to_owned(),
                        tool_result_code: "PROTOCOL_ERROR".to_owned(),
                    },
                )
                .await
                .unwrap();
            assert!(result.newly_applied);
            assert_eq!(result.consecutive_protocol_errors, attempt);
            assert_eq!(result.requeue, attempt < MAX_CONSECUTIVE_PROTOCOL_ERRORS);
            assert_eq!(
                result.workflow.execution,
                if attempt < MAX_CONSECUTIVE_PROTOCOL_ERRORS {
                    ExecutionStatus::Queued
                } else {
                    ExecutionStatus::Stopped
                }
            );
            // 落库的计数必须等于同一轮事件公布的计数，且每一轮都是这样：被拒回合的
            // 计数与收束状态在同一次写入里落库，任何分支都不得只写一半。
            let persisted = persisted_protocol_error_count(&store, run_id).await;
            assert_eq!(persisted, attempt);
            assert_eq!(
                published_protocol_error_count(&store, dispatched.id).await,
                persisted
            );
            let duplicate = store
                .record_model_protocol_correction(
                    ProtocolCorrectionTarget::WritingRun,
                    ModelProtocolCorrection {
                        provider_request_id: Some(format!("provider-{attempt}")),
                        usage: json!({ "totalTokens": "1" }),
                        assistant_message: if attempt == 1 {
                            NewAgentMessage {
                                role: AgentMessageRole::Assistant,
                                content: json!({
                                    "kind": "assistant_tool_calls",
                                    "content": null,
                                    "toolCalls": [
                                        { "id": "bad-call-1", "name": "append_content", "arguments": {} },
                                        { "id": "bad-call-2", "name": "replace_content", "arguments": {} },
                                    ],
                                }),
                                request_id: Some(dispatched.id),
                                tool_call_id: None,
                            }
                        } else {
                            NewAgentMessage {
                                role: AgentMessageRole::Assistant,
                                content: json!({
                                    "kind": "text",
                                    "role": "assistant",
                                    "content": "这不是合法的阶段 JSON",
                                }),
                                request_id: Some(dispatched.id),
                                tool_call_id: None,
                            }
                        },
                        error: json!({
                            "kind": "protocol_error",
                            "message": "终态提交不是合法 JSON",
                        }),
                        correction: "请重新返回一个合法的阶段结果。".to_owned(),
                        tool_result_code: "PROTOCOL_ERROR".to_owned(),
                    },
                )
                .await
                .unwrap();
            assert!(!duplicate.newly_applied);
            assert!(!duplicate.requeue);
        }

        let messages = store.list_agent_messages(run_id).await.unwrap();
        assert_eq!(messages[0].role, AgentMessageRole::Assistant);
        assert_eq!(messages[1].role, AgentMessageRole::Tool);
        assert_eq!(messages[1].tool_call_id.as_deref(), Some("bad-call-1"));
        assert_eq!(messages[2].role, AgentMessageRole::Tool);
        assert_eq!(messages[2].tool_call_id.as_deref(), Some("bad-call-2"));
        assert_eq!(messages[3].role, AgentMessageRole::User);
        assert_eq!(messages[3].content["kind"], "protocol_correction");

        let stopped_run = entity::run::Entity::find_by_id(db_id(run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stopped_run.status, "stopped");
        assert_eq!(
            stopped_run.consecutive_protocol_errors,
            MAX_CONSECUTIVE_PROTOCOL_ERRORS
        );
        let stopped = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let resumed = store
            .continue_writing_run(run_id, Uuid::new_v4(), stopped.state_revision)
            .await
            .unwrap();
        assert_eq!(resumed.execution, ExecutionStatus::Queued);
        let resumed_run = entity::run::Entity::find_by_id(db_id(run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resumed_run.consecutive_protocol_errors, 0);
    }

    /// 子 Run 行里落库的连续协议错误计数。
    async fn persisted_protocol_error_count(store: &Store, run_id: RunId) -> i64 {
        entity::run::Entity::find_by_id(db_id(run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap()
            .consecutive_protocol_errors
    }

    /// 这一轮被拒时事件公布出来的连续协议错误计数。
    ///
    /// 这个数字只通过事件到达客户端（快照与 Dashboard 的 DTO 都不含它），所以它和
    /// 落库值必须来自同一次写入、永远相等：否则行里的数字会与它刚发出的事件矛盾。
    /// 两条 lane 的纠正事件都带 `request_id`，用它把范围锁在这一轮。
    async fn published_protocol_error_count(store: &Store, request_id: Uuid) -> i64 {
        store
            .events_after(0)
            .await
            .unwrap()
            .into_iter()
            .rfind(|event| {
                event.payload["protocol_correction"] == true
                    && event.payload["request_id"].as_str() == Some(&request_id.to_string())
            })
            .expect("被拒的一轮必须留下纠正事件")
            .payload["consecutive_protocol_errors"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap()
    }

    /// 委托审核的每一轮（含达上限的那一轮）落库的计数都必须等于这一轮事件公布的
    /// 计数。
    ///
    /// 修复前达上限的那一轮只写失败状态、不写计数，子 Run 行里停在 2，而它刚写的
    /// 两条事件都说 3——落库状态与自己发出的事件互相矛盾。
    #[tokio::test]
    async fn rejected_review_round_persists_the_counter_it_publishes() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("审核协议纠正", "被拒的审核提交计数必须落库", None::<&str>)
                    .unwrap(),
            )
            .await
            .unwrap();
        let version_id = created.version.id;
        store
            .save_novel_brief(
                version_id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一部围绕旧码头和匿名报纸的悬疑小说。").unwrap(),
            )
            .await
            .unwrap();
        let started = store
            .start_preparation(version_id, Uuid::new_v4(), 0)
            .await
            .unwrap();
        let mut snapshot = store
            .submit_preparation_artifacts(
                version_id,
                Uuid::new_v4(),
                started.workflow.state_revision,
                vec![
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Background,
                        "港城的旧码头将在雨季后拆除。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Research,
                        "旧报纸来自 1998 年的港城晚报。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Characters,
                        "林舟是追查父亲旧案的记者。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Outline,
                        "林舟在码头收到匿名报纸，循着署名进入仓库并决定公开线索。",
                    )
                    .unwrap(),
                ],
                Some(vec![
                    ChapterPlanDraft::new("雨夜来客", "林舟在码头收到匿名报纸。").unwrap(),
                ]),
            )
            .await
            .unwrap();
        let review_id = snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
            .and_then(|artifact| artifact.review.as_ref())
            .map(|review| review.id)
            .unwrap();
        let delegation = store
            .delegate_review(
                review_id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                "检查设定是否足以支撑第一章。".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();

        for attempt in 1..=MAX_CONSECUTIVE_PROTOCOL_ERRORS {
            let request = store
                .capture_and_dispatch_model_request(
                    NewModelRequest {
                        run_id: delegation.run_id,
                        settings_snapshot: json!({ "model": "review-protocol-test" }),
                        prompt_snapshot: json!({ "key": "review.preparation.background" }),
                        context_snapshot: json!({ "messages": [] }),
                    },
                    Vec::new(),
                )
                .await
                .unwrap();
            let outcome = store
                .record_model_protocol_correction(
                    ProtocolCorrectionTarget::DelegatedReview { review_id },
                    ModelProtocolCorrection {
                        provider_request_id: Some(format!("review-provider-{attempt}")),
                        usage: json!({ "totalTokens": "1" }),
                        assistant_message: crate::agent_messages::phase_submission_test_message(
                            request.id,
                            "submit_review",
                            r#"{"passed":true,"confidence":0.82,"issues":["【建议】章节清单还不完整"]}"#,
                        ),
                        error: json!({
                            "kind": "protocol_error",
                            "message": "审核提交与本阶段合同不符",
                        }),
                        correction: "请重新调用 submit_review。".to_owned(),
                        tool_result_code: "PHASE_SUBMISSION_INVALID".to_owned(),
                    },
                )
                .await
                .unwrap();
            assert!(outcome.newly_applied);
            assert_eq!(outcome.consecutive_protocol_errors, attempt);
            assert_eq!(outcome.requeue, attempt < MAX_CONSECUTIVE_PROTOCOL_ERRORS);

            let persisted = persisted_protocol_error_count(&store, delegation.run_id).await;
            assert_eq!(persisted, attempt);
            assert_eq!(
                published_protocol_error_count(&store, request.id).await,
                persisted,
                "第 {attempt} 轮：落库计数与同一轮事件公布的计数必须一致"
            );
        }

        // 达上限那轮之后：子 Run 失败但计数仍然落库，Review 重新开放。
        let child_run = entity::run::Entity::find_by_id(db_id(delegation.run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(child_run.status, "failed");
        assert_eq!(
            child_run.consecutive_protocol_errors,
            MAX_CONSECUTIVE_PROTOCOL_ERRORS
        );
        snapshot = store.get_preparation_snapshot(version_id).await.unwrap();
        let reopened = snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
            .and_then(|artifact| artifact.review.as_ref())
            .unwrap();
        assert_eq!(reopened.status, ReviewStatus::Open);
    }

    #[tokio::test]
    async fn entity_schema_enforces_active_top_level_document_slots() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("实体索引", "顶层文档槽", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let document_slot = document::Entity::find()
            .filter(
                document::Entity::COLUMN
                    .version_id
                    .eq(db_id(created.version.id)),
            )
            .filter(document::Entity::COLUMN.role.eq(NOVEL_BRIEF_ROLE))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();

        let mut active_duplicate = document_slot.clone().into_active_model();
        active_duplicate.id = Set(db_id(DocumentId::new_v4()));
        assert!(active_duplicate.insert(&store.database).await.is_err());

        let mut archived_duplicate = document_slot.into_active_model();
        archived_duplicate.id = Set(db_id(DocumentId::new_v4()));
        archived_duplicate.deleted_at = Set(Some(utc_now_millis()));
        archived_duplicate.insert(&store.database).await.unwrap();
    }

    #[tokio::test]
    async fn entity_schema_enforces_active_chapter_order() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("章节索引", "章节顺序", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let now = utc_now_millis();
        let chapter_id = Uuid::new_v4();
        let first = entity::chapter::ActiveModel {
            id: Set(db_id(chapter_id)),
            version_id: Set(db_id(created.version.id)),
            kind: Set("main".to_owned()),
            plan_item_id: Set(Some(Uuid::new_v4().to_string())),
            order_index: Set(1),
            title: Set("第一章".to_owned()),
            status: Set("planned".to_owned()),
            approved_revision_id: Set(None),
            revision: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
            ..Default::default()
        };
        first.insert(&store.database).await.unwrap();

        let duplicate = entity::chapter::ActiveModel {
            id: Set(db_id(Uuid::new_v4())),
            version_id: Set(db_id(created.version.id)),
            kind: Set("main".to_owned()),
            plan_item_id: Set(Some(Uuid::new_v4().to_string())),
            order_index: Set(1),
            title: Set("重复顺序".to_owned()),
            status: Set("planned".to_owned()),
            approved_revision_id: Set(None),
            revision: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
            ..Default::default()
        }
        .insert(&store.database)
        .await;
        assert!(duplicate.is_err());

        let guidance = document::ActiveModel {
            id: Set(db_id(Uuid::new_v4())),
            version_id: Set(db_id(created.version.id)),
            chapter_id: Set(Some(chapter_id.to_string())),
            role: Set("chapter_guidance".to_owned()),
            current_revision_id: Set(None),
            revision: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
            ..Default::default()
        };
        guidance.insert(&store.database).await.unwrap();
        let duplicate_guidance = document::ActiveModel {
            id: Set(db_id(Uuid::new_v4())),
            version_id: Set(db_id(created.version.id)),
            chapter_id: Set(Some(chapter_id.to_string())),
            role: Set("chapter_guidance".to_owned()),
            current_revision_id: Set(None),
            revision: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
            ..Default::default()
        }
        .insert(&store.database)
        .await;
        assert!(duplicate_guidance.is_err());
    }

    #[tokio::test]
    async fn old_versions_remain_readable_after_the_selected_version_changes() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let first = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("多稿本快照", "旧稿仍可查看", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let second = store
            .create_version(
                first.project.id,
                Uuid::new_v4(),
                NewNovelVersion::new("第二稿", "旧稿仍可查看").unwrap(),
            )
            .await
            .unwrap();

        let old_snapshot = store.get_version_snapshot(first.version.id).await.unwrap();
        assert_eq!(old_snapshot.version.id, first.version.id);
        assert_eq!(old_snapshot.project.selected_version_id, second.version.id);
        assert_eq!(old_snapshot.workflow.version_id, first.version.id);

        let select_command_id = Uuid::new_v4();
        let selected = store
            .select_version(
                first.project.id,
                first.version.id,
                select_command_id,
                second.project.revision,
            )
            .await
            .unwrap();
        let replayed = store
            .select_version(
                first.project.id,
                first.version.id,
                select_command_id,
                second.project.revision,
            )
            .await
            .unwrap();
        assert_eq!(replayed, selected);
        assert_eq!(selected.version.id, first.version.id);
        assert_eq!(selected.project.selected_version_id, first.version.id);
    }

    #[tokio::test]
    async fn entity_schema_allows_only_one_active_top_level_writing_run_per_project() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let first = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("Run 互斥", "跨稿本不能并行写作", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let second = store
            .create_version(
                first.project.id,
                Uuid::new_v4(),
                NewNovelVersion::new("第二稿", "跨稿本不能并行写作").unwrap(),
            )
            .await
            .unwrap();
        let now = utc_now_millis();
        let first_run_id = Uuid::new_v4();
        entity::run::ActiveModel {
            id: Set(db_id(first_run_id)),
            project_id: Set(db_id(first.project.id)),
            version_id: Set(db_id(first.version.id)),
            kind: Set("preparation".to_owned()),
            parent_run_id: Set(None),
            stage: Set("preparing".to_owned()),
            status: Set("queued".to_owned()),
            resume_count: Set(0),
            request_count: Set(0),
            request_allowance_end: Set(100),
            consecutive_protocol_errors: Set(0),
            checkpoint_json: Set("{}".to_owned()),
            quota_scope_id: Set(None),
            started_at: Set(now),
            ended_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&store.database)
        .await
        .unwrap();

        let second_run = entity::run::ActiveModel {
            id: Set(db_id(Uuid::new_v4())),
            project_id: Set(db_id(first.project.id)),
            version_id: Set(db_id(second.version.id)),
            kind: Set("chapter_outline".to_owned()),
            parent_run_id: Set(None),
            stage: Set("writing.outline".to_owned()),
            status: Set("queued".to_owned()),
            resume_count: Set(0),
            request_count: Set(0),
            request_allowance_end: Set(100),
            consecutive_protocol_errors: Set(0),
            checkpoint_json: Set("{}".to_owned()),
            quota_scope_id: Set(None),
            started_at: Set(now),
            ended_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        assert!(second_run.clone().insert(&store.database).await.is_err());

        let first_run = entity::run::Entity::find_by_id(db_id(first_run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        let mut completed_run = first_run.into_active_model();
        completed_run.status = Set("succeeded".to_owned());
        completed_run.ended_at = Set(Some(now));
        completed_run.update(&store.database).await.unwrap();
        second_run.insert(&store.database).await.unwrap();
    }

    #[tokio::test]
    async fn version_snapshot_watermark_excludes_expired_events() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("事件水位", "快照只看可恢复事件", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let stale = entity::event::ActiveModel {
            version_id: Set(Some(created.version.id.to_string())),
            event_type: Set("workflow.changed".to_owned()),
            entity_id: Set(db_id(Uuid::new_v4())),
            revision: Set("1".to_owned()),
            payload_json: Set("{}".to_owned()),
            occurred_at: Set(utc_now_millis() - event_log::EVENT_RETENTION_MILLIS - 1),
            ..Default::default()
        }
        .insert(&store.database)
        .await
        .unwrap();
        let retained = entity::event::ActiveModel {
            version_id: Set(Some(created.version.id.to_string())),
            event_type: Set("workflow.changed".to_owned()),
            entity_id: Set(db_id(Uuid::new_v4())),
            revision: Set("2".to_owned()),
            payload_json: Set("{}".to_owned()),
            occurred_at: Set(utc_now_millis()),
            ..Default::default()
        }
        .insert(&store.database)
        .await
        .unwrap();

        let snapshot = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap();
        assert_eq!(snapshot.event_watermark, retained.seq.to_string());
        assert_ne!(snapshot.event_watermark, stale.seq.to_string());
        let events = store.events_after(0).await.unwrap();
        assert_eq!(events.len(), 2);
        assert!(
            events
                .iter()
                .any(|event| event.seq == retained.seq.to_string())
        );
        assert!(
            events
                .iter()
                .all(|event| event.seq != stale.seq.to_string())
        );
    }

    #[tokio::test]
    async fn persisted_events_are_ordered_and_wake_subscribers() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("事件同步", "持久事件", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let mut notifications = store.subscribe_events();

        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("通过 SSE 同步的小说目标。").unwrap(),
            )
            .await
            .unwrap();

        assert!(notifications.try_recv().is_ok());
        let events = store.events_after(0).await.unwrap();
        assert!(!events.is_empty());
        assert!(events.windows(2).all(|pair| {
            pair[0].seq.parse::<i64>().unwrap() < pair[1].seq.parse::<i64>().unwrap()
        }));
        let document_event = events
            .iter()
            .find(|event| event.event_type == "document.committed")
            .unwrap();
        assert_eq!(document_event.version_id, Some(created.version.id));
        assert_eq!(document_event.payload["role"], NOVEL_BRIEF_ROLE);
        assert!(
            store
                .events_after(document_event.seq.parse().unwrap())
                .await
                .unwrap()
                .is_empty()
        );
        let bounds = store.event_stream_bounds().await.unwrap();
        assert_eq!(bounds.minimum_cursor, 0);
        assert_eq!(bounds.watermark, document_event.seq.parse::<i64>().unwrap());
    }

    #[tokio::test]
    async fn approved_preparation_artifacts_fold_to_complete_plaintext_after_the_last_review() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let project = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("折叠测试", "港城雨夜", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let version_id = project.version.id;
        store
            .save_novel_brief(
                version_id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一部关于海港城与旧报纸的悬疑小说。").unwrap(),
            )
            .await
            .unwrap();
        let mut snapshot = store
            .start_preparation(version_id, Uuid::new_v4(), 0)
            .await
            .unwrap();
        let preparation_run_id = snapshot.workflow.active_run_id.unwrap();
        store
            .append_agent_messages(
                preparation_run_id,
                vec![NewAgentMessage {
                    role: AgentMessageRole::Assistant,
                    content: json!({
                        "kind": "text",
                        "role": "assistant",
                        "content": "我会先形成四项准备产物。"
                    }),
                    request_id: None,
                    tool_call_id: None,
                }],
            )
            .await
            .unwrap();
        snapshot = store
            .submit_preparation_artifacts(
                version_id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                vec![
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Outline,
                        "# 粗纲\n\n第一章：雨夜的来客",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Characters,
                        "林舟：调查旧报纸的记者。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Research,
                        "资料：用户提供的 1998 年港城晚报。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Background,
                        "海港城连续七日暴雨，旧码头即将拆除。",
                    )
                    .unwrap(),
                ],
                Some(vec![
                    nekowriter_core::ChapterPlanDraft::new(
                        "雨夜的来客",
                        "林舟在暴雨中收到一张来自旧码头的匿名报纸，决定追查线索。",
                    )
                    .unwrap(),
                    nekowriter_core::ChapterPlanDraft::new(
                        "仓库里的旧名",
                        "林舟循着报纸背面的名字回到仓库，必须决定是否公开父亲旧案。",
                    )
                    .unwrap(),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(snapshot.workflow.stage, WorkflowStage::PreparingReviewing);
        let submitted_plan_item_id = snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Outline)
            .and_then(|artifact| artifact.chapter_plan.as_ref())
            .map(|plan| {
                assert_eq!(plan.items.len(), 2);
                assert_eq!(plan.items[0].title, "雨夜的来客");
                assert_eq!(plan.items[1].title, "仓库里的旧名");
                plan.items[0].plan_item_id
            })
            .unwrap();

        for kind in [
            PreparationArtifactKind::Outline,
            PreparationArtifactKind::Background,
            PreparationArtifactKind::Research,
            PreparationArtifactKind::Characters,
        ] {
            let review_id = snapshot
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == kind)
                .and_then(|artifact| artifact.review.as_ref())
                .map(|review| review.id)
                .unwrap();
            snapshot = store
                .decide_preparation_review(
                    review_id,
                    Uuid::new_v4(),
                    snapshot.workflow.state_revision,
                    HumanReviewDecision::new(true, "").unwrap(),
                    Vec::new(),
                )
                .await
                .unwrap();
        }

        assert_eq!(snapshot.workflow.stage, WorkflowStage::WritingChapterReady);
        assert_eq!(snapshot.workflow.active_run_id, None);
        let fold = snapshot.context_fold.as_ref().unwrap();
        assert_eq!(
            fold.plain_text,
            "海港城连续七日暴雨，旧码头即将拆除。\n\n资料：用户提供的 1998 年港城晚报。\n\n林舟：调查旧报纸的记者。\n\n# 粗纲\n\n第一章：雨夜的来客\n\n第1章：雨夜的来客\n林舟在暴雨中收到一张来自旧码头的匿名报纸，决定追查线索。\n\n第2章：仓库里的旧名\n林舟循着报纸背面的名字回到仓库，必须决定是否公开父亲旧案。"
        );
        assert!(!fold.plain_text.contains("背景设定"));
        assert_eq!(fold.through_message_sequence, 1);
        let chapters = store.list_chapters(version_id).await.unwrap();
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].title, "雨夜的来客");
        assert_eq!(chapters[0].kind, nekowriter_core::ChapterKind::Main);
        assert_eq!(chapters[0].status, nekowriter_core::ChapterStatus::Planned);
        assert_eq!(chapters[0].plan_item_id, Some(submitted_plan_item_id));
        assert_eq!(chapters[1].title, "仓库里的旧名");
        assert_eq!(chapters[1].kind, nekowriter_core::ChapterKind::Main);
        assert_eq!(chapters[1].status, nekowriter_core::ChapterStatus::Planned);
        assert_eq!(snapshot.workflow.active_chapter_id, Some(chapters[0].id));
        let initial_guidance = store.get_chapter_guidance(chapters[0].id).await.unwrap();
        assert_eq!(initial_guidance.content, None);

        let saved_guidance = store
            .save_chapter_guidance(
                chapters[0].id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                NewChapterGuidance::new("让林舟先观察码头的异常灯火。 ").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            saved_guidance.content.as_deref(),
            Some("让林舟先观察码头的异常灯火。 ")
        );
        let after_guidance = store.get_preparation_snapshot(version_id).await.unwrap();
        let outline_started = store
            .start_chapter_outline(
                chapters[0].id,
                Uuid::new_v4(),
                after_guidance.workflow.state_revision,
            )
            .await
            .unwrap();
        assert_eq!(
            outline_started.stage,
            WorkflowStage::WritingOutlineGenerating
        );
        assert_eq!(outline_started.active_chapter_id, Some(chapters[0].id));
        assert!(outline_started.active_run_id.is_some());
        assert_eq!(
            store.list_chapters(version_id).await.unwrap()[0].status,
            nekowriter_core::ChapterStatus::InProgress
        );

        let first_outline = store
            .submit_chapter_outline(
                chapters[0].id,
                Uuid::new_v4(),
                outline_started.state_revision,
                NewChapterOutline::new(
                    "1. 林舟观察码头的异常灯火。\n2. 他在仓库外发现被撕去名字的报纸。\n3. 结尾让陌生来电打断追查。",
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            first_outline.workflow.stage,
            WorkflowStage::WritingOutlineReviewing
        );
        assert_eq!(first_outline.context_fold, None);
        let first_review_id = first_outline.outline.review.as_ref().unwrap().id;
        let rejected = store
            .decide_review(
                first_review_id,
                Uuid::new_v4(),
                first_outline.workflow.state_revision,
                HumanReviewDecision::new(false, "请让报纸线索与林舟的个人风险更直接相关。")
                    .unwrap(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            rejected.workflow.stage,
            WorkflowStage::WritingOutlineGenerating
        );
        assert_eq!(
            rejected.workflow.active_run_id,
            outline_started.active_run_id
        );
        assert_eq!(
            store
                .get_chapter_outline(chapters[0].id)
                .await
                .unwrap()
                .outline
                .review
                .unwrap()
                .status,
            nekowriter_core::ReviewStatus::Rejected
        );

        let revised_outline_content = "1. 林舟观察码头的异常灯火。\n2. 他发现父亲旧案中的报纸被人藏进仓库。\n3. 陌生来电警告他立刻离开。";
        let revised_outline = store
            .submit_chapter_outline(
                chapters[0].id,
                Uuid::new_v4(),
                rejected.workflow.state_revision,
                NewChapterOutline::new(revised_outline_content).unwrap(),
            )
            .await
            .unwrap();
        let revised_review_id = revised_outline.outline.review.as_ref().unwrap().id;
        let approval_command_id = Uuid::new_v4();
        let drafting = store
            .decide_review(
                revised_review_id,
                approval_command_id,
                revised_outline.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(drafting.workflow.stage, WorkflowStage::WritingDrafting);
        let body_run_id = drafting.workflow.active_run_id.unwrap();
        let replayed = store
            .decide_review(
                revised_review_id,
                approval_command_id,
                revised_outline.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(replayed, drafting);

        let approved_outline = store.get_chapter_outline(chapters[0].id).await.unwrap();
        let outline_fold = approved_outline.context_fold.as_ref().unwrap();
        assert_eq!(outline_fold.plain_text, revised_outline_content);
        assert_eq!(outline_fold.artifact_revision_ids.len(), 1);
        assert_eq!(outline_fold.through_message_sequence, 0);
        assert!(!outline_fold.plain_text.contains("审核"));

        let first_body = store
            .append_chapter_body(
                chapters[0].id,
                Uuid::new_v4(),
                drafting.workflow.state_revision,
                AppendChapterContent::new("林舟踏进码头仓库，雨水沿着衣角滴下。").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first_body.workflow.stage, WorkflowStage::WritingDrafting);
        assert_eq!(first_body.workflow.active_run_id, Some(body_run_id));
        let appended_body = store
            .append_chapter_body(
                chapters[0].id,
                Uuid::new_v4(),
                first_body.workflow.state_revision,
                AppendChapterContent::new("\n他把父亲的名字写在报纸背面。").unwrap(),
            )
            .await
            .unwrap();
        let replaced_body = store
            .replace_chapter_body(
                chapters[0].id,
                Uuid::new_v4(),
                appended_body.workflow.state_revision,
                ReplaceChapterContent::new("码头仓库", "旧仓库").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            replaced_body.body.content.as_deref(),
            Some("林舟踏进旧仓库，雨水沿着衣角滴下。\n他把父亲的名字写在报纸背面。")
        );
        let unchanged_body = store
            .replace_chapter_body(
                chapters[0].id,
                Uuid::new_v4(),
                replaced_body.workflow.state_revision,
                ReplaceChapterContent::new("旧仓库", "旧仓库").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unchanged_body, replaced_body);

        let first_final_review = store
            .submit_chapter_final_review(
                chapters[0].id,
                Uuid::new_v4(),
                replaced_body.workflow.state_revision,
            )
            .await
            .unwrap();
        assert_eq!(
            first_final_review.workflow.stage,
            WorkflowStage::WritingChapterReviewing
        );
        let first_final_review_id = first_final_review.body.review.as_ref().unwrap().id;
        let body_rejected = store
            .decide_review(
                first_final_review_id,
                Uuid::new_v4(),
                first_final_review.workflow.state_revision,
                HumanReviewDecision::new(false, "请补足林舟带走报纸的选择与后果。").unwrap(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(body_rejected.workflow.stage, WorkflowStage::WritingDrafting);
        assert_eq!(body_rejected.workflow.active_run_id, Some(body_run_id));
        let revised_body = store
            .append_chapter_body(
                chapters[0].id,
                Uuid::new_v4(),
                body_rejected.workflow.state_revision,
                AppendChapterContent::new("\n他决定先带走报纸，再追查来电来源。").unwrap(),
            )
            .await
            .unwrap();
        let final_review = store
            .submit_chapter_final_review(
                chapters[0].id,
                Uuid::new_v4(),
                revised_body.workflow.state_revision,
            )
            .await
            .unwrap();
        let final_review_id = final_review.body.review.as_ref().unwrap().id;
        let final_approval_command_id = Uuid::new_v4();
        let chapter_done = store
            .decide_review(
                final_review_id,
                final_approval_command_id,
                final_review.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            chapter_done.workflow.stage,
            WorkflowStage::WritingChapterDone
        );
        assert_eq!(chapter_done.workflow.active_run_id, None);
        assert_eq!(
            store.list_chapters(version_id).await.unwrap()[0].status,
            nekowriter_core::ChapterStatus::Approved
        );
        let advance_command_id = Uuid::new_v4();
        let next_chapter_ready = store
            .advance_to_next_chapter(
                version_id,
                advance_command_id,
                chapter_done.workflow.state_revision,
            )
            .await
            .unwrap();
        assert_eq!(
            next_chapter_ready.workflow.stage,
            WorkflowStage::WritingChapterReady
        );
        assert_eq!(next_chapter_ready.workflow.active_run_id, None);
        assert_eq!(
            next_chapter_ready.workflow.active_chapter_id,
            Some(chapters[1].id)
        );
        assert_eq!(
            store.list_chapters(version_id).await.unwrap()[1].status,
            nekowriter_core::ChapterStatus::Planned
        );
        let replayed_next_chapter = store
            .advance_to_next_chapter(
                version_id,
                advance_command_id,
                chapter_done.workflow.state_revision,
            )
            .await
            .unwrap();
        assert_eq!(replayed_next_chapter, next_chapter_ready);
        assert_eq!(
            store
                .get_chapter_guidance(chapters[1].id)
                .await
                .unwrap()
                .content,
            None
        );
        let approved_body = store.get_chapter_body(chapters[0].id).await.unwrap();
        let body_fold = approved_body.context_fold.as_ref().unwrap();
        assert_eq!(body_fold.artifact_revision_ids.len(), 1);
        assert_eq!(
            body_fold.plain_text,
            "林舟踏进旧仓库，雨水沿着衣角滴下。\n他把父亲的名字写在报纸背面。\n他决定先带走报纸，再追查来电来源。"
        );
        assert!(approved_body.outline_context_fold.is_some());

        let copy_command_id = Uuid::new_v4();
        let copied = store
            .copy_version(
                version_id,
                copy_command_id,
                NewCopiedNovelVersion::new("第二稿").unwrap(),
            )
            .await
            .unwrap();
        let replayed_copy = store
            .copy_version(
                version_id,
                copy_command_id,
                NewCopiedNovelVersion::new("第二稿").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replayed_copy, copied);
        assert_eq!(copied.version.source_version_id, Some(version_id));
        assert_eq!(copied.workflow.stage, WorkflowStage::WritingChapterReady);
        assert_eq!(copied.workflow.execution, ExecutionStatus::Idle);
        assert_eq!(copied.workflow.active_run_id, None);
        let copied_chapters = store.list_chapters(copied.version.id).await.unwrap();
        assert_eq!(copied_chapters.len(), 2);
        assert_eq!(
            copied_chapters[0].status,
            nekowriter_core::ChapterStatus::Approved
        );
        assert_eq!(
            copied_chapters[1].status,
            nekowriter_core::ChapterStatus::Planned
        );
        assert_eq!(
            copied.workflow.active_chapter_id,
            Some(copied_chapters[1].id)
        );
        let copied_preparation = store
            .get_preparation_snapshot(copied.version.id)
            .await
            .unwrap();
        assert_eq!(
            copied_preparation
                .context_fold
                .as_ref()
                .map(|fold| fold.plain_text.as_str()),
            Some(fold.plain_text.as_str())
        );
        let copied_contexts = store
            .list_completed_run_context_folds(copied.version.id)
            .await
            .unwrap();
        assert_eq!(copied_contexts.len(), 3);
        assert!(copied_contexts.iter().any(|context| {
            context.stage == CompletedRunContextStage::Preparation
                && context.chapter_order.is_none()
        }));
        assert!(copied_contexts.iter().any(|context| {
            context.stage == CompletedRunContextStage::ChapterOutline
                && context.chapter_order == Some(1)
        }));
        assert!(copied_contexts.iter().any(|context| {
            context.stage == CompletedRunContextStage::ChapterBody
                && context.chapter_order == Some(1)
        }));
        let copied_first_body = store.get_chapter_body(copied_chapters[0].id).await.unwrap();
        assert_eq!(
            copied_first_body
                .context_fold
                .as_ref()
                .map(|fold| fold.plain_text.as_str()),
            Some(body_fold.plain_text.as_str())
        );
        let second_guidance = store
            .save_chapter_guidance(
                chapters[1].id,
                Uuid::new_v4(),
                next_chapter_ready.workflow.state_revision,
                NewChapterGuidance::new("让林舟决定是否公开父亲旧案。 ").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            second_guidance.content.as_deref(),
            Some("让林舟决定是否公开父亲旧案。 ")
        );
        let after_second_guidance = store.get_preparation_snapshot(version_id).await.unwrap();
        let second_outline_started = store
            .start_chapter_outline(
                chapters[1].id,
                Uuid::new_v4(),
                after_second_guidance.workflow.state_revision,
            )
            .await
            .unwrap();
        let second_outline = store
            .submit_chapter_outline(
                chapters[1].id,
                Uuid::new_v4(),
                second_outline_started.state_revision,
                NewChapterOutline::new(
                    "1. 林舟回到仓库确认报纸背面的旧名。\n2. 他发现父亲旧案被人刻意遮掩。\n3. 结尾让他决定公开线索。",
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let second_drafting = store
            .decide_review(
                second_outline.outline.review.as_ref().unwrap().id,
                Uuid::new_v4(),
                second_outline.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
                Vec::new(),
            )
            .await
            .unwrap();
        let second_body = store
            .append_chapter_body(
                chapters[1].id,
                Uuid::new_v4(),
                second_drafting.workflow.state_revision,
                AppendChapterContent::new("林舟把旧名刊登在清晨的报纸上。 ").unwrap(),
            )
            .await
            .unwrap();
        let second_final_review = store
            .submit_chapter_final_review(
                chapters[1].id,
                Uuid::new_v4(),
                second_body.workflow.state_revision,
            )
            .await
            .unwrap();
        let second_chapter_done = store
            .decide_review(
                second_final_review.body.review.as_ref().unwrap().id,
                Uuid::new_v4(),
                second_final_review.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            second_chapter_done.workflow.stage,
            WorkflowStage::WritingChapterDone
        );
        assert_eq!(
            second_chapter_done.workflow.active_chapter_id,
            Some(chapters[1].id)
        );
        let completion_command_id = Uuid::new_v4();
        let completed = store
            .confirm_novel_completion(
                version_id,
                completion_command_id,
                second_chapter_done.workflow.state_revision,
            )
            .await
            .unwrap();
        assert_eq!(completed.workflow.stage, WorkflowStage::Completed);
        assert_eq!(completed.workflow.active_run_id, None);
        assert_eq!(completed.workflow.active_chapter_id, Some(chapters[1].id));
        assert_eq!(
            store.list_chapters(version_id).await.unwrap()[1].status,
            nekowriter_core::ChapterStatus::Approved
        );
        let replayed_completion = store
            .confirm_novel_completion(
                version_id,
                completion_command_id,
                second_chapter_done.workflow.state_revision,
            )
            .await
            .unwrap();
        assert_eq!(replayed_completion, completed);
        assert!(
            store
                .get_version_snapshot(version_id)
                .await
                .unwrap()
                .version
                .body_completed_at_ms
                .is_some()
        );

        let body_completed_at = store
            .get_version_snapshot(version_id)
            .await
            .unwrap()
            .version
            .body_completed_at_ms;
        let extra_command_id = Uuid::new_v4();
        let extra_ready = store
            .add_extra_chapter(
                version_id,
                extra_command_id,
                completed.workflow.state_revision,
                NewExtraChapter::new("雨停之后").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            extra_ready.workflow.stage,
            WorkflowStage::WritingChapterReady
        );
        let replayed_extra = store
            .add_extra_chapter(
                version_id,
                extra_command_id,
                completed.workflow.state_revision,
                NewExtraChapter::new("雨停之后").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replayed_extra, extra_ready);
        let extra = store
            .list_chapters(version_id)
            .await
            .unwrap()
            .into_iter()
            .find(|chapter| chapter.kind == nekowriter_core::ChapterKind::Extra)
            .unwrap();
        assert_eq!(extra.order_index, 3);
        assert_eq!(extra.plan_item_id, None);
        assert_eq!(
            store.get_chapter_guidance(extra.id).await.unwrap().content,
            None
        );

        store
            .save_chapter_guidance(
                extra.id,
                Uuid::new_v4(),
                extra_ready.workflow.state_revision,
                NewChapterGuidance::new("让林舟在雨停后回到旧码头，确认来信人的身份。").unwrap(),
            )
            .await
            .unwrap();
        let extra_after_guidance = store.get_preparation_snapshot(version_id).await.unwrap();
        let extra_outline_started = store
            .start_chapter_outline(
                extra.id,
                Uuid::new_v4(),
                extra_after_guidance.workflow.state_revision,
            )
            .await
            .unwrap();
        let extra_outline = store
            .submit_chapter_outline(
                extra.id,
                Uuid::new_v4(),
                extra_outline_started.state_revision,
                NewChapterOutline::new(
                    "1. 雨停后林舟重返码头。\n2. 他从来信人手中得知旧案的最后真相。\n3. 以清晨的报纸收束这一段后记。",
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let extra_drafting = store
            .decide_review(
                extra_outline.outline.review.as_ref().unwrap().id,
                Uuid::new_v4(),
                extra_outline.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
                Vec::new(),
            )
            .await
            .unwrap();
        let extra_body = store
            .append_chapter_body(
                extra.id,
                Uuid::new_v4(),
                extra_drafting.workflow.state_revision,
                AppendChapterContent::new("雨停时，林舟在旧码头读完了最后一封信。").unwrap(),
            )
            .await
            .unwrap();
        let extra_final_review = store
            .submit_chapter_final_review(
                extra.id,
                Uuid::new_v4(),
                extra_body.workflow.state_revision,
            )
            .await
            .unwrap();
        let extra_completed = store
            .decide_review(
                extra_final_review.body.review.as_ref().unwrap().id,
                Uuid::new_v4(),
                extra_final_review.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(extra_completed.workflow.stage, WorkflowStage::Completed);
        assert_eq!(extra_completed.workflow.active_chapter_id, Some(extra.id));
        assert_eq!(extra_completed.workflow.active_run_id, None);
        assert_eq!(
            store
                .get_version_snapshot(version_id)
                .await
                .unwrap()
                .version
                .body_completed_at_ms,
            body_completed_at
        );
        assert_eq!(
            store
                .get_chapter_body(extra.id)
                .await
                .unwrap()
                .context_fold
                .as_ref()
                .map(|fold| fold.plain_text.as_str()),
            Some("雨停时，林舟在旧码头读完了最后一封信。")
        );

        // 回收站：番外软删除后正文、修订 id 与位置都不动，恢复时原样回来。
        let body_before_delete = store.get_chapter_body(extra.id).await.unwrap().body;
        let revision_before_delete = body_before_delete.revision_id.unwrap();
        let deleted_snapshot = store
            .delete_extra_chapter(
                extra.id,
                Uuid::new_v4(),
                extra_completed.workflow.state_revision,
            )
            .await
            .unwrap();
        assert_eq!(deleted_snapshot.workflow.stage, WorkflowStage::Completed);
        assert_eq!(
            deleted_snapshot.workflow.active_chapter_id,
            Some(chapters[1].id)
        );
        assert!(
            store
                .list_chapters(version_id)
                .await
                .unwrap()
                .iter()
                .all(|chapter| chapter.id != extra.id)
        );
        let bin = store.get_recycle_bin().await.unwrap();
        let bin_chapter = bin
            .items
            .iter()
            .find(|item| item.id == extra.id)
            .expect("番外应该出现在回收站里");
        assert_eq!(bin_chapter.kind, RecycleItemKind::Chapter);
        assert!(bin_chapter.restorable);
        assert!(!bin_chapter.purgeable);
        assert!(
            bin_chapter
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("字正文"))
        );

        // 正文不提供物理删除：还在回收站里时明确拒绝。
        let purge_error = store
            .purge_recycle_item(RecycleItemKind::Chapter, extra.id, Uuid::new_v4())
            .await
            .unwrap_err();
        assert!(matches!(purge_error, StoreError::RecycleNotPurgeable(_)));
        assert_eq!(
            store.get_recycle_bin().await.unwrap().items.len(),
            bin.items.len()
        );

        let restored_bin = store
            .restore_recycle_item(RecycleItemKind::Chapter, extra.id, Uuid::new_v4())
            .await
            .unwrap();
        assert!(restored_bin.items.iter().all(|item| item.id != extra.id));
        let body_after_restore = store.get_chapter_body(extra.id).await.unwrap().body;
        assert_eq!(body_after_restore.revision_id, Some(revision_before_delete));
        assert_eq!(body_after_restore.content, body_before_delete.content);
        let restored_chapter = store
            .list_chapters(version_id)
            .await
            .unwrap()
            .into_iter()
            .find(|chapter| chapter.id == extra.id)
            .expect("恢复后番外应该回到目录");
        assert_eq!(restored_chapter.order_index, extra.order_index);
        assert_eq!(
            restored_chapter.status,
            nekowriter_core::ChapterStatus::Approved
        );

        // 重复恢复同一条记录：明确报错，而不是假装成功。
        let restore_error = store
            .restore_recycle_item(RecycleItemKind::Chapter, extra.id, Uuid::new_v4())
            .await
            .unwrap_err();
        assert!(matches!(restore_error, StoreError::RecycleItemNotFound));

        // 祖先仍在回收站里时，章节不能被单独恢复。
        let before_ancestor = store.get_preparation_snapshot(version_id).await.unwrap();
        store
            .delete_extra_chapter(
                extra.id,
                Uuid::new_v4(),
                before_ancestor.workflow.state_revision,
            )
            .await
            .unwrap();
        let bundle = store.get_version_snapshot(version_id).await.unwrap();
        assert_ne!(bundle.project.selected_version_id, version_id);
        let deleted_version_bin = store
            .delete_version(version_id, Uuid::new_v4(), bundle.version.revision)
            .await
            .unwrap();
        let bin_version = deleted_version_bin
            .items
            .iter()
            .find(|item| item.id == version_id)
            .expect("稿本应该出现在回收站里");
        assert_eq!(bin_version.kind, RecycleItemKind::Version);
        assert!(!bin_version.purgeable);
        assert!(bin_version.restorable);
        assert!(
            bin_version
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("章已定稿")),
            "回收站应该说明稿本里还剩多少章：{:?}",
            bin_version.detail
        );
        let ancestor_error = store
            .restore_recycle_item(RecycleItemKind::Chapter, extra.id, Uuid::new_v4())
            .await
            .unwrap_err();
        assert!(matches!(
            ancestor_error,
            StoreError::RecycleAncestorDeleted(_)
        ));

        // 先恢复稿本，再恢复番外，内容与修订仍然是删除前的那一份。
        store
            .restore_recycle_item(RecycleItemKind::Version, version_id, Uuid::new_v4())
            .await
            .unwrap();
        store
            .restore_recycle_item(RecycleItemKind::Chapter, extra.id, Uuid::new_v4())
            .await
            .unwrap();
        assert_eq!(
            store
                .get_chapter_body(extra.id)
                .await
                .unwrap()
                .body
                .revision_id,
            Some(revision_before_delete)
        );

        let copied_before_guidance = store
            .get_preparation_snapshot(copied.version.id)
            .await
            .unwrap();
        store
            .save_chapter_guidance(
                copied_chapters[1].id,
                Uuid::new_v4(),
                copied_before_guidance.workflow.state_revision,
                NewChapterGuidance::new("从复制稿本继续写第二章。").unwrap(),
            )
            .await
            .unwrap();
        let copied_outline_started = store
            .start_chapter_outline(
                copied_chapters[1].id,
                Uuid::new_v4(),
                store
                    .get_preparation_snapshot(copied.version.id)
                    .await
                    .unwrap()
                    .workflow
                    .state_revision,
            )
            .await
            .unwrap();
        assert_eq!(
            copied_outline_started.stage,
            WorkflowStage::WritingOutlineGenerating
        );

        drop(store);
        let reopened = Store::open(data_dir.path()).await.unwrap();
        let persisted = reopened.get_preparation_snapshot(version_id).await.unwrap();
        assert_eq!(persisted.context_fold, snapshot.context_fold);
        assert_eq!(persisted.workflow.stage, WorkflowStage::Completed);
        // 恢复只把数据与在作品里的位置放回去，不把写作光标搬回来：删除时工作流已经
        // 从那一章收回到仍然可见的最后一章，之后用户可能已经在别处继续写，所以
        // 光标保持删除后的位置。
        assert_eq!(persisted.workflow.active_chapter_id, Some(chapters[1].id));
        assert_eq!(
            reopened
                .get_chapter_outline(chapters[0].id)
                .await
                .unwrap()
                .context_fold
                .as_ref()
                .unwrap()
                .plain_text,
            revised_outline_content
        );
        assert!(
            reopened
                .get_chapter_body(chapters[0].id)
                .await
                .unwrap()
                .context_fold
                .is_some()
        );
    }

    #[tokio::test]
    async fn review_decision_persists_validated_citations_and_rejects_bad_ones() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let project = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("决策引用", "旧码头", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let version_id = project.version.id;
        store
            .save_novel_brief(
                version_id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一部关于旧码头与匿名报纸的悬疑小说。").unwrap(),
            )
            .await
            .unwrap();
        let mut snapshot = store
            .start_preparation(version_id, Uuid::new_v4(), 0)
            .await
            .unwrap();
        let preparation_run_id = snapshot.workflow.active_run_id.unwrap();
        store
            .append_agent_messages(
                preparation_run_id,
                vec![NewAgentMessage {
                    role: AgentMessageRole::Assistant,
                    content: json!({
                        "kind": "text",
                        "role": "assistant",
                        "content": "我会先形成四项准备产物。"
                    }),
                    request_id: None,
                    tool_call_id: None,
                }],
            )
            .await
            .unwrap();
        snapshot = store
            .submit_preparation_artifacts(
                version_id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                vec![
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Outline,
                        "# 粗纲\n\n第一章：旧码头的报纸",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Characters,
                        "林舟：追查旧报纸的记者。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Research,
                        "资料：1998 年港城晚报。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Background,
                        "海港城连续七日暴雨，旧码头即将拆除。",
                    )
                    .unwrap(),
                ],
                Some(vec![
                    nekowriter_core::ChapterPlanDraft::new(
                        "旧码头的报纸",
                        "林舟在暴雨中收到匿名报纸并决定追查。",
                    )
                    .unwrap(),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(snapshot.workflow.stage, WorkflowStage::PreparingReviewing);

        let background = snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
            .unwrap();
        let review_id = background.review.as_ref().unwrap().id;
        let document_id = background.document_id;
        let revision_id = background.revision_id.unwrap();
        let content = background.content.clone().unwrap();
        assert_eq!(content, "海港城连续七日暴雨，旧码头即将拆除。");

        // 负向：quote 与修订原文不一致时必须拒绝，且审核保持未决。
        assert!(matches!(
            store
                .decide_review(
                    review_id,
                    Uuid::new_v4(),
                    snapshot.workflow.state_revision,
                    HumanReviewDecision::new(true, "").unwrap(),
                    vec![nekowriter_core::FeedbackCitation::Document {
                        document_id,
                        revision_id,
                        range: nekowriter_core::TextCitationRange {
                            start_char: 0,
                            end_char: 2,
                            quote: "暴雨".to_owned(),
                        },
                        chapter_id: None,
                    }],
                )
                .await,
            Err(StoreError::InvalidCitation(_))
        ));
        let after_rejection = store.get_preparation_snapshot(version_id).await.unwrap();
        assert_eq!(
            after_rejection
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
                .and_then(|artifact| artifact.review.as_ref())
                .map(|review| review.status.clone()),
            Some(ReviewStatus::Open)
        );

        // 负向：空文字且没有引用仍不能形成一条可执行的退回意见。
        assert!(matches!(
            store
                .decide_review(
                    review_id,
                    Uuid::new_v4(),
                    snapshot.workflow.state_revision,
                    HumanReviewDecision::new(false, "").unwrap(),
                    vec![],
                )
                .await,
            Err(StoreError::InvalidReviewDecision(_))
        ));

        // 正向：退回可以只附一条经验证的引用，不必再拼一段重复文字。
        snapshot = store
            .decide_review(
                review_id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                HumanReviewDecision::new(false, "").unwrap(),
                vec![nekowriter_core::FeedbackCitation::Document {
                    document_id,
                    revision_id,
                    range: nekowriter_core::TextCitationRange {
                        start_char: 7,
                        end_char: 9,
                        quote: "暴雨".to_owned(),
                    },
                    chapter_id: None,
                }],
            )
            .await
            .unwrap();
        assert_eq!(
            snapshot
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
                .and_then(|artifact| artifact.review.as_ref())
                .map(|review| review.status.clone()),
            Some(ReviewStatus::Rejected)
        );

        let review_model = entity::review::Entity::find_by_id(db_id(review_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        let user_input: serde_json::Value =
            serde_json::from_str(review_model.user_input_json.as_deref().unwrap()).unwrap();
        assert_eq!(user_input["source"], "user");
        assert_eq!(user_input["citations"][0]["kind"], "document");
        assert_eq!(user_input["citations"][0]["quote"], "暴雨");
        assert_eq!(
            user_input["citations"][0]["revisionId"],
            revision_id.to_string()
        );
    }

    #[tokio::test]
    async fn delegated_review_applies_once_and_cancellation_rejects_late_results() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let project = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("审核委托", "港城旧报纸", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let version_id = project.version.id;
        store
            .save_novel_brief(
                version_id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一部围绕旧码头和匿名报纸的悬疑小说。 ").unwrap(),
            )
            .await
            .unwrap();
        let started = store
            .start_preparation(version_id, Uuid::new_v4(), 0)
            .await
            .unwrap();
        let mut snapshot = store
            .submit_preparation_artifacts(
                version_id,
                Uuid::new_v4(),
                started.workflow.state_revision,
                vec![
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Background,
                        "港城的旧码头将在雨季后拆除。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Research,
                        "旧报纸来自 1998 年的港城晚报，来源尚待核验。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Characters,
                        "林舟是追查父亲旧案的记者。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Outline,
                        "林舟在码头收到匿名报纸，循着署名进入仓库并决定公开线索。",
                    )
                    .unwrap(),
                ],
                Some(vec![
                    ChapterPlanDraft::new("雨夜来客", "林舟在码头收到匿名报纸。").unwrap(),
                ]),
            )
            .await
            .unwrap();
        let background_review_id = snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
            .and_then(|artifact| artifact.review.as_ref())
            .map(|review| review.id)
            .unwrap();
        let delegation = store
            .delegate_review(
                background_review_id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                "检查设定是否足以支撑第一章。".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        let task = store
            .delegated_review_task(background_review_id, delegation.run_id)
            .await
            .unwrap();
        assert!(matches!(
            task.kind,
            DelegatedReviewKind::Preparation(PreparationArtifactKind::Background)
        ));
        assert_eq!(task.user_notes, "检查设定是否足以支撑第一章。");
        let request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: delegation.run_id,
                    settings_snapshot: json!({ "model": "review-test" }),
                    prompt_snapshot: json!({ "key": "review.preparation.background" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        snapshot = store
            .commit_delegated_review_result(
                background_review_id,
                delegation.run_id,
                request.id,
                Some("review-provider-1".to_owned()),
                json!({}),
                crate::agent_messages::phase_submission_test_message(
                    request.id,
                    "submit_review",
                    r#"{"passed":true,"confidence":0.91,"issues":[]}"#,
                ),
                true,
                0.91,
                Vec::new(),
            )
            .await
            .unwrap();
        let background_review = snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
            .and_then(|artifact| artifact.review.as_ref())
            .unwrap();
        assert_eq!(background_review.status, ReviewStatus::Approved);
        assert_eq!(background_review.passed, Some(true));
        let replayed = store
            .commit_delegated_review_result(
                background_review_id,
                delegation.run_id,
                request.id,
                Some("review-provider-1".to_owned()),
                json!({}),
                crate::agent_messages::phase_submission_test_message(
                    request.id,
                    "submit_review",
                    r#"{"passed":true,"confidence":0.91,"issues":[]}"#,
                ),
                true,
                0.91,
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(replayed, snapshot);

        let research_review_id = snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Research)
            .and_then(|artifact| artifact.review.as_ref())
            .map(|review| review.id)
            .unwrap();
        let delegation = store
            .delegate_review(
                research_review_id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                String::new(),
                Vec::new(),
            )
            .await
            .unwrap();
        let request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: delegation.run_id,
                    settings_snapshot: json!({ "model": "review-test" }),
                    prompt_snapshot: json!({ "key": "review.preparation.research" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let cancellation = store
            .cancel_review_delegation(
                research_review_id,
                Uuid::new_v4(),
                delegation.snapshot.workflow.state_revision,
            )
            .await
            .unwrap();
        let research_review = cancellation
            .snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Research)
            .and_then(|artifact| artifact.review.as_ref())
            .unwrap();
        assert_eq!(research_review.status, ReviewStatus::Open);
        assert!(matches!(
            store
                .commit_delegated_review_result(
                    research_review_id,
                    delegation.run_id,
                    request.id,
                    None,
                    json!({}),
                    crate::agent_messages::phase_submission_test_message(
                        request.id,
                        "submit_review",
                        r#"{"passed":true,"confidence":0.8,"issues":[]}"#,
                    ),
                    true,
                    0.8,
                    Vec::new(),
                )
                .await,
            Err(StoreError::ReviewAlreadyResolved)
        ));

        store
            .delegate_review(
                research_review_id,
                Uuid::new_v4(),
                cancellation.snapshot.workflow.state_revision,
                "重启恢复不应保留悬挂的审核请求。".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        drop(store);
        let reopened = Store::open(data_dir.path()).await.unwrap();
        let recovered = reopened.get_preparation_snapshot(version_id).await.unwrap();
        assert_eq!(
            recovered
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == PreparationArtifactKind::Research)
                .and_then(|artifact| artifact.review.as_ref())
                .unwrap()
                .status,
            ReviewStatus::Open
        );
    }

    #[tokio::test]
    async fn delegated_outline_edit_and_final_reviews_apply_their_workflows() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let project = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("审核流程", "雨夜车站", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let version_id = project.version.id;
        store
            .save_novel_brief(
                version_id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一部围绕雨夜车站和失踪信件展开的悬疑小说。").unwrap(),
            )
            .await
            .unwrap();
        let started = store
            .start_preparation(version_id, Uuid::new_v4(), 0)
            .await
            .unwrap();
        let mut snapshot = store
            .submit_preparation_artifacts(
                version_id,
                Uuid::new_v4(),
                started.workflow.state_revision,
                vec![
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Background,
                        "雨夜的旧车站即将拆除，末班车仍在运行。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Research,
                        "用户提供了一封没有署名的旧信，出处尚待核验。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Characters,
                        "沈宁是追查失踪姐姐下落的摄影记者。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Outline,
                        "沈宁在末班车上收到旧信，决定进入废弃候车室寻找寄信人。",
                    )
                    .unwrap(),
                ],
                Some(vec![
                    ChapterPlanDraft::new("末班车", "沈宁在雨夜车站收到一封旧信。").unwrap(),
                ]),
            )
            .await
            .unwrap();
        for kind in PreparationArtifactKind::ALL {
            let review_id = snapshot
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == kind)
                .and_then(|artifact| artifact.review.as_ref())
                .map(|review| review.id)
                .unwrap();
            snapshot = store
                .decide_review(
                    review_id,
                    Uuid::new_v4(),
                    snapshot.workflow.state_revision,
                    HumanReviewDecision::new(true, "").unwrap(),
                    Vec::new(),
                )
                .await
                .unwrap();
        }

        let chapter_id = snapshot.workflow.active_chapter_id.unwrap();
        store
            .save_chapter_guidance(
                chapter_id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                NewChapterGuidance::new("让沈宁先辨认旧信上的车站印章。").unwrap(),
            )
            .await
            .unwrap();
        let ready = store.get_preparation_snapshot(version_id).await.unwrap();
        let outline_started = store
            .start_chapter_outline(chapter_id, Uuid::new_v4(), ready.workflow.state_revision)
            .await
            .unwrap();
        let outline_content = "1. 沈宁辨认旧信上的车站印章。\n2. 她在候车室发现姐姐留下的胶卷。\n3. 末班车鸣笛，迫使她决定是否追上寄信人。";
        let outline = store
            .submit_chapter_outline(
                chapter_id,
                Uuid::new_v4(),
                outline_started.state_revision,
                nekowriter_core::NewChapterOutline::new(outline_content).unwrap(),
            )
            .await
            .unwrap();
        let outline_review_id = outline.outline.review.as_ref().unwrap().id;
        let outline_delegation = store
            .delegate_review(
                outline_review_id,
                Uuid::new_v4(),
                outline.workflow.state_revision,
                "检查细纲是否保留悬念。".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        let outline_task = store
            .delegated_review_task(outline_review_id, outline_delegation.run_id)
            .await
            .unwrap();
        assert_eq!(outline_task.kind, DelegatedReviewKind::ChapterOutline);
        assert_eq!(outline_task.target_content, outline_content);
        assert!(outline_task.chapter_context.contains("末班车"));
        let outline_request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: outline_delegation.run_id,
                    settings_snapshot: json!({ "model": "review-test" }),
                    prompt_snapshot: json!({ "key": "review.chapter.outline" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let after_outline = store
            .commit_delegated_review_result(
                outline_review_id,
                outline_delegation.run_id,
                outline_request.id,
                Some("review-provider-outline".to_owned()),
                json!({}),
                crate::agent_messages::phase_submission_test_message(
                    outline_request.id,
                    "submit_review",
                    r#"{"passed":true,"confidence":0.82,"issues":[]}"#,
                ),
                true,
                0.82,
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(after_outline.workflow.stage, WorkflowStage::WritingDrafting);
        assert_eq!(after_outline.workflow.execution, ExecutionStatus::Queued);
        let body_run_id = after_outline.workflow.active_run_id.unwrap();
        let approved_outline = store.get_chapter_outline(chapter_id).await.unwrap();
        assert_eq!(
            approved_outline.outline.review.as_ref().unwrap().status,
            ReviewStatus::Approved
        );
        assert_eq!(
            approved_outline.context_fold.as_ref().unwrap().plain_text,
            outline_content
        );

        let approve_required = store
            .set_approval_mode(
                version_id,
                Uuid::new_v4(),
                after_outline.workflow.state_revision,
                nekowriter_core::ApprovalMode::ApproveRequired,
            )
            .await
            .unwrap();
        let body_request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: body_run_id,
                    settings_snapshot: json!({ "model": "writing-test" }),
                    prompt_snapshot: json!({ "key": "chapter.write" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let proposal_content = "沈宁攥紧胶卷，踏上刚刚进站的末班车。";
        let applied = store
            .execute_chapter_body_agent_tool_call(
                body_request.id,
                chapter_id,
                Some("writing-provider-proposal".to_owned()),
                json!({}),
                ChapterBodyAgentToolCall {
                    provider_call_id: "proposal-call".to_owned(),
                    name: "append_content".to_owned(),
                    arguments: json!({ "content": proposal_content }),
                },
            )
            .await
            .unwrap();
        assert!(!applied.newly_resolved);
        let pending = store.get_chapter_body(chapter_id).await.unwrap();
        assert_eq!(pending.workflow.stage, WorkflowStage::WritingDrafting);
        assert_eq!(pending.workflow.execution, ExecutionStatus::WaitingReview);
        assert_eq!(
            pending.workflow.active_run_id,
            approve_required.active_run_id
        );
        let proposal = pending.pending_proposal.as_ref().unwrap();
        let edit_review = pending.pending_proposal_review.as_ref().unwrap();
        assert_eq!(edit_review.status, ReviewStatus::Open);
        // 委托时固化的引用必须逐字复核：错误的候选哈希不能进入子 Run。
        assert!(matches!(
            store
                .delegate_review(
                    edit_review.id,
                    Uuid::new_v4(),
                    pending.workflow.state_revision,
                    String::new(),
                    vec![nekowriter_core::FeedbackCitation::Candidate {
                        tool_call_id: proposal.tool_call_id,
                        candidate_hash: "0".repeat(64),
                        candidate_range: nekowriter_core::TextCitationRange {
                            start_char: 4,
                            end_char: 6,
                            quote: "胶卷".to_owned(),
                        },
                    }],
                )
                .await,
            Err(StoreError::InvalidCitation(_))
        ));
        let edit_delegation = store
            .delegate_review(
                edit_review.id,
                Uuid::new_v4(),
                pending.workflow.state_revision,
                "检查这次追加是否保留人物动机。".to_owned(),
                vec![nekowriter_core::FeedbackCitation::Candidate {
                    tool_call_id: proposal.tool_call_id,
                    candidate_hash: proposal.candidate_hash.clone(),
                    candidate_range: nekowriter_core::TextCitationRange {
                        start_char: 4,
                        end_char: 6,
                        quote: "胶卷".to_owned(),
                    },
                }],
            )
            .await
            .unwrap();
        let edit_task = store
            .delegated_review_task(edit_review.id, edit_delegation.run_id)
            .await
            .unwrap();
        assert_eq!(edit_task.kind, DelegatedReviewKind::ChapterEdit);
        assert_eq!(edit_task.base_content, "");
        assert_eq!(edit_task.target_content, proposal.candidate_content);
        assert_eq!(
            edit_task.citations,
            vec![nekowriter_core::FeedbackCitation::Candidate {
                tool_call_id: proposal.tool_call_id,
                candidate_hash: proposal.candidate_hash.clone(),
                candidate_range: nekowriter_core::TextCitationRange {
                    start_char: 4,
                    end_char: 6,
                    quote: "胶卷".to_owned(),
                },
            }]
        );
        let edit_request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: edit_delegation.run_id,
                    settings_snapshot: json!({ "model": "review-test" }),
                    prompt_snapshot: json!({ "key": "review.chapter.edit" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let after_edit = store
            .commit_delegated_review_result(
                edit_review.id,
                edit_delegation.run_id,
                edit_request.id,
                Some("review-provider-edit".to_owned()),
                json!({}),
                crate::agent_messages::phase_submission_test_message(
                    edit_request.id,
                    "submit_review",
                    r#"{"passed":true,"confidence":0.77,"issues":[]}"#,
                ),
                true,
                0.77,
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(after_edit.workflow.stage, WorkflowStage::WritingDrafting);
        assert_eq!(after_edit.workflow.execution, ExecutionStatus::Queued);
        let edited_body = store.get_chapter_body(chapter_id).await.unwrap();
        assert_eq!(edited_body.body.content.as_deref(), Some(proposal_content));
        assert!(edited_body.pending_proposal.is_none());
        assert!(edited_body.pending_proposal_review.is_none());
        let body_revision_id = edited_body.body.revision_id.unwrap();
        let revision_view = store
            .get_document_revision(edited_body.body.document_id, body_revision_id)
            .await
            .unwrap();
        assert_eq!(revision_view.content, proposal_content);
        assert_eq!(revision_view.role, CHAPTER_BODY_ROLE);
        assert_eq!(revision_view.chapter_id, Some(chapter_id));
        assert!(revision_view.is_current);
        assert!(matches!(
            store
                .get_document_revision(edited_body.body.document_id, Uuid::new_v4())
                .await,
            Err(StoreError::DocumentRevisionNotFound)
        ));

        let final_review = store
            .submit_chapter_final_review(
                chapter_id,
                Uuid::new_v4(),
                after_edit.workflow.state_revision,
            )
            .await
            .unwrap();
        let final_review_id = final_review.body.review.as_ref().unwrap().id;
        let final_delegation = store
            .delegate_review(
                final_review_id,
                Uuid::new_v4(),
                final_review.workflow.state_revision,
                "检查结尾是否完成本章目标。".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        let final_task = store
            .delegated_review_task(final_review_id, final_delegation.run_id)
            .await
            .unwrap();
        assert_eq!(final_task.kind, DelegatedReviewKind::ChapterFinal);
        assert_eq!(final_task.target_content, proposal_content);
        assert_eq!(final_task.chapter_outline, outline_content);
        let final_request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: final_delegation.run_id,
                    settings_snapshot: json!({ "model": "review-test" }),
                    prompt_snapshot: json!({ "key": "review.chapter.final" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let done = store
            .commit_delegated_review_result(
                final_review_id,
                final_delegation.run_id,
                final_request.id,
                Some("review-provider-final".to_owned()),
                json!({}),
                crate::agent_messages::phase_submission_test_message(
                    final_request.id,
                    "submit_review",
                    r#"{"passed":true,"confidence":0.94,"issues":[]}"#,
                ),
                true,
                0.94,
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(done.workflow.stage, WorkflowStage::WritingChapterDone);
        assert_eq!(done.workflow.execution, ExecutionStatus::Idle);
        assert_eq!(
            store.list_chapters(version_id).await.unwrap()[0].status,
            nekowriter_core::ChapterStatus::Approved
        );
    }

    #[tokio::test]
    async fn outline_submission_requires_a_structured_chapter_plan() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let project = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("章节计划校验", "港城雨夜", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let version_id = project.version.id;
        store
            .save_novel_brief(
                version_id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一部在港城旧码头展开的悬疑小说。 ").unwrap(),
            )
            .await
            .unwrap();
        let started = store
            .start_preparation(version_id, Uuid::new_v4(), 0)
            .await
            .unwrap();

        let error = store
            .submit_preparation_artifacts(
                version_id,
                Uuid::new_v4(),
                started.workflow.state_revision,
                vec![
                    PreparationArtifactInput::new(PreparationArtifactKind::Background, "背景")
                        .unwrap(),
                    PreparationArtifactInput::new(PreparationArtifactKind::Research, "资料")
                        .unwrap(),
                    PreparationArtifactInput::new(PreparationArtifactKind::Characters, "人物")
                        .unwrap(),
                    PreparationArtifactInput::new(PreparationArtifactKind::Outline, "粗纲")
                        .unwrap(),
                ],
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            StoreError::InvalidPreparationArtifacts(message)
                if message.contains("章节计划")
        ));
        assert_eq!(
            store
                .get_preparation_snapshot(version_id)
                .await
                .unwrap()
                .workflow
                .stage,
            WorkflowStage::PreparingGenerating
        );
    }

    #[tokio::test]
    async fn lancedb_smoke_can_write_and_query_a_table() {
        let data_dir = TempDir::new().unwrap();
        let lance_dir = data_dir.path().join("lancedb");
        let connection = lancedb::connect(lance_dir.to_str().unwrap())
            .execute()
            .await
            .unwrap();
        let rows = record_batch!(("id", Int32, [1, 2])).unwrap();
        let table = connection
            .create_table("m0_smoke", rows)
            .execute()
            .await
            .unwrap();
        let batches = table
            .query()
            .limit(1)
            .execute()
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_eq!(
            batches
                .iter()
                .map(arrow_array::RecordBatch::num_rows)
                .sum::<usize>(),
            1
        );
    }

    #[tokio::test]
    async fn process_shutdown_interrupts_in_flight_runs_and_close_keeps_data() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("退出收尾", "退出服务不丢已提交内容", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个雨夜车站重逢的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let before_start = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(
                created.version.id,
                Uuid::new_v4(),
                before_start.state_revision,
            )
            .await
            .unwrap();
        let run_id = started.workflow.active_run_id.unwrap();
        let dispatched = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id,
                    settings_snapshot: json!({ "model": "shutdown-test" }),
                    prompt_snapshot: json!({ "contentHash": "shutdown-test" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(dispatched.status, ModelRequestStatus::Dispatched);

        let outcome = store
            .interrupt_active_runs(InterruptionSource::ProcessShutdown)
            .await
            .unwrap();
        assert_eq!(outcome.writing_runs, 1);
        assert_eq!(outcome.review_delegations, 0);

        let workflow = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        assert_eq!(workflow.execution, ExecutionStatus::Interrupted);
        let requests = store.list_model_requests(run_id).await.unwrap();
        assert_eq!(requests[0].status, ModelRequestStatus::Unknown);
        let run_model = entity::run::Entity::find_by_id(db_id(run_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run_model.status, "interrupted");
        assert!(run_model.ended_at.is_some());
        let request_model = entity::model_request::Entity::find_by_id(db_id(dispatched.id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        let error_json = request_model.error_json.unwrap();
        assert!(error_json.contains("process_shutdown"), "{error_json}");
        assert!(!error_json.contains("startup_recovery"), "{error_json}");
        // 两种中断来源在事件里可区分，界面才能解释这次中断来自哪个动作。
        let events = store.events_after(0).await.unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.payload["reason"] == "process_shutdown")
                .count(),
            2
        );
        assert!(
            events
                .iter()
                .all(|event| event.payload["reason"] != "startup_recovery")
        );

        // 已经没有自动执行的 Run，重复收尾不再计入。
        let repeated = store
            .interrupt_active_runs(InterruptionSource::ProcessShutdown)
            .await
            .unwrap();
        assert_eq!(repeated, InterruptionOutcome::default());

        store.close().await.unwrap();
        // 关闭之后连接池真的不再接受查询。
        assert!(store.list_projects().await.is_err());

        // WAL 已落回主库文件：重新打开能读回全部记录，且不需要再次恢复。
        let reopened = Store::open(data_dir.path()).await.unwrap();
        assert_eq!(reopened.list_projects().await.unwrap().len(), 1);
        assert_eq!(
            reopened.list_model_requests(run_id).await.unwrap()[0].status,
            ModelRequestStatus::Unknown
        );
    }

    /// 回收站覆盖提示词模板与知识对象：彻底删除之后不可恢复，父对象未恢复时
    /// 子对象拒绝恢复，而不是安静地半恢复。
    #[tokio::test]
    async fn recycle_bin_covers_templates_and_knowledge_objects() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();

        let template = store
            .create_prompt_template(
                Uuid::new_v4(),
                NewPromptTemplate {
                    title: "雨景描写".to_owned(),
                    body: "写一段雨夜街景。".to_owned(),
                    variables: Vec::new(),
                },
            )
            .await
            .unwrap();
        let space = store
            .create_knowledge_space(
                Uuid::new_v4(),
                KnowledgeSpaceInput {
                    title: "人物设定".to_owned(),
                },
            )
            .await
            .unwrap();
        let entry = store
            .create_knowledge_entry(
                Uuid::new_v4(),
                NewKnowledgeEntry {
                    space_id: space.id,
                    title: "林舟".to_owned(),
                    keywords: vec!["主角".to_owned()],
                    content: "林舟是港城报社的记者。".to_owned(),
                    source: None,
                },
            )
            .await
            .unwrap();

        store
            .delete_prompt_template(template.id, Uuid::new_v4(), template.revision)
            .await
            .unwrap();
        store
            .delete_knowledge_entry(entry.id, Uuid::new_v4(), entry.revision)
            .await
            .unwrap();
        // 条目已软删除，所以 Space 现在可以删除：这正是「父对象在回收站里」的场景。
        let space_revision = store.list_knowledge_spaces().await.unwrap()[0].revision;
        store
            .delete_knowledge_space(space.id, Uuid::new_v4(), space_revision)
            .await
            .unwrap();

        let bin = store.get_recycle_bin().await.unwrap();
        assert_eq!(bin.items.len(), 3);
        let bin_entry = bin
            .items
            .iter()
            .find(|item| item.id == entry.id)
            .expect("知识条目应该在回收站里");
        assert_eq!(bin_entry.kind, RecycleItemKind::KnowledgeEntry);
        assert!(!bin_entry.restorable);
        assert!(bin_entry.purgeable);
        assert!(
            bin_entry
                .restore_blocked_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("Space"))
        );
        let bin_space = bin
            .items
            .iter()
            .find(|item| item.id == space.id)
            .expect("Space 应该在回收站里");
        assert_eq!(bin_space.kind, RecycleItemKind::KnowledgeSpace);
        assert!(bin_space.restorable);
        let bin_template = bin
            .items
            .iter()
            .find(|item| item.id == template.id)
            .expect("模板应该在回收站里");
        assert_eq!(bin_template.kind, RecycleItemKind::PromptTemplate);
        assert!(bin_template.purgeable);

        let ancestor_error = store
            .restore_recycle_item(RecycleItemKind::KnowledgeEntry, entry.id, Uuid::new_v4())
            .await
            .unwrap_err();
        assert!(matches!(
            ancestor_error,
            StoreError::RecycleAncestorDeleted(_)
        ));

        // 先恢复 Space，条目才能恢复；恢复后内容与修订都对得上。
        store
            .restore_recycle_item(RecycleItemKind::KnowledgeSpace, space.id, Uuid::new_v4())
            .await
            .unwrap();
        let restored = store
            .restore_recycle_item(RecycleItemKind::KnowledgeEntry, entry.id, Uuid::new_v4())
            .await
            .unwrap();
        assert!(restored.items.iter().all(|item| item.id != entry.id));
        let restored_entry = store.get_knowledge_entry(entry.id).await.unwrap();
        assert_eq!(restored_entry.content, "林舟是港城报社的记者。");
        assert!(restored_entry.revision > entry.revision);

        // 彻底删除模板：行真的没了，之后再想恢复必须报「不在回收站里」。
        let purged = store
            .purge_recycle_item(RecycleItemKind::PromptTemplate, template.id, Uuid::new_v4())
            .await
            .unwrap();
        assert!(purged.items.iter().all(|item| item.id != template.id));
        let restore_error = store
            .restore_recycle_item(RecycleItemKind::PromptTemplate, template.id, Uuid::new_v4())
            .await
            .unwrap_err();
        assert!(matches!(restore_error, StoreError::RecycleItemNotFound));

        // 重复恢复同一条记录同样报错。
        let repeated = store
            .restore_recycle_item(RecycleItemKind::KnowledgeSpace, space.id, Uuid::new_v4())
            .await
            .unwrap_err();
        assert!(matches!(repeated, StoreError::RecycleItemNotFound));

        // 已经恢复的对象不在回收站里，再想彻底删除同样报错，而不是假装成功。
        let not_in_bin = store
            .purge_recycle_item(RecycleItemKind::KnowledgeSpace, space.id, Uuid::new_v4())
            .await
            .unwrap_err();
        assert!(matches!(not_in_bin, StoreError::RecycleItemNotFound));
        assert_eq!(store.list_knowledge_spaces().await.unwrap().len(), 1);
        assert_eq!(
            store
                .list_knowledge_entries(Some(space.id), None)
                .await
                .unwrap()
                .len(),
            1
        );

        // 条目先删、Space 后删，然后两者一起彻底删除：回收站真的空下来。
        store
            .delete_knowledge_entry(entry.id, Uuid::new_v4(), restored_entry.revision)
            .await
            .unwrap();
        let space_revision = store.list_knowledge_spaces().await.unwrap()[0].revision;
        store
            .delete_knowledge_space(space.id, Uuid::new_v4(), space_revision)
            .await
            .unwrap();
        store
            .purge_recycle_item(RecycleItemKind::KnowledgeEntry, entry.id, Uuid::new_v4())
            .await
            .unwrap();
        store
            .purge_recycle_item(RecycleItemKind::KnowledgeSpace, space.id, Uuid::new_v4())
            .await
            .unwrap();
        assert_eq!(
            store.list_knowledge_spaces().await.unwrap(),
            [] as [nekowriter_core::KnowledgeSpace; 0]
        );
        assert!(store.get_knowledge_entry(entry.id).await.is_err());
        assert_eq!(
            store.get_recycle_bin().await.unwrap().items,
            [] as [nekowriter_core::recycle::RecycleItem; 0]
        );
    }
}
