//! 每稿本串行命令循环与全局模型请求调度。
//!
//! SQLite 是流程事实来源；本模块只负责把 HTTP 入口和模型回调放入同一个稿本的
//! 有界队列。模型网络 I/O 在独立任务中进行，因此等待许可或流式响应不会阻塞该
//! 稿本处理后续命令。

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use futures_util::future::BoxFuture;
use nekowriter_agent::{
    AgentToolDefinition, CompletedStageContext, ContextBudget, ContextBuildRequest, ContextBuilder,
    ContextFoldStage, ContextMessageRole, CurrentContextInput, PromptKey, RenderedBuiltinPrompt,
    UnfoldedStageMessage,
    provider::{
        ChatCompletionsAdapter, ChatCompletionsConfig, ModelCapabilities, ModelCompletion,
        ModelFailureCode, ModelRequest, ModelRequestMessage, ModelTimeouts, ModelToolCall,
        ProviderError, StructuredOutput,
    },
    runner::{
        AgentRoundOutcome, PhaseSubmission, PhaseSubmissionKind, ToolHost, ToolHostError,
        ToolResult, classify_model_completion,
    },
};
use nekowriter_core::{
    ChapterId, ChapterKind, ChapterPlan, ChapterSummary, ExecutionStatus, FeedbackStatus,
    PreparationArtifactKind, ReviewId, RunId, ThinkingLevel, VersionId, WorkflowSnapshot,
    WorkflowStage,
};
use nekowriter_store::{
    AgentMessageRole, AgentQuestionToolCall, AppliedAgentQuestionToolCall,
    AppliedChapterBodyToolCall, ChapterBodyAgentToolCall, CompletedRunContextFold,
    CompletedRunContextStage, DelegatedReviewKind, MaterializedChapterBodyFeedback,
    ModelProtocolCorrection, ModelRequestResolution, NewAgentMessage, NewModelRequest,
    PersistedAgentMessage, Store, StoreError,
};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::{
    sync::{Mutex, OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    task::JoinHandle,
};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::prompts::{PromptSource, resolve_prompt_for_version};

/// 按「项目覆盖 → 系统覆盖 → 代码内建」渲染本次请求实际使用的提示词。
///
/// 返回内容与来源：来源会写进请求快照，便于事后解释某次请求到底用了哪一份提示词。
async fn render_effective_prompt(
    store: &Store,
    version_id: VersionId,
    key: PromptKey,
    variables: &BTreeMap<String, String>,
) -> Result<(RenderedBuiltinPrompt, PromptSource), SchedulerError> {
    let effective = resolve_prompt_for_version(store, version_id, key).await?;
    let source = effective.source;
    let rendered = effective
        .render(key, variables)
        .map_err(|error| SchedulerError::Context(error.to_string()))?;
    Ok((rendered, source))
}

use crate::credentials;

const LANE_CHANNEL_CAPACITY: usize = 32;

/// 内存调度层的可恢复错误。所有有价值的工作流状态仍由 Store 的事务维护。
#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Credential(#[from] credentials::CredentialStoreError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("当前稿本没有可用的系统写作模型设置")]
    MissingWritingModel,
    #[error("当前写作模型不支持此阶段所需的结构化提交")]
    StructuredOutputRequired,
    #[error("项目设置的思考等级需要模型支持思考参数，但当前模型未标记支持思考")]
    ThinkingUnsupported,
    #[error("准备阶段缺少小说目标")]
    MissingNovelBrief,
    #[error("模型上下文构建失败：{0}")]
    Context(String),
    #[error("模型请求快照无法序列化：{0}")]
    Serialization(#[from] serde_json::Error),
    #[error("模型上下文包含当前阶段不支持的消息类型")]
    UnsupportedContextMessage,
    #[error("模型上下文 Token 上限超出当前平台范围")]
    ContextLimitOutOfRange,
    #[error("稿本 {version_id} 的命令循环不可用")]
    LaneUnavailable { version_id: VersionId },
}

/// 每个活跃稿本一条有界通道；所有真实模型请求复用同一个全局许可。
#[derive(Clone)]
pub struct WorkflowScheduler {
    store: Store,
    model_request_permit: Arc<Semaphore>,
    settings_snapshot_lock: Arc<Mutex<()>>,
    lanes: Arc<Mutex<HashMap<VersionId, mpsc::Sender<LaneCommand>>>>,
    in_flight_requests: Arc<Mutex<HashMap<Uuid, InFlightModelRequest>>>,
}

enum LaneCommand {
    Start {
        task: ScheduledRun,
    },
    PermitGranted {
        task: ScheduledRun,
        permit: OwnedSemaphorePermit,
    },
    ModelCompleted {
        task: ScheduledRun,
        request_id: Uuid,
        request: ModelRequest,
        result: Result<ModelCompletion, ProviderError>,
    },
    RunStopped {
        run_id: RunId,
    },
}

/// 一个已经发往模型适配器、但仍可被用户停止命令中止的 HTTP 任务。
///
/// SQLite 中的 request 先被标记为 cancelled；这个句柄只负责尽快释放网络许可与
/// 本地任务资源。两层都需要：句柄不能替代持久化的迟到回调防线，反之亦然。
struct InFlightModelRequest {
    task_handle: JoinHandle<()>,
}

/// 一条稿本队列中可运行的顶层写作任务。
///
/// 任务类型和章节 ID 一起随许可与回调传递，避免回调只凭一个 Run ID 猜测应该
/// 采用哪一种提示词或结构化提交合同。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ScheduledRun {
    run_id: RunId,
    stage: ScheduledStage,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ScheduledStage {
    Preparation,
    ChapterOutline { chapter_id: ChapterId },
    ChapterBody { chapter_id: ChapterId },
    Review { review_id: ReviewId },
}

impl ScheduledRun {
    const fn kind(self) -> &'static str {
        match self.stage {
            ScheduledStage::Preparation => "preparation",
            ScheduledStage::ChapterOutline { .. } => "chapter_outline",
            ScheduledStage::ChapterBody { .. } => "chapter_body",
            ScheduledStage::Review { .. } => "review",
        }
    }
}

/// 正文阶段唯一允许触碰持久化正文的 Agent 工具边界。
///
/// Store 将模型请求终态、调用审计、assistant/tool 配对消息、正文修订和下一轮
/// `queued` 状态放在一笔事务中；这里仅把模型协议的调用形状适配为 Store 的中立
/// 类型，并把结果序列化为下一次 Chat Completions 请求可见的 `tool` 消息。
struct ChapterBodyToolHost {
    store: Store,
    chapter_id: ChapterId,
    request_id: Uuid,
    provider_request_id: Option<String>,
    usage: Value,
    execution: Mutex<Option<AppliedChapterBodyToolCall>>,
}

impl ChapterBodyToolHost {
    async fn take_execution(&self) -> Option<AppliedChapterBodyToolCall> {
        self.execution.lock().await.take()
    }
}

impl ToolHost for ChapterBodyToolHost {
    fn execute_tool<'a>(
        &'a self,
        call: &'a ModelToolCall,
    ) -> BoxFuture<'a, Result<ToolResult, ToolHostError>> {
        Box::pin(async move {
            let execution = self
                .store
                .execute_chapter_body_agent_tool_call(
                    self.request_id,
                    self.chapter_id,
                    self.provider_request_id.clone(),
                    self.usage.clone(),
                    ChapterBodyAgentToolCall {
                        provider_call_id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    },
                )
                .await
                .map_err(|error| ToolHostError {
                    code: "TOOL_EXECUTION_FAILED".to_owned(),
                    message: error.to_string(),
                })?;
            let content =
                serde_json::to_string(&execution.result).map_err(|error| ToolHostError {
                    code: "TOOL_RESULT_SERIALIZATION_FAILED".to_owned(),
                    message: error.to_string(),
                })?;
            *self.execution.lock().await = Some(execution);
            Ok(ToolResult {
                tool_call_id: call.id.clone(),
                content,
            })
        })
    }
}

