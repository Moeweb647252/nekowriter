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
        ChatCompletionsAdapter, ChatCompletionsConfig, ModelCompletion, ModelEvent,
        ModelFailureCode, ModelRequest, ModelRequestMessage, ModelTimeouts, ModelToolCall,
        ProviderError,
    },
    runner::{
        AgentRoundOutcome, AgentRunnerError, PhaseSubmission, PhaseSubmissionKind,
        SUBMIT_CHAPTER_OUTLINE_TOOL, SUBMIT_CHAPTER_TOOL, SUBMIT_PREPARATION_TOOL,
        SUBMIT_REVIEW_TOOL, ToolHost, ToolHostError, ToolResult, classify_model_completion,
        submission_contract, submission_tool,
    },
};
use nekowriter_core::{
    AuxiliaryToolCharge, ChapterId, ChapterKind, ChapterPlan, ChapterSummary, ExecutionStatus,
    FeedbackCitation, FeedbackStatus, PreparationArtifactKind, QuotaBalance, ReviewId,
    ReviewStatus, RunId, TOOL_QUERY_KNOWLEDGE, VersionId, WorkflowSnapshot, WorkflowStage,
};

use crate::model_settings::{
    CapabilityProblem, capability_problem_message, pre_dispatch_capability_problem,
    resolve_effective_model,
};
use nekowriter_store::{
    AgentMessageRole, AgentQuestionToolCall, AppliedAgentQuestionToolCall,
    AppliedChapterBodyToolCall, ChapterBodyAgentToolCall, CompletedRunContextFold,
    CompletedRunContextStage, DelegatedReviewKind, FreeAgentToolCall,
    MaterializedChapterBodyFeedback, ModelRequestResolution, NewAgentMessage, NewModelRequest,
    PersistedAgentMessage, ProtocolCorrectionTarget, RejectedRoundRecord, Store, StoreError,
};
use serde::Serialize;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::{
    sync::{Mutex, OwnedSemaphorePermit, Semaphore, broadcast, mpsc, oneshot},
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

const LANE_CHANNEL_CAPACITY: usize = 32;
#[cfg(test)]
const PREVIEW_DELTA_CHANNEL_CAPACITY: usize = 256;

/// 退出时等待在途模型请求任务结束的上限。
const CANCEL_SETTLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// 内存调度层的可恢复错误。所有有价值的工作流状态仍由 Store 的事务维护。
#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("当前稿本没有可用的系统写作模型设置")]
    MissingWritingModel,
    #[error("{0}")]
    ThinkingUnsupported(String),
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
    preview_deltas: broadcast::Sender<PreviewDelta>,
}

/// 一个仅在当前进程内存中存在的模型流式片段。
///
/// 它永不写入 SQLite，也没有可恢复的事件序列号；客户端以 `request_id + tool +
/// offset` 管理这层临时缓冲，任何丢失都不会影响已提交修订。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewDelta {
    pub version_id: VersionId,
    pub request_id: Uuid,
    pub tool: Option<PreviewTool>,
    #[serde(with = "nekowriter_core::decimal_i64")]
    pub offset: i64,
    pub content: String,
}

/// 工具参数流的临时归属。`index` 在同一模型响应内稳定；供应商稍后才补齐 ID 或
/// 名称时，后续 delta 会携带最新值。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewTool {
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Default)]
struct PreviewToolState {
    offset: i64,
    id: Option<String>,
    name: Option<String>,
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
    version_id: VersionId,
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
    /// 仅供本模块的集成测试创建独立预览通道；生产实例必须与 SSE 层共享通道。
    #[cfg(test)]
    #[must_use]
    pub fn new(
        store: Store,
        model_request_permit: Arc<Semaphore>,
        settings_snapshot_lock: Arc<Mutex<()>>,
    ) -> Self {
        let (preview_deltas, _) = broadcast::channel(PREVIEW_DELTA_CHANNEL_CAPACITY);
        Self::with_preview_deltas(
            store,
            model_request_permit,
            settings_snapshot_lock,
            preview_deltas,
        )
    }

    /// 创建与 API SSE 层共用同一临时预览广播的调度器。
    #[must_use]
    pub fn with_preview_deltas(
        store: Store,
        model_request_permit: Arc<Semaphore>,
        settings_snapshot_lock: Arc<Mutex<()>>,
        preview_deltas: broadcast::Sender<PreviewDelta>,
    ) -> Self {
        Self {
            store,
            model_request_permit,
            settings_snapshot_lock,
            lanes: Arc::new(Mutex::new(HashMap::new())),
            in_flight_requests: Arc::new(Mutex::new(HashMap::new())),
            preview_deltas,
        }
    }