impl WorkflowScheduler {
    #[must_use]
    pub fn new(
        store: Store,
        model_request_permit: Arc<Semaphore>,
        settings_snapshot_lock: Arc<Mutex<()>>,
    ) -> Self {
        Self {
            store,
            model_request_permit,
            settings_snapshot_lock,
            lanes: Arc::new(Mutex::new(HashMap::new())),
            in_flight_requests: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 终止某 Run 的尚未发送或正在发送的本地调度工作。
    ///
    /// 调用方必须先在 Store 中原子地将 Run 和 request attempt 标记为 stopped /
    /// cancelled，再调用本方法；这样即使这里的任务恰好已完成或应用进程崩溃，迟到
    /// 回调也会被持久化状态拒绝。`RunStopped` 同时清除等待全局许可的任务。
    pub async fn stop_run(
        &self,
        version_id: VersionId,
        run_id: RunId,
        cancelled_request_ids: &[Uuid],
    ) {
        let task_handles = {
            let mut in_flight = self.in_flight_requests.lock().await;
            cancelled_request_ids
                .iter()
                .filter_map(|request_id| in_flight.remove(request_id))
                .map(|request| request.task_handle)
                .collect::<Vec<_>>()
        };
        for task_handle in task_handles {
            task_handle.abort();
        }
        let sender = self.lane_sender(version_id).await;
        if sender
            .send(LaneCommand::RunStopped { run_id })
            .await
            .is_err()
        {
            warn!(%version_id, %run_id, "停止命令未能投递到已关闭的稿本命令循环");
        }
    }

    /// 将准备阶段执行入口投递给该稿本的串行循环。
    ///
    /// 相同 Run 的重复投递会在循环内合并，不会排队两个模型请求或重复消耗额度。
    pub async fn enqueue_preparation(
        &self,
        version_id: VersionId,
        run_id: RunId,
    ) -> Result<(), SchedulerError> {
        self.enqueue(
            version_id,
            ScheduledRun {
                run_id,
                stage: ScheduledStage::Preparation,
            },
        )
        .await
    }

    /// 将当前章节的细纲生成任务投递给稿本命令循环。
    pub async fn enqueue_chapter_outline(
        &self,
        version_id: VersionId,
        chapter_id: ChapterId,
        run_id: RunId,
    ) -> Result<(), SchedulerError> {
        self.enqueue(
            version_id,
            ScheduledRun {
                run_id,
                stage: ScheduledStage::ChapterOutline { chapter_id },
            },
        )
        .await
    }

    /// 将当前章节的正文 Agent 任务投递给稿本命令循环。
    pub async fn enqueue_chapter_body(
        &self,
        version_id: VersionId,
        chapter_id: ChapterId,
        run_id: RunId,
    ) -> Result<(), SchedulerError> {
        self.enqueue(
            version_id,
            ScheduledRun {
                run_id,
                stage: ScheduledStage::ChapterBody { chapter_id },
            },
        )
        .await
    }

    /// 将已经创建的审核 Agent 子 Run 投递到稿本的串行循环。
    pub async fn enqueue_review(
        &self,
        version_id: VersionId,
        review_id: ReviewId,
        run_id: RunId,
    ) -> Result<(), SchedulerError> {
        self.enqueue(
            version_id,
            ScheduledRun {
                run_id,
                stage: ScheduledStage::Review { review_id },
            },
        )
        .await
    }

    async fn enqueue(
        &self,
        version_id: VersionId,
        task: ScheduledRun,
    ) -> Result<(), SchedulerError> {
        let sender = self.lane_sender(version_id).await;
        sender
            .send(LaneCommand::Start { task })
            .await
            .map_err(|_| SchedulerError::LaneUnavailable { version_id })
    }

    /// 审核结论可能恢复父写作 Run，或在细纲通过后创建正文 Run。审核子 Run 本身
    /// 不应被重新排队，因此根据持久化后的工作流重新选择真正的写作入口。
    async fn enqueue_queued_parent_after_review(
        &self,
        workflow: &WorkflowSnapshot,
    ) -> Result<(), SchedulerError> {
        if workflow.execution != ExecutionStatus::Queued {
            return Ok(());
        }
        let run_id = workflow.active_run_id.ok_or_else(|| {
            SchedulerError::Context("审核结论后的排队工作流缺少活跃 Run".to_owned())
        })?;
        match workflow.stage {
            WorkflowStage::PreparingGenerating | WorkflowStage::PreparingRevising => {
                self.enqueue_preparation(workflow.version_id, run_id).await
            }
            WorkflowStage::WritingOutlineGenerating => {
                let chapter_id = workflow.active_chapter_id.ok_or_else(|| {
                    SchedulerError::Context("审核结论后的细纲任务缺少章节".to_owned())
                })?;
                self.enqueue_chapter_outline(workflow.version_id, chapter_id, run_id)
                    .await
            }
            WorkflowStage::WritingDrafting => {
                let chapter_id = workflow.active_chapter_id.ok_or_else(|| {
                    SchedulerError::Context("审核结论后的正文任务缺少章节".to_owned())
                })?;
                self.enqueue_chapter_body(workflow.version_id, chapter_id, run_id)
                    .await
            }
            _ => Ok(()),
        }
    }

    async fn lane_sender(&self, version_id: VersionId) -> mpsc::Sender<LaneCommand> {
        let mut lanes = self.lanes.lock().await;
        if let Some(sender) = lanes.get(&version_id)
            && !sender.is_closed()
        {
            return sender.clone();
        }

        let (sender, receiver) = mpsc::channel(LANE_CHANNEL_CAPACITY);
        lanes.insert(version_id, sender.clone());
        let scheduler = self.clone();
        let callback_sender = sender.clone();
        tokio::spawn(async move {
            scheduler
                .run_lane(version_id, receiver, callback_sender)
                .await;
        });
        sender
    }

    async fn run_lane(
        &self,
        version_id: VersionId,
        mut receiver: mpsc::Receiver<LaneCommand>,
        sender: mpsc::Sender<LaneCommand>,
    ) {
        let mut active_runs = HashSet::new();
        while let Some(command) = receiver.recv().await {
            match command {
                LaneCommand::Start { task } => {
                    if !active_runs.insert(task.run_id) {
                        debug!(%version_id, run_id = %task.run_id, kind = task.kind(), "已合并重复的模型调度命令");
                        continue;
                    }
                    self.wait_for_model_permit(task, sender.clone());
                }
                LaneCommand::PermitGranted { task, permit } => {
                    if !active_runs.contains(&task.run_id) {
                        drop(permit);
                        continue;
                    }
                    if let Err(error) = self
                        .begin_model_request(version_id, task, permit, sender.clone())
                        .await
                    {
                        active_runs.remove(&task.run_id);
                        if matches!(
                            &error,
                            SchedulerError::Store(StoreError::FeedbackSnapshotStale)
                        ) {
                            debug!(%version_id, run_id = %task.run_id, kind = task.kind(), "自由意见批次在封存前变化，重新构建请求上下文");
                            if sender.send(LaneCommand::Start { task }).await.is_err() {
                                warn!(%version_id, run_id = %task.run_id, kind = task.kind(), "自由意见重建请求未能投递到已关闭的命令循环");
                            }
                            continue;
                        }
                        warn!(%version_id, run_id = %task.run_id, kind = task.kind(), error = %error, "无法启动模型请求");
                        let failure = json!({
                            "kind": if matches!(
                                &error,
                                SchedulerError::Store(StoreError::RequestAllowanceExhausted { .. })
                            ) {
                                "request_limit"
                            } else {
                                "pre_dispatch"
                            },
                            "message": error.to_string(),
                        });
                        let persisted = match task.stage {
                            ScheduledStage::Review { review_id } => self
                                .store
                                .fail_delegated_review_attempt(
                                    review_id,
                                    task.run_id,
                                    None,
                                    None,
                                    failure,
                                    None,
                                    false,
                                )
                                .await
                                .map(|_| ()),
                            _ if matches!(
                                &error,
                                SchedulerError::Store(StoreError::RequestAllowanceExhausted { .. })
                            ) =>
                            {
                                self.store
                                    .stop_queued_model_run(task.run_id, failure)
                                    .await
                                    .map(|_| ())
                            }
                            _ => self
                                .store
                                .fail_queued_model_run(task.run_id, failure)
                                .await
                                .map(|_| ()),
                        };
                        if let Err(persist_error) = persisted {
                            warn!(%version_id, run_id = %task.run_id, kind = task.kind(), error = %persist_error, "无法记录模型请求启动失败");
                        }
                    }
                }
                LaneCommand::ModelCompleted {
                    task,
                    request_id,
                    request,
                    result,
                } => {
                    let requeue = self
                        .handle_model_completion(task, request_id, &request, result)
                        .await;
                    active_runs.remove(&task.run_id);
                    if requeue && sender.send(LaneCommand::Start { task }).await.is_err() {
                        warn!(%version_id, run_id = %task.run_id, kind = task.kind(), "正文工具结果已提交，但模型命令循环已关闭");
                    }
                }
                LaneCommand::RunStopped { run_id } => {
                    if active_runs.remove(&run_id) {
                        debug!(%version_id, %run_id, "已移除停止 Run 的待执行调度状态");
                    }
                }
            }
        }
    }

    fn wait_for_model_permit(&self, task: ScheduledRun, sender: mpsc::Sender<LaneCommand>) {
        let semaphore = self.model_request_permit.clone();
        tokio::spawn(async move {
            match semaphore.acquire_owned().await {
                Ok(permit) => {
                    if sender
                        .send(LaneCommand::PermitGranted { task, permit })
                        .await
                        .is_err()
                    {
                        warn!(run_id = %task.run_id, kind = task.kind(), "模型命令循环已关闭，已释放模型许可");
                    }
                }
                Err(_) => {
                    warn!(run_id = %task.run_id, kind = task.kind(), "模型调度器已关闭，无法取得模型许可")
                }
            }
        });
    }

    async fn begin_model_request(
        &self,
        version_id: VersionId,
        task: ScheduledRun,
        permit: OwnedSemaphorePermit,
        sender: mpsc::Sender<LaneCommand>,
    ) -> Result<(), SchedulerError> {
        // 许可真正到手后才读设置、提示词和上下文。排队期间的设置修改因此自然
        // 应用于本次请求，而已 dispatch 的快照永远不会被原地改写。
        // 保存设置与这里的读取/封存共用一个短锁，因此设置命令不会夹在“读取旧
        // 配置”和“将其标为 dispatched”之间；锁绝不覆盖后续网络 I/O。
        let settings_guard = self.settings_snapshot_lock.lock().await;
        let settings = self
            .store
            .get_system_model_settings()
            .await?
            .ok_or(SchedulerError::MissingWritingModel)?;
        if !settings.capabilities.structured_output {
            return Err(SchedulerError::StructuredOutputRequired);
        }
        if !settings.capabilities.tools && !matches!(task.stage, ScheduledStage::Review { .. }) {
            return Err(SchedulerError::Context(
                "当前写作模型不支持 ask_user 所需的工具调用".to_owned(),
            ));
        }
        // 项目设置也在每次取得许可后重新解析：有效模型引用优先取项目覆盖，思考
        // 等级取项目设置。这样保存设置后的下一次请求自然按新值构建，而已 dispatch
        // 的请求快照不会被原地改写。
        let project_id = self.store.project_id_for_version(version_id).await?;
        let project_settings = self.store.get_project_settings(project_id).await?;
        let effective_model_id = project_settings
            .model_id
            .clone()
            .unwrap_or_else(|| settings.model_id.clone());
        if project_settings.thinking_level != ThinkingLevel::Off && !settings.capabilities.thinking
        {
            return Err(SchedulerError::ThinkingUnsupported);
        }
        let api_key = match settings.credential_ref.clone() {
            Some(credential_ref) => Some(credentials::read_api_key(credential_ref).await?),
            None => None,
        };
        let max_context_tokens = usize::try_from(settings.max_context_tokens)
            .map_err(|_| SchedulerError::ContextLimitOutOfRange)?;
        let reserved_output_tokens = usize::try_from(settings.reserved_output_tokens)
            .map_err(|_| SchedulerError::ContextLimitOutOfRange)?;
        let (
            prompt,
            prompt_source,
            context,
            persist_initial_messages,
            feedback_binding,
            tool_result_binding,
        ) = match task.stage {
            ScheduledStage::Preparation => {
                let snapshot = self.store.get_preparation_snapshot(version_id).await?;
                if snapshot.workflow.active_run_id != Some(task.run_id) {
                    return Err(SchedulerError::Context(
                        "当前活跃 Run 已改变，拒绝发送过期准备请求".to_owned(),
                    ));
                }
                let novel_brief = snapshot
                    .novel_brief
                    .content
                    .ok_or(SchedulerError::MissingNovelBrief)?;
                let mut variables = BTreeMap::new();
                variables.insert("novel_brief".to_owned(), novel_brief);
                let (prompt, prompt_source) = render_effective_prompt(
                    &self.store,
                    version_id,
                    PromptKey::PreparationGenerate,
                    &variables,
                )
                .await?;
                let persisted_messages = self.store.list_agent_messages(task.run_id).await?;
                let persist_initial_messages = persisted_messages.is_empty();
                let tool_result_binding = self
                    .store
                    .list_unsealed_agent_tool_result_provider_ids(task.run_id)
                    .await?;
                let context = build_context_with_history(
                    prompt.system.clone(),
                    ask_user_tools(),
                    Vec::new(),
                    vec![CurrentContextInput {
                        role: ContextMessageRole::User,
                        content: prompt.user.clone(),
                        source_revision_id: None,
                    }],
                    writing_agent_history_messages(task.run_id, persisted_messages, &[])?,
                    max_context_tokens,
                    reserved_output_tokens,
                )?;
                (
                    prompt,
                    prompt_source,
                    context,
                    persist_initial_messages,
                    None,
                    Some(tool_result_binding),
                )
            }
            ScheduledStage::ChapterOutline { chapter_id } => {
                let snapshot = self.store.get_chapter_outline(chapter_id).await?;
                if snapshot.workflow.version_id != version_id
                    || snapshot.workflow.active_run_id != Some(task.run_id)
                    || snapshot.workflow.active_chapter_id != Some(chapter_id)
                    || snapshot.workflow.stage != WorkflowStage::WritingOutlineGenerating
                    || snapshot.workflow.execution != ExecutionStatus::Queued
                {
                    return Err(SchedulerError::Context(
                        "当前章节细纲任务已改变，拒绝发送过期模型请求".to_owned(),
                    ));
                }

                let chapter = self
                    .store
                    .list_chapters(version_id)
                    .await?
                    .into_iter()
                    .find(|chapter| chapter.id == chapter_id)
                    .ok_or_else(|| SchedulerError::Context("当前章节不属于目标稿本".to_owned()))?;
                let chapter_plan = match chapter.kind {
                    ChapterKind::Main => {
                        let preparation = self.store.get_preparation_snapshot(version_id).await?;
                        let approved_chapter_plan = preparation
                            .artifacts
                            .iter()
                            .find(|artifact| artifact.kind == PreparationArtifactKind::Outline)
                            .and_then(|artifact| artifact.chapter_plan.as_ref())
                            .ok_or_else(|| {
                                SchedulerError::Context("审核通过的小说粗纲缺少章节计划".to_owned())
                            })?;
                        chapter_outline_plan_context(&chapter, Some(approved_chapter_plan))?
                    }
                    ChapterKind::Extra => chapter_outline_plan_context(&chapter, None)?,
                };
                let mut variables = BTreeMap::new();
                variables.insert("chapter_plan".to_owned(), chapter_plan);
                variables.insert(
                    "chapter_guidance".to_owned(),
                    snapshot.guidance.content.unwrap_or_default(),
                );
                let (prompt, prompt_source) = render_effective_prompt(
                    &self.store,
                    version_id,
                    PromptKey::ChapterOutline,
                    &variables,
                )
                .await?;
                let completed_stages = completed_stage_contexts(
                    self.store
                        .list_completed_run_context_folds(version_id)
                        .await?,
                );
                if !completed_stages
                    .iter()
                    .any(|stage| stage.stage == ContextFoldStage::Preparation)
                {
                    return Err(SchedulerError::Context(
                        "开始章节细纲前缺少准备阶段纯文本折叠".to_owned(),
                    ));
                }
                let persisted_messages = self.store.list_agent_messages(task.run_id).await?;
                let persist_initial_messages = persisted_messages.is_empty();
                let tool_result_binding = self
                    .store
                    .list_unsealed_agent_tool_result_provider_ids(task.run_id)
                    .await?;
                let context = build_context_with_history(
                    prompt.system.clone(),
                    ask_user_tools(),
                    completed_stages,
                    vec![CurrentContextInput {
                        role: ContextMessageRole::User,
                        content: prompt.user.clone(),
                        source_revision_id: None,
                    }],
                    writing_agent_history_messages(task.run_id, persisted_messages, &[])?,
                    max_context_tokens,
                    reserved_output_tokens,
                )?;
                (
                    prompt,
                    prompt_source,
                    context,
                    persist_initial_messages,
                    None,
                    Some(tool_result_binding),
                )
            }
            ScheduledStage::ChapterBody { chapter_id } => {
                let snapshot = self.store.get_chapter_body(chapter_id).await?;
                if snapshot.workflow.version_id != version_id
                    || snapshot.workflow.active_run_id != Some(task.run_id)
                    || snapshot.workflow.active_chapter_id != Some(chapter_id)
                    || snapshot.workflow.stage != WorkflowStage::WritingDrafting
                    || snapshot.workflow.execution != ExecutionStatus::Queued
                {
                    return Err(SchedulerError::Context(
                        "当前章节正文任务已改变，拒绝发送过期模型请求".to_owned(),
                    ));
                }
                let outline_snapshot = self.store.get_chapter_outline(chapter_id).await?;
                let outline = outline_snapshot.outline.content.ok_or_else(|| {
                    SchedulerError::Context("正文开始前缺少已审核章节细纲".to_owned())
                })?;
                let outline_fold = snapshot.outline_context_fold.ok_or_else(|| {
                    SchedulerError::Context("正文开始前缺少章节细纲纯文本折叠".to_owned())
                })?;
                if outline_fold.plain_text != outline {
                    return Err(SchedulerError::Context(
                        "当前章节细纲折叠与正文输入不一致".to_owned(),
                    ));
                }
                let chapter = self
                    .store
                    .list_chapters(version_id)
                    .await?
                    .into_iter()
                    .find(|chapter| chapter.id == chapter_id)
                    .ok_or_else(|| SchedulerError::Context("当前章节不属于目标稿本".to_owned()))?;
                let completed_stages = completed_stage_contexts(
                    self.store
                        .list_completed_run_context_folds(version_id)
                        .await?,
                );
                if !completed_stages
                    .iter()
                    .any(|stage| stage.stage == ContextFoldStage::Preparation)
                    || !completed_stages.iter().any(|stage| {
                        stage.stage == ContextFoldStage::ChapterOutline
                            && stage.chapter_order == Some(chapter.order_index)
                    })
                {
                    return Err(SchedulerError::Context(
                        "正文开始前缺少已完成阶段的必要纯文本折叠".to_owned(),
                    ));
                }
                let persisted_messages = self.store.list_agent_messages(task.run_id).await?;
                let persist_initial_messages = persisted_messages.is_empty();
                let materialized_feedback = self
                    .store
                    .list_materialized_chapter_body_feedback(task.run_id)
                    .await?;
                let feedback_binding = materialized_feedback
                    .iter()
                    .filter(|feedback| feedback.status == FeedbackStatus::Bound)
                    .map(|feedback| feedback.id)
                    .collect::<Vec<_>>();
                let tool_result_binding = self
                    .store
                    .list_unsealed_agent_tool_result_provider_ids(task.run_id)
                    .await?;
                let current_messages = writing_agent_history_messages(
                    task.run_id,
                    persisted_messages,
                    &materialized_feedback,
                )?;
                let mut variables = BTreeMap::new();
                variables.insert(
                    "chapter_content".to_owned(),
                    snapshot.body.content.unwrap_or_default(),
                );
                variables.insert(
                    "chapter_guidance".to_owned(),
                    outline_snapshot.guidance.content.unwrap_or_default(),
                );
                let (prompt, prompt_source) = render_effective_prompt(
                    &self.store,
                    version_id,
                    PromptKey::ChapterWrite,
                    &variables,
                )
                .await?;
                let context = build_context_with_history(
                    prompt.system.clone(),
                    chapter_body_tools(),
                    completed_stages,
                    vec![CurrentContextInput {
                        role: ContextMessageRole::User,
                        content: prompt.user.clone(),
                        source_revision_id: None,
                    }],
                    current_messages,
                    max_context_tokens,
                    reserved_output_tokens,
                )?;
                (
                    prompt,
                    prompt_source,
                    context,
                    persist_initial_messages,
                    Some(feedback_binding),
                    Some(tool_result_binding),
                )
            }
            ScheduledStage::Review { review_id } => {
                let review = self
                    .store
                    .delegated_review_task(review_id, task.run_id)
                    .await?;
                if review.version_id != version_id {
                    return Err(SchedulerError::Context(
                        "审核 Agent 子 Run 不属于目标稿本".to_owned(),
                    ));
                }

                let mut variables = BTreeMap::new();
                let prompt_key = match review.kind {
                    DelegatedReviewKind::Preparation(PreparationArtifactKind::Background) => {
                        variables.insert("novel_brief".to_owned(), review.novel_brief.clone());
                        variables
                            .insert("target_content".to_owned(), review.target_content.clone());
                        PromptKey::ReviewPreparationBackground
                    }
                    DelegatedReviewKind::Preparation(PreparationArtifactKind::Research) => {
                        variables.insert("novel_brief".to_owned(), review.novel_brief.clone());
                        variables
                            .insert("target_content".to_owned(), review.target_content.clone());
                        PromptKey::ReviewPreparationResearch
                    }
                    DelegatedReviewKind::Preparation(PreparationArtifactKind::Characters) => {
                        variables.insert(
                            "approved_context".to_owned(),
                            review.approved_context.clone(),
                        );
                        variables
                            .insert("target_content".to_owned(), review.target_content.clone());
                        PromptKey::ReviewPreparationCharacters
                    }
                    DelegatedReviewKind::Preparation(PreparationArtifactKind::Outline) => {
                        variables.insert(
                            "approved_context".to_owned(),
                            review.approved_context.clone(),
                        );
                        variables
                            .insert("target_content".to_owned(), review.target_content.clone());
                        PromptKey::ReviewPreparationOutline
                    }
                    DelegatedReviewKind::ChapterOutline => {
                        variables
                            .insert("chapter_context".to_owned(), review.chapter_context.clone());
                        variables
                            .insert("target_content".to_owned(), review.target_content.clone());
                        PromptKey::ReviewChapterOutline
                    }
                    DelegatedReviewKind::ChapterEdit => {
                        variables.insert("base_content".to_owned(), review.base_content.clone());
                        variables
                            .insert("target_content".to_owned(), review.target_content.clone());
                        PromptKey::ReviewChapterEdit
                    }
                    DelegatedReviewKind::ChapterFinal => {
                        variables
                            .insert("chapter_outline".to_owned(), review.chapter_outline.clone());
                        variables
                            .insert("target_content".to_owned(), review.target_content.clone());
                        PromptKey::ReviewChapterFinal
                    }
                };
                let (prompt, prompt_source) =
                    render_effective_prompt(&self.store, version_id, prompt_key, &variables)
                        .await?;
                let user_content = if review.user_notes.trim().is_empty() {
                    prompt.user.clone()
                } else {
                    format!("{}\n\n用户审核关注点：{}", prompt.user, review.user_notes)
                };
                let context = build_context_with_history(
                    prompt.system.clone(),
                    Vec::new(),
                    Vec::new(),
                    vec![CurrentContextInput {
                        role: ContextMessageRole::User,
                        content: user_content,
                        source_revision_id: None,
                    }],
                    Vec::new(),
                    max_context_tokens,
                    reserved_output_tokens,
                )?;
                (prompt, prompt_source, context, true, None, None)
            }
        };
        let request = context.model_request(
            Some(StructuredOutput::JsonObject),
            project_settings.thinking_level,
        );
        let messages = if persist_initial_messages {
            agent_messages_for_new_stage_request(&request)?
        } else {
            Vec::new()
        };
        let provider = ChatCompletionsAdapter::new(ChatCompletionsConfig::new(
            &settings.api_base_url,
            effective_model_id.clone(),
            api_key,
            ModelCapabilities {
                streaming: settings.capabilities.streaming,
                tools: settings.capabilities.tools,
                structured_output: settings.capabilities.structured_output,
                stream_usage: settings.capabilities.stream_usage,
                thinking: settings.capabilities.thinking,
            },
            ModelTimeouts::default(),
        )?)?;
        let mut context_snapshot = json!({
            "stage": task.kind(),
            "prompt": {
                "key": prompt.key.as_str(),
                "source": prompt_source.as_str(),
            },
            "build": context,
            "request": &request,
        });
        if let Some(feedback_binding) = feedback_binding {
            context_snapshot["feedbackBinding"] = json!({ "ids": feedback_binding });
        }
        if let Some(tool_result_binding) = tool_result_binding {
            context_snapshot["toolResultBinding"] = json!({
                "providerCallIds": tool_result_binding,
            });
        }
        let request_checkpoint = self
            .store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: task.run_id,
                    settings_snapshot: json!({
                        "key": "system.writing_model",
                        "revision": settings.revision.to_string(),
                        "apiBaseUrl": settings.api_base_url,
                        "modelId": settings.model_id,
                        "capabilities": settings.capabilities,
                        "maxContextTokens": settings.max_context_tokens.to_string(),
                        "reservedOutputTokens": settings.reserved_output_tokens.to_string(),
                        "project": {
                            "id": project_id.to_string(),
                            "revision": project_settings.revision.to_string(),
                            "modelOverrideId": project_settings.model_id,
                            "thinkingLevel": project_settings.thinking_level.as_str(),
                            "auxiliaryToolBudget": project_settings.auxiliary_tool_budget.to_string(),
                        },
                        "effectiveModelId": effective_model_id,
                        "effectiveThinkingLevel": project_settings.thinking_level.as_str(),
                    }),
                    prompt_snapshot: serde_json::to_value(&prompt)?,
                    context_snapshot,
                },
                messages,
            )
            .await?;
        let request_id = request_checkpoint.id;
        drop(settings_guard);
        // 在网络 future 真正启动前先把可取消句柄注册到内存表。否则一个极快的 stop
        // 命令可能只作废数据库 request，却错过仍在等待/执行的 HTTP 任务。
        let (start_sender, start_receiver) = oneshot::channel();
        let in_flight_requests = self.in_flight_requests.clone();
        let callback_sender = sender.clone();
        let task_handle = tokio::spawn(async move {
            if start_receiver.await.is_err() {
                drop(permit);
                return;
            }
            let result = provider.execute_with(&request, |_| {}).await;
            if callback_sender
                .send(LaneCommand::ModelCompleted {
                    task,
                    request_id,
                    request,
                    result,
                })
                .await
                .is_err()
            {
                warn!(run_id = %task.run_id, kind = task.kind(), %request_id, "模型完成回调未能投递");
            }
            in_flight_requests.lock().await.remove(&request_id);
            // `permit` 在网络调用与回调投递完成后释放；后续数据库提交不会长期占用
            // 全局模型网络许可。
            drop(permit);
        });
        self.in_flight_requests
            .lock()
            .await
            .insert(request_id, InFlightModelRequest { task_handle });
        let _ = start_sender.send(());
        Ok(())
    }

    async fn handle_model_completion(
        &self,
        task: ScheduledRun,
        request_id: Uuid,
        request: &ModelRequest,
        result: Result<ModelCompletion, ProviderError>,
    ) -> bool {
        match task.stage {
            ScheduledStage::Preparation => {
                self.handle_preparation_completion(request_id, request, result)
                    .await
            }
            ScheduledStage::ChapterOutline { chapter_id } => {
                self.handle_chapter_outline_completion(chapter_id, request_id, request, result)
                    .await
            }
            ScheduledStage::ChapterBody { chapter_id } => {
                self.handle_chapter_body_completion(chapter_id, request_id, request, result)
                    .await
            }
            ScheduledStage::Review { review_id } => {
                self.handle_review_completion(review_id, task.run_id, request_id, request, result)
                    .await
            }
        }
    }

    async fn handle_preparation_completion(
        &self,
        request_id: Uuid,
        request: &ModelRequest,
        result: Result<ModelCompletion, ProviderError>,
    ) -> bool {
        match result {
            Ok(completion) => {
                let provider_request_id = completion.provider_request_id.clone();
                let assistant_message = assistant_message_from_completion(&completion, request_id);
                let raw_response = completion.content.clone().unwrap_or_default();
                let usage = match completion
                    .usage
                    .as_ref()
                    .map(serde_json::to_value)
                    .transpose()
                {
                    Ok(Some(usage)) => usage,
                    Ok(None) => json!({ "status": "unknown" }),
                    Err(error) => json!({
                        "status": "invalid",
                        "reason": error.to_string(),
                    }),
                };

                match classify_model_completion(
                    PhaseSubmissionKind::Preparation,
                    request,
                    completion,
                ) {
                    Ok(AgentRoundOutcome::Submission(PhaseSubmission::Preparation(submission))) => {
                        match self
                            .store
                            .commit_preparation_model_submission(
                                request_id,
                                provider_request_id.clone(),
                                usage,
                                raw_response,
                                submission.artifacts,
                                submission.chapter_plan,
                            )
                            .await
                        {
                            Ok(_) => {
                                info!(%request_id, "准备阶段模型产物已提交，等待审核");
                                false
                            }
                            Err(error) => {
                                self.persist_model_failure(
                                    request_id,
                                    provider_request_id,
                                    Some(assistant_message),
                                    "submission_commit",
                                    error.to_string(),
                                    false,
                                )
                                .await;
                                false
                            }
                        }
                    }
                    Ok(AgentRoundOutcome::ToolCall(call)) if call.name == "ask_user" => {
                        self.execute_agent_question_tool_call(
                            request_id,
                            provider_request_id,
                            usage,
                            call,
                            assistant_message,
                        )
                        .await
                    }
                    Ok(outcome) => {
                        self.persist_model_failure(
                            request_id,
                            provider_request_id,
                            Some(assistant_message),
                            "unexpected_outcome",
                            format!("准备阶段不接受模型结果：{outcome:?}"),
                            false,
                        )
                        .await;
                        false
                    }
                    Err(error) => {
                        self.record_model_protocol_correction(
                            request_id,
                            provider_request_id,
                            usage,
                            assistant_message,
                            error.to_string(),
                        )
                        .await
                    }
                }
            }
            Err(error) => {
                let unknown = matches!(
                    error.failure_code(),
                    ModelFailureCode::Connection | ModelFailureCode::Timeout
                );
                self.persist_model_failure(
                    request_id,
                    None,
                    None,
                    "provider_error",
                    error.to_string(),
                    unknown,
                )
                .await;
                false
            }
        }
    }

    async fn handle_chapter_outline_completion(
        &self,
        chapter_id: ChapterId,
        request_id: Uuid,
        request: &ModelRequest,
        result: Result<ModelCompletion, ProviderError>,
    ) -> bool {
        match result {
            Ok(completion) => {
                let provider_request_id = completion.provider_request_id.clone();
                let assistant_message = assistant_message_from_completion(&completion, request_id);
                let raw_response = completion.content.clone().unwrap_or_default();
                let usage = completion_usage(&completion);

                match classify_model_completion(
                    PhaseSubmissionKind::ChapterOutline { chapter_id },
                    request,
                    completion,
                ) {
                    Ok(AgentRoundOutcome::Submission(PhaseSubmission::ChapterOutline(
                        submission,
                    ))) => {
                        match self
                            .store
                            .commit_chapter_outline_model_submission(
                                request_id,
                                chapter_id,
                                provider_request_id.clone(),
                                usage,
                                raw_response,
                                submission.content,
                            )
                            .await
                        {
                            Ok(_) => {
                                info!(%request_id, %chapter_id, "章节细纲模型产物已提交，等待审核");
                                false
                            }
                            Err(error) => {
                                self.persist_model_failure(
                                    request_id,
                                    provider_request_id,
                                    Some(assistant_message),
                                    "submission_commit",
                                    error.to_string(),
                                    false,
                                )
                                .await;
                                false
                            }
                        }
                    }
                    Ok(AgentRoundOutcome::ToolCall(call)) if call.name == "ask_user" => {
                        self.execute_agent_question_tool_call(
                            request_id,
                            provider_request_id,
                            usage,
                            call,
                            assistant_message,
                        )
                        .await
                    }
                    Ok(outcome) => {
                        self.persist_model_failure(
                            request_id,
                            provider_request_id,
                            Some(assistant_message),
                            "unexpected_outcome",
                            format!("章节细纲不接受模型结果：{outcome:?}"),
                            false,
                        )
                        .await;
                        false
                    }
                    Err(error) => {
                        self.record_model_protocol_correction(
                            request_id,
                            provider_request_id,
                            usage,
                            assistant_message,
                            error.to_string(),
                        )
                        .await
                    }
                }
            }
            Err(error) => {
                let unknown = matches!(
                    error.failure_code(),
                    ModelFailureCode::Connection | ModelFailureCode::Timeout
                );
                self.persist_model_failure(
                    request_id,
                    None,
                    None,
                    "provider_error",
                    error.to_string(),
                    unknown,
                )
                .await;
                false
            }
        }
    }

    /// 将所有写作阶段共用的 `ask_user` 调用交给 Store 的问题事务。
    ///
    /// 合法调用停在 `waiting_answer`；无效参数已经由 Store 形成稳定工具错误并重新
    /// 排队，因此只有后者返回 `true` 以安排下一轮模型请求。
    async fn execute_agent_question_tool_call(
        &self,
        request_id: Uuid,
        provider_request_id: Option<String>,
        usage: Value,
        call: ModelToolCall,
        assistant_message: NewAgentMessage,
    ) -> bool {
        let provider_call_id = call.id.clone();
        match self
            .store
            .execute_agent_question_tool_call(
                request_id,
                provider_request_id.clone(),
                usage,
                AgentQuestionToolCall {
                    provider_call_id: provider_call_id.clone(),
                    arguments: call.arguments,
                },
            )
            .await
        {
            Ok(AppliedAgentQuestionToolCall::AwaitingAnswer {
                question,
                newly_created,
                ..
            }) => {
                if newly_created {
                    info!(%request_id, question_id = %question.id, "Agent 正在等待用户回答问题");
                }
                false
            }
            Ok(AppliedAgentQuestionToolCall::Resolved {
                newly_resolved,
                result,
                ..
            }) => {
                if newly_resolved {
                    info!(%request_id, tool_call_id = %provider_call_id, status = %result["status"], "ask_user 参数无效，已作为工具结果回送");
                }
                newly_resolved
            }
            Err(error) => {
                self.persist_model_failure(
                    request_id,
                    provider_request_id,
                    Some(assistant_message),
                    "question_tool",
                    error.to_string(),
                    false,
                )
                .await;
                false
            }
        }
    }

    /// 正文一轮完成后的分支：工具调用在事务中应用并重新排队；结构化提交只负责
    /// 打开整章终稿审核，绝不携带或覆盖正文。
    async fn handle_chapter_body_completion(
        &self,
        chapter_id: ChapterId,
        request_id: Uuid,
        request: &ModelRequest,
        result: Result<ModelCompletion, ProviderError>,
    ) -> bool {
        match result {
            Ok(completion) => {
                let provider_request_id = completion.provider_request_id.clone();
                let assistant_message = assistant_message_from_completion(&completion, request_id);
                let raw_response = completion.content.clone().unwrap_or_default();
                let usage = completion_usage(&completion);
                match classify_model_completion(
                    PhaseSubmissionKind::Chapter { chapter_id },
                    request,
                    completion,
                ) {
                    Ok(AgentRoundOutcome::ToolCall(call)) if call.name == "ask_user" => {
                        self.execute_agent_question_tool_call(
                            request_id,
                            provider_request_id,
                            usage,
                            call,
                            assistant_message,
                        )
                        .await
                    }
                    Ok(AgentRoundOutcome::ToolCall(call)) => {
                        let tool_host = ChapterBodyToolHost {
                            store: self.store.clone(),
                            chapter_id,
                            request_id,
                            provider_request_id: provider_request_id.clone(),
                            usage,
                            execution: Mutex::new(None),
                        };
                        match tool_host.execute_tool(&call).await {
                            Ok(tool_result) => {
                                let Some(execution) = tool_host.take_execution().await else {
                                    self.persist_model_failure(
                                        request_id,
                                        provider_request_id,
                                        Some(assistant_message),
                                        "tool_host",
                                        "正文 ToolHost 没有返回持久化执行结果".to_owned(),
                                        false,
                                    )
                                    .await;
                                    return false;
                                };
                                info!(
                                    %request_id,
                                    %chapter_id,
                                    tool_call_id = %tool_result.tool_call_id,
                                    status = %execution.result["status"],
                                    "章节正文工具调用已处理"
                                );
                                execution.newly_resolved
                            }
                            Err(error) => {
                                self.persist_model_failure(
                                    request_id,
                                    provider_request_id,
                                    Some(assistant_message),
                                    "tool_execution",
                                    error.to_string(),
                                    false,
                                )
                                .await;
                                false
                            }
                        }
                    }
                    Ok(AgentRoundOutcome::Submission(PhaseSubmission::Chapter(submission))) => {
                        match self
                            .store
                            .commit_chapter_body_model_submission(
                                request_id,
                                chapter_id,
                                provider_request_id.clone(),
                                usage,
                                raw_response,
                                submission.summary,
                            )
                            .await
                        {
                            Ok(snapshot) => {
                                if snapshot.workflow.execution == ExecutionStatus::Queued {
                                    info!(%request_id, %chapter_id, "章节提交因未投递意见返回正文工具循环");
                                    return true;
                                }
                                info!(%request_id, %chapter_id, "章节正文模型提交已进入整章审核");
                            }
                            Err(error) => {
                                self.persist_model_failure(
                                    request_id,
                                    provider_request_id,
                                    Some(assistant_message),
                                    "submission_commit",
                                    error.to_string(),
                                    false,
                                )
                                .await;
                            }
                        }
                        false
                    }
                    Ok(outcome) => {
                        self.persist_model_failure(
                            request_id,
                            provider_request_id,
                            Some(assistant_message),
                            "unexpected_outcome",
                            format!("章节正文不接受模型结果：{outcome:?}"),
                            false,
                        )
                        .await;
                        false
                    }
                    Err(error) => {
                        self.record_model_protocol_correction(
                            request_id,
                            provider_request_id,
                            usage,
                            assistant_message,
                            error.to_string(),
                        )
                        .await
                    }
                }
            }
            Err(error) => {
                let unknown = matches!(
                    error.failure_code(),
                    ModelFailureCode::Connection | ModelFailureCode::Timeout
                );
                self.persist_model_failure(
                    request_id,
                    None,
                    None,
                    "provider_error",
                    error.to_string(),
                    unknown,
                )
                .await;
                false
            }
        }
    }