    /// 当前 Run 的辅助工具能力与余额。
    ///
    /// `query_knowledge` 只有在 Embedding 已配置、索引确有可检索条目、且本项目还剩
    /// 预算时才会被宣告；未宣告的调用仍会返回明确的工具错误，而不是静默降级。
    async fn agent_auxiliary_capabilities(
        &self,
        run_id: RunId,
    ) -> Result<(bool, QuotaBalance), SchedulerError> {
        let balance = self.store.quota_balance(run_id).await?;
        let available = balance.remaining > 0
            && self
                .store
                .knowledge_index_coverage(&[])
                .await?
                .ready_entries
                > 0;
        Ok((available, balance))
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

    /// 退出服务时的本地收尾：取消所有在途模型请求并关闭各稿本的命令循环。
    ///
    /// 已发往外部服务的调用是否取消成功不作保证，因此这里不写任何持久状态；
    /// 调用方随后用 [`nekowriter_store::InterruptionSource::ProcessShutdown`] 把在途
    /// Run 与请求收束到可解释的终态。等待被取消的任务有时间上限，避免卡住的
    /// 网络调用把进程退出拖成孤儿进程。
    pub async fn cancel_in_flight(&self) {
        let task_handles = {
            let mut in_flight = self.in_flight_requests.lock().await;
            in_flight
                .drain()
                .map(|(_, request)| request.task_handle)
                .collect::<Vec<_>>()
        };
        let cancelled = task_handles.len();
        for task_handle in &task_handles {
            task_handle.abort();
        }
        let settled = async {
            for task_handle in task_handles {
                let _ = task_handle.await;
            }
        };
        if tokio::time::timeout(CANCEL_SETTLE_TIMEOUT, settled)
            .await
            .is_err()
        {
            warn!("有模型请求任务未在退出超时内结束，继续收尾");
        }
        // 丢弃发送端即结束各稿本命令循环；循环持有的 Run 状态已在 Store 中收束。
        self.lanes.lock().await.clear();
        if cancelled > 0 {
            info!(cancelled, "已取消在途模型请求");
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
                version_id,
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
                version_id,
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
                version_id,
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
                version_id,
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
                    warn!(run_id = %task.run_id, kind = task.kind(), "模型调度器已关闭，无法取得模型许可");
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
        // 项目设置也在每次取得许可后重新解析：有效模型引用优先取项目覆盖，思考
        // 等级取项目设置。这样保存设置后的下一次请求自然按新值构建，而已 dispatch
        // 的请求快照不会被原地改写。
        let project_id = self.store.project_id_for_version(version_id).await?;
        let project_settings = self.store.get_project_settings(project_id).await?;
        let effective = resolve_effective_model(&settings, &project_settings);
        // 能力资料必须与有效模型同源：项目可以引用另一个模型，那个模型的能力不能
        // 用系统模型的资料代替。不满足时在 dispatch（封存请求与占用网络许可）之前
        // 就失败，并且错误消息必须指明是哪个模型。
        if let Some(problem) = pre_dispatch_capability_problem(
            *effective.capabilities,
            project_settings.thinking_level,
        ) {
            let message = capability_problem_message(problem, &effective.describe());
            return Err(match problem {
                CapabilityProblem::Tools => SchedulerError::Context(message),
                CapabilityProblem::Thinking => SchedulerError::ThinkingUnsupported(message),
            });
        }
        // 密钥在取得全局网络许可之后、真正构建请求之前从库里读出，不进入任何
        // 请求快照或跨异步边界传递的结构。
        let api_key = self.store.system_model_api_key().await?;
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
                // 本轮必须提交的产物项与系统消息里的合同同源，必须在把快照内容
                // 移出之前算好。
                let required_artifacts = required_preparation_artifacts(&snapshot)?;
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
                let (knowledge_available, quota_balance) =
                    self.agent_auxiliary_capabilities(task.run_id).await?;
                let stage = PhaseSubmissionKind::Preparation { required_artifacts };
                let system_instructions = stage_system_instructions(&prompt, &stage);
                let context = build_context_with_history(
                    system_instructions,
                    preparation_agent_tools(
                        submission_tool(&stage),
                        knowledge_available,
                        quota_balance.remaining,
                    ),
                    Vec::new(),
                    vec![CurrentContextInput {
                        role: ContextMessageRole::User,
                        content: prompt.user.clone(),
                        source_revision_id: None,
                    }],
                    stage_agent_history_messages(task.run_id, persisted_messages, &[])?,
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
                let (knowledge_available, quota_balance) =
                    self.agent_auxiliary_capabilities(task.run_id).await?;
                let stage = PhaseSubmissionKind::ChapterOutline { chapter_id };
                let system_instructions = stage_system_instructions(&prompt, &stage);
                let context = build_context_with_history(
                    system_instructions,
                    preparation_agent_tools(
                        submission_tool(&stage),
                        knowledge_available,
                        quota_balance.remaining,
                    ),
                    completed_stages,
                    vec![CurrentContextInput {
                        role: ContextMessageRole::User,
                        content: prompt.user.clone(),
                        source_revision_id: None,
                    }],
                    stage_agent_history_messages(task.run_id, persisted_messages, &[])?,
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
                let current_messages = stage_agent_history_messages(
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
                let stage = PhaseSubmissionKind::Chapter { chapter_id };
                let system_instructions = stage_system_instructions(&prompt, &stage);
                let context = build_context_with_history(
                    system_instructions,
                    chapter_body_tools(submission_tool(&stage)),
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
                let user_content = delegated_review_user_content(
                    prompt.user.clone(),
                    &review.user_notes,
                    &review.citations,
                );
                let system_instructions =
                    stage_system_instructions(&prompt, &PhaseSubmissionKind::Review);
                // 审核子 Run 不重放整个历史，但必须重放协议纠正的配对：被拒的那次
                // 调用、它的错误工具结果与纠正提示。否则重排后的下一轮请求与第一次
                // 完全相同，而模型会原样再错一次。
                let persisted_messages = self.store.list_agent_messages(task.run_id).await?;
                let persist_initial_messages = persisted_messages.is_empty();
                let context = build_context_with_history(
                    system_instructions,
                    vec![submission_tool(&PhaseSubmissionKind::Review)],
                    Vec::new(),
                    vec![CurrentContextInput {
                        role: ContextMessageRole::User,
                        content: user_content,
                        source_revision_id: None,
                    }],
                    stage_agent_history_messages(task.run_id, persisted_messages, &[])?,
                    max_context_tokens,
                    reserved_output_tokens,
                )?;
                (
                    prompt,
                    prompt_source,
                    context,
                    persist_initial_messages,
                    None,
                    None,
                )
            }
        };
        // 每个阶段的终态提交都是一次声明过的提交工具调用：不再需要
        // `response_format`，因为「本轮必须给出 JSON」曾让 OpenAI 兼容服务直接跳过
        // 工具。请求只声明工具，模型可以选择业务工具，也可以调用本阶段的提交工具。
        let request = context.model_request(project_settings.thinking_level);
        let messages = if persist_initial_messages {
            agent_messages_for_new_stage_request(&request)?
        } else {
            Vec::new()
        };
        let provider = ChatCompletionsAdapter::new(ChatCompletionsConfig::new(
            &settings.api_base_url,
            effective.model_id.to_owned(),
            api_key,
            *effective.capabilities,
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
                            "modelOverrideId": project_settings
                                .model
                                .as_ref()
                                .map(|model| model.model_id.clone()),
                            "thinkingLevel": project_settings.thinking_level.as_str(),
                            "auxiliaryToolBudget": project_settings
                                .auxiliary_tool_budget
                                .to_string(),
                        },
                        "effectiveModelId": effective.model_id,
                        "effectiveModelSource": effective.source.as_str(),
                        "effectiveCapabilities": effective.capabilities,
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
        let preview_deltas = self.preview_deltas.clone();
        let callback_sender = sender.clone();
        let task_handle = tokio::spawn(async move {
            if start_receiver.await.is_err() {
                drop(permit);
                return;
            }
            let mut text_offset = 0_i64;
            let mut tool_states = HashMap::<usize, PreviewToolState>::new();
            let result = provider
                .execute_with(&request, |event| match event {
                    ModelEvent::TextDelta { content } if !content.is_empty() => {
                        let offset = text_offset;
                        let length = i64::try_from(content.chars().count()).unwrap_or(i64::MAX);
                        text_offset = text_offset.saturating_add(length);
                        let _ = preview_deltas.send(PreviewDelta {
                            version_id: task.version_id,
                            request_id,
                            tool: None,
                            offset,
                            content,
                        });
                    }
                    ModelEvent::ToolCallDelta {
                        index,
                        id,
                        name,
                        arguments_delta,
                    } => {
                        let (offset, tool) = {
                            let state = tool_states.entry(index).or_default();
                            if id.is_some() {
                                state.id = id;
                            }
                            if name.is_some() {
                                state.name = name;
                            }
                            let offset = state.offset;
                            let length =
                                i64::try_from(arguments_delta.chars().count()).unwrap_or(i64::MAX);
                            state.offset = state.offset.saturating_add(length);
                            (
                                offset,
                                PreviewTool {
                                    index,
                                    id: state.id.clone(),
                                    name: state.name.clone(),
                                },
                            )
                        };
                        let _ = preview_deltas.send(PreviewDelta {
                            version_id: task.version_id,
                            request_id,
                            tool: Some(tool),
                            offset,
                            content: arguments_delta,
                        });
                    }
                    ModelEvent::TextDelta { .. }
                    | ModelEvent::Usage { .. }
                    | ModelEvent::Completed { .. }
                    | ModelEvent::Failed { .. } => {}
                })
                .await;
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
                self.handle_preparation_completion(
                    task.version_id,
                    task.run_id,
                    request_id,
                    request,
                    result,
                )
                .await
            }
            ScheduledStage::ChapterOutline { chapter_id } => {
                self.handle_chapter_outline_completion(
                    chapter_id,
                    task.run_id,
                    request_id,
                    request,
                    result,
                )
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
        version_id: VersionId,
        run_id: RunId,
        request_id: Uuid,
        request: &ModelRequest,
        result: Result<ModelCompletion, ProviderError>,
    ) -> bool {
        match result {
            Ok(completion) => {
                let provider_request_id = completion.provider_request_id.clone();
                let assistant_message = assistant_message_from_completion(&completion, request_id);
                let usage = completion_usage(&completion);
                // 同一轮请求的构建与解析必须用同一份必需项清单。
                let required_artifacts = match self.store.get_preparation_snapshot(version_id).await
                {
                    Ok(snapshot) => match required_preparation_artifacts(&snapshot) {
                        Ok(required) => required,
                        Err(error) => {
                            self.persist_model_failure(
                                request_id,
                                provider_request_id,
                                Some(assistant_message),
                                "preparation_scope",
                                error.to_string(),
                                false,
                            )
                            .await;
                            return false;
                        }
                    },
                    Err(error) => {
                        self.persist_model_failure(
                            request_id,
                            provider_request_id,
                            Some(assistant_message),
                            "preparation_scope",
                            error.to_string(),
                            false,
                        )
                        .await;
                        return false;
                    }
                };

                match classify_model_completion(
                    PhaseSubmissionKind::Preparation { required_artifacts },
                    request,
                    completion,
                ) {
                    Ok(AgentRoundOutcome::Submission(PhaseSubmission::Preparation(submission))) => {
                        match self
                            .store
                            .commit_preparation_model_submission(
                                request_id,
                                provider_request_id.clone(),
                                usage.clone(),
                                assistant_message.clone(),
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
                                return self
                                    .record_phase_submission_rejection(
                                        ProtocolCorrectionTarget::WritingRun,
                                        provider_request_id,
                                        usage,
                                        assistant_message,
                                        SUBMIT_PREPARATION_TOOL,
                                        "PHASE_SUBMISSION_REJECTED",
                                        &error,
                                    )
                                    .await;
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
                    Ok(AgentRoundOutcome::ToolCall(call)) if call.name == TOOL_QUERY_KNOWLEDGE => {
                        self.execute_knowledge_tool_call(
                            run_id,
                            request_id,
                            provider_request_id,
                            usage,
                            call,
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
                        self.record_protocol_correction(
                            ProtocolCorrectionTarget::WritingRun,
                            provider_request_id,
                            usage,
                            assistant_message,
                            &error,
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
        run_id: RunId,
        request_id: Uuid,
        request: &ModelRequest,
        result: Result<ModelCompletion, ProviderError>,
    ) -> bool {
        match result {
            Ok(completion) => {
                let provider_request_id = completion.provider_request_id.clone();
                let assistant_message = assistant_message_from_completion(&completion, request_id);
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
                                usage.clone(),
                                assistant_message.clone(),
                                submission.content,
                            )
                            .await
                        {
                            Ok(_) => {
                                info!(%request_id, %chapter_id, "章节细纲模型产物已提交，等待审核");
                                false
                            }
                            Err(error) => {
                                return self
                                    .record_phase_submission_rejection(
                                        ProtocolCorrectionTarget::WritingRun,
                                        provider_request_id,
                                        usage,
                                        assistant_message,
                                        SUBMIT_CHAPTER_OUTLINE_TOOL,
                                        "PHASE_SUBMISSION_REJECTED",
                                        &error,
                                    )
                                    .await;
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
                    Ok(AgentRoundOutcome::ToolCall(call)) if call.name == TOOL_QUERY_KNOWLEDGE => {
                        self.execute_knowledge_tool_call(
                            run_id,
                            request_id,
                            provider_request_id,
                            usage,
                            call,
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
                        self.record_protocol_correction(
                            ProtocolCorrectionTarget::WritingRun,
                            provider_request_id,
                            usage,
                            assistant_message,
                            &error,
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

    /// 执行一次 `query_knowledge`（细则 §4.2/§7.1/§7.3）。
    ///
    /// 顺序固定：先看是否已经有过结果（重放不重复检索、不重复扣费），再算工具结果，
    /// 最后把结果与工作流推进写进同一个事务。参数无效、没有可用检索、预算耗尽都不
    /// 扣费；一旦真的发起检索，无结果或失败仍计已消耗的 1 次。
    async fn execute_knowledge_tool_call(
        &self,
        run_id: RunId,
        request_id: Uuid,
        provider_request_id: Option<String>,
        usage: Value,
        call: ModelToolCall,
    ) -> bool {
        let provider_call_id = call.id.clone();
        match self
            .store
            .free_agent_tool_result(request_id, &provider_call_id)
            .await
        {
            // 同一次回调的重放：结果已在库里，不再投递新请求。
            Ok(Some(_)) => return false,
            Ok(None) => {}
            Err(error) => {
                self.persist_model_failure(
                    request_id,
                    provider_request_id,
                    None,
                    "knowledge_tool",
                    error.to_string(),
                    false,
                )
                .await;
                return false;
            }
        }

        let Some((failed, error_code, result)) = self
            .resolve_knowledge_tool_result(
                run_id,
                request_id,
                provider_request_id.as_deref(),
                &provider_call_id,
                &call.arguments,
            )
            .await
        else {
            return false;
        };

        match self
            .store
            .execute_free_agent_tool_call(
                request_id,
                provider_request_id,
                usage,
                FreeAgentToolCall {
                    provider_call_id,
                    name: TOOL_QUERY_KNOWLEDGE.to_owned(),
                    arguments: call.arguments,
                    result,
                    failed,
                    error_code: error_code.map(str::to_owned),
                },
            )
            .await
        {
            Ok(execution) => execution.requeue,
            Err(error) => {
                self.persist_model_failure(
                    request_id,
                    None,
                    None,
                    "knowledge_tool",
                    error.to_string(),
                    false,
                )
                .await;
                false
            }
        }
    }

    /// 算一次 `query_knowledge` 的工具结果；`None` 表示调用本身无法落库（失败已记录）。
    async fn resolve_knowledge_tool_result(
        &self,
        run_id: RunId,
        request_id: Uuid,
        provider_request_id: Option<&str>,
        provider_call_id: &str,
        arguments: &Value,
    ) -> Option<(bool, Option<&'static str>, Value)> {
        let arguments = match crate::knowledge::parse_knowledge_query_arguments(arguments) {
            Ok(arguments) => arguments,
            Err(error) => {
                return Some((
                    true,
                    Some("TOOL_ARGUMENTS_INVALID"),
                    knowledge_tool_error_result(
                        provider_call_id,
                        "TOOL_ARGUMENTS_INVALID",
                        &error.to_string(),
                    ),
                ));
            }
        };
        // 执行前验证：没有可用的 Embedding 属于「工具不可用」，不扣费。
        if let Err(error) = crate::knowledge::require_knowledge_query(&self.store).await {
            return Some((
                true,
                Some("EMBEDDING_NOT_CONFIGURED"),
                knowledge_tool_error_result(
                    provider_call_id,
                    "EMBEDDING_NOT_CONFIGURED",
                    &error.to_string(),
                ),
            ));
        }
        match self
            .store
            .charge_auxiliary_tool(run_id, provider_call_id, TOOL_QUERY_KNOWLEDGE)
            .await
        {
            Err(error) => {
                self.persist_model_failure(
                    request_id,
                    provider_request_id.map(str::to_owned),
                    None,
                    "knowledge_tool",
                    error.to_string(),
                    false,
                )
                .await;
                None
            }
            Ok(AuxiliaryToolCharge::Exhausted(balance)) => Some((
                true,
                Some("QUOTA_EXHAUSTED"),
                knowledge_tool_error_result(
                    provider_call_id,
                    "QUOTA_EXHAUSTED",
                    &format!(
                        "本项目辅助工具预算已用完（已用 {} / 上限 {}）。你可以继续撰写、询问用户，或延后检索。",
                        balance.used, balance.limit
                    ),
                ),
            )),
            Ok(AuxiliaryToolCharge::Charged(_) | AuxiliaryToolCharge::AlreadyCharged(_)) => {
                match crate::knowledge::run_knowledge_query(&self.store, arguments).await {
                    Ok(search) => Some((
                        false,
                        None,
                        serde_json::to_value(&search).unwrap_or(Value::Null),
                    )),
                    Err(error) => {
                        warn!(%request_id, tool_call_id = %provider_call_id, error = %error, "知识检索工具执行失败");
                        Some((
                            true,
                            Some("KNOWLEDGE_SEARCH_FAILED"),
                            knowledge_tool_error_result(
                                provider_call_id,
                                "KNOWLEDGE_SEARCH_FAILED",
                                &error.to_string(),
                            ),
                        ))
                    }
                }
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
                requeue,
                result,
                ..
            }) => {
                if newly_resolved {
                    info!(%request_id, tool_call_id = %provider_call_id, status = %result["status"], "ask_user 参数无效，已作为工具结果回送");
                }
                newly_resolved && requeue
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
                                execution.requeue
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
                                usage.clone(),
                                assistant_message.clone(),
                                submission.summary,
                            )
                            .await
                        {
                            Ok(snapshot) => {
                                if snapshot.workflow.execution == ExecutionStatus::Queued {
                                    info!(%request_id, %chapter_id, "章节提交因未投递意见返回正文工具循环");
                                    return true;
                                }
                                if snapshot.workflow.execution == ExecutionStatus::Stopped {
                                    info!(%request_id, %chapter_id, "章节提交连续不推进，已达到工程执行限制并停止");
                                } else {
                                    info!(%request_id, %chapter_id, "章节正文模型提交已进入整章审核");
                                }
                            }
                            Err(error) => {
                                return self
                                    .record_phase_submission_rejection(
                                        ProtocolCorrectionTarget::WritingRun,
                                        provider_request_id,
                                        usage,
                                        assistant_message,
                                        SUBMIT_CHAPTER_TOOL,
                                        "PHASE_SUBMISSION_REJECTED",
                                        &error,
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
                        self.record_protocol_correction(
                            ProtocolCorrectionTarget::WritingRun,
                            provider_request_id,
                            usage,
                            assistant_message,
                            &error,
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

    /// 审核子 Run 只接受一次提交工具的结论。不合合同的提交与写作 lane 同一形状：
    /// 配对错误工具结果 + 纠正 + 连续协议错误计数，模型在限额内重提；传输失败、
    /// 提交写库失败，或连续错误达到上限时才按原有行为收束（子 Run 失败、同一
    /// Review 重新打开供用户重试或人工决定）。父 Run 始终留在等待审核状态。
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
                                usage.clone(),
                                assistant_message.clone(),
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
                                return self
                                    .record_phase_submission_rejection(
                                        ProtocolCorrectionTarget::DelegatedReview { review_id },
                                        provider_request_id,
                                        usage,
                                        assistant_message,
                                        SUBMIT_REVIEW_TOOL,
                                        "INVALID_REVIEW_RESULT",
                                        &error,
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
                    // 不符合合同的提交与写作 lane 同一形状：配对错误工具结果 +
                    // 纠正提示 + 计数，模型在限额内可以重提；只有真正无法修正的
                    // 传输失败才会让这次委托失败、Review 重新开放。
                    Err(error) => {
                        return self
                            .record_protocol_correction(
                                ProtocolCorrectionTarget::DelegatedReview { review_id },
                                provider_request_id,
                                usage,
                                assistant_message,
                                &error,
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
    /// 纠正提示都会进入被拒回合所属 Run 的消息账本，因此下一次请求可以在不破坏调用
    /// 协议的前提下继续。Store 负责连续计数与收束（写作 Run 停止等用户显式继续、
    /// 审核子 Run 失败并重新开放 Review），返回值表示是否应当立刻重排同一 Run /
    /// 同一次委托。
    async fn record_protocol_correction(
        &self,
        target: ProtocolCorrectionTarget,
        provider_request_id: Option<String>,
        usage: Value,
        assistant_message: NewAgentMessage,
        error: &AgentRunnerError,
    ) -> bool {
        let request_id = assistant_message.request_id;
        let (correction, tool_result_code) = protocol_correction(error);
        match self
            .store
            .record_model_protocol_correction(
                target,
                RejectedRoundRecord {
                    provider_request_id,
                    usage,
                    assistant_message,
                    error: json!({ "kind": "protocol_error", "message": error.to_string() }),
                    correction,
                    tool_result_code: tool_result_code.to_owned(),
                },
            )
            .await
        {
            Ok(result) => {
                if result.newly_applied {
                    if result.requeue {
                        info!(
                            request_id = ?request_id,
                            lane = ?target,
                            consecutive_protocol_errors = result.consecutive_protocol_errors,
                            "模型协议错误已保存，已要求同一 Run 纠正"
                        );
                    } else {
                        warn!(
                            request_id = ?request_id,
                            lane = ?target,
                            consecutive_protocol_errors = result.consecutive_protocol_errors,
                            "连续模型协议错误达到上限，已按 lane 收束"
                        );
                    }
                }
                result.requeue
            }
            Err(error) => {
                warn!(
                    request_id = ?request_id,
                    lane = ?target,
                    error = %error,
                    "未能写入模型协议纠正结果"
                );
                false
            }
        }
    }

    /// 把被业务守卫拒绝的阶段提交记为被拒回合：配对错误工具结果 + 纠正提示，同一
    /// Run（或同一次委托）立即重排。
    ///
    /// 与 [`Self::record_protocol_correction`] 一样，原始响应与用量会写进消息账本与
    /// 请求行；它不参与连续协议错误计数——被拒的是提交内容，不是模型协议。因此这一轮
    /// 不会把 Run 停在协议错误上限上：非进展回合的工程上限由请求额度承担。返回值表示
    /// 是否应当立刻重排同一 Run / 同一次委托。
    #[allow(clippy::too_many_arguments)]
    async fn record_phase_submission_rejection(
        &self,
        target: ProtocolCorrectionTarget,
        provider_request_id: Option<String>,
        usage: Value,
        assistant_message: NewAgentMessage,
        tool: &str,
        tool_result_code: &'static str,
        error: &StoreError,
    ) -> bool {
        let request_id = assistant_message.request_id;
        let (correction, tool_result_code) =
            phase_submission_rejection(tool, tool_result_code, error);
        match self
            .store
            .record_phase_submission_rejection(
                target,
                RejectedRoundRecord {
                    provider_request_id,
                    usage,
                    assistant_message,
                    error: json!({ "kind": "submission_commit", "message": error.to_string() }),
                    correction,
                    tool_result_code: tool_result_code.to_owned(),
                },
            )
            .await
        {
            Ok(result) => {
                if result.newly_applied {
                    info!(
                        request_id = ?request_id,
                        lane = ?target,
                        consecutive_protocol_errors = result.consecutive_protocol_errors,
                        "阶段提交被业务守卫拒绝，已要求同一 Run 按纠正重提"
                    );
                }
                result.requeue
            }
            Err(error) => {
                warn!(
                    request_id = ?request_id,
                    lane = ?target,
                    error = %error,
                    "未能写入被拒阶段提交的纠正结果"
                );
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

/// 把一次已完成请求的用量封存成持久请求接受的 JSON 形状。
fn completion_usage(completion: &ModelCompletion) -> Value {
    serde_json::to_value(completion.persisted_usage()).expect("用量记录只包含可序列化标量")
}

/// 把已校验的委托关注点与引用拼成审核子 Run 的用户内容。
///
/// 引用只以引文文本进入提示词：审核 Agent 不需要也不应该拿到文档、修订或工具调用
/// ID，引用在委托时已经逐字复核过。
fn delegated_review_user_content(
    user_prompt: String,
    user_notes: &str,
    citations: &[FeedbackCitation],
) -> String {
    let mut content = user_prompt;
    if !user_notes.trim().is_empty() {
        content.push_str("\n\n用户审核关注点：");
        content.push_str(user_notes);
    }
    if !citations.is_empty() {
        content.push_str("\n\n用户引用的原文片段：");
        for citation in citations {
            let (label, quote) = match citation {
                FeedbackCitation::Document { range, .. } => ("正文引用", range.quote.as_str()),
                FeedbackCitation::Candidate {
                    candidate_range, ..
                } => ("待审候选引用", candidate_range.quote.as_str()),
            };
            content.push_str(&format!("\n- {label}：「{quote}」"));
        }
    }
    content
}

/// 阶段角色 = 提示词 + 本阶段的提交结构。提交结构由协议层拥有（
/// [`submission_contract`]），这里只负责把它接到系统消息上，避免提示词与校验各自
/// 漂移；模型无法自行推断的章节级 `chapter_id` 也由此进入请求。
fn stage_system_instructions(
    prompt: &RenderedBuiltinPrompt,
    stage: &PhaseSubmissionKind,
) -> String {
    format!("{}\n\n{}", prompt.system, submission_contract(stage))
}

/// 本轮准备阶段真正需要模型提交的产物项。
///
/// 首次生成要求四项齐全；修订轮只包含审核已拒绝或已失效的项 —— Store 在修订轮
/// 拒绝提交其它项，所以合同与解析器必须用同一份清单，否则模型照合同提交反而会失败。
fn required_preparation_artifacts(
    snapshot: &nekowriter_core::PreparationSnapshot,
) -> Result<Vec<PreparationArtifactKind>, SchedulerError> {
    if snapshot.workflow.stage == WorkflowStage::PreparingGenerating {
        return Ok(vec![
            PreparationArtifactKind::Background,
            PreparationArtifactKind::Research,
            PreparationArtifactKind::Characters,
            PreparationArtifactKind::Outline,
        ]);
    }
    let required = snapshot
        .artifacts
        .iter()
        .filter(|artifact| {
            matches!(
                artifact.review.as_ref().map(|review| &review.status),
                Some(&ReviewStatus::Rejected | &ReviewStatus::Stale)
            )
        })
        .map(|artifact| artifact.kind)
        .collect::<Vec<_>>();
    if required.is_empty() {
        return Err(SchedulerError::Context(
            "准备修订轮没有任何需要修订的产物，拒绍发送模型请求".to_owned(),
        ));
    }
    Ok(required)
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

/// 模型这一轮响应不符合协议时的纠正提示与工具结果原因码。
///
/// 写作 lane 与审核 lane 共用这一处：同一个失败在两条路径上必须得到同一句话、
/// 同一个原因码，否则模型看到的纠正会随阶段漂移。每一句都要点名该用哪个工具、
/// 具体哪里不合格（缺哪一项产物、参数从哪一字符开始不完整、哪个字段形状不对）。
fn protocol_correction(error: &AgentRunnerError) -> (String, &'static str) {
    const CORRECTION: &str = "上一轮响应不符合当前阶段的协议，未应用任何阶段提交或工具副作用。请重新严格按当前阶段要求返回：恰好一次已声明的工具调用；阶段结果只能通过本阶段的提交工具提交，不要在工具调用之外附带普通文本。";
    // 并行工具调用是可纠正的协议偏差：整批不执行，每个调用 ID 都要拿到配对的
    // 错误结果，并明确要求模型改为串行调用。
    const PARALLEL_CORRECTION: &str = "上一轮响应同时给出了多个工具调用，本批次全部未执行。请改为串行：每轮只调用一个已声明的工具，等结果返回后再进行下一次调用。";
    match error {
        AgentRunnerError::ParallelToolCalls { .. } => (
            PARALLEL_CORRECTION.to_owned(),
            "PARALLEL_TOOL_CALLS_NOT_ALLOWED",
        ),
        // 普通文本不是提交：说清该用哪个工具，模型才能在下一轮换个动作。
        AgentRunnerError::MissingSubmissionToolCall { tool, .. } => (
            format!(
                "上一轮没有调用本阶段的提交工具 `{tool}`，普通文本不算提交。请恰好调用一次 `{tool}` 提交本阶段结果。"
            ),
            "PROTOCOL_ERROR",
        ),
        // 提交工具的参数校验失败（含被端点截断、无法解码的那一轮）：工具结果
        // 与本段纠正都带上解析器给出的具体原因（缺少哪一项产物、参数从哪一
        // 字符开始不完整、哪个字段形状不对、哪个取值非法且合法值是什么），模型
        // 可以在同一次调用里按原因改对后重试。
        AgentRunnerError::InvalidPhaseSubmission {
            tool,
            error: reason,
            ..
        } => (
            format!(
                "上一轮对 `{tool}` 的调用未通过本阶段校验：{reason}\n请按上面的原因修正后重新调用 `{tool}`；已提交且合法的内容可以原样复用，不必重写。"
            ),
            "PHASE_SUBMISSION_INVALID",
        ),
        // 业务工具的调用参数不可解码：同样给一条说明原因的配对结果，让模型重写
        // 一次合法的调用，而不是整轮以传输错误结束。
        AgentRunnerError::UndecodableToolArguments { tool, reason } => (
            format!(
                "上一轮对 `{tool}` 的调用参数不可用：{reason}\n该调用未执行；请把参数写成完整的 JSON object 后重新调用 `{tool}`。"
            ),
            "TOOL_ARGUMENTS_INVALID",
        ),
        other => (
            format!("{CORRECTION}\n本次未通过的原因：{other}"),
            "PROTOCOL_ERROR",
        ),
    }
}

/// 阶段提交被业务守卫拒绝时的纠正提示与理由码。
///
/// 提示必须点名本阶段的提交工具与真实原因：模型要能据此把同一件事重做对，而不是只
/// 看到一句「提交失败」。`error` 来自 Store 的业务守卫，文本原样进入工具结果与提示，
/// 因此「缺什么、哪里不合法」对模型是可读的。
fn phase_submission_rejection(
    tool: &str,
    tool_result_code: &'static str,
    error: &StoreError,
) -> (String, &'static str) {
    (
        format!(
            "上一轮对 `{tool}` 的提交被本阶段守卫拒绝，本轮没有产生任何正式结果：{error}\n请按上面的原因修正后重新调用 `{tool}`；此前已提交且合法的内容可以原样复用，不必重写。"
        ),
        tool_result_code,
    )
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

/// 一个稳定可读的工具错误结果；模型可以据此纠正参数或改变做法，而不会看到一次
/// 静默消失的调用。
fn knowledge_tool_error_result(provider_call_id: &str, code: &str, message: &str) -> Value {
    json!({
        "status": "error",
        "tool_call_id": provider_call_id,
        "code": code,
        "message": message,
    })
}

/// 准备 Agent 与细纲 Agent 的工具白名单：`ask_user` 始终可用，`query_knowledge`
/// 只在 Embedding 已配置且检索确实可用时宣告，并在描述里带上当前余额。
///
/// 未宣告的工具被旧请求调用时不会被静默降级：调用方会返回明确的工具错误。
fn preparation_agent_tools(
    submission: AgentToolDefinition,
    knowledge_available: bool,
    remaining_quota: i64,
) -> Vec<AgentToolDefinition> {
    let mut tools = vec![submission];
    tools.extend(ask_user_tools());
    if knowledge_available {
        tools.push(query_knowledge_tool(remaining_quota));
    }
    tools
}

fn query_knowledge_tool(remaining_quota: i64) -> AgentToolDefinition {
    AgentToolDefinition {
        name: TOOL_QUERY_KNOWLEDGE.to_owned(),
        description: format!(
            "用语义检索在本稿作家自己的知识库里查找设定与资料。查询应为自然语言；可选 spaceIds 限定 Space，topK 默认为 5、最大为 10。每次真实检索消耗 1 次辅助工具预算，当前剩余 {remaining_quota} 次。返回结果包含标题、原文块、条目/修订/块 ID 与相关分值（分值不是事实置信度）。同一问题不要重复调用。"
        ),
        parameters: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["query"],
            "properties": {
                "query": { "type": "string", "minLength": 1 },
                "spaceIds": {
                    "type": "array",
                    "items": { "type": "string", "minLength": 1 }
                },
                "topK": { "type": "integer", "minimum": 1, "maximum": 10 }
            }
        }),
    }
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

fn chapter_body_tools(submission: AgentToolDefinition) -> Vec<AgentToolDefinition> {
    vec![
        submission,
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

/// 将当前 Run 的不可变消息账本还原为下一轮请求所需的 tool-call 配对。
///
/// 四条 lane 共用这一个还原器：三条写作 lane 回放工具调用与它被拒的配对结果，
/// 审核 lane 回放同一种配对（它同样不重放初始输入）。初始 system/user 输入由每轮
/// 重新渲染的 prompt 提供，所以只回放 assistant/tool 历史；这样不会在每次工具后
/// 不断重复初始指令和正文快照。任何损坏的 envelope 都拒绝继续，而不是猜测出
/// 一个会破坏调用配对的模型消息。
fn stage_agent_history_messages(
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
                // 失败后写入的明确纠正消息需要留在该 Run 的历史中。
                AgentMessageRole::System => None,
                AgentMessageRole::User => {
                    decode_stage_protocol_correction_message(run_id, sequence, persisted.content)
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
                    decode_stage_assistant_message(run_id, sequence, persisted.content).map(
                        |message| UnfoldedStageMessage {
                            source_run_id: run_id,
                            sequence,
                            message,
                        },
                    ),
                ),
                AgentMessageRole::Tool => Some(
                    decode_stage_tool_message(
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

fn decode_stage_protocol_correction_message(
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
            invalid_stage_history_message(
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

fn decode_stage_assistant_message(
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
                        invalid_stage_history_message(run_id, sequence, error.to_string())
                    })?;
            let tool_calls: Vec<ModelToolCall> =
                serde_json::from_value(content.get("toolCalls").cloned().ok_or_else(|| {
                    invalid_stage_history_message(
                        run_id,
                        sequence,
                        "assistant_tool_calls 缺少 toolCalls".to_owned(),
                    )
                })?)
                .map_err(|error| {
                    invalid_stage_history_message(run_id, sequence, error.to_string())
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
                    invalid_stage_history_message(
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
        other => Err(invalid_stage_history_message(
            run_id,
            sequence,
            format!("assistant 消息 kind 不支持：{other}"),
        )),
    }
}

fn decode_stage_tool_message(
    run_id: RunId,
    sequence: i64,
    tool_call_id: Option<String>,
    content: Value,
    feedback: &[Value],
) -> Result<ModelRequestMessage, SchedulerError> {
    if message_kind(&content, run_id, sequence)? != "tool_result" {
        return Err(invalid_stage_history_message(
            run_id,
            sequence,
            "tool 消息 kind 必须为 tool_result".to_owned(),
        ));
    }
    let tool_call_id = tool_call_id.ok_or_else(|| {
        invalid_stage_history_message(run_id, sequence, "tool 消息缺少 tool_call_id".to_owned())
    })?;
    let content = content
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            invalid_stage_history_message(
                run_id,
                sequence,
                "tool_result 缺少字符串 content".to_owned(),
            )
        })?
        .to_owned();
    let mut result: Value = serde_json::from_str(&content).map_err(|error| {
        invalid_stage_history_message(
            run_id,
            sequence,
            format!("tool_result 不是有效 JSON：{error}"),
        )
    })?;
    let result_object = result.as_object_mut().ok_or_else(|| {
        invalid_stage_history_message(
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
            invalid_stage_history_message(
                run_id,
                sequence,
                "tool_result.feedback 必须是 JSON array".to_owned(),
            )
        })?;
    result_feedback.extend(feedback.iter().cloned());
    let content = serde_json::to_string(&result).map_err(|error| {
        invalid_stage_history_message(
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

fn message_kind(content: &Value, run_id: RunId, sequence: i64) -> Result<&str, SchedulerError> {
    content.get("kind").and_then(Value::as_str).ok_or_else(|| {
        invalid_stage_history_message(run_id, sequence, "消息缺少字符串 kind".to_owned())
    })
}

fn invalid_stage_history_message(run_id: RunId, sequence: i64, reason: String) -> SchedulerError {
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
        FeedbackStatus, HumanReviewDecision, MAX_CONSECUTIVE_PROTOCOL_ERRORS,
        ModelCapabilityProfile, NewChapterGuidance, NewNovelBrief, NewProject, NewProjectSettings,
        PreparationArtifactInput, PreparationArtifactKind, ProjectModelReference, ThinkingLevel,
        UserQuestionAnswer, UserQuestionAnswers, WorkflowStage,
    };
    use nekowriter_store::{
        AgentMessageRole, AgentQuestionToolCall, ChapterBodyAgentToolCall, CredentialPatch,
        ModelRequestResolution, ModelRequestStatus, NewAgentMessage, NewModelRequest,
        NewSystemModelSettings,
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

    /// 准备阶段请求里宣告的提交工具：与调度器真正宣告的那一个同源。
    fn preparation_submission_tool() -> nekowriter_agent::AgentToolDefinition {
        submission_tool(&PhaseSubmissionKind::Preparation {
            required_artifacts: PreparationArtifactKind::ALL.to_vec(),
        })
    }

    /// 测试用的正文阶段提交工具调用消息：与账本里真实的 assistant 工具调用同形。
    ///
    /// 阶段提交是一次声明过的工具调用，账本只接受这种形态，测试也不再能用散文
    /// JSON 直接提交。
    fn body_submission_message(request_id: Uuid, summary: &str) -> NewAgentMessage {
        NewAgentMessage {
            role: AgentMessageRole::Assistant,
            content: json!({
                "kind": "assistant_tool_calls",
                "content": null,
                "toolCalls": [{
                    "id": "call-submit-chapter",
                    "name": nekowriter_agent::runner::SUBMIT_CHAPTER_TOOL,
                    "arguments": { "summary": summary },
                }],
            }),
            request_id: Some(request_id),
            tool_call_id: None,
        }
    }

    #[test]
    fn extra_chapter_outline_context_does_not_require_a_main_plan_item() {
        let extra = ChapterSummary {
            id: Uuid::new_v4(),
            version_id: Uuid::new_v4(),
            kind: ChapterKind::Extra,
            plan_item_id: None,
            order_index: 3,
            title: "雨停之后".to_owned(),
            label: "番外 1 雨停之后".to_owned(),
            body_word_count: 0,
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
            label: "第1章 雨夜的来客".to_owned(),
            body_word_count: 0,
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

    #[test]
    fn delegated_review_user_content_appends_notes_and_quoted_citations() {
        let citations = vec![
            FeedbackCitation::Document {
                document_id: Uuid::new_v4(),
                revision_id: Uuid::new_v4(),
                chapter_id: Some(Uuid::new_v4()),
                range: nekowriter_core::TextCitationRange {
                    start_char: 0,
                    end_char: 2,
                    quote: "雨夜".to_owned(),
                },
            },
            FeedbackCitation::Candidate {
                tool_call_id: Uuid::new_v4(),
                candidate_hash: "a".repeat(64),
                candidate_range: nekowriter_core::TextCitationRange {
                    start_char: 0,
                    end_char: 2,
                    quote: "胶卷".to_owned(),
                },
            },
        ];
        let content =
            delegated_review_user_content("审核模板".to_owned(), "关注人物动机", &citations);
        assert!(content.starts_with("审核模板\n\n用户审核关注点：关注人物动机"));
        assert!(content.contains("- 正文引用：「雨夜」"));
        assert!(content.contains("- 待审候选引用：「胶卷」"));
        // 引文只以文本进入提示词，不泄露内部 ID。
        let FeedbackCitation::Document { document_id, .. } = &citations[0] else {
            panic!("测试引用应为正式文档引用");
        };
        assert!(!content.contains(&document_id.to_string()));

        let without_extras = delegated_review_user_content("审核模板".to_owned(), "  ", &[]);
        assert_eq!(without_extras, "审核模板");
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
                    api_key: CredentialPatch::Keep,
                    model_id: "test-writing-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
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
        // 阶段提交是配对的两条记录：提交工具的调用，以及它被接受的结果。
        assert_eq!(store.list_agent_messages(run_id).await.unwrap().len(), 4);

        let request = timeout(Duration::from_secs(2), request_receiver)
            .await
            .expect("适配器应向假服务发送请求")
            .unwrap();
        assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(request.contains("\"name\":\"submit_preparation\""));
        assert!(!request.contains("response_format"));
    }

    /// 走一遍准备阶段的一轮提交：假服务把 `arguments` 按原文回给应用，等这一轮
    /// 真正落库后返回。断言留给调用方，通过与被拒两条路径共用这一套装置。
    async fn preparation_round_with_raw_arguments(
        raw_arguments: &str,
    ) -> (TempDir, Store, VersionId, RunId) {
        let (base_url, request_receiver) =
            preparation_sse_server_with_raw_arguments("preparation-submission-1", raw_arguments)
                .await;
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        store
            .save_system_model_settings(
                Uuid::new_v4(),
                0,
                NewSystemModelSettings {
                    api_base_url: base_url,
                    api_key: CredentialPatch::Keep,
                    model_id: "test-writing-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
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
                NewProject::new("提交拒绝入账", "被拒的提交必须留下配对回执", None::<&str>)
                    .unwrap(),
            )
            .await
            .unwrap();
        let version_id = created.version.id;
        store
            .save_novel_brief(
                version_id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一个雨夜车站的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let before_start = store
            .get_version_snapshot(version_id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(version_id, Uuid::new_v4(), before_start.state_revision)
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
            .enqueue_preparation(version_id, run_id)
            .await
            .unwrap();

        timeout(Duration::from_secs(5), async {
            loop {
                let _ = events.recv().await;
                let messages = store.list_agent_messages(run_id).await.unwrap();
                if messages
                    .iter()
                    .any(|message| message.role == AgentMessageRole::Tool)
                {
                    return;
                }
            }
        })
        .await
        .expect("这一轮提交必须留下配对的工具结果");
        let _ = timeout(Duration::from_secs(2), request_receiver).await;
        (data_dir, store, version_id, run_id)
    }

    /// 提取这一轮账本里的提交调用、它保留的参数原文与配对结果正文。参数可以是已经
    /// 解码的 object，也可以是模型给的原文本（此时按原样返回）。
    fn submission_ledger(
        messages: &[nekowriter_store::PersistedAgentMessage],
    ) -> (String, String, String) {
        let assistant = messages
            .iter()
            .find(|message| {
                message.role == AgentMessageRole::Assistant
                    && message.content["kind"] == "assistant_tool_calls"
            })
            .expect("提交工具调用必须留在账本里");
        let call_id = assistant.content["toolCalls"][0]["id"]
            .as_str()
            .expect("工具调用必须有 id")
            .to_owned();
        let arguments = assistant.content["toolCalls"][0]["arguments"].clone();
        let tool_message = messages
            .iter()
            .find(|message| message.role == AgentMessageRole::Tool)
            .expect("被拒的提交必须拿到配对工具结果");
        assert_eq!(tool_message.tool_call_id.as_deref(), Some(call_id.as_str()));
        (
            call_id,
            arguments
                .as_str()
                .map_or_else(|| arguments.to_string(), str::to_owned),
            tool_message.content["content"]
                .as_str()
                .expect("工具结果内容是 JSON 文本")
                .to_owned(),
        )
    }

    /// 缺 `chapter_plan` 的粗纲提交必须落到账本上：配对的错误工具结果点名缺的字段，
    /// 模型在剩余额度里可以补齐后重试，而不是只拿到一句泛泛的协议错误。
    #[tokio::test]
    async fn submission_without_a_chapter_plan_records_a_paired_rejection() {
        let submission = json!({
            "artifacts": PreparationArtifactKind::ALL
                .into_iter()
                .map(|kind| json!({ "kind": kind, "content": format!("{} 的模型产物", kind.label()) }))
                .collect::<Vec<_>>(),
        });
        let (_data_dir, store, version_id, run_id) =
            preparation_round_with_raw_arguments(&submission.to_string()).await;

        assert_eq!(
            store
                .get_preparation_snapshot(version_id)
                .await
                .unwrap()
                .workflow
                .execution,
            ExecutionStatus::Queued
        );
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

        let messages = store.list_agent_messages(run_id).await.unwrap();
        let (call_id, _arguments, receipt) = submission_ledger(&messages);
        assert_ne!(call_id, "");
        assert!(receipt.contains("PHASE_SUBMISSION_INVALID"), "{receipt}");
        assert!(receipt.contains("chapter_plan"), "{receipt}");
        let correction = messages.last().unwrap();
        assert_eq!(correction.role, AgentMessageRole::User);
        assert_eq!(correction.content["kind"], "protocol_correction");

        // 配对结果留在纠正历史里：重放这条调用不会触发 MissingToolResult。
        let history = stage_agent_history_messages(run_id, messages, &[]).unwrap();
        assert!(matches!(
            history[0].message,
            ModelRequestMessage::AssistantToolCalls { .. }
        ));
        assert!(matches!(
            history[1].message,
            ModelRequestMessage::ToolResult { .. }
        ));
        assert!(matches!(
            history[2].message,
            ModelRequestMessage::Text {
                role: ContextMessageRole::User,
                ..
            }
        ));
        assert!(
            store
                .events_after(0)
                .await
                .unwrap()
                .into_iter()
                .any(|event| event.event_type == "run.changed"
                    && event.payload["consecutive_protocol_errors"] == "1")
        );
    }

    /// 真实端点把参数截断的那一轮：`finish_reason` 正常收尾，但 `arguments` 在字符串
    /// 中途断掉。它必须按「被拒的提交」处理——配对错误工具结果 + 纠正消息 + 计入
    /// 连续协议错误——而不是以 `provider_error` 结束整轮。
    #[tokio::test]
    async fn truncated_submission_arguments_are_rejected_instead_of_failing_the_run() {
        let truncated = r#"{"artifacts":[{"kind":"background","content":"【创作约定】未写完的背"#;
        let (_data_dir, store, version_id, run_id) =
            preparation_round_with_raw_arguments(truncated).await;

        assert_eq!(
            store
                .get_preparation_snapshot(version_id)
                .await
                .unwrap()
                .workflow
                .execution,
            ExecutionStatus::Queued
        );
        let persisted = store.list_model_requests(run_id).await.unwrap();
        assert_eq!(
            persisted[0]
                .error
                .as_ref()
                .and_then(|error| error.get("kind"))
                .and_then(Value::as_str),
            Some("protocol_error"),
            "不可解码的提交是可纠正的协议错误，不是 provider_error"
        );

        let messages = store.list_agent_messages(run_id).await.unwrap();
        let (_call_id, arguments, receipt) = submission_ledger(&messages);
        assert_eq!(arguments, truncated);
        assert!(receipt.contains("PHASE_SUBMISSION_INVALID"), "{receipt}");
        assert!(receipt.contains("不是合法 JSON"), "{receipt}");
        let correction = messages.last().unwrap();
        assert_eq!(correction.role, AgentMessageRole::User);
        assert_eq!(correction.content["kind"], "protocol_correction");
        let history = stage_agent_history_messages(run_id, messages, &[]).unwrap();
        assert!(matches!(
            history[1].message,
            ModelRequestMessage::ToolResult { .. }
        ));
        assert!(
            store
                .events_after(0)
                .await
                .unwrap()
                .into_iter()
                .any(|event| event.event_type == "run.changed"
                    && event.payload["consecutive_protocol_errors"] == "1")
        );
    }

    /// 假服务把给定的 `arguments` 原文整段回给应用；原文可以是不完整的 JSON。
    async fn preparation_sse_server_with_raw_arguments(
        call_id: &str,
        raw_arguments: &str,
    ) -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = oneshot::channel();
        let payload = raw_submission_sse_payload(
            call_id,
            nekowriter_agent::runner::SUBMIT_PREPARATION_TOOL,
            raw_arguments,
        );
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut stream, request) = read_http_request(stream).await;
            let _ = sender.send(request);
            write_sse_response(&mut stream, &payload).await;
        });
        (format!("http://{address}/v1"), receiver)
    }

    /// 整段参数在一条 delta 里给出，与真实端点的分块方式一致。
    fn raw_submission_sse_payload(call_id: &str, tool: &str, raw_arguments: &str) -> String {
        let chunk = json!({
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": call_id,
                        "type": "function",
                        "function": { "name": tool, "arguments": raw_arguments },
                    }],
                },
                "finish_reason": Value::Null,
            }],
        });
        let finished = json!({
            "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }],
            "usage": { "prompt_tokens": 17, "completion_tokens": 11, "total_tokens": 28 },
        });
        format!("data: {chunk}\n\ndata: {finished}\n\ndata: [DONE]\n\n")
    }

    /// 建立一个已经进入准备阶段的 Run，并按给定能力保存系统模型与项目模型引用。
    async fn capability_fixture(
        base_url: String,
        system_capabilities: ModelCapabilityProfile,
        project_model: Option<ProjectModelReference>,
        project_thinking_level: ThinkingLevel,
    ) -> (TempDir, Store, VersionId, RunId) {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        store
            .save_system_model_settings(
                Uuid::new_v4(),
                0,
                NewSystemModelSettings {
                    api_base_url: base_url,
                    api_key: CredentialPatch::Keep,
                    model_id: "system-model".to_owned(),
                    capabilities: system_capabilities,
                    max_context_tokens: 32_768,
                    reserved_output_tokens: 4_096,
                },
            )
            .await
            .unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("能力归属", "项目引用的模型必须自带能力资料", None::<&str>)
                    .unwrap(),
            )
            .await
            .unwrap();
        store
            .save_project_settings(
                created.project.id,
                Uuid::new_v4(),
                0,
                NewProjectSettings {
                    model: project_model,
                    thinking_level: project_thinking_level,
                    auxiliary_tool_budget: 10,
                },
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
        (
            data_dir,
            store,
            created.version.id,
            started.workflow.active_run_id.unwrap(),
        )
    }

    fn capability_profile(tools: bool, thinking: bool) -> ModelCapabilityProfile {
        ModelCapabilityProfile {
            streaming: true,
            tools,
            stream_usage: true,
            thinking,
        }
    }

    fn project_model(
        model_id: &str,
        capabilities: ModelCapabilityProfile,
    ) -> ProjectModelReference {
        ProjectModelReference {
            model_id: model_id.to_owned(),
            capabilities,
        }
    }

    /// 项目引用的模型自己声明支持思考时，即使系统模型未声明，请求也必须照常发出。
    #[tokio::test]
    async fn project_model_override_uses_its_own_capabilities() {
        let (base_url, request_receiver) = preparation_sse_server().await;
        let (_data_dir, store, version_id, run_id) = capability_fixture(
            base_url,
            capability_profile(true, false),
            Some(project_model(
                "project-model",
                capability_profile(true, true),
            )),
            ThinkingLevel::High,
        )
        .await;
        let scheduler = WorkflowScheduler::new(
            store.clone(),
            Arc::new(Semaphore::new(1)),
            Arc::new(Mutex::new(())),
        );
        let mut events = store.subscribe_events();
        scheduler
            .enqueue_preparation(version_id, run_id)
            .await
            .unwrap();

        timeout(Duration::from_secs(5), async {
            loop {
                let _ = events.recv().await;
                let snapshot = store.get_preparation_snapshot(version_id).await.unwrap();
                if snapshot.workflow.execution == ExecutionStatus::WaitingReview {
                    return snapshot;
                }
            }
        })
        .await
        .expect("项目引用声明支持思考时，准备阶段应正常到达等待审核");

        let requests = store.list_model_requests(run_id).await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].settings_snapshot["effectiveModelId"],
            json!("project-model")
        );
        assert_eq!(
            requests[0].settings_snapshot["effectiveModelSource"],
            json!("project")
        );
        assert_eq!(
            requests[0].settings_snapshot["effectiveCapabilities"]["thinking"],
            json!(true)
        );
        let request = timeout(Duration::from_secs(2), request_receiver)
            .await
            .expect("适配器应向假服务发送请求")
            .unwrap();
        assert!(request.contains("\"model\":\"project-model\""));
    }

    /// 项目引用的模型未声明支持思考时，必须在 dispatch 之前被拒绝。
    #[tokio::test]
    async fn project_model_override_without_thinking_fails_before_dispatch() {
        let (base_url, request_receiver) = preparation_sse_server().await;
        let (_data_dir, store, version_id, run_id) = capability_fixture(
            base_url,
            capability_profile(true, true),
            Some(project_model(
                "project-model",
                capability_profile(true, false),
            )),
            ThinkingLevel::High,
        )
        .await;
        let scheduler = WorkflowScheduler::new(
            store.clone(),
            Arc::new(Semaphore::new(1)),
            Arc::new(Mutex::new(())),
        );
        let error = dispatch_once(&scheduler, version_id, run_id)
            .await
            .unwrap_err();
        let SchedulerError::ThinkingUnsupported(message) = &error else {
            panic!("应因思考能力不足而拒绝，实际为 {error:?}");
        };
        assert!(
            message.contains("project-model"),
            "消息应指明是哪个模型：{message}"
        );
        assert_eq!(
            store.list_model_requests(run_id).await.unwrap(),
            [] as [nekowriter_store::PersistedModelRequest; 0]
        );
        assert!(
            timeout(Duration::from_millis(200), request_receiver)
                .await
                .is_err(),
            "被拒绝的请求不得发出任何网络调用"
        );
    }

    /// 项目引用的模型未声明支持工具时，写作阶段必须在 dispatch 之前被拒绝。
    #[tokio::test]
    async fn project_model_override_without_tools_fails_before_dispatch() {
        let (base_url, request_receiver) = preparation_sse_server().await;
        let (_data_dir, store, version_id, run_id) = capability_fixture(
            base_url,
            capability_profile(true, true),
            Some(project_model(
                "project-model",
                capability_profile(false, true),
            )),
            ThinkingLevel::Off,
        )
        .await;
        let scheduler = WorkflowScheduler::new(
            store.clone(),
            Arc::new(Semaphore::new(1)),
            Arc::new(Mutex::new(())),
        );
        let error = dispatch_once(&scheduler, version_id, run_id)
            .await
            .unwrap_err();
        let SchedulerError::Context(message) = &error else {
            panic!("应因缺少工具调用而拒绝，实际为 {error:?}");
        };
        assert!(
            message.contains("project-model"),
            "消息应指明是哪个模型：{message}"
        );
        assert_eq!(
            store.list_model_requests(run_id).await.unwrap(),
            [] as [nekowriter_store::PersistedModelRequest; 0]
        );
        assert!(
            timeout(Duration::from_millis(200), request_receiver)
                .await
                .is_err(),
            "被拒绝的请求不得发出任何网络调用"
        );
    }

    /// 直接驱动一次模型请求启动，用于断言 dispatch 之前的拒绝行为。
    async fn dispatch_once(
        scheduler: &WorkflowScheduler,
        version_id: VersionId,
        run_id: RunId,
    ) -> Result<(), SchedulerError> {
        let (sender, _receiver) = mpsc::channel(1);
        let permit = Arc::new(Semaphore::new(1)).acquire_owned().await.unwrap();
        scheduler
            .begin_model_request(
                version_id,
                ScheduledRun {
                    version_id,
                    run_id,
                    stage: ScheduledStage::Preparation,
                },
                permit,
                sender,
            )
            .await
    }

    #[tokio::test]
    async fn delegated_review_lane_declares_only_the_submission_tool() {
        let (base_url, request_receiver) = review_sse_server(true, 0.86, Vec::new()).await;
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        store
            .save_system_model_settings(
                Uuid::new_v4(),
                0,
                NewSystemModelSettings {
                    api_base_url: base_url,
                    api_key: CredentialPatch::Keep,
                    model_id: "test-review-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
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
                Vec::new(),
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
                    .is_some_and(|review| review.status == nekowriter_core::ReviewStatus::Approved)
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
        assert!(request.contains("\"name\":\"submit_review\""));
        assert!(!request.contains("append_content"));
        assert!(!request.contains("response_format"));
    }

    /// 每一轮被拒的 `submit_review` 调用都必须拿到配对错误工具结果，回执里点名
    /// `issues` 与正确形状；返回配对回执的条数，由调用方断言轮数。
    fn count_paired_rejected_review_calls(
        messages: &[nekowriter_store::PersistedAgentMessage],
    ) -> usize {
        let mut paired = 0;
        for assistant in messages
            .iter()
            .filter(|message| message.content["kind"] == "assistant_tool_calls")
        {
            let call_id = assistant.content["toolCalls"][0]["id"].as_str().unwrap();
            assert_eq!(assistant.content["toolCalls"][0]["name"], "submit_review");
            let receipt = messages
                .iter()
                .find(|message| {
                    message.role == AgentMessageRole::Tool
                        && message.tool_call_id.as_deref() == Some(call_id)
                })
                .expect("被拒的审核提交必须拿到配对工具结果");
            let receipt_text = receipt.content["content"].as_str().unwrap();
            assert!(
                receipt_text.contains("PHASE_SUBMISSION_INVALID"),
                "{receipt_text}"
            );
            assert!(receipt_text.contains("issues"), "{receipt_text}");
            assert!(
                receipt_text.contains("不能是装着数组的 JSON 字符串"),
                "{receipt_text}"
            );
            paired += 1;
        }
        paired
    }

    /// 一次委托里每一轮被拒都要在事件流里留下与写作 lane 同形状的纠正事件。
    ///
    /// `consecutive_protocol_errors` 只通过事件到达客户端（快照与 Dashboard 的 DTO
    /// 都不含它），缺了这些事件，用户只会看到审核卡在 `delegated`、随后突然重新
    /// 开放。`rejected_rounds` 是本次委托被拒的轮数；达到连续协议错误上限时最后一
    /// 轮必须带上 `wait_reason = protocol_error_limit`。
    fn assert_review_correction_events(
        events: &[nekowriter_store::PersistedEvent],
        review_id: ReviewId,
        run_id: RunId,
        rejected_rounds: usize,
    ) {
        let limit_reached = usize::try_from(MAX_CONSECUTIVE_PROTOCOL_ERRORS) == Ok(rejected_rounds);
        let run_events: Vec<_> = events
            .iter()
            .filter(|event| {
                event.event_type == "run.changed" && event.payload["protocol_correction"] == true
            })
            .collect();
        assert_eq!(run_events.len(), rejected_rounds, "{events:#?}");
        for (index, event) in run_events.iter().enumerate() {
            let round = index + 1;
            assert_eq!(event.payload["kind"], "review");
            assert_eq!(event.payload["status"], "running");
            assert_eq!(event.payload["review_id"], review_id.to_string());
            assert!(event.payload["request_id"].is_string());
            assert_eq!(
                event.payload["consecutive_protocol_errors"],
                round.to_string()
            );
            assert_eq!(
                event.payload["wait_reason"],
                if limit_reached && round == rejected_rounds {
                    Value::String("protocol_error_limit".to_owned())
                } else {
                    Value::Null
                },
                "只有达到上限的那一轮说明原因"
            );
        }
        let review_events: Vec<_> = events
            .iter()
            .filter(|event| {
                event.event_type == "review.changed" && event.payload["protocol_correction"] == true
            })
            .collect();
        assert_eq!(review_events.len(), rejected_rounds, "{events:#?}");
        assert_eq!(review_events[0].payload["status"], "delegated");
        assert_eq!(review_events[0].payload["attempt_id"], run_id.to_string());
        assert_eq!(review_events[0].payload["consecutive_protocol_errors"], "1");
    }

    /// 真实端点上那一轮的形状：模型把 `issues` 写成「装着数组的 JSON 字符串」。
    ///
    /// 内容是 JSON 字符串，`issues` 必须是数组；这份参数在真实复跑里让审核委派以
    /// `provider_error` 式的失败收场，账本里只留下未配对的 assistant 调用。它必须
    /// 与写作 lane 同形状：配对的错误工具结果点名 `issues` 与正确形状、点名
    /// `submit_review` 的纠正消息、计入连续协议错误并在限额内重提。
    fn review_issues_as_a_json_string() -> Value {
        json!({
            "passed": true,
            "confidence": 0.82,
            "issues": "[\"【建议】第三章：后半页被发现的位置与广播室监控设定之间的承接稍显含糊。\"]",
        })
    }

    /// 被拒的审核提交必须留下配对回执，并在连续协议错误限额内重提；达到限额才
    /// 收束为「子 Run 失败、Review 重新开放」。
    #[tokio::test]
    async fn rejected_review_submissions_are_paired_and_retried_up_to_the_limit() {
        let malformed = review_issues_as_a_json_string().to_string();
        let (_data_dir, store, version_id, delegation, request_receiver) =
            delegated_background_review(vec![
                ("review-call-1", malformed.clone()),
                ("review-call-2", malformed.clone()),
                ("review-call-3", malformed),
            ])
            .await;
        let review_id = delegation.review_id;
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

        let reopened = timeout(Duration::from_secs(10), async {
            loop {
                let _ = events.recv().await;
                let snapshot = store.get_preparation_snapshot(version_id).await.unwrap();
                if snapshot
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
                    .and_then(|artifact| artifact.review.as_ref())
                    .is_some_and(|review| review.status == nekowriter_core::ReviewStatus::Open)
                {
                    return snapshot;
                }
            }
        })
        .await
        .expect("连续协议错误达到上限后 Review 应重新开放");
        assert_eq!(reopened.workflow.execution, ExecutionStatus::WaitingReview);

        // 三次提交都被拒：限额内重提了两次，第三次不再重排。
        let requests = timeout(Duration::from_secs(2), request_receiver)
            .await
            .expect("被拒的审核提交必须重提")
            .unwrap();
        assert_eq!(requests.len(), 3);

        let persisted = store.list_model_requests(delegation.run_id).await.unwrap();
        assert_eq!(persisted.len(), 3);
        for request in &persisted {
            assert_eq!(request.status, ModelRequestStatus::Succeeded);
            assert_eq!(
                request
                    .error
                    .as_ref()
                    .and_then(|error| error.get("kind"))
                    .and_then(Value::as_str),
                Some("protocol_error"),
                "不可解码的提交是可纠正的协议错误，不是 provider_error"
            );
        }

        // 每一条被拒的调用都拿到配对结果，且原因点名了字段与正确形状。
        let messages = store.list_agent_messages(delegation.run_id).await.unwrap();
        assert_eq!(count_paired_rejected_review_calls(&messages), 3);
        let correction = messages.last().unwrap();
        assert_eq!(correction.role, AgentMessageRole::User);
        assert_eq!(correction.content["kind"], "protocol_correction");
        assert!(
            correction.content["content"]
                .as_str()
                .unwrap()
                .contains("submit_review")
        );

        // 重提的那一轮真的带着自己的错误历史：被拒的调用、它的回执与纠正都在里面。
        assert!(requests[1].contains("review-call-1"));
        assert!(requests[1].contains("PHASE_SUBMISSION_INVALID"));
        assert!(requests[1].contains("未通过本阶段校验"));
        // 重提请求的序列是「重新渲染的 user 输入 + 被拒的调用/回执/纠正」：初始
        // 输入每轮重新渲染、不会重复堆叠，纠正则恰好出现一次。
        assert_eq!(requests[0].matches("\"role\":\"user\"").count(), 1);
        assert_eq!(requests[1].matches("\"role\":\"user\"").count(), 2);
        assert_eq!(requests[1].matches("\"role\":\"tool\"").count(), 1);

        // 三轮被拒都留下与写作 lane 同形状的纠正事件，且上限之后的收束仍按原
        // 行为广播：委派失败、Review 重新开放。
        let events = store.events_after(0).await.unwrap();
        assert_review_correction_events(&events, review_id, delegation.run_id, 3);
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "review.changed"
                    && event.payload["status"] == "open"
                    && event.payload["failed_attempt_id"] == delegation.run_id.to_string())
        );
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "run.changed"
                    && event.payload["status"] == "failed")
        );
    }

    /// §35.1 真实端点出现过的那一类：`finish_reason` 正常收尾，但工具参数在字符串
    /// 中途断掉（`arguments` 不是合法 JSON）。审核 lane 必须与写作 lane 同形状——
    /// 配对错误工具结果 + 点名原因的纠正 + 连续协议错误计数——而不是让整次委托
    /// 作废、等用户手动重新委托。
    #[tokio::test]
    async fn truncated_review_arguments_are_corrected_then_resubmitted() {
        let truncated = r#"{"passed": true, "confidence": 0.82, "issues": ["【建议】第三章"#;
        let corrected = json!({
            "passed": true,
            "confidence": 0.85,
            "issues": [],
        })
        .to_string();
        let (_data_dir, store, version_id, delegation, request_receiver) =
            delegated_background_review(vec![
                ("review-call-1", truncated.to_owned()),
                ("review-call-2", corrected),
            ])
            .await;
        let review_id = delegation.review_id;
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

        let completed = timeout(Duration::from_secs(10), async {
            loop {
                let _ = events.recv().await;
                let snapshot = store.get_preparation_snapshot(version_id).await.unwrap();
                if snapshot
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
                    .and_then(|artifact| artifact.review.as_ref())
                    .is_some_and(|review| review.status == nekowriter_core::ReviewStatus::Approved)
                {
                    return snapshot;
                }
            }
        })
        .await
        .expect("截断的参数必须被纠正，随后一次合法重提要应用决定");
        assert_eq!(completed.workflow.execution, ExecutionStatus::WaitingReview);

        let persisted = store.list_model_requests(delegation.run_id).await.unwrap();
        assert_eq!(persisted.len(), 2);
        assert_eq!(persisted[0].status, ModelRequestStatus::Succeeded);
        assert_eq!(
            persisted[0]
                .error
                .as_ref()
                .and_then(|error| error.get("kind"))
                .and_then(Value::as_str),
            Some("protocol_error"),
            "不可解码的提交是可纠正的协议错误，不是 provider_error"
        );

        // 模型给出的原文逐字留在账本里，回执点名「不是合法 JSON」。
        let messages = store.list_agent_messages(delegation.run_id).await.unwrap();
        let (_call_id, arguments, receipt) = submission_ledger(&messages);
        assert_eq!(arguments, truncated);
        assert!(receipt.contains("PHASE_SUBMISSION_INVALID"), "{receipt}");
        assert!(receipt.contains("不是合法 JSON"), "{receipt}");
        let events = store.events_after(0).await.unwrap();
        assert_review_correction_events(&events, review_id, delegation.run_id, 1);

        let requests = timeout(Duration::from_secs(2), request_receiver)
            .await
            .expect("审核 Agent 应向假服务发送两轮请求")
            .unwrap();
        assert_eq!(requests.len(), 2);
        // 第二轮必须看得到第一次被拒的原因，否则只是把同一份提示重发一次。
        assert!(requests[1].contains("不是合法 JSON"));
    }

    /// 被拒之后重提：一次合法的 `submit_review` 必须真的落成审核决定，而不是
    /// 让整次委托作废、等用户手动重新委托。
    #[tokio::test]
    async fn corrected_review_resubmission_commits_the_decision() {
        let malformed = review_issues_as_a_json_string().to_string();
        let corrected = json!({
            "passed": true,
            "confidence": 0.85,
            "issues": ["【建议】补一处非决定性线索。"],
        })
        .to_string();
        let (_data_dir, store, version_id, delegation, request_receiver) =
            delegated_background_review(vec![
                ("review-call-1", malformed),
                ("review-call-2", corrected),
            ])
            .await;
        let review_id = delegation.review_id;
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

        let completed = timeout(Duration::from_secs(10), async {
            loop {
                let _ = events.recv().await;
                let snapshot = store.get_preparation_snapshot(version_id).await.unwrap();
                if snapshot
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
                    .and_then(|artifact| artifact.review.as_ref())
                    .is_some_and(|review| review.status == nekowriter_core::ReviewStatus::Approved)
                {
                    return snapshot;
                }
            }
        })
        .await
        .expect("合法重提必须应用审核决定");
        assert_eq!(completed.workflow.execution, ExecutionStatus::WaitingReview);
        let review = completed
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
            .and_then(|artifact| artifact.review.as_ref())
            .unwrap();
        assert_eq!(review.passed, Some(true));
        assert!(
            review
                .issues
                .as_deref()
                .unwrap_or_default()
                .contains("补一处非决定性线索"),
            "落库的应是重提那份结论"
        );

        let persisted = store.list_model_requests(delegation.run_id).await.unwrap();
        assert_eq!(persisted.len(), 2);
        assert_eq!(persisted[0].status, ModelRequestStatus::Succeeded);
        assert_eq!(persisted[1].status, ModelRequestStatus::Succeeded);

        let requests = timeout(Duration::from_secs(2), request_receiver)
            .await
            .expect("审核 Agent 应向假服务发送两轮请求")
            .unwrap();
        assert_eq!(requests.len(), 2);
        // 第二轮必须看得到自己第一次被拒的原因，否则只是把同一份提示重发一次。
        assert!(requests[1].contains("不能是装着数组的 JSON 字符串"));
        assert!(requests[1].contains("\"name\":\"submit_review\""));
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
                Vec::new(),
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
                    api_key: CredentialPatch::Keep,
                    model_id: "reenqueue-test-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
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

    /// 创建一个已派出准备请求的稿本，返回数据目录与调度器。
    async fn preparation_request_fixture() -> (TempDir, Uuid, Uuid, Uuid, Uuid, WorkflowScheduler) {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("知识检索工具", "预算与工具可用性", None::<&str>).unwrap(),
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
        store
            .save_project_settings(
                created.project.id,
                Uuid::new_v4(),
                0,
                NewProjectSettings {
                    model: None,
                    thinking_level: ThinkingLevel::default(),
                    auxiliary_tool_budget: 1,
                },
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
        let dispatched = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id,
                    settings_snapshot: json!({ "model": "knowledge-tool-test" }),
                    prompt_snapshot: json!({ "contentHash": "knowledge-tool-test" }),
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
        (
            data_dir,
            created.project.id,
            created.version.id,
            run_id,
            dispatched.id,
            scheduler,
        )
    }

    async fn configure_embedding(store: &Store) {
        store
            .save_embedding_settings(
                Uuid::new_v4(),
                0,
                nekowriter_store::NewEmbeddingSettings {
                    // 一个确定不可达的本地端口：验证「已进入真实调用、失败仍计费」。
                    api_base_url: "http://127.0.0.1:1/v1".to_owned(),
                    model_id: "stub-embed".to_owned(),
                    dimension: 3,
                    api_key: CredentialPatch::Keep,
                },
            )
            .await
            .unwrap();
    }

    fn knowledge_tool_completion(provider_call_id: &str) -> ModelCompletion {
        ModelCompletion {
            provider_request_id: Some("provider-knowledge-tool".to_owned()),
            content: None,
            tool_calls: vec![ModelToolCall {
                id: provider_call_id.to_owned(),
                name: TOOL_QUERY_KNOWLEDGE.to_owned(),
                arguments: json!({ "query": "灯塔的燃料", "topK": 3 }),
            }],
            usage: None,
            usage_raw: None,
            finish_reason: nekowriter_agent::provider::ModelFinishReason::ToolCalls,
        }
    }

    #[test]
    fn query_knowledge_is_only_declared_when_it_can_actually_run() {
        let declared = preparation_agent_tools(preparation_submission_tool(), true, 7);
        let knowledge = declared
            .iter()
            .find(|tool| tool.name == TOOL_QUERY_KNOWLEDGE)
            .expect("可用时必须宣告查询工具");
        assert!(
            knowledge.description.contains("当前剩余 7 次"),
            "请求构建时必须说明余额：{}",
            knowledge.description
        );

        let hidden = preparation_agent_tools(preparation_submission_tool(), false, 7);
        assert_eq!(hidden.len(), 2);
        assert_eq!(hidden[0].name, "submit_preparation");
        assert_eq!(hidden[1].name, "ask_user");
    }

    #[tokio::test]
    async fn knowledge_tool_without_embedding_returns_an_error_and_does_not_charge() {
        let (_data_dir, _project_id, version_id, run_id, request_id, scheduler) =
            preparation_request_fixture().await;
        let store = &scheduler.store;

        let continues = scheduler
            .handle_preparation_completion(
                version_id,
                run_id,
                request_id,
                &ModelRequest {
                    messages: Vec::new(),
                    tools: preparation_agent_tools(preparation_submission_tool(), true, 1),
                    thinking_level: ThinkingLevel::Off,
                },
                Ok(knowledge_tool_completion("call-no-embedding")),
            )
            .await;
        assert!(continues, "工具结果落库后应当安排下一轮请求");
        let result = store
            .free_agent_tool_result(request_id, "call-no-embedding")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result["code"], "EMBEDDING_NOT_CONFIGURED");
        assert_eq!(result["status"], "error");
        assert_eq!(store.quota_balance(run_id).await.unwrap().used, 0);
    }

    #[tokio::test]
    async fn knowledge_tool_stops_at_an_exhausted_budget_without_charging() {
        let (_data_dir, project_id, version_id, run_id, request_id, scheduler) =
            preparation_request_fixture().await;
        let store = &scheduler.store;
        configure_embedding(store).await;
        let settings = store.get_project_settings(project_id).await.unwrap();
        store
            .save_project_settings(
                project_id,
                Uuid::new_v4(),
                settings.revision,
                NewProjectSettings {
                    model: None,
                    thinking_level: ThinkingLevel::default(),
                    auxiliary_tool_budget: 0,
                },
            )
            .await
            .unwrap();

        assert!(
            scheduler
                .handle_preparation_completion(
                    version_id,
                    run_id,
                    request_id,
                    &ModelRequest {
                        messages: Vec::new(),
                        tools: preparation_agent_tools(preparation_submission_tool(), true, 1),
                        thinking_level: ThinkingLevel::Off,
                    },
                    Ok(knowledge_tool_completion("call-no-budget")),
                )
                .await
        );
        let result = store
            .free_agent_tool_result(request_id, "call-no-budget")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result["code"], "QUOTA_EXHAUSTED");
        assert_eq!(store.quota_balance(run_id).await.unwrap().used, 0);
    }

    #[tokio::test]
    async fn knowledge_tool_charges_once_even_when_the_search_fails_and_never_recharges() {
        let (_data_dir, _project_id, version_id, run_id, request_id, scheduler) =
            preparation_request_fixture().await;
        let store = &scheduler.store;
        configure_embedding(store).await;

        assert!(
            scheduler
                .handle_preparation_completion(
                    version_id,
                    run_id,
                    request_id,
                    &ModelRequest {
                        messages: Vec::new(),
                        tools: preparation_agent_tools(preparation_submission_tool(), true, 1),
                        thinking_level: ThinkingLevel::Off,
                    },
                    Ok(knowledge_tool_completion("call-failing-search")),
                )
                .await
        );
        let result = store
            .free_agent_tool_result(request_id, "call-failing-search")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result["code"], "KNOWLEDGE_SEARCH_FAILED");
        // 进入真实检索后才失败：仍计 1 次，且不再多扣。
        let balance = store.quota_balance(run_id).await.unwrap();
        assert_eq!(balance.used, 1);
        assert_eq!(balance.remaining, 0);

        // 同一回调重放：已有结果，直接不再投递新请求，也不重复扣费。
        let workflow = store
            .get_preparation_snapshot(version_id)
            .await
            .unwrap()
            .workflow;
        assert_eq!(workflow.execution, ExecutionStatus::Queued);
        let replayed = scheduler
            .handle_preparation_completion(
                version_id,
                run_id,
                request_id,
                &ModelRequest {
                    messages: Vec::new(),
                    tools: preparation_agent_tools(preparation_submission_tool(), true, 1),
                    thinking_level: ThinkingLevel::Off,
                },
                Ok(knowledge_tool_completion("call-failing-search")),
            )
            .await;
        assert!(!replayed, "重放已经落库的调用不应该再安排请求");
        assert_eq!(store.quota_balance(run_id).await.unwrap().used, 1);
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
            thinking_level: ThinkingLevel::Off,
        };

        assert!(
            scheduler
                .handle_preparation_completion(
                    created.version.id,
                    dispatched.run_id,
                    dispatched.id,
                    &request,
                    Ok(ModelCompletion {
                        provider_request_id: Some("provider-protocol-test".to_owned()),
                        content: Some("这不是合法的准备阶段 JSON".to_owned()),
                        tool_calls: Vec::new(),
                        usage: None,
                        usage_raw: None,
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

        let history = stage_agent_history_messages(
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
            } if content.contains("没有调用本阶段的提交工具 `submit_preparation`")
        ));
    }

    /// 被业务守卫拒绝的阶段提交（正文为空）必须留下配对结果，而且下一轮请求仍然可以
    /// 组装。
    ///
    /// 这是实战里卡死的那条路径：`submit_chapter` 的提交被守卫拒绝，却只写下 assistant
    /// 调用；此后每一次「显式继续」都在构建上下文时以 `MissingToolResult` 失败，请求
    /// 一次也没发出去。
    #[tokio::test]
    async fn business_rejected_chapter_submission_leaves_a_buildable_history() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let (_version_id, chapter_id, body_run_id, request_id) =
            in_flight_empty_chapter_body_submission(&store, "rejected-submission").await;
        let scheduler = WorkflowScheduler::new(
            store.clone(),
            Arc::new(Semaphore::new(1)),
            Arc::new(Mutex::new(())),
        );
        let stage = PhaseSubmissionKind::Chapter { chapter_id };
        let request = ModelRequest {
            messages: Vec::new(),
            tools: chapter_body_tools(submission_tool(&stage)),
            thinking_level: ThinkingLevel::Off,
        };

        assert!(
            scheduler
                .handle_chapter_body_completion(
                    chapter_id,
                    request_id,
                    &request,
                    Ok(ModelCompletion {
                        provider_request_id: Some("provider-rejected-submission".to_owned()),
                        content: None,
                        tool_calls: vec![ModelToolCall {
                            id: "rejected-submit-call".to_owned(),
                            name: nekowriter_agent::runner::SUBMIT_CHAPTER_TOOL.to_owned(),
                            arguments: json!({
                                "chapter_id": chapter_id,
                                "summary": "本章已经完成，并留下后续追查的悬念。",
                            }),
                        }],
                        usage: None,
                        usage_raw: None,
                        finish_reason: nekowriter_agent::provider::ModelFinishReason::ToolCalls,
                    }),
                )
                .await,
            "被拒的阶段提交必须立即退回模型"
        );

        let snapshot = store.get_chapter_body(chapter_id).await.unwrap();
        assert_eq!(snapshot.workflow.execution, ExecutionStatus::Queued);
        assert!(snapshot.body.content.is_none());
        assert!(snapshot.body.review.is_none());

        let messages = store.list_agent_messages(body_run_id).await.unwrap();
        let (call_id, _arguments, receipt) = submission_ledger(&messages);
        assert_eq!(call_id, "rejected-submit-call");
        assert!(receipt.contains("PHASE_SUBMISSION_REJECTED"), "{receipt}");
        assert!(receipt.contains("章节正文不能为空"), "{receipt}");
        let correction = messages.last().unwrap();
        assert_eq!(correction.role, AgentMessageRole::User);
        assert_eq!(correction.content["kind"], "protocol_correction");
        assert!(
            correction.content["content"]
                .as_str()
                .is_some_and(|content| content.contains("submit_chapter")),
            "{correction:?}"
        );

        // 这一轮不计入连续协议错误，事件只解释为什么没有推进。
        let rejected = store
            .events_after(0)
            .await
            .unwrap()
            .into_iter()
            .rfind(|event| event.event_type == "run.changed")
            .unwrap();
        assert_eq!(rejected.payload["reason"], "submission_rejected");
        assert!(
            rejected
                .payload
                .get("consecutive_protocol_errors")
                .is_none()
        );
        assert!(rejected.payload.get("wait_reason").is_none());

        // 关键不变量：账本可以还原成下一轮请求。实战里卡死的就是这一步。
        let history = stage_agent_history_messages(body_run_id, messages, &[]).unwrap();
        assert!(matches!(
            history[0].message,
            ModelRequestMessage::AssistantToolCalls { .. }
        ));
        assert!(matches!(
            history[1].message,
            ModelRequestMessage::ToolResult { .. }
        ));
        build_context_with_history(
            "你是中文短篇小说写作助手。".to_owned(),
            chapter_body_tools(submission_tool(&stage)),
            Vec::new(),
            Vec::new(),
            history,
            32_768,
            4_096,
        )
        .expect("被业务守卫拒绝的提交之后仍然必须能组装下一次请求");
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
                    api_key: CredentialPatch::Keep,
                    model_id: "stop-test-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
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
                    api_key: CredentialPatch::Keep,
                    model_id: "test-writing-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
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
        // 归属仍在准备 Run 中，并由请求快照中的 fold source 追溯。阶段提交本身是
        // 提交工具调用与配对结果两条记录。
        assert_eq!(store.list_agent_messages(run_id).await.unwrap().len(), 4);
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
        // 阶段角色必须带着本章的提交结构发出，而 `chapter_id` 无法由模型自行推断。
        assert!(request.contains("chapter_id"));
        assert!(request.contains(&chapter_id.to_string()));
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
                Vec::new(),
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
                    api_key: CredentialPatch::Keep,
                    model_id: "test-writing-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
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
        // 一次正文工具轮（调用 + 结果）加上阶段提交（提交工具调用 + 配对结果）。
        assert_eq!(
            store.list_agent_messages(body_run_id).await.unwrap().len(),
            6
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
        // 正文阶段也要拿到本章的提交结构，否则模型无法构造合法的章节提交。
        assert!(first_request.contains("chapter_id"));
        assert!(first_request.contains(&chapter_id.to_string()));
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
                Vec::new(),
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
                    api_key: CredentialPatch::Keep,
                    model_id: "test-writing-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
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
                Vec::new(),
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

        // F13：工具结果已经绑定 A、但下一请求尚未封存时追加 B，两个意见必须仍然
        // 作为同一批次进入同一个工具结果，不能把 B 推迟到下一次工具调用。
        let full_batch = store
            .submit_run_feedback(
                body_run_id,
                Uuid::new_v4(),
                bound.workflow.state_revision,
                "补充保留两人没有说出口的旧事。".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(full_batch.pending_feedback.len(), 2);
        assert!(
            full_batch
                .pending_feedback
                .iter()
                .all(|feedback| feedback.status == FeedbackStatus::Bound)
        );
        let materialized = store
            .list_materialized_chapter_body_feedback(body_run_id)
            .await
            .unwrap();
        assert_eq!(materialized.len(), 2);
        assert_eq!(
            materialized
                .iter()
                .map(|feedback| feedback.text.as_str())
                .collect::<Vec<_>>(),
            vec![
                "请让人物先通过动作试探彼此，不要直接解释旧日误会。",
                "补充保留两人没有说出口的旧事。",
            ]
        );
        assert!(materialized.iter().all(|feedback| {
            feedback.provider_tool_call_id == "feedback-tool-1"
                && feedback.status == FeedbackStatus::Bound
        }));

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
                        "feedbackBinding": {
                            "ids": materialized.iter().map(|feedback| feedback.id).collect::<Vec<_>>(),
                        },
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
        assert_eq!(
            after_seal.pending_feedback,
            [] as [nekowriter_core::PendingChapterBodyFeedback; 0]
        );
        assert_eq!(after_seal.delivered_feedback.len(), 2);
        assert!(after_seal.delivered_feedback.iter().all(|feedback| {
            feedback.request_id == dispatched.id
                && feedback.delivery_status == nekowriter_core::FeedbackDeliveryStatus::Sent
        }));
        let included = store
            .list_materialized_chapter_body_feedback(body_run_id)
            .await
            .unwrap();
        assert!(
            included
                .iter()
                .all(|feedback| feedback.status == FeedbackStatus::Included)
        );

        store
            .resolve_model_request(
                dispatched.id,
                ModelRequestResolution::Failed {
                    provider_request_id: None,
                    error: json!({ "kind": "network" }),
                },
            )
            .await
            .unwrap();
        let failed = store.get_chapter_body(chapter_id).await.unwrap();
        assert_eq!(failed.delivered_feedback.len(), 2);
        assert!(failed.delivered_feedback.iter().all(|feedback| {
            feedback.delivery_status == nekowriter_core::FeedbackDeliveryStatus::Failed
        }));
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

        // F12/F14：自由意见在等待正式答案时只进入独立批次，绝不替代问题回答；
        // 连续追加的意见也必须等正式答案产生工具结果后作为一个批次交付。
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
        let with_second_feedback = store
            .submit_run_feedback(
                body_run_id,
                Uuid::new_v4(),
                with_feedback.workflow.state_revision,
                "补充保留站钟在雨声里的回响。".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        let full_feedback_batch = store
            .submit_run_feedback(
                body_run_id,
                Uuid::new_v4(),
                with_second_feedback.workflow.state_revision,
                "补充让回答落在人物动作上。".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            full_feedback_batch.workflow.execution,
            ExecutionStatus::WaitingAnswer
        );
        assert_eq!(full_feedback_batch.pending_feedback.len(), 3);
        assert!(
            full_feedback_batch
                .pending_feedback
                .iter()
                .all(|feedback| feedback.status == FeedbackStatus::Queued)
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
                full_feedback_batch.workflow.state_revision,
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
                full_feedback_batch.workflow.state_revision,
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
        assert_eq!(feedback.len(), 3);
        assert_eq!(
            feedback
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            vec![
                "让人物的沉默比解释更有分量。",
                "补充保留站钟在雨声里的回响。",
                "补充让回答落在人物动作上。",
            ]
        );
        assert!(feedback.iter().all(|item| {
            item.status == FeedbackStatus::Bound && item.provider_tool_call_id == "ask-user-call-1"
        }));

        let dispatched = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: body_run_id,
                    settings_snapshot: json!({ "model": "ask-user-test" }),
                    prompt_snapshot: json!({ "contentHash": "ask-user-answer" }),
                    context_snapshot: json!({
                        "feedbackBinding": {
                            "ids": feedback.iter().map(|item| item.id).collect::<Vec<_>>(),
                        },
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
        assert_eq!(
            sealed.pending_feedback,
            [] as [nekowriter_core::PendingChapterBodyFeedback; 0]
        );
        assert_eq!(sealed.delivered_feedback.len(), 3);
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
                Vec::new(),
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

        // F15：暂停边界的第一条意见已把批次绑到最近工具结果后，紧接着追加的意见
        // 仍继承这份绑定；不能拆成下一次工具调用的独立批次。
        let full_batch = store
            .submit_run_feedback(
                body_run_id,
                Uuid::new_v4(),
                resumed.workflow.state_revision,
                "补充让伞沿保持沉默的距离。".to_owned(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(full_batch.workflow.execution, ExecutionStatus::Queued);
        assert_eq!(full_batch.pending_feedback.len(), 2);
        assert!(
            full_batch
                .pending_feedback
                .iter()
                .all(|feedback| feedback.status == FeedbackStatus::Bound)
        );
        let materialized = store
            .list_materialized_chapter_body_feedback(body_run_id)
            .await
            .unwrap();
        assert_eq!(materialized.len(), 2);
        assert!(materialized.iter().all(|feedback| {
            feedback.provider_tool_call_id == "pause-feedback-tool"
                && feedback.status == FeedbackStatus::Bound
        }));
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
                body_submission_message(request_id, "模型尝试直接提交本章。"),
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

        // 反复直接提交是被业务守卫拒绝的「被拒回合」：每次都只重排请求，既不推进
        // 连续协议错误，也不触发工程执行限制——未投递意见原样保留，等模型用
        // append/replace 形成下一条工具结果把它投递出去。
        for label in ["second", "third", "fourth"] {
            let rejected =
                commit_direct_chapter_submission(&store, body_run_id, chapter_id, label).await;
            assert_eq!(rejected.workflow.stage, WorkflowStage::WritingDrafting);
            assert_eq!(rejected.workflow.execution, ExecutionStatus::Queued);
            assert_eq!(rejected.workflow.wait_reason, None);
            assert!(rejected.body.review.is_none());
            assert_eq!(rejected.pending_feedback.len(), 1);
            assert_eq!(rejected.pending_feedback[0].status, FeedbackStatus::Queued);
        }
        let last_rejection = store
            .events_after(0)
            .await
            .unwrap()
            .into_iter()
            .rfind(|event| event.event_type == "run.changed")
            .unwrap();
        assert_eq!(last_rejection.payload["status"], "queued");
        assert_eq!(
            last_rejection.payload["reason"],
            "pending_feedback_requires_tool_result"
        );
        assert!(last_rejection.payload.get("wait_reason").is_none());
        assert!(
            last_rejection
                .payload
                .get("consecutive_protocol_errors")
                .is_none()
        );
    }

    /// 为同一条正文 Run 新开一次在途请求，并让模型再次直接提交本章。
    async fn commit_direct_chapter_submission(
        store: &Store,
        body_run_id: RunId,
        chapter_id: ChapterId,
        label: &str,
    ) -> nekowriter_core::ChapterBodySnapshot {
        let request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: body_run_id,
                    settings_snapshot: json!({ "model": format!("direct-submission-feedback-{label}") }),
                    prompt_snapshot: json!({ "contentHash": format!("direct-submission-feedback-{label}") }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        store
            .commit_chapter_body_model_submission(
                request.id,
                chapter_id,
                Some(format!("direct-submission-feedback-{label}-request")),
                json!({ "promptTokens": 18, "completionTokens": 7, "totalTokens": 25 }),
                body_submission_message(request.id, "模型反复直接提交本章。"),
                "模型反复直接提交本章。".to_owned(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn invalid_chapter_body_tools_stop_at_the_protocol_limit() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let (_version_id, chapter_id, body_run_id, mut request_id) =
            in_flight_chapter_body_submission(&store, "invalid-body-tool").await;

        for attempt in 1..=MAX_CONSECUTIVE_PROTOCOL_ERRORS {
            let execution = store
                .execute_chapter_body_agent_tool_call(
                    request_id,
                    chapter_id,
                    Some(format!("invalid-body-tool-provider-{attempt}")),
                    json!({ "totalTokens": "1" }),
                    ChapterBodyAgentToolCall {
                        provider_call_id: format!("invalid-body-tool-call-{attempt}"),
                        name: "append_content".to_owned(),
                        // 空正文会形成稳定的工具错误，但不应无限重新投递。
                        arguments: json!({ "content": "" }),
                    },
                )
                .await
                .unwrap();
            assert!(execution.newly_resolved);
            assert_eq!(execution.result["status"], "error");
            assert_eq!(execution.requeue, attempt < MAX_CONSECUTIVE_PROTOCOL_ERRORS);

            let snapshot = store.get_chapter_body(chapter_id).await.unwrap();
            assert_eq!(
                snapshot.workflow.execution,
                if attempt < MAX_CONSECUTIVE_PROTOCOL_ERRORS {
                    ExecutionStatus::Queued
                } else {
                    ExecutionStatus::Stopped
                }
            );
            if attempt < MAX_CONSECUTIVE_PROTOCOL_ERRORS {
                request_id = store
                    .capture_and_dispatch_model_request(
                        NewModelRequest {
                            run_id: body_run_id,
                            settings_snapshot: json!({ "model": "invalid-body-tool" }),
                            prompt_snapshot: json!({ "contentHash": format!("invalid-body-tool-{attempt}") }),
                            context_snapshot: json!({ "messages": [] }),
                        },
                        Vec::new(),
                    )
                    .await
                    .unwrap()
                    .id;
            }
        }

        let stopped = store.get_chapter_body(chapter_id).await.unwrap();
        assert_eq!(stopped.workflow.execution, ExecutionStatus::Stopped);
        assert_eq!(
            stopped.workflow.wait_reason,
            Some(nekowriter_core::RunWaitReason::ProtocolErrorLimit)
        );
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
                body_submission_message(request_id, "模型尝试直接提交本章。"),
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

    #[tokio::test]
    async fn paused_direct_submission_without_body_resumes_to_a_retryable_queue() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let (_version_id, chapter_id, body_run_id, request_id) =
            in_flight_empty_chapter_body_submission(&store, "empty-submission-pause").await;
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

        let paused = store
            .commit_chapter_body_model_submission(
                request_id,
                chapter_id,
                Some("empty-submission-pause-request".to_owned()),
                json!({ "promptTokens": 18, "completionTokens": 7, "totalTokens": 25 }),
                body_submission_message(request_id, "模型过早提交本章。"),
                "模型过早提交本章。".to_owned(),
            )
            .await
            .unwrap();
        assert_eq!(paused.workflow.execution, ExecutionStatus::Paused);
        assert!(paused.held_submission.is_some());
        assert!(paused.body.content.is_none());

        let resumed = store
            .resume_chapter_body_pause(body_run_id, Uuid::new_v4(), paused.workflow.state_revision)
            .await
            .unwrap();
        assert_eq!(resumed.workflow.stage, WorkflowStage::WritingDrafting);
        assert_eq!(resumed.workflow.execution, ExecutionStatus::Queued);
        assert!(!resumed.workflow.pause_requested);
        assert!(resumed.held_submission.is_none());
        assert!(resumed.body.review.is_none());
        let messages = store.list_agent_messages(body_run_id).await.unwrap();
        assert_eq!(
            messages.last().unwrap().content["kind"],
            "protocol_correction"
        );
        assert!(
            messages.last().unwrap().content["content"]
                .as_str()
                .is_some_and(|content| content.contains("当前直接提交无法应用"))
        );

        // 恢复后的 Run 可以立即创建下一次请求，证明它没有永久停在 paused。
        let next_request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: body_run_id,
                    settings_snapshot: json!({ "model": "empty-submission-pause" }),
                    prompt_snapshot: json!({ "contentHash": "empty-submission-pause-retry" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(next_request.status, ModelRequestStatus::Dispatched);
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
                    Vec::new(),
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
                Vec::new(),
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

    /// 构造已经进入正文阶段、但模型尚未写入任何正文的在途请求。
    async fn in_flight_empty_chapter_body_submission(
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
                Vec::new(),
            )
            .await
            .unwrap();
        let body_run_id = drafting.workflow.active_run_id.unwrap();
        let request = store
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
        (version_id, chapter_id, body_run_id, request.id)
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
            });
            let stream_payload = submission_sse_payload(
                "preparation-call-1",
                nekowriter_agent::runner::SUBMIT_PREPARATION_TOOL,
                &submission,
                17,
                11,
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
            });
            let stream_payload = submission_sse_payload(
                "review-call-1",
                nekowriter_agent::runner::SUBMIT_REVIEW_TOOL,
                &decision,
                13,
                7,
            );
            write_sse_response(&mut stream, &stream_payload).await;
        });
        (format!("http://{address}/v1"), receiver)
    }

    /// 假审核服务：按顺序把多份 `submit_review` 参数原文发给应用。
    ///
    /// 同一次委托的重提会再次连到同一个 base URL，所以「被拒 → 纠正 → 重提」是
    /// 一条真实链路：服务器按连接顺序取用下一份参数，并把每条收到的请求原文交回
    /// 测试断言。
    async fn review_sse_server_with_raw_arguments(
        responses: Vec<(&'static str, String)>,
    ) -> (String, oneshot::Receiver<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let mut requests = Vec::with_capacity(responses.len());
            for (call_id, raw_arguments) in responses {
                let (stream, _) = listener.accept().await.unwrap();
                let (mut stream, request) = read_http_request(stream).await;
                requests.push(request);
                let payload = raw_submission_sse_payload(
                    call_id,
                    nekowriter_agent::runner::SUBMIT_REVIEW_TOOL,
                    &raw_arguments,
                );
                write_sse_response(&mut stream, &payload).await;
            }
            let _ = sender.send(requests);
        });
        (format!("http://{address}/v1"), receiver)
    }

    /// 走一遍「四项产物已提交、背景产物的审核已委托」的链路。
    ///
    /// 返回的 `Store` 与稿本可直接接到调度器；假服务按 `responses` 顺序应答，
    /// 请求原文由 oneshot 交回，供调用方断言真正发出去的内容。
    async fn delegated_background_review(
        responses: Vec<(&'static str, String)>,
    ) -> (
        TempDir,
        Store,
        VersionId,
        nekowriter_store::ReviewDelegationResult,
        oneshot::Receiver<Vec<String>>,
    ) {
        let (base_url, request_receiver) = review_sse_server_with_raw_arguments(responses).await;
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        store
            .save_system_model_settings(
                Uuid::new_v4(),
                0,
                NewSystemModelSettings {
                    api_base_url: base_url,
                    api_key: CredentialPatch::Keep,
                    model_id: "test-review-model".to_owned(),
                    capabilities: ModelCapabilityProfile {
                        streaming: true,
                        tools: true,
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
                    "审核提交纠正",
                    "被拒的审核提交必须拿到配对回执",
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
                Vec::new(),
            )
            .await
            .unwrap();
        (data_dir, store, version_id, delegation, request_receiver)
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
            });
            let stream_payload = submission_sse_payload(
                "outline-call-1",
                nekowriter_agent::runner::SUBMIT_CHAPTER_OUTLINE_TOOL,
                &submission,
                19,
                12,
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
            });
            let second_response = submission_sse_payload(
                "body-submission-call-1",
                nekowriter_agent::runner::SUBMIT_CHAPTER_TOOL,
                &submission,
                29,
                11,
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

    /// 阶段提交工具调用的完整流式响应：增量携带 `tool_calls`，结束原因是
    /// `tool_calls`，并附带用量。阶段终态只能是提交工具调用，假服务也必须这样回答。
    fn submission_sse_payload(
        call_id: &str,
        tool: &str,
        arguments: &Value,
        prompt_tokens: i64,
        completion_tokens: i64,
    ) -> String {
        let chunk = json!({
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": tool,
                            "arguments": arguments.to_string(),
                        },
                    }],
                },
                "finish_reason": Value::Null,
            }],
        });
        let finished = json!({
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "tool_calls",
            }],
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens,
            },
        });
        format!("data: {chunk}\n\ndata: {finished}\n\ndata: [DONE]\n\n")
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