    /// 审核子 Run 只接受一次无工具的结构化结论。无论模型或提交失败，父 Run 都
    /// 保持在等待审核状态，并把同一 Review 重新打开供用户重试或人工决定。
    async fn handle_review_completion(
        &self,
        review_id: ReviewId,
        run_id: RunId,
        request_id: Uuid,
        request: &ModelRequest,
        result: Result<ModelCompletion, ProviderError>,
    ) -> bool {
        match result {
            Ok(completion) => {
                let provider_request_id = completion.provider_request_id.clone();
                let assistant_message = assistant_message_from_completion(&completion, request_id);
                let raw_response = completion.content.clone().unwrap_or_default();
                let usage = completion_usage(&completion);
                match classify_model_completion(PhaseSubmissionKind::Review, request, completion) {
                    Ok(AgentRoundOutcome::Submission(PhaseSubmission::Review(submission))) => {
                        match self
                            .store
                            .commit_delegated_review_result(
                                review_id,
                                run_id,
                                request_id,
                                provider_request_id.clone(),
                                usage,
                                raw_response,
                                submission.passed,
                                submission.confidence,
                                submission.issues,
                            )
                            .await
                        {
                            Ok(snapshot) => {
                                info!(%review_id, %run_id, %request_id, "审核 Agent 结论已应用");
                                if let Err(error) = self
                                    .enqueue_queued_parent_after_review(&snapshot.workflow)
                                    .await
                                {
                                    warn!(
                                        %review_id,
                                        %run_id,
                                        error = %error,
                                        "审核结论已应用，但未能投递后续写作 Run"
                                    );
                                }
                            }
                            Err(error) => {
                                self.persist_delegated_review_failure(
                                    review_id,
                                    run_id,
                                    request_id,
                                    provider_request_id,
                                    Some(assistant_message),
                                    "submission_commit",
                                    error.to_string(),
                                    false,
                                )
                                .await;
                            }
                        }
                    }
                    Ok(outcome) => {
                        self.persist_delegated_review_failure(
                            review_id,
                            run_id,
                            request_id,
                            provider_request_id,
                            Some(assistant_message),
                            "unexpected_outcome",
                            format!("审核 Agent 不接受模型结果：{outcome:?}"),
                            false,
                        )
                        .await;
                    }
                    Err(error) => {
                        self.persist_delegated_review_failure(
                            review_id,
                            run_id,
                            request_id,
                            provider_request_id,
                            Some(assistant_message),
                            "protocol_error",
                            error.to_string(),
                            false,
                        )
                        .await;
                    }
                }
            }
            Err(error) => {
                let unknown = matches!(
                    error.failure_code(),
                    ModelFailureCode::Connection | ModelFailureCode::Timeout
                );
                self.persist_delegated_review_failure(
                    review_id,
                    run_id,
                    request_id,
                    None,
                    None,
                    "provider_error",
                    error.to_string(),
                    unknown,
                )
                .await;
            }
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    async fn persist_delegated_review_failure(
        &self,
        review_id: ReviewId,
        run_id: RunId,
        request_id: Uuid,
        provider_request_id: Option<String>,
        assistant_message: Option<NewAgentMessage>,
        kind: &'static str,
        message: String,
        unknown: bool,
    ) {
        let failure = json!({ "kind": kind, "message": message });
        match self
            .store
            .fail_delegated_review_attempt(
                review_id,
                run_id,
                Some(request_id),
                provider_request_id,
                failure,
                assistant_message,
                unknown,
            )
            .await
        {
            Ok(true) => {
                warn!(%review_id, %run_id, %request_id, "审核 Agent 失败，审核已重新开放");
            }
            Ok(false) => {
                debug!(%review_id, %run_id, %request_id, "忽略已取消或过期的审核 Agent 回调");
            }
            Err(error) => {
                warn!(%review_id, %run_id, %request_id, error = %error, "未能收束审核 Agent 失败");
            }
        }
    }

    /// 保存模型已经完整返回、但不满足本阶段协议的响应，并在工程上限内要求模型纠正。
    ///
    /// 这与网络或提供商失败不同：完整原始响应、任意 tool-call 的配对错误结果以及
    /// 纠正提示都会进入同一 Run 的消息账本，因此下一次请求可以在不破坏调用协议的
    /// 前提下继续。Store 负责连续计数和第三次错误后的 stopped 转换。
    async fn record_model_protocol_correction(
        &self,
        request_id: Uuid,
        provider_request_id: Option<String>,
        usage: Value,
        assistant_message: NewAgentMessage,
        message: String,
    ) -> bool {
        const CORRECTION: &str = "上一轮响应不符合当前阶段的结构化协议，未应用任何阶段提交或工具副作用。请重新严格按当前阶段要求返回：要么恰好一个已声明的工具调用，要么完整合法的 JSON 阶段提交；工具调用不要同时附带正文文本。";
        match self
            .store
            .record_model_protocol_correction(
                request_id,
                ModelProtocolCorrection {
                    provider_request_id,
                    usage,
                    assistant_message,
                    error: json!({ "kind": "protocol_error", "message": message }),
                    correction: CORRECTION.to_owned(),
                },
            )
            .await
        {
            Ok(result) => {
                if result.newly_applied {
                    if result.requeue {
                        info!(
                            %request_id,
                            consecutive_protocol_errors = result.consecutive_protocol_errors,
                            "模型协议错误已保存，已要求同一 Run 纠正"
                        );
                    } else {
                        warn!(
                            %request_id,
                            consecutive_protocol_errors = result.consecutive_protocol_errors,
                            "连续模型协议错误达到上限，Run 已停止等待用户显式继续"
                        );
                    }
                }
                result.requeue
            }
            Err(error) => {
                warn!(%request_id, error = %error, "未能写入模型协议纠正结果");
                false
            }
        }
    }

    async fn persist_model_failure(
        &self,
        request_id: Uuid,
        provider_request_id: Option<String>,
        assistant_message: Option<NewAgentMessage>,
        kind: &'static str,
        message: String,
        unknown: bool,
    ) {
        let error = json!({ "kind": kind, "message": message });
        let resolution = if unknown {
            ModelRequestResolution::Unknown {
                provider_request_id,
                error,
            }
        } else {
            ModelRequestResolution::Failed {
                provider_request_id,
                error,
            }
        };
        let result = match assistant_message {
            Some(assistant_message) => {
                self.store
                    .resolve_model_request_with_assistant_message(
                        request_id,
                        resolution,
                        assistant_message,
                    )
                    .await
            }
            None => {
                self.store
                    .resolve_model_request(request_id, resolution)
                    .await
            }
        };
        if let Err(error) = result {
            warn!(%request_id, error = %error, "未能写入失败模型请求终态");
        }
    }
}

fn completion_usage(completion: &ModelCompletion) -> Value {
    match completion
        .usage
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
    {
        Ok(Some(usage)) => usage,
        Ok(None) => json!({ "status": "unknown" }),
        Err(error) => json!({
            "status": "invalid",
            "reason": error.to_string(),
        }),
    }
}

fn build_context_with_history(
    system_instructions: String,
    tools: Vec<AgentToolDefinition>,
    completed_stages: Vec<CompletedStageContext>,
    current_inputs: Vec<CurrentContextInput>,
    current_messages: Vec<UnfoldedStageMessage>,
    max_context_tokens: usize,
    reserved_output_tokens: usize,
) -> Result<nekowriter_agent::BuiltAgentContext, SchedulerError> {
    ContextBuilder::build(ContextBuildRequest {
        system_instructions,
        tools,
        completed_stages,
        current_inputs,
        current_messages,
        budget: ContextBudget {
            max_context_tokens,
            reserved_output_tokens,
        },
    })
    .map_err(|error| SchedulerError::Context(error.to_string()))
}

fn completed_stage_contexts(folds: Vec<CompletedRunContextFold>) -> Vec<CompletedStageContext> {
    folds
        .into_iter()
        .map(|fold| CompletedStageContext {
            stage: match fold.stage {
                CompletedRunContextStage::Preparation => ContextFoldStage::Preparation,
                CompletedRunContextStage::ChapterOutline => ContextFoldStage::ChapterOutline,
                CompletedRunContextStage::ChapterBody => ContextFoldStage::ChapterBody,
            },
            chapter_order: fold.chapter_order,
            source_run_id: fold.source_run_id,
            fold: fold.fold,
        })
        .collect()
}

fn chapter_outline_plan_context(
    chapter: &ChapterSummary,
    approved_plan: Option<&ChapterPlan>,
) -> Result<String, SchedulerError> {
    match chapter.kind {
        ChapterKind::Main => {
            let plan = approved_plan.ok_or_else(|| {
                SchedulerError::Context("审核通过的小说粗纲缺少章节计划".to_owned())
            })?;
            let plan_item_id = chapter.plan_item_id.ok_or_else(|| {
                SchedulerError::Context("当前章节缺少对应的章节计划项".to_owned())
            })?;
            let plan_item = plan
                .items
                .iter()
                .find(|item| item.plan_item_id == plan_item_id)
                .ok_or_else(|| {
                    SchedulerError::Context("当前章节计划项不在已审核粗纲中".to_owned())
                })?;
            Ok(format!(
                "第{}章：{}\n{}",
                plan_item.order_index, plan_item.title, plan_item.goal
            ))
        }
        ChapterKind::Extra => Ok(format!(
            "番外：{}\n这是正文已经完本后新增的番外。请结合全部已通过正文和用户指导，确定独立而自洽的情节目标。",
            chapter.title
        )),
    }
}

/// 只写入当前 Run 的原始输入。完成阶段的折叠块已经随 `context_snapshot` 封存，
/// 不能伪装成当前 Run 的 assistant 历史，否则下一个阶段会把前序产物错误归属给
/// 当前阶段的折叠水位。
fn agent_messages_for_new_stage_request(
    request: &ModelRequest,
) -> Result<Vec<NewAgentMessage>, SchedulerError> {
    request
        .messages
        .iter()
        .filter_map(|message| match message {
            ModelRequestMessage::Text { role, content } => {
                match role {
                    ContextMessageRole::System | ContextMessageRole::User => {
                        let role = if *role == ContextMessageRole::System {
                            AgentMessageRole::System
                        } else {
                            AgentMessageRole::User
                        };
                        Some(Ok(NewAgentMessage {
                            role,
                            content: json!({
                                "kind": "text",
                                "role": role_name(role),
                                "content": content,
                            }),
                            request_id: None,
                            tool_call_id: None,
                        }))
                    }
                    // ContextBuilder 用 assistant 角色承载已完成阶段的纯文本 fold；
                    // fold 的所有权始终留在源 Run，当前 Run 不再复制它。
                    ContextMessageRole::Assistant => None,
                    ContextMessageRole::Tool => {
                        Some(Err(SchedulerError::UnsupportedContextMessage))
                    }
                }
            }
            ModelRequestMessage::AssistantToolCalls { .. }
            | ModelRequestMessage::ToolResult { .. } => {
                Some(Err(SchedulerError::UnsupportedContextMessage))
            }
        })
        .collect()
}

fn ask_user_tools() -> Vec<AgentToolDefinition> {
    vec![ask_user_tool()]
}

fn ask_user_tool() -> AgentToolDefinition {
    AgentToolDefinition {
        name: "ask_user".to_owned(),
        description: "向用户提出一组必须等待回答的问题。每题使用稳定 id；single 和 multiple 都允许 custom_text，自定义答案不需要伪造为预设选项。".to_owned(),
        parameters: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["questions"],
            "properties": {
                "questions": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 20,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["id", "title", "kind", "options", "required"],
                        "properties": {
                            "id": { "type": "string", "minLength": 1 },
                            "title": { "type": "string", "minLength": 1 },
                            "kind": { "type": "string", "enum": ["single", "multiple", "text"] },
                            "options": {
                                "type": "array",
                                "maxItems": 50,
                                "items": {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "required": ["id", "label"],
                                    "properties": {
                                        "id": { "type": "string", "minLength": 1 },
                                        "label": { "type": "string", "minLength": 1 }
                                    }
                                }
                            },
                            "required": { "type": "boolean" }
                        }
                    }
                }
            }
        }),
    }
}

fn chapter_body_tools() -> Vec<AgentToolDefinition> {
    vec![
        ask_user_tool(),
        AgentToolDefinition {
            name: "append_content".to_owned(),
            description: "向当前章节正文原样追加一段非空文本。不要传递章节或项目 ID。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["content"],
                "properties": {
                    "content": { "type": "string", "minLength": 1 }
                }
            }),
        },
        AgentToolDefinition {
            name: "replace_content".to_owned(),
            description: "在当前章节正文中精确替换唯一出现的一段文本；new_str 可为空。不要传递章节或项目 ID。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["old_str", "new_str"],
                "properties": {
                    "old_str": { "type": "string", "minLength": 1 },
                    "new_str": { "type": "string" }
                }
            }),
        },
    ]
}

/// 将当前正文 Run 的不可变消息账本还原为下一轮请求所需的 tool-call 配对。
///
/// 初始 system/user 输入由每轮重新渲染的 prompt 提供，所以只回放 assistant/tool
/// 历史；这样不会在每次工具后不断重复初始指令和正文快照。任何损坏的 envelope
/// 都拒绝继续，而不是猜测出一个会破坏调用配对的模型消息。
fn writing_agent_history_messages(
    run_id: RunId,
    messages: Vec<PersistedAgentMessage>,
    materialized_feedback: &[MaterializedChapterBodyFeedback],
) -> Result<Vec<UnfoldedStageMessage>, SchedulerError> {
    let feedback_by_tool_call = materialized_feedback.iter().fold(
        HashMap::<String, Vec<Value>>::new(),
        |mut grouped, feedback| {
            grouped
                .entry(feedback.provider_tool_call_id.clone())
                .or_default()
                .push(json!({
                    "id": feedback.id,
                    "text": feedback.text.clone(),
                    "citations": feedback.citations.clone(),
                }));
            grouped
        },
    );
    messages
        .into_iter()
        .filter_map(|persisted| {
            let sequence = persisted.sequence;
            match persisted.role {
                // 初始 system/user 输入由本轮 prompt 重新渲染；只有 Store 在协议守卫
                // 失败后写入的明确纠正消息需要留在正文 Agent 历史中。
                AgentMessageRole::System => None,
                AgentMessageRole::User => {
                    decode_body_protocol_correction_message(run_id, sequence, persisted.content)
                        .transpose()
                        .map(|message| {
                            message.map(|message| UnfoldedStageMessage {
                                source_run_id: run_id,
                                sequence,
                                message,
                            })
                        })
                }
                AgentMessageRole::Assistant => Some(
                    decode_body_assistant_message(run_id, sequence, persisted.content).map(
                        |message| UnfoldedStageMessage {
                            source_run_id: run_id,
                            sequence,
                            message,
                        },
                    ),
                ),
                AgentMessageRole::Tool => Some(
                    decode_body_tool_message(
                        run_id,
                        sequence,
                        persisted.tool_call_id.clone(),
                        persisted.content,
                        persisted
                            .tool_call_id
                            .as_deref()
                            .and_then(|tool_call_id| feedback_by_tool_call.get(tool_call_id))
                            .map(Vec::as_slice)
                            .unwrap_or(&[]),
                    )
                    .map(|message| UnfoldedStageMessage {
                        source_run_id: run_id,
                        sequence,
                        message,
                    }),
                ),
            }
        })
        .collect()
}

fn decode_body_protocol_correction_message(
    run_id: RunId,
    sequence: i64,
    content: Value,
) -> Result<Option<ModelRequestMessage>, SchedulerError> {
    if message_kind(&content, run_id, sequence)? != "protocol_correction" {
        return Ok(None);
    }
    let correction = content
        .get("content")
        .and_then(Value::as_str)
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| {
            invalid_body_history_message(
                run_id,
                sequence,
                "protocol_correction 缺少非空字符串 content".to_owned(),
            )
        })?;
    Ok(Some(ModelRequestMessage::Text {
        role: ContextMessageRole::User,
        content: correction.to_owned(),
    }))
}

fn decode_body_assistant_message(
    run_id: RunId,
    sequence: i64,
    content: Value,
) -> Result<ModelRequestMessage, SchedulerError> {
    let kind = message_kind(&content, run_id, sequence)?;
    match kind {
        "assistant_tool_calls" => {
            let assistant_content: Option<String> =
                serde_json::from_value(content.get("content").cloned().unwrap_or(Value::Null))
                    .map_err(|error| {
                        invalid_body_history_message(run_id, sequence, error.to_string())
                    })?;
            let tool_calls: Vec<ModelToolCall> =
                serde_json::from_value(content.get("toolCalls").cloned().ok_or_else(|| {
                    invalid_body_history_message(
                        run_id,
                        sequence,
                        "assistant_tool_calls 缺少 toolCalls".to_owned(),
                    )
                })?)
                .map_err(|error| {
                    invalid_body_history_message(run_id, sequence, error.to_string())
                })?;
            Ok(ModelRequestMessage::AssistantToolCalls {
                content: assistant_content,
                tool_calls,
            })
        }
        "text" => {
            let text = content
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    invalid_body_history_message(
                        run_id,
                        sequence,
                        "assistant text 消息缺少字符串 content".to_owned(),
                    )
                })?
                .to_owned();
            Ok(ModelRequestMessage::Text {
                role: ContextMessageRole::Assistant,
                content: text,
            })
        }
        other => Err(invalid_body_history_message(
            run_id,
            sequence,
            format!("assistant 消息 kind 不支持：{other}"),
        )),
    }
}

fn decode_body_tool_message(
    run_id: RunId,
    sequence: i64,
    tool_call_id: Option<String>,
    content: Value,
    feedback: &[Value],
) -> Result<ModelRequestMessage, SchedulerError> {
    if message_kind(&content, run_id, sequence)? != "tool_result" {
        return Err(invalid_body_history_message(
            run_id,
            sequence,
            "tool 消息 kind 必须为 tool_result".to_owned(),
        ));
    }
    let tool_call_id = tool_call_id.ok_or_else(|| {
        invalid_body_history_message(run_id, sequence, "tool 消息缺少 tool_call_id".to_owned())
    })?;
    let content = content
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            invalid_body_history_message(
                run_id,
                sequence,
                "tool_result 缺少字符串 content".to_owned(),
            )
        })?
        .to_owned();
    let mut result: Value = serde_json::from_str(&content).map_err(|error| {
        invalid_body_history_message(
            run_id,
            sequence,
            format!("tool_result 不是有效 JSON：{error}"),
        )
    })?;
    let result_object = result.as_object_mut().ok_or_else(|| {
        invalid_body_history_message(
            run_id,
            sequence,
            "tool_result 必须是 JSON object".to_owned(),
        )
    })?;
    let result_feedback = result_object
        .entry("feedback".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| {
            invalid_body_history_message(
                run_id,
                sequence,
                "tool_result.feedback 必须是 JSON array".to_owned(),
            )
        })?;
    result_feedback.extend(feedback.iter().cloned());
    let content = serde_json::to_string(&result).map_err(|error| {
        invalid_body_history_message(
            run_id,
            sequence,
            format!("tool_result 无法重新序列化：{error}"),
        )
    })?;
    Ok(ModelRequestMessage::ToolResult {
        tool_call_id,
        content,
    })
}

fn message_kind<'a>(
    content: &'a Value,
    run_id: RunId,
    sequence: i64,
) -> Result<&'a str, SchedulerError> {
    content.get("kind").and_then(Value::as_str).ok_or_else(|| {
        invalid_body_history_message(run_id, sequence, "消息缺少字符串 kind".to_owned())
    })
}

fn invalid_body_history_message(run_id: RunId, sequence: i64, reason: String) -> SchedulerError {
    SchedulerError::Context(format!(
        "正文 Run {run_id} 的消息 {sequence} 无法重放：{reason}"
    ))
}

fn assistant_message_from_completion(
    completion: &ModelCompletion,
    request_id: Uuid,
) -> NewAgentMessage {
    let content = if completion.tool_calls.is_empty() {
        json!({
            "kind": "text",
            "role": "assistant",
            // 无文本也要保存为空字符串而非 JSON null；后续协议纠正会回放这条真实
            // assistant 消息，`ModelRequestMessage::Text` 不能携带 null。
            "content": completion.content.clone().unwrap_or_default(),
        })
    } else {
        json!({
            "kind": "assistant_tool_calls",
            "content": completion.content,
            "toolCalls": &completion.tool_calls,
        })
    };
    NewAgentMessage {
        role: AgentMessageRole::Assistant,
        content,
        request_id: Some(request_id),
        tool_call_id: None,
    }
}

const fn role_name(role: AgentMessageRole) -> &'static str {
    match role {
        AgentMessageRole::System => "system",
        AgentMessageRole::User => "user",
        AgentMessageRole::Assistant => "assistant",
        AgentMessageRole::Tool => "tool",
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use nekowriter_core::{
        ChapterKind, ChapterPlan, ChapterPlanDraft, ChapterStatus, ChapterSummary, ExecutionStatus,
        FeedbackStatus, HumanReviewDecision, NewChapterGuidance, NewNovelBrief, NewProject,
        PreparationArtifactInput, PreparationArtifactKind, UserQuestionAnswer, UserQuestionAnswers,
        WorkflowStage,
    };
    use nekowriter_store::{
        AgentMessageRole, AgentQuestionToolCall, ChapterBodyAgentToolCall, ModelCapabilityProfile,
        ModelRequestStatus, NewModelRequest, NewSystemModelSettings,
    };
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{mpsc, oneshot},
        time::timeout,
    };
    use uuid::Uuid;

    use super::*;

    #[test]
    fn extra_chapter_outline_context_does_not_require_a_main_plan_item() {
        let extra = ChapterSummary {
            id: Uuid::new_v4(),
            version_id: Uuid::new_v4(),
            kind: ChapterKind::Extra,
            plan_item_id: None,
            order_index: 3,
            title: "雨停之后".to_owned(),
            status: ChapterStatus::Planned,
            approved_revision_id: None,
            revision: 0,
        };
        let extra_context = chapter_outline_plan_context(&extra, None).unwrap();
        assert!(extra_context.starts_with("番外：雨停之后\n"));

        let plan = ChapterPlan::from_drafts(&[ChapterPlanDraft::new(
            "雨夜的来客",
            "让林舟收到匿名报纸。",
        )
        .unwrap()])
        .unwrap();
        let main = ChapterSummary {
            id: Uuid::new_v4(),
            version_id: extra.version_id,
            kind: ChapterKind::Main,
            plan_item_id: Some(plan.items[0].plan_item_id),
            order_index: 1,
            title: plan.items[0].title.clone(),
            status: ChapterStatus::Planned,
            approved_revision_id: None,
            revision: 0,
        };
        assert_eq!(
            chapter_outline_plan_context(&main, Some(&plan)).unwrap(),
            "第1章：雨夜的来客\n让林舟收到匿名报纸。"
        );
        assert!(chapter_outline_plan_context(&main, None).is_err());
    }

    #[tokio::test]
    async fn preparation_lane_dispatches_a_real_sse_response_into_waiting_review() {
        let (base_url, request_receiver) = preparation_sse_server().await;
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        store
            .save_system_model_settings(
                Uuid::new_v4(),
                0,
                NewSystemModelSettings {
                    api_base_url: base_url,
                    credential_ref: None,
                    credential_fingerprint: None,
                    model_id: "test-writing-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
                        structured_output: true,
                        stream_usage: true,
                        thinking: true,
                    },
                    max_context_tokens: 32_768,
                    reserved_output_tokens: 4_096,
                },
            )
            .await
            .unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("调度器集成", "从真实 SSE 回调提交准备产物", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个有关旧车站重逢的短篇小说").unwrap(),
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
        let scheduler = WorkflowScheduler::new(
            store.clone(),
            Arc::new(Semaphore::new(1)),
            Arc::new(Mutex::new(())),
        );
        let mut events = store.subscribe_events();
        scheduler
            .enqueue_preparation(created.version.id, run_id)
            .await
            .unwrap();

        let completed = timeout(Duration::from_secs(5), async {
            loop {
                let _ = events.recv().await;
                let snapshot = store
                    .get_preparation_snapshot(created.version.id)
                    .await
                    .unwrap();
                if snapshot.workflow.execution == ExecutionStatus::WaitingReview {
                    return snapshot;
                }
            }
        })
        .await
        .expect("准备阶段模型回调应在时限内完成");
        assert_eq!(
            completed.workflow.stage,
            nekowriter_core::WorkflowStage::PreparingReviewing
        );
        assert!(
            completed
                .artifacts
                .iter()
                .all(|artifact| artifact.revision_id.is_some() && artifact.review.is_some())
        );
        assert_eq!(
            store.list_model_requests(run_id).await.unwrap()[0].status,
            ModelRequestStatus::Succeeded
        );
        assert_eq!(store.list_agent_messages(run_id).await.unwrap().len(), 3);

        let request = timeout(Duration::from_secs(2), request_receiver)
            .await
            .expect("适配器应向假服务发送请求")
            .unwrap();
        assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(request.contains("\"response_format\":{\"type\":\"json_object\"}"));
    }

    #[tokio::test]
    async fn delegated_review_lane_uses_a_tool_free_structured_review_request() {
        let (base_url, request_receiver) = review_sse_server(true, 0.86, Vec::new()).await;
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        store
            .save_system_model_settings(
                Uuid::new_v4(),
                0,
                NewSystemModelSettings {
                    api_base_url: base_url,
                    credential_ref: None,
                    credential_fingerprint: None,
                    model_id: "test-review-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: false,
                        structured_output: true,
                        stream_usage: true,
                        thinking: true,
                    },
                    max_context_tokens: 32_768,
                    reserved_output_tokens: 4_096,
                },
            )
            .await
            .unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new(
                    "审核调度器集成",
                    "审核 Agent 不应获得正文工具",
                    None::<&str>,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let version_id = created.version.id;
        store
            .save_novel_brief(
                version_id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个围绕旧车站和匿名来信的悬疑故事。").unwrap(),
            )
            .await
            .unwrap();
        let started = store
            .start_preparation(version_id, Uuid::new_v4(), 0)
            .await
            .unwrap();
        let submitted = store
            .submit_preparation_artifacts(
                version_id,
                Uuid::new_v4(),
                started.workflow.state_revision,
                vec![
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Background,
                        "旧车站将在雨季结束后拆除。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Research,
                        "匿名来信的来源尚待核验。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Characters,
                        "沈宁在寻找失踪的姐姐。",
                    )
                    .unwrap(),
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Outline,
                        "沈宁在末班车上收到匿名来信，决定去候车室寻找线索。",
                    )
                    .unwrap(),
                ],
                Some(vec![
                    ChapterPlanDraft::new("末班车", "沈宁在末班车上收到匿名来信。").unwrap(),
                ]),
            )
            .await
            .unwrap();
        let review_id = submitted
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
            .and_then(|artifact| artifact.review.as_ref())
            .unwrap()
            .id;
        let delegation = store
            .delegate_review(
                review_id,
                Uuid::new_v4(),
                submitted.workflow.state_revision,
                "检查背景是否能支撑第一章。".to_owned(),
            )
            .await
            .unwrap();
        let scheduler = WorkflowScheduler::new(
            store.clone(),
            Arc::new(Semaphore::new(1)),
            Arc::new(Mutex::new(())),
        );
        let mut events = store.subscribe_events();
        scheduler
            .enqueue_review(version_id, review_id, delegation.run_id)
            .await
            .unwrap();

        let completed = timeout(Duration::from_secs(5), async {
            loop {
                let _ = events.recv().await;
                let snapshot = store.get_preparation_snapshot(version_id).await.unwrap();
                if snapshot
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
                    .and_then(|artifact| artifact.review.as_ref())
                    .map(|review| review.status == nekowriter_core::ReviewStatus::Approved)
                    .unwrap_or(false)
                {
                    return snapshot;
                }
            }
        })
        .await
        .expect("审核 Agent 回调应在时限内应用决定");
        assert_eq!(completed.workflow.execution, ExecutionStatus::WaitingReview);
        assert_eq!(
            store.list_model_requests(delegation.run_id).await.unwrap()[0].status,
            ModelRequestStatus::Succeeded
        );

        let request = timeout(Duration::from_secs(2), request_receiver)
            .await
            .expect("审核 Agent 应向假服务发送请求")
            .unwrap();
        assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(request.contains("旧车站将在雨季结束后拆除。"));
        assert!(request.contains("检查背景是否能支撑第一章。"));
        assert!(request.contains("\"response_format\":{\"type\":\"json_object\"}"));
        assert!(!request.contains("\"tools\":"));
    }

    #[tokio::test]
    async fn queued_body_parent_after_review_is_reenqueued() {
        let (base_url, request_receiver) = hanging_sse_server().await;
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let (version_id, chapter_id) = ready_chapter_for_outline(&store).await;
        let ready = store.get_preparation_snapshot(version_id).await.unwrap();
        let outline_started = store
            .start_chapter_outline(chapter_id, Uuid::new_v4(), ready.workflow.state_revision)
            .await
            .unwrap();
        let outline = store
            .submit_chapter_outline(
                chapter_id,
                Uuid::new_v4(),
                outline_started.state_revision,
                nekowriter_core::NewChapterOutline::new("审核后必须继续执行的细纲").unwrap(),
            )
            .await
            .unwrap();
        let drafting = store
            .decide_review(
                outline.outline.review.as_ref().unwrap().id,
                Uuid::new_v4(),
                outline.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
            )
            .await
            .unwrap();
        let body_run_id = drafting.workflow.active_run_id.unwrap();
        store
            .save_system_model_settings(
                Uuid::new_v4(),
                0,
                NewSystemModelSettings {
                    api_base_url: base_url,
                    credential_ref: None,
                    credential_fingerprint: None,
                    model_id: "reenqueue-test-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
                        structured_output: true,
                        stream_usage: true,
                        thinking: true,
                    },
                    max_context_tokens: 32_768,
                    reserved_output_tokens: 4_096,
                },
            )
            .await
            .unwrap();
        let permit = Arc::new(Semaphore::new(1));
        let scheduler =
            WorkflowScheduler::new(store.clone(), permit.clone(), Arc::new(Mutex::new(())));
        scheduler
            .enqueue_queued_parent_after_review(&drafting.workflow)
            .await
            .unwrap();

        let request = timeout(Duration::from_secs(2), request_receiver)
            .await
            .expect("审核后的排队正文 Run 应发送新的模型请求")
            .unwrap();
        assert!(request.contains("\"append_content\""));
        let in_flight = store.get_chapter_body(chapter_id).await.unwrap().workflow;
        assert_eq!(in_flight.execution, ExecutionStatus::RequestInFlight);
        let stopped = store
            .stop_writing_run(body_run_id, Uuid::new_v4(), in_flight.state_revision)
            .await
            .unwrap();
        scheduler
            .stop_run(version_id, body_run_id, &stopped.cancelled_request_ids)
            .await;
        timeout(Duration::from_secs(2), async {
            while permit.available_permits() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("停止重新投递的正文请求应释放模型许可");
    }

    #[tokio::test]
    async fn preparation_protocol_error_requeues_and_replays_the_correction() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new(
                    "调度器协议纠正",
                    "无效响应必须带着纠正消息重新排队",
                    None::<&str>,
                )
                .unwrap(),
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
                    settings_snapshot: json!({ "model": "protocol-test" }),
                    prompt_snapshot: json!({ "contentHash": "protocol-test" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let scheduler = WorkflowScheduler::new(
            store.clone(),
            Arc::new(Semaphore::new(1)),
            Arc::new(Mutex::new(())),
        );
        let request = ModelRequest {
            messages: Vec::new(),
            tools: ask_user_tools(),
            response_format: Some(StructuredOutput::JsonObject),
            thinking_level: ThinkingLevel::Off,
        };

        assert!(
            scheduler
                .handle_preparation_completion(
                    dispatched.id,
                    &request,
                    Ok(ModelCompletion {
                        provider_request_id: Some("provider-protocol-test".to_owned()),
                        content: Some("这不是合法的准备阶段 JSON".to_owned()),
                        tool_calls: Vec::new(),
                        usage: None,
                        finish_reason: nekowriter_agent::provider::ModelFinishReason::Stop,
                    }),
                )
                .await
        );

        let snapshot = store
            .get_preparation_snapshot(created.version.id)
            .await
            .unwrap();
        assert_eq!(snapshot.workflow.execution, ExecutionStatus::Queued);
        let persisted = store.list_model_requests(run_id).await.unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].status, ModelRequestStatus::Succeeded);
        assert_eq!(
            persisted[0]
                .error
                .as_ref()
                .and_then(|error| error.get("kind"))
                .and_then(Value::as_str),
            Some("protocol_error")
        );

        let history = writing_agent_history_messages(
            run_id,
            store.list_agent_messages(run_id).await.unwrap(),
            &[],
        )
        .unwrap();
        assert_eq!(history.len(), 2);
        assert!(matches!(
            &history[0].message,
            ModelRequestMessage::Text {
                role: ContextMessageRole::Assistant,
                content
            } if content == "这不是合法的准备阶段 JSON"
        ));
        assert!(matches!(
            &history[1].message,
            ModelRequestMessage::Text {
                role: ContextMessageRole::User,
                content
            } if content.contains("重新严格按当前阶段要求返回")
        ));
    }

    #[tokio::test]
    async fn explicit_stop_aborts_the_in_flight_model_task_and_releases_its_permit() {
        let (base_url, request_receiver) = hanging_sse_server().await;
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        store
            .save_system_model_settings(
                Uuid::new_v4(),
                0,
                NewSystemModelSettings {
                    api_base_url: base_url,
                    credential_ref: None,
                    credential_fingerprint: None,
                    model_id: "stop-test-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
                        structured_output: true,
                        stream_usage: true,
                        thinking: true,
                    },
                    max_context_tokens: 32_768,
                    reserved_output_tokens: 4_096,
                },
            )
            .await
            .unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("调度器停止", "停止必须取消悬挂模型请求", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个发生在雨夜车站的短篇小说").unwrap(),
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
        let permit = Arc::new(Semaphore::new(1));
        let scheduler =
            WorkflowScheduler::new(store.clone(), permit.clone(), Arc::new(Mutex::new(())));
        scheduler
            .enqueue_preparation(created.version.id, run_id)
            .await
            .unwrap();
        let request = timeout(Duration::from_secs(2), request_receiver)
            .await
            .expect("模型请求应已到达悬挂测试服务")
            .unwrap();
        assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
        let in_flight = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let dispatched = store.list_model_requests(run_id).await.unwrap();
        assert_eq!(in_flight.execution, ExecutionStatus::RequestInFlight);
        assert_eq!(dispatched[0].status, ModelRequestStatus::Dispatched);

        let stopped = store
            .stop_writing_run(run_id, Uuid::new_v4(), in_flight.state_revision)
            .await
            .unwrap();
        scheduler
            .stop_run(created.version.id, run_id, &stopped.cancelled_request_ids)
            .await;
        timeout(Duration::from_secs(2), async {
            while permit.available_permits() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("停止应释放全局模型请求许可");
        assert_eq!(
            store.list_model_requests(run_id).await.unwrap()[0].status,
            ModelRequestStatus::Cancelled
        );
        assert_eq!(
            store
                .get_preparation_snapshot(created.version.id)
                .await
                .unwrap()
                .workflow
                .execution,
            ExecutionStatus::Stopped
        );
    }

    #[tokio::test]
    async fn outline_lane_uses_preparation_fold_and_commits_a_reviewable_outline() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let (version_id, chapter_id) = ready_chapter_for_outline(&store).await;
        let (base_url, request_receiver) = chapter_outline_sse_server(chapter_id).await;
        store
            .save_system_model_settings(
                Uuid::new_v4(),
                0,
                NewSystemModelSettings {
                    api_base_url: base_url,
                    credential_ref: None,
                    credential_fingerprint: None,
                    model_id: "test-writing-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
                        structured_output: true,
                        stream_usage: true,
                        thinking: true,
                    },
                    max_context_tokens: 32_768,
                    reserved_output_tokens: 4_096,
                },
            )
            .await
            .unwrap();
        let before_start = store.get_preparation_snapshot(version_id).await.unwrap();
        assert_eq!(
            before_start.workflow.stage,
            WorkflowStage::WritingChapterReady
        );
        let started = store
            .start_chapter_outline(
                chapter_id,
                Uuid::new_v4(),
                before_start.workflow.state_revision,
            )
            .await
            .unwrap();
        let run_id = started.active_run_id.unwrap();
        let scheduler = WorkflowScheduler::new(
            store.clone(),
            Arc::new(Semaphore::new(1)),
            Arc::new(Mutex::new(())),
        );
        let mut events = store.subscribe_events();
        scheduler
            .enqueue_chapter_outline(version_id, chapter_id, run_id)
            .await
            .unwrap();

        let completed = timeout(Duration::from_secs(5), async {
            loop {
                let _ = events.recv().await;
                let snapshot = store.get_chapter_outline(chapter_id).await.unwrap();
                if snapshot.workflow.execution == ExecutionStatus::WaitingReview {
                    return snapshot;
                }
            }
        })
        .await
        .expect("章节细纲模型回调应在时限内完成");
        assert_eq!(
            completed.workflow.stage,
            WorkflowStage::WritingOutlineReviewing
        );
        assert_eq!(
            completed.outline.content.as_deref(),
            Some("模型生成的车站重逢细纲")
        );
        assert_eq!(
            store.list_model_requests(run_id).await.unwrap()[0].status,
            ModelRequestStatus::Succeeded
        );
        // 当前 Run 只保留自己的 system/user/assistant 原始消息；准备阶段 fold 的
        // 归属仍在准备 Run 中，并由请求快照中的 fold source 追溯。
        assert_eq!(store.list_agent_messages(run_id).await.unwrap().len(), 3);
        let model_requests = store.list_model_requests(run_id).await.unwrap();
        let model_request = &model_requests[0];
        assert_eq!(
            model_request.context_snapshot["build"]["foldSources"][0]["stage"],
            "preparation"
        );

        let request = timeout(Duration::from_secs(2), request_receiver)
            .await
            .expect("细纲适配器应向假服务发送请求")
            .unwrap();
        assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(request.contains("背景设定的已审核内容"));
        assert!(request.contains("第一章：车站重逢"));
    }

    #[tokio::test]
    async fn body_lane_applies_one_tool_call_then_submits_the_chapter_for_review() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let (version_id, chapter_id) = ready_chapter_for_outline(&store).await;
        let ready = store.get_preparation_snapshot(version_id).await.unwrap();
        let outline_started = store
            .start_chapter_outline(chapter_id, Uuid::new_v4(), ready.workflow.state_revision)
            .await
            .unwrap();
        let outline = store
            .submit_chapter_outline(
                chapter_id,
                Uuid::new_v4(),
                outline_started.state_revision,
                nekowriter_core::NewChapterOutline::new("模型正文应遵循的已审核章节细纲").unwrap(),
            )
            .await
            .unwrap();
        let drafting = store
            .decide_review(
                outline.outline.review.as_ref().unwrap().id,
                Uuid::new_v4(),
                outline.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
            )
            .await
            .unwrap();
        let body_run_id = drafting.workflow.active_run_id.unwrap();
        let (base_url, mut requests) = chapter_body_sse_server(chapter_id).await;
        store
            .save_system_model_settings(
                Uuid::new_v4(),
                0,
                NewSystemModelSettings {
                    api_base_url: base_url,
                    credential_ref: None,
                    credential_fingerprint: None,
                    model_id: "test-writing-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
                        structured_output: true,
                        stream_usage: true,
                        thinking: true,
                    },
                    max_context_tokens: 32_768,
                    reserved_output_tokens: 4_096,
                },
            )
            .await
            .unwrap();
        let scheduler = WorkflowScheduler::new(
            store.clone(),
            Arc::new(Semaphore::new(1)),
            Arc::new(Mutex::new(())),
        );
        let mut events = store.subscribe_events();
        scheduler
            .enqueue_chapter_body(version_id, chapter_id, body_run_id)
            .await
            .unwrap();

        let completed = timeout(Duration::from_secs(5), async {
            loop {
                let _ = events.recv().await;
                let snapshot = store.get_chapter_body(chapter_id).await.unwrap();
                if snapshot.workflow.execution == ExecutionStatus::WaitingReview {
                    return snapshot;
                }
            }
        })
        .await
        .expect("正文工具调用及终态提交应在时限内完成");
        assert_eq!(
            completed.workflow.stage,
            WorkflowStage::WritingChapterReviewing
        );
        assert_eq!(
            completed.body.content.as_deref(),
            Some("车站的钟声穿过雨幕。")
        );
        assert!(completed.body.review.is_some());

        let model_requests = store.list_model_requests(body_run_id).await.unwrap();
        assert_eq!(model_requests.len(), 2);
        assert!(
            model_requests
                .iter()
                .all(|request| request.status == ModelRequestStatus::Succeeded)
        );
        assert_eq!(
            store.list_agent_messages(body_run_id).await.unwrap().len(),
            5
        );
        let replayed_tool = store
            .execute_chapter_body_agent_tool_call(
                model_requests[0].id,
                chapter_id,
                Some("body-scheduler-test-request".to_owned()),
                json!({ "promptTokens": 21, "completionTokens": 8, "totalTokens": 29 }),
                ChapterBodyAgentToolCall {
                    provider_call_id: "body-call-1".to_owned(),
                    name: "append_content".to_owned(),
                    arguments: json!({ "content": "车站的钟声穿过雨幕。" }),
                },
            )
            .await
            .unwrap();
        assert!(!replayed_tool.newly_resolved);
        assert_eq!(replayed_tool.result["status"], "applied");
        assert_eq!(
            store
                .get_chapter_body(chapter_id)
                .await
                .unwrap()
                .body
                .content
                .as_deref(),
            Some("车站的钟声穿过雨幕。")
        );
        assert_eq!(
            model_requests[1].context_snapshot["build"]["foldSources"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|source| source["stage"] == "chapter_outline")
                .count(),
            1
        );

        let first_request = timeout(Duration::from_secs(2), requests.recv())
            .await
            .expect("正文首轮应发送到假服务")
            .unwrap();
        let second_request = timeout(Duration::from_secs(2), requests.recv())
            .await
            .expect("正文工具结果应触发第二轮")
            .unwrap();
        assert!(first_request.contains("\"append_content\""));
        assert!(first_request.contains("模型正文应遵循的已审核章节细纲"));
        assert!(second_request.contains("\"tool_call_id\":\"body-call-1\""));
        assert!(second_request.contains("车站的钟声穿过雨幕。"));
    }

    #[tokio::test]
    async fn approve_required_body_tool_stays_a_proposal_until_an_explicit_decision() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let (version_id, chapter_id) = ready_chapter_for_outline(&store).await;
        let ready = store.get_preparation_snapshot(version_id).await.unwrap();
        let outline_started = store
            .start_chapter_outline(chapter_id, Uuid::new_v4(), ready.workflow.state_revision)
            .await
            .unwrap();
        let outline = store
            .submit_chapter_outline(
                chapter_id,
                Uuid::new_v4(),
                outline_started.state_revision,
                nekowriter_core::NewChapterOutline::new("待确认正文必须遵循的章节细纲").unwrap(),
            )
            .await
            .unwrap();
        let drafting = store
            .decide_review(
                outline.outline.review.as_ref().unwrap().id,
                Uuid::new_v4(),
                outline.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
            )
            .await
            .unwrap();
        let approve_required = store
            .set_approval_mode(
                version_id,
                Uuid::new_v4(),
                drafting.workflow.state_revision,
                nekowriter_core::ApprovalMode::ApproveRequired,
            )
            .await
            .unwrap();
        let body_run_id = approve_required.active_run_id.unwrap();
        let (base_url, request_receiver) = chapter_body_tool_only_sse_server().await;
        store
            .save_system_model_settings(
                Uuid::new_v4(),
                0,
                NewSystemModelSettings {
                    api_base_url: base_url,
                    credential_ref: None,
                    credential_fingerprint: None,
                    model_id: "test-writing-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
                        structured_output: true,
                        stream_usage: true,
                        thinking: true,
                    },
                    max_context_tokens: 32_768,
                    reserved_output_tokens: 4_096,
                },
            )
            .await
            .unwrap();
        let scheduler = WorkflowScheduler::new(
            store.clone(),
            Arc::new(Semaphore::new(1)),
            Arc::new(Mutex::new(())),
        );
        let mut events = store.subscribe_events();
        scheduler
            .enqueue_chapter_body(version_id, chapter_id, body_run_id)
            .await
            .unwrap();

        let pending = timeout(Duration::from_secs(5), async {
            loop {
                let _ = events.recv().await;
                let snapshot = store.get_chapter_body(chapter_id).await.unwrap();
                if snapshot.pending_proposal.is_some() {
                    return snapshot;
                }
            }
        })
        .await
        .expect("逐次确认工具调用应在时限内进入待审提案");
        let proposal = pending.pending_proposal.as_ref().unwrap();
        assert_eq!(pending.workflow.execution, ExecutionStatus::WaitingReview);
        assert_eq!(pending.body.content, None);
        assert_eq!(proposal.candidate_content, "车站的钟声穿过雨幕。");
        assert_eq!(
            store.list_agent_messages(body_run_id).await.unwrap().len(),
            3
        );
        assert_eq!(
            store.list_model_requests(body_run_id).await.unwrap()[0].status,
            ModelRequestStatus::Succeeded
        );
        let first_request = timeout(Duration::from_secs(2), request_receiver)
            .await
            .expect("正文首轮应发送到假服务")
            .unwrap();
        assert!(first_request.contains("\"append_content\""));

        let switched = store
            .set_approval_mode(
                version_id,
                Uuid::new_v4(),
                pending.workflow.state_revision,
                nekowriter_core::ApprovalMode::AutoApprove,
            )
            .await
            .unwrap();
        assert_eq!(switched.execution, ExecutionStatus::WaitingReview);
        assert_eq!(
            switched.approval_mode,
            nekowriter_core::ApprovalMode::AutoApprove
        );
        assert!(
            store
                .get_chapter_body(chapter_id)
                .await
                .unwrap()
                .pending_proposal
                .is_some()
        );
        assert!(matches!(
            store
                .append_chapter_body(
                    chapter_id,
                    Uuid::new_v4(),
                    switched.state_revision,
                    nekowriter_core::AppendChapterContent::new("不应绕过待审提案的手工编辑。")
                        .unwrap(),
                )
                .await,
            Err(StoreError::InvalidChapterState(_))
        ));

        let decision_command_id = Uuid::new_v4();
        let accepted = store
            .decide_chapter_body_proposal(
                chapter_id,
                proposal.tool_call_id,
                decision_command_id,
                switched.state_revision,
                "   ".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(accepted.workflow.execution, ExecutionStatus::Queued);
        assert_eq!(
            accepted.body.content.as_deref(),
            Some("车站的钟声穿过雨幕。")
        );
        assert_eq!(accepted.pending_proposal, None);
        assert_eq!(
            store.list_agent_messages(body_run_id).await.unwrap().len(),
            4
        );
        let event_types = store
            .events_after(0)
            .await
            .unwrap()
            .into_iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>();
        assert!(
            event_types
                .iter()
                .any(|event_type| event_type == "tool.proposed")
        );
        assert!(
            event_types
                .iter()
                .any(|event_type| event_type == "tool.resolved")
        );
        let replayed = store
            .decide_chapter_body_proposal(
                chapter_id,
                proposal.tool_call_id,
                decision_command_id,
                switched.state_revision,
                "   ".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(replayed, accepted);

        store
            .set_approval_mode(
                version_id,
                Uuid::new_v4(),
                accepted.workflow.state_revision,
                nekowriter_core::ApprovalMode::ApproveRequired,
            )
            .await
            .unwrap();
        let second_request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: body_run_id,
                    settings_snapshot: json!({ "model": "test-writing-model" }),
                    prompt_snapshot: json!({ "contentHash": "proposal-test" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let held = store
            .execute_chapter_body_agent_tool_call(
                second_request.id,
                chapter_id,
                Some("proposal-rejection-test-request".to_owned()),
                json!({ "promptTokens": 21, "completionTokens": 8, "totalTokens": 29 }),
                ChapterBodyAgentToolCall {
                    provider_call_id: "proposal-call-2".to_owned(),
                    name: "append_content".to_owned(),
                    arguments: json!({ "content": "这句候选不应直接进入正文。" }),
                },
            )
            .await
            .unwrap();
        assert!(!held.newly_resolved);
        let second_pending = store.get_chapter_body(chapter_id).await.unwrap();
        let second_proposal = second_pending.pending_proposal.unwrap();
        let rejected = store
            .decide_chapter_body_proposal(
                chapter_id,
                second_proposal.tool_call_id,
                Uuid::new_v4(),
                second_pending.workflow.state_revision,
                "请让场景继续推进，不要直接总结。".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.workflow.execution, ExecutionStatus::Queued);
        assert_eq!(
            rejected.body.content.as_deref(),
            Some("车站的钟声穿过雨幕。")
        );
        assert_eq!(rejected.pending_proposal, None);
        let messages = store.list_agent_messages(body_run_id).await.unwrap();
        assert_eq!(messages.last().unwrap().role, AgentMessageRole::Tool);
        assert!(
            messages.last().unwrap().content["content"]
                .as_str()
                .unwrap()
                .contains("请让场景继续推进")
        );
    }

    #[tokio::test]
    async fn free_feedback_binds_to_the_next_tool_and_is_sealed_once() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let (version_id, chapter_id) = ready_chapter_for_outline(&store).await;
        let ready = store.get_preparation_snapshot(version_id).await.unwrap();
        let outline_started = store
            .start_chapter_outline(chapter_id, Uuid::new_v4(), ready.workflow.state_revision)
            .await
            .unwrap();
        let outline = store
            .submit_chapter_outline(
                chapter_id,
                Uuid::new_v4(),
                outline_started.state_revision,
                nekowriter_core::NewChapterOutline::new("自由意见测试需要的已审核细纲").unwrap(),
            )
            .await
            .unwrap();
        let drafting = store
            .decide_review(
                outline.outline.review.as_ref().unwrap().id,
                Uuid::new_v4(),
                outline.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
            )
            .await
            .unwrap();
        let body_run_id = drafting.workflow.active_run_id.unwrap();
        let first_request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: body_run_id,
                    settings_snapshot: json!({ "model": "feedback-test" }),
                    prompt_snapshot: json!({ "contentHash": "feedback-test" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let in_flight = store.get_chapter_body(chapter_id).await.unwrap();
        let queued_feedback = store
            .submit_run_feedback(
                body_run_id,
                Uuid::new_v4(),
                in_flight.workflow.state_revision,
                "请让人物先通过动作试探彼此，不要直接解释旧日误会。".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(queued_feedback.pending_feedback.len(), 1);
        assert_eq!(
            queued_feedback.pending_feedback[0].status,
            FeedbackStatus::Queued
        );

        let execution = store
            .execute_chapter_body_agent_tool_call(
                first_request.id,
                chapter_id,
                Some("feedback-test-request-1".to_owned()),
                json!({ "promptTokens": 12, "completionTokens": 7, "totalTokens": 19 }),
                ChapterBodyAgentToolCall {
                    provider_call_id: "feedback-tool-1".to_owned(),
                    name: "append_content".to_owned(),
                    arguments: json!({ "content": "她把伞往他那边挪了半寸。" }),
                },
            )
            .await
            .unwrap();
        assert!(execution.newly_resolved);
        let bound = store.get_chapter_body(chapter_id).await.unwrap();
        assert_eq!(bound.workflow.execution, ExecutionStatus::Queued);
        assert_eq!(bound.pending_feedback.len(), 1);
        assert_eq!(bound.pending_feedback[0].status, FeedbackStatus::Bound);
        let materialized = store
            .list_materialized_chapter_body_feedback(body_run_id)
            .await
            .unwrap();
        assert_eq!(materialized.len(), 1);
        assert_eq!(materialized[0].provider_tool_call_id, "feedback-tool-1");
        assert_eq!(materialized[0].status, FeedbackStatus::Bound);

        let stale = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: body_run_id,
                    settings_snapshot: json!({ "model": "feedback-test" }),
                    prompt_snapshot: json!({ "contentHash": "feedback-test-stale" }),
                    context_snapshot: json!({ "feedbackBinding": { "ids": [] } }),
                },
                Vec::new(),
            )
            .await;
        assert!(matches!(stale, Err(StoreError::FeedbackSnapshotStale)));

        let dispatched = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: body_run_id,
                    settings_snapshot: json!({ "model": "feedback-test" }),
                    prompt_snapshot: json!({ "contentHash": "feedback-test-sealed" }),
                    context_snapshot: json!({
                        "feedbackBinding": { "ids": [materialized[0].id] },
                        "toolResultBinding": {
                            "providerCallIds": [materialized[0].provider_tool_call_id],
                        },
                    }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(dispatched.status, ModelRequestStatus::Dispatched);
        let after_seal = store.get_chapter_body(chapter_id).await.unwrap();
        assert!(after_seal.pending_feedback.is_empty());
        let included = store
            .list_materialized_chapter_body_feedback(body_run_id)
            .await
            .unwrap();
        assert_eq!(included[0].status, FeedbackStatus::Included);
    }

    #[tokio::test]
    async fn ask_user_waits_for_valid_answers_and_requeues_with_a_tool_result() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let (_version_id, chapter_id, body_run_id, request_id) =
            in_flight_chapter_body_submission(&store, "ask-user-test").await;

        let waiting = store
            .execute_agent_question_tool_call(
                request_id,
                Some("ask-user-test-request".to_owned()),
                json!({ "promptTokens": 18, "completionTokens": 5, "totalTokens": 23 }),
                AgentQuestionToolCall {
                    provider_call_id: "ask-user-call-1".to_owned(),
                    arguments: json!({
                        "questions": [
                            {
                                "id": "tone",
                                "title": "这一章更接近哪种情绪？",
                                "kind": "single",
                                "options": [
                                    { "id": "restrained", "label": "克制" },
                                    { "id": "urgent", "label": "紧张" }
                                ],
                                "required": true
                            },
                            {
                                "id": "detail",
                                "title": "还有必须保留的细节吗？",
                                "kind": "text",
                                "options": [],
                                "required": false
                            }
                        ]
                    }),
                },
            )
            .await
            .unwrap();
        let question = match waiting {
            AppliedAgentQuestionToolCall::AwaitingAnswer { question, .. } => question,
            other => panic!("expected pending question, got {other:?}"),
        };

        let waiting_snapshot = store.get_chapter_body(chapter_id).await.unwrap();
        assert_eq!(
            waiting_snapshot.workflow.execution,
            ExecutionStatus::WaitingAnswer
        );
        assert_eq!(waiting_snapshot.pending_question.as_ref(), Some(&question));

        // F12：自由意见在等待正式答案时只进入独立批次，绝不替代问题回答。
        let with_feedback = store
            .submit_run_feedback(
                body_run_id,
                Uuid::new_v4(),
                waiting_snapshot.workflow.state_revision,
                "让人物的沉默比解释更有分量。".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            with_feedback.workflow.execution,
            ExecutionStatus::WaitingAnswer
        );
        assert_eq!(
            with_feedback.pending_feedback[0].status,
            FeedbackStatus::Queued
        );

        let answer_command_id = Uuid::new_v4();
        let answers = UserQuestionAnswers {
            answers: vec![
                UserQuestionAnswer {
                    question_id: "tone".to_owned(),
                    selected_option_ids: vec!["restrained".to_owned()],
                    custom_text: None,
                },
                UserQuestionAnswer {
                    question_id: "detail".to_owned(),
                    selected_option_ids: Vec::new(),
                    custom_text: Some("保留雨声和旧站钟。".to_owned()),
                },
            ],
        };
        let answered = store
            .answer_user_question(
                question.id,
                answer_command_id,
                with_feedback.workflow.state_revision,
                answers.clone(),
            )
            .await
            .unwrap();
        assert_eq!(answered.run_id, body_run_id);
        assert_eq!(answered.workflow.execution, ExecutionStatus::Queued);

        // 同 command_id 的网络重试只回放首个回执，不会再添加工具消息或回答一次。
        let replayed = store
            .answer_user_question(
                question.id,
                answer_command_id,
                with_feedback.workflow.state_revision,
                answers,
            )
            .await
            .unwrap();
        assert_eq!(replayed, answered);
        let messages = store.list_agent_messages(body_run_id).await.unwrap();
        assert_eq!(
            messages[messages.len() - 2].role,
            AgentMessageRole::Assistant
        );
        assert_eq!(messages.last().unwrap().role, AgentMessageRole::Tool);
        assert_eq!(
            messages.last().unwrap().tool_call_id.as_deref(),
            Some("ask-user-call-1")
        );

        let provider_call_ids = store
            .list_unsealed_agent_tool_result_provider_ids(body_run_id)
            .await
            .unwrap();
        assert_eq!(provider_call_ids, vec!["ask-user-call-1"]);
        let feedback = store
            .list_materialized_chapter_body_feedback(body_run_id)
            .await
            .unwrap();
        assert_eq!(feedback[0].status, FeedbackStatus::Bound);
        assert_eq!(feedback[0].provider_tool_call_id, "ask-user-call-1");

        let dispatched = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: body_run_id,
                    settings_snapshot: json!({ "model": "ask-user-test" }),
                    prompt_snapshot: json!({ "contentHash": "ask-user-answer" }),
                    context_snapshot: json!({
                        "feedbackBinding": { "ids": [feedback[0].id] },
                        "toolResultBinding": { "providerCallIds": provider_call_ids },
                    }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(dispatched.status, ModelRequestStatus::Dispatched);
        let sealed = store.get_chapter_body(chapter_id).await.unwrap();
        assert!(sealed.pending_question.is_none());
        assert!(sealed.pending_feedback.is_empty());
    }

    #[tokio::test]
    async fn feedback_at_a_held_tool_boundary_resumes_once_and_binds_the_full_batch() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let (version_id, chapter_id) = ready_chapter_for_outline(&store).await;
        let ready = store.get_preparation_snapshot(version_id).await.unwrap();
        let outline_started = store
            .start_chapter_outline(chapter_id, Uuid::new_v4(), ready.workflow.state_revision)
            .await
            .unwrap();
        let outline = store
            .submit_chapter_outline(
                chapter_id,
                Uuid::new_v4(),
                outline_started.state_revision,
                nekowriter_core::NewChapterOutline::new("暂停恢复测试需要的已审核细纲").unwrap(),
            )
            .await
            .unwrap();
        let drafting = store
            .decide_review(
                outline.outline.review.as_ref().unwrap().id,
                Uuid::new_v4(),
                outline.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
            )
            .await
            .unwrap();
        let body_run_id = drafting.workflow.active_run_id.unwrap();
        let request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: body_run_id,
                    settings_snapshot: json!({ "model": "pause-feedback-test" }),
                    prompt_snapshot: json!({ "contentHash": "pause-feedback-test" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let in_flight = store.get_chapter_body(chapter_id).await.unwrap();
        let pause_requested = store
            .request_chapter_body_pause(
                body_run_id,
                Uuid::new_v4(),
                in_flight.workflow.state_revision,
            )
            .await
            .unwrap();
        assert!(pause_requested.workflow.pause_requested);
        assert_eq!(
            pause_requested.workflow.execution,
            ExecutionStatus::RequestInFlight
        );

        let held = store
            .execute_chapter_body_agent_tool_call(
                request.id,
                chapter_id,
                Some("pause-feedback-request".to_owned()),
                json!({ "promptTokens": 10, "completionTokens": 4, "totalTokens": 14 }),
                ChapterBodyAgentToolCall {
                    provider_call_id: "pause-feedback-tool".to_owned(),
                    name: "append_content".to_owned(),
                    arguments: json!({ "content": "她把伞沿着长椅推过去。" }),
                },
            )
            .await
            .unwrap();
        assert!(!held.newly_resolved);
        assert_eq!(held.result["status"], "held");
        let paused = store.get_chapter_body(chapter_id).await.unwrap();
        assert_eq!(paused.workflow.execution, ExecutionStatus::Paused);
        assert!(paused.workflow.pause_requested);
        assert_eq!(paused.body.content, None);
        assert_eq!(
            paused
                .held_tool
                .as_ref()
                .map(|tool| tool.provider_call_id.as_str()),
            Some("pause-feedback-tool")
        );

        let resumed = store
            .submit_run_feedback(
                body_run_id,
                Uuid::new_v4(),
                paused.workflow.state_revision,
                "让两人先用动作试探，不要立即解释过去。".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(resumed.workflow.execution, ExecutionStatus::Queued);
        assert!(!resumed.workflow.pause_requested);
        assert_eq!(resumed.held_tool, None);
        assert_eq!(
            resumed.body.content.as_deref(),
            Some("她把伞沿着长椅推过去。")
        );
        assert_eq!(resumed.pending_feedback.len(), 1);
        assert_eq!(resumed.pending_feedback[0].status, FeedbackStatus::Bound);
        let materialized = store
            .list_materialized_chapter_body_feedback(body_run_id)
            .await
            .unwrap();
        assert_eq!(materialized.len(), 1);
        assert_eq!(materialized[0].provider_tool_call_id, "pause-feedback-tool");
        assert_eq!(materialized[0].status, FeedbackStatus::Bound);
    }

    #[tokio::test]
    async fn direct_chapter_submission_with_pending_feedback_requeues_for_a_tool_result() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let (version_id, chapter_id, body_run_id, request_id) =
            in_flight_chapter_body_submission(&store, "direct-submission-feedback").await;
        let in_flight = store.get_chapter_body(chapter_id).await.unwrap();
        let feedback = store
            .submit_run_feedback(
                body_run_id,
                Uuid::new_v4(),
                in_flight.workflow.state_revision,
                "请让人物用一个新的动作回应刚才的沉默。".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            feedback.workflow.execution,
            ExecutionStatus::RequestInFlight
        );
        assert_eq!(feedback.pending_feedback.len(), 1);
        assert_eq!(feedback.pending_feedback[0].status, FeedbackStatus::Queued);

        let corrected = store
            .commit_chapter_body_model_submission(
                request_id,
                chapter_id,
                Some("direct-submission-feedback-request".to_owned()),
                json!({ "promptTokens": 18, "completionTokens": 7, "totalTokens": 25 }),
                "{\"chapter_id\":\"ignored-by-store\",\"summary\":\"模型尝试直接提交本章。\"}"
                    .to_owned(),
                "模型尝试直接提交本章。".to_owned(),
            )
            .await
            .unwrap();
        assert_eq!(corrected.workflow.stage, WorkflowStage::WritingDrafting);
        assert_eq!(corrected.workflow.execution, ExecutionStatus::Queued);
        assert!(corrected.body.review.is_none());
        assert_eq!(corrected.pending_feedback.len(), 1);
        assert_eq!(corrected.pending_feedback[0].status, FeedbackStatus::Queued);
        let messages = store.list_agent_messages(body_run_id).await.unwrap();
        assert_eq!(messages.last().unwrap().role, AgentMessageRole::User);
        assert_eq!(
            messages.last().unwrap().content["kind"],
            "protocol_correction"
        );
        assert_eq!(
            store.list_model_requests(body_run_id).await.unwrap()[1].status,
            ModelRequestStatus::Succeeded
        );
        assert_eq!(corrected.workflow.version_id, version_id);
    }

    #[tokio::test]
    async fn direct_chapter_submission_holds_at_pause_boundary_and_resumes_to_final_review() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let (_version_id, chapter_id, body_run_id, request_id) =
            in_flight_chapter_body_submission(&store, "direct-submission-pause").await;
        let in_flight = store.get_chapter_body(chapter_id).await.unwrap();
        let pause_requested = store
            .request_chapter_body_pause(
                body_run_id,
                Uuid::new_v4(),
                in_flight.workflow.state_revision,
            )
            .await
            .unwrap();
        assert_eq!(
            pause_requested.workflow.execution,
            ExecutionStatus::RequestInFlight
        );
        assert!(pause_requested.workflow.pause_requested);

        let paused = store
            .commit_chapter_body_model_submission(
                request_id,
                chapter_id,
                Some("direct-submission-pause-request".to_owned()),
                json!({ "promptTokens": 18, "completionTokens": 7, "totalTokens": 25 }),
                "{\"chapter_id\":\"ignored-by-store\",\"summary\":\"模型尝试直接提交本章。\"}"
                    .to_owned(),
                "模型尝试直接提交本章。".to_owned(),
            )
            .await
            .unwrap();
        assert_eq!(paused.workflow.execution, ExecutionStatus::Paused);
        assert!(paused.workflow.pause_requested);
        assert!(paused.held_tool.is_none());
        assert!(paused.held_submission.is_some());
        assert!(paused.body.review.is_none());

        let resumed = store
            .resume_chapter_body_pause(body_run_id, Uuid::new_v4(), paused.workflow.state_revision)
            .await
            .unwrap();
        assert_eq!(
            resumed.workflow.stage,
            WorkflowStage::WritingChapterReviewing
        );
        assert_eq!(resumed.workflow.execution, ExecutionStatus::WaitingReview);
        assert!(!resumed.workflow.pause_requested);
        assert!(resumed.held_submission.is_none());
        assert!(resumed.body.review.is_some());
        assert_eq!(
            store.list_model_requests(body_run_id).await.unwrap()[1].status,
            ModelRequestStatus::Succeeded
        );
    }

    async fn ready_chapter_for_outline(store: &Store) -> (VersionId, ChapterId) {
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("细纲调度器集成", "从准备折叠进入章节细纲", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let version_id = created.version.id;
        store
            .save_novel_brief(
                version_id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个旧车站重逢的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let before_start = store.get_preparation_snapshot(version_id).await.unwrap();
        store
            .start_preparation(
                version_id,
                Uuid::new_v4(),
                before_start.workflow.state_revision,
            )
            .await
            .unwrap();
        let artifacts = PreparationArtifactKind::ALL
            .into_iter()
            .map(|kind| {
                PreparationArtifactInput::new(kind, format!("{}的已审核内容", kind.label()))
                    .unwrap()
            })
            .collect();
        let preparing = store.get_preparation_snapshot(version_id).await.unwrap();
        store
            .submit_preparation_artifacts(
                version_id,
                Uuid::new_v4(),
                preparing.workflow.state_revision,
                artifacts,
                Some(vec![
                    nekowriter_core::ChapterPlanDraft::new(
                        "第一章：车站重逢",
                        "让主角在旧车站收到改变命运的信件。",
                    )
                    .unwrap(),
                ]),
            )
            .await
            .unwrap();
        for kind in PreparationArtifactKind::ALL {
            let snapshot = store.get_preparation_snapshot(version_id).await.unwrap();
            let review_id = snapshot
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == kind)
                .and_then(|artifact| artifact.review.as_ref())
                .unwrap()
                .id;
            store
                .decide_review(
                    review_id,
                    Uuid::new_v4(),
                    snapshot.workflow.state_revision,
                    HumanReviewDecision::new(true, "").unwrap(),
                )
                .await
                .unwrap();
        }
        let chapter_id = store.list_chapters(version_id).await.unwrap()[0].id;
        let ready = store.get_preparation_snapshot(version_id).await.unwrap();
        store
            .save_chapter_guidance(
                chapter_id,
                Uuid::new_v4(),
                ready.workflow.state_revision,
                NewChapterGuidance::new("保持克制且有悬念的节奏").unwrap(),
            )
            .await
            .unwrap();
        (version_id, chapter_id)
    }

    /// 构造已经有合法正文、且下一条正文模型请求正处于在途状态的最小场景。
    ///
    /// 初始工具结果在第二次请求快照中被标记 included，因此测试中的自由意见不会
    /// 意外回填到历史结果，能准确覆盖“模型直接提交”这条没有新工具结果的边界。
    async fn in_flight_chapter_body_submission(
        store: &Store,
        label: &str,
    ) -> (VersionId, ChapterId, RunId, Uuid) {
        let (version_id, chapter_id) = ready_chapter_for_outline(store).await;
        let ready = store.get_preparation_snapshot(version_id).await.unwrap();
        let outline_started = store
            .start_chapter_outline(chapter_id, Uuid::new_v4(), ready.workflow.state_revision)
            .await
            .unwrap();
        let outline = store
            .submit_chapter_outline(
                chapter_id,
                Uuid::new_v4(),
                outline_started.state_revision,
                nekowriter_core::NewChapterOutline::new(format!("{label} 所需的已审核细纲"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let drafting = store
            .decide_review(
                outline.outline.review.as_ref().unwrap().id,
                Uuid::new_v4(),
                outline.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
            )
            .await
            .unwrap();
        let body_run_id = drafting.workflow.active_run_id.unwrap();
        let first_request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: body_run_id,
                    settings_snapshot: json!({ "model": label }),
                    prompt_snapshot: json!({ "contentHash": format!("{label}-first") }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let provider_tool_call_id = format!("{label}-append");
        let execution = store
            .execute_chapter_body_agent_tool_call(
                first_request.id,
                chapter_id,
                Some(format!("{label}-first-request")),
                json!({ "promptTokens": 12, "completionTokens": 5, "totalTokens": 17 }),
                ChapterBodyAgentToolCall {
                    provider_call_id: provider_tool_call_id.clone(),
                    name: "append_content".to_owned(),
                    arguments: json!({ "content": "她把伞沿着长椅推过去。" }),
                },
            )
            .await
            .unwrap();
        assert!(execution.newly_resolved);
        let second_request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: body_run_id,
                    settings_snapshot: json!({ "model": label }),
                    prompt_snapshot: json!({ "contentHash": format!("{label}-second") }),
                    context_snapshot: json!({
                        "toolResultBinding": {
                            "providerCallIds": [provider_tool_call_id],
                        },
                    }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        (version_id, chapter_id, body_run_id, second_request.id)
    }

    async fn preparation_sse_server() -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut stream, request) = read_http_request(stream).await;
            let _ = sender.send(request);
            let submission = json!({
                "artifacts": PreparationArtifactKind::ALL.into_iter().map(|kind| json!({
                    "kind": kind,
                    "content": format!("{} 的模型产物", kind.label()),
                })).collect::<Vec<_>>(),
                "chapter_plan": [{
                    "title": "第一章：车站重逢",
                    "goal": "让主角在旧车站收到改变命运的信件。",
                }],
            })
            .to_string();
            let stream_payload = format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({
                    "choices": [{
                        "index": 0,
                        "delta": { "content": submission },
                        "finish_reason": Value::Null,
                    }],
                }),
                json!({
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "stop",
                    }],
                    "usage": {
                        "prompt_tokens": 17,
                        "completion_tokens": 11,
                        "total_tokens": 28,
                    },
                }),
            );
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream; charset=utf-8\r\nx-request-id: scheduler-test-request\r\nconnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            stream.write_all(stream_payload.as_bytes()).await.unwrap();
            stream.shutdown().await.unwrap();
        });
        (format!("http://{address}/v1"), receiver)
    }

    async fn review_sse_server(
        passed: bool,
        confidence: f64,
        issues: Vec<String>,
    ) -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut stream, request) = read_http_request(stream).await;
            let _ = sender.send(request);
            let decision = json!({
                "passed": passed,
                "confidence": confidence,
                "issues": issues,
            })
            .to_string();
            let stream_payload = format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({
                    "choices": [{
                        "index": 0,
                        "delta": { "content": decision },
                        "finish_reason": Value::Null,
                    }],
                }),
                json!({
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "stop",
                    }],
                    "usage": {
                        "prompt_tokens": 13,
                        "completion_tokens": 7,
                        "total_tokens": 20,
                    },
                }),
            );
            write_sse_response(&mut stream, &stream_payload).await;
        });
        (format!("http://{address}/v1"), receiver)
    }

    /// 接受完整 HTTP 请求后不返回响应，直到测试通过显式停止中止客户端 future。
    async fn hanging_sse_server() -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (_stream, request) = read_http_request(stream).await;
            let _ = sender.send(request);
            std::future::pending::<()>().await;
        });
        (format!("http://{address}/v1"), receiver)
    }

    async fn chapter_outline_sse_server(
        chapter_id: ChapterId,
    ) -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut stream, request) = read_http_request(stream).await;
            let _ = sender.send(request);
            let submission = json!({
                "chapter_id": chapter_id,
                "content": "模型生成的车站重逢细纲",
                "goals": ["建立重逢的情绪张力"],
            })
            .to_string();
            let stream_payload = format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({
                    "choices": [{
                        "index": 0,
                        "delta": { "content": submission },
                        "finish_reason": Value::Null,
                    }],
                }),
                json!({
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "stop",
                    }],
                    "usage": {
                        "prompt_tokens": 19,
                        "completion_tokens": 12,
                        "total_tokens": 31,
                    },
                }),
            );
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream; charset=utf-8\r\nx-request-id: outline-scheduler-test-request\r\nconnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            stream.write_all(stream_payload.as_bytes()).await.unwrap();
            stream.shutdown().await.unwrap();
        });
        (format!("http://{address}/v1"), receiver)
    }

    async fn chapter_body_sse_server(chapter_id: ChapterId) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel(2);
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut stream, request) = read_http_request(stream).await;
            sender.send(request).await.unwrap();
            let first_response = concat!(
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"body-call-1\",\"type\":\"function\",\"function\":{\"name\":\"append_content\",\"arguments\":\"{\\\"content\\\":\\\"车站的钟声穿过雨幕。\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":21,\"completion_tokens\":8,\"total_tokens\":29}}\n\n",
                "data: [DONE]\n\n"
            );
            write_sse_response(&mut stream, first_response).await;

            let (stream, _) = listener.accept().await.unwrap();
            let (mut stream, request) = read_http_request(stream).await;
            sender.send(request).await.unwrap();
            let submission = json!({
                "chapter_id": chapter_id,
                "summary": "车站重逢场景已经完成，并留下后续追查的悬念。",
            })
            .to_string();
            let second_response = format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({
                    "choices": [{
                        "index": 0,
                        "delta": { "content": submission },
                        "finish_reason": Value::Null,
                    }],
                }),
                json!({
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "stop",
                    }],
                    "usage": {
                        "prompt_tokens": 29,
                        "completion_tokens": 11,
                        "total_tokens": 40,
                    },
                }),
            );
            write_sse_response(&mut stream, &second_response).await;
        });
        (format!("http://{address}/v1"), receiver)
    }

    async fn chapter_body_tool_only_sse_server() -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut stream, request) = read_http_request(stream).await;
            let _ = sender.send(request);
            let response = concat!(
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"proposal-call-1\",\"type\":\"function\",\"function\":{\"name\":\"append_content\",\"arguments\":\"{\\\"content\\\":\\\"车站的钟声穿过雨幕。\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":21,\"completion_tokens\":8,\"total_tokens\":29}}\n\n",
                "data: [DONE]\n\n"
            );
            write_sse_response(&mut stream, response).await;
        });
        (format!("http://{address}/v1"), receiver)
    }

    async fn write_sse_response(stream: &mut TcpStream, payload: &str) {
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream; charset=utf-8\r\nx-request-id: body-scheduler-test-request\r\nconnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        stream.write_all(payload.as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
    }

    async fn read_http_request(mut stream: TcpStream) -> (TcpStream, String) {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4_096];
        let header_end;
        loop {
            let read = stream.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0, "客户端在发送请求头前关闭连接");
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                header_end = position + 4;
                break;
            }
        }
        let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse::<usize>()
            .unwrap();
        while bytes.len() < header_end + content_length {
            let read = stream.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0, "客户端在发送完整请求体前关闭连接");
            bytes.extend_from_slice(&buffer[..read]);
        }
        (
            stream,
            String::from_utf8(bytes[..header_end + content_length].to_vec()).unwrap(),
        )
    }
}
