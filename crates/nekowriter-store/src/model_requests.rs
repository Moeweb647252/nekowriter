//! 模型请求快照和请求额度的持久化检查点。
//!
//! 模型网络 I/O 永远不在 SQLite 事务中进行。这里仅负责在发送前不可变地封存
//! 请求、在真正 dispatch 时原子消耗额度，并在回调到达后记录终态。这样进程中断
//! 或重复回调都不会让请求计数或用量被重复累计。

use std::collections::HashSet;

use nekowriter_core::{
    ChapterId, ChapterOutlineSnapshot, ChapterPlanDraft, ExecutionStatus,
    MAX_CONSECUTIVE_PROTOCOL_ERRORS, ModelRequestCompletion, NewChapterOutline,
    PreparationArtifactInput, PreparationSnapshot, RunId, VersionId, WorkflowSnapshot,
    WorkflowStage, complete_model_request as complete_workflow_model_request,
    continue_after_agent_tool, decimal_i64,
    dispatch_model_request as dispatch_workflow_model_request,
    fail_queued_model_run as fail_queued_workflow_run,
    stop_queued_model_run as stop_queued_workflow_run, stop_writing_after_protocol_error,
    utc_now_millis,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
    TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    AgentMessageRole, NewAgentMessage, Store, StoreError,
    agent_messages::{append_agent_messages_for_run, validate_new_messages},
    db_id,
    entity::{feedback, model_request, run, tool_call},
    preparation::{
        chapter_outline_snapshot, insert_event, persist_chapter_outline_submission,
        persist_preparation_submission, persist_workflow, preparation_snapshot,
        reset_run_protocol_errors, update_preparation_run_status, workflow_for_version,
    },
    transaction_error,
};

const STATUS_PREPARED: &str = "prepared";
const STATUS_DISPATCHED: &str = "dispatched";
const STATUS_SUCCEEDED: &str = "succeeded";
const STATUS_FAILED: &str = "failed";
const STATUS_CANCELLED: &str = "cancelled";
const STATUS_UNKNOWN: &str = "unknown";

#[derive(Clone, Copy)]
enum QueuedRunTerminal {
    Failed,
    Stopped,
}

impl QueuedRunTerminal {
    const fn status(self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// 一次请求封存时由应用服务提供的非敏感快照。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewModelRequest {
    pub run_id: RunId,
    pub settings_snapshot: Value,
    pub prompt_snapshot: Value,
    pub context_snapshot: Value,
}

/// `model_requests.status` 的稳定值。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRequestStatus {
    Prepared,
    Dispatched,
    Succeeded,
    Failed,
    Cancelled,
    Unknown,
}

/// 一次已持久化模型请求的完整可诊断视图。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedModelRequest {
    pub id: Uuid,
    pub run_id: RunId,
    #[serde(with = "decimal_i64")]
    pub attempt_no: i64,
    pub status: ModelRequestStatus,
    pub settings_snapshot: Value,
    pub prompt_snapshot: Value,
    pub context_snapshot: Value,
    pub provider_request_id: Option<String>,
    pub usage: Option<Value>,
    pub error: Option<Value>,
    #[serde(with = "decimal_i64")]
    pub created_at_ms: i64,
    #[serde(with = "decimal_i64")]
    pub updated_at_ms: i64,
    #[serde(with = "optional_decimal_i64")]
    pub dispatched_at_ms: Option<i64>,
    #[serde(with = "optional_decimal_i64")]
    pub completed_at_ms: Option<i64>,
}

/// 已 dispatch 的请求网络调用结束后的唯一终态。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelRequestResolution {
    Succeeded {
        provider_request_id: Option<String>,
        usage: Value,
    },
    Failed {
        provider_request_id: Option<String>,
        error: Value,
    },
    Unknown {
        provider_request_id: Option<String>,
        error: Value,
    },
}

/// 一次完整模型响应不符合本阶段协议时由调度器提交的持久化事实。
///
/// `assistant_message` 始终保存原始完整响应；若其中含有工具调用，Store 会为每一条
/// 调用生成配对的稳定错误结果，避免下一次请求出现不配对的 assistant tool call。
/// `tool_result_code` 是写入这些配对结果的机器可读原因码（例如
/// `PARALLEL_TOOL_CALLS_NOT_ALLOWED`）；普通协议错误使用 `PROTOCOL_ERROR`。
#[derive(Clone, Debug)]
pub struct ModelProtocolCorrection {
    pub provider_request_id: Option<String>,
    pub usage: Value,
    pub assistant_message: NewAgentMessage,
    pub error: Value,
    pub correction: String,
    pub tool_result_code: String,
}

/// 协议纠正事务的结果。
///
/// 同一 request 的重复回调返回 `newly_applied = false`，绝不会再次追加纠正消息、
/// 增加连续错误次数或重新入队。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelProtocolCorrectionResult {
    pub workflow: WorkflowSnapshot,
    pub newly_applied: bool,
    pub requeue: bool,
    pub consecutive_protocol_errors: i64,
}

impl Store {
    /// 在模型请求真正取得执行许可前封存一次不可变请求。
    ///
    /// # Errors
    ///
    /// Run 不存在、已有 in-flight 请求、快照不是 JSON object 或数据库写入失败时返回
    /// 错误。此操作不增加 `request_count`，取消 prepared 请求也不会消耗额度。
    pub async fn prepare_model_request(
        &self,
        input: NewModelRequest,
    ) -> Result<PersistedModelRequest, StoreError> {
        validate_new_request(&input)?;
        let now = utc_now_millis();
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(input.run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    ensure_run_accepts_model_request(&run_model)?;
                    let in_flight = model_request::Entity::find()
                        .filter(model_request::Column::RunId.eq(run_model.id))
                        .filter(
                            model_request::Column::Status
                                .is_in([STATUS_PREPARED, STATUS_DISPATCHED]),
                        )
                        .one(transaction)
                        .await?;
                    if in_flight.is_some() {
                        return Err(StoreError::ResourceBusy(
                            "该 Run 已有正在准备或发送的模型请求".to_owned(),
                        ));
                    }
                    let previous = model_request::Entity::find()
                        .filter(model_request::Column::RunId.eq(run_model.id))
                        .order_by_desc(model_request::Column::AttemptNo)
                        .one(transaction)
                        .await?;
                    let attempt_no = match previous {
                        Some(request) => request.attempt_no.checked_add(1).ok_or_else(|| {
                            StoreError::InvalidModelRequest(
                                "模型请求 attempt_no 已达到 i64 上限".to_owned(),
                            )
                        })?,
                        None => 1,
                    };
                    let model = model_request::ActiveModel {
                        id: Set(db_id(Uuid::new_v4())),
                        run_id: Set(run_model.id),
                        attempt_no: Set(attempt_no),
                        status: Set(STATUS_PREPARED.to_owned()),
                        settings_snapshot_json: Set(serde_json::to_string(
                            &input.settings_snapshot,
                        )?),
                        prompt_snapshot_json: Set(serde_json::to_string(&input.prompt_snapshot)?),
                        context_snapshot_json: Set(serde_json::to_string(&input.context_snapshot)?),
                        provider_request_id: Set(None),
                        usage_json: Set(None),
                        error_json: Set(None),
                        created_at: Set(now),
                        updated_at: Set(now),
                        dispatched_at: Set(None),
                        completed_at: Set(None),
                        ..Default::default()
                    }
                    .insert(transaction)
                    .await?;
                    persisted_model_request_from_model(&model)
                })
            })
            .await
            .map_err(transaction_error)
    }

    /// 在同一短事务内追加本轮原始消息、封存请求并取得发送许可。
    ///
    /// 调度器只应在已经持有全局模型网络许可后调用本方法。这样上下文账本、不可
    /// 变请求快照、额度、Run 状态和工作流 `request_in_flight` 不会出现半提交；
    /// 网络 I/O 必须在本方法返回后才开始。
    ///
    /// # Errors
    ///
    /// 消息/快照无效、Run 或工作流不允许发送、已有 in-flight 请求、额度耗尽或
    /// 数据库操作失败时返回错误；任何错误都会回滚全部本地写入。
    pub async fn capture_and_dispatch_model_request(
        &self,
        input: NewModelRequest,
        messages: Vec<NewAgentMessage>,
    ) -> Result<PersistedModelRequest, StoreError> {
        validate_new_request(&input)?;
        let feedback_binding = feedback_binding_from_context_snapshot(&input.context_snapshot)?;
        let tool_result_binding =
            tool_result_binding_from_context_snapshot(&input.context_snapshot)?;
        // 首次请求封存 system/user 输入；后续正文 Agent 请求只复用已经不可变的
        // assistant/tool 历史，不应为了通过 API 人为再插入一条重复消息。
        if !messages.is_empty() {
            validate_new_messages(&messages)?;
        }
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(input.run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    ensure_run_accepts_model_request(&run_model)?;
                    if run_model.request_count >= run_model.request_allowance_end {
                        return Err(StoreError::RequestAllowanceExhausted {
                            request_count: run_model.request_count,
                            request_allowance_end: run_model.request_allowance_end,
                        });
                    }
                    let in_flight = model_request::Entity::find()
                        .filter(model_request::Column::RunId.eq(run_model.id))
                        .filter(
                            model_request::Column::Status
                                .is_in([STATUS_PREPARED, STATUS_DISPATCHED]),
                        )
                        .one(transaction)
                        .await?;
                    if in_flight.is_some() {
                        return Err(StoreError::ResourceBusy(
                            "该 Run 已有正在准备或发送的模型请求".to_owned(),
                        ));
                    }
                    let previous = model_request::Entity::find()
                        .filter(model_request::Column::RunId.eq(run_model.id))
                        .order_by_desc(model_request::Column::AttemptNo)
                        .one(transaction)
                        .await?;
                    let attempt_no = match previous {
                        Some(request) => request.attempt_no.checked_add(1).ok_or_else(|| {
                            StoreError::InvalidModelRequest(
                                "模型请求 attempt_no 已达到 i64 上限".to_owned(),
                            )
                        })?,
                        None => 1,
                    };
                    let next_count = run_model.request_count.checked_add(1).ok_or_else(|| {
                        StoreError::InvalidModelRequest(
                            "Run request_count 已达到 i64 上限".to_owned(),
                        )
                    })?;
                    let run_id: RunId = run_model.id.into();
                    let version_id = run_model.version_id.into();
                    let run_kind = run_model.kind.clone();
                    let tool_results = match tool_result_binding.as_ref() {
                        Some(provider_call_ids) => {
                            if run_model.parent_run_id.is_some()
                                || !matches!(
                                    run_model.kind.as_str(),
                                    "preparation" | "chapter_outline" | "chapter_body"
                                )
                            {
                                return Err(StoreError::InvalidModelRequest(
                                    "只有顶层写作 Run 可以封存工具结果".to_owned(),
                                ));
                            }
                            tool_result_models_for_capture(
                                transaction,
                                &run_model,
                                provider_call_ids,
                            )
                            .await?
                        }
                        None => Vec::new(),
                    };
                    let bound_feedback = match feedback_binding.as_ref() {
                        Some(ids) => {
                            if run_model.parent_run_id.is_some() || run_model.kind != "chapter_body"
                            {
                                return Err(StoreError::InvalidModelRequest(
                                    "只有顶层章节正文 Run 可以封存自由意见".to_owned(),
                                ));
                            }
                            feedback_models_for_capture(transaction, &run_model, ids).await?
                        }
                        None => Vec::new(),
                    };
                    ensure_bound_feedback_tools_are_captured(&bound_feedback, &tool_results)?;
                    let next_workflow = if run_model.parent_run_id.is_none() {
                        let workflow = workflow_for_version(transaction, version_id).await?;
                        Some(dispatch_workflow_model_request(&workflow, run_id)?)
                    } else {
                        None
                    };
                    if !messages.is_empty() {
                        append_agent_messages_for_run(transaction, &run_model, messages, now)
                            .await?;
                    }
                    let request_id = Uuid::new_v4();
                    let request = model_request::ActiveModel {
                        id: Set(db_id(request_id)),
                        run_id: Set(run_model.id),
                        attempt_no: Set(attempt_no),
                        status: Set(STATUS_DISPATCHED.to_owned()),
                        settings_snapshot_json: Set(serde_json::to_string(
                            &input.settings_snapshot,
                        )?),
                        prompt_snapshot_json: Set(serde_json::to_string(&input.prompt_snapshot)?),
                        context_snapshot_json: Set(serde_json::to_string(&input.context_snapshot)?),
                        provider_request_id: Set(None),
                        usage_json: Set(None),
                        error_json: Set(None),
                        created_at: Set(now),
                        updated_at: Set(now),
                        dispatched_at: Set(Some(now)),
                        completed_at: Set(None),
                        ..Default::default()
                    }
                    .insert(transaction)
                    .await?;
                    for tool_model in tool_results {
                        let tool_call_id: Uuid = tool_model.id.into();
                        let mut active_tool = tool_model.into_active_model();
                        active_tool.status = Set("included".to_owned());
                        active_tool.included_request_id = Set(Some(request.id.to_string()));
                        active_tool.update(transaction).await?;
                        insert_event(
                            transaction,
                            version_id,
                            "tool.included",
                            tool_call_id,
                            next_workflow
                                .as_ref()
                                .map_or(attempt_no, |workflow| workflow.state_revision),
                            json!({
                                "run_id": run_id,
                                "included_request_id": request_id,
                            }),
                            now,
                        )
                        .await?;
                    }
                    for feedback_model in bound_feedback {
                        let feedback_id: Uuid = feedback_model.id.into();
                        let mut active_feedback = feedback_model.into_active_model();
                        active_feedback.status = Set("included".to_owned());
                        active_feedback.included_request_id = Set(Some(request.id.to_string()));
                        active_feedback.updated_at = Set(now);
                        active_feedback.update(transaction).await?;
                        insert_event(
                            transaction,
                            version_id,
                            "feedback.changed",
                            feedback_id,
                            next_workflow
                                .as_ref()
                                .map_or(attempt_no, |workflow| workflow.state_revision),
                            json!({
                                "run_id": run_id,
                                "status": "included",
                                "included_request_id": request_id,
                            }),
                            now,
                        )
                        .await?;
                    }
                    let mut active_run = run_model.into_active_model();
                    active_run.request_count = Set(next_count);
                    active_run.status = Set("running".to_owned());
                    active_run.updated_at = Set(now);
                    active_run.update(transaction).await?;
                    if let Some(workflow) = &next_workflow {
                        persist_workflow(transaction, workflow).await?;
                    }
                    let event_revision = next_workflow
                        .as_ref()
                        .map_or(attempt_no, |workflow| workflow.state_revision);
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        event_revision,
                        json!({
                            "kind": run_kind,
                            "status": "running",
                            "request_id": request_id,
                            "request_count": next_count.to_string(),
                        }),
                        now,
                    )
                    .await?;
                    if let Some(workflow) = next_workflow {
                        insert_event(
                            transaction,
                            version_id,
                            "workflow.changed",
                            version_id,
                            workflow.state_revision,
                            json!({
                                "stage": workflow.stage,
                                "execution": workflow.execution,
                                "run_id": run_id,
                                "request_id": request_id,
                            }),
                            now,
                        )
                        .await?;
                    }
                    persisted_model_request_from_model(&request)
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok(request) => {
                self.notify_event_listeners();
                Ok(request)
            }
            Err(error) => Err(error),
        }
    }

    /// 将尚未创建 `model_requests` 记录的排队任务显式标记为失败。
    ///
    /// 调度器在取得网络许可后才读取最新设置和构建上下文；若这一步失败，不能让
    /// Run 永远停留在 `queued`，也不能伪造一条从未发送的模型请求。本方法只会处理
    /// 仍是当前顶层写作 Run 的 queued 状态；过期回调安全地返回 `false`。
    pub async fn fail_queued_model_run(
        &self,
        run_id: RunId,
        error: Value,
    ) -> Result<bool, StoreError> {
        self.finish_queued_model_run(run_id, error, QueuedRunTerminal::Failed)
            .await
    }

    /// 因本轮工程额度耗尽而停止尚未发送的排队任务。
    ///
    /// 停止不伪造模型请求，也不删除未投递内容；用户只能通过显式 continue 为同一
    /// Run 续授额度，普通暂停恢复或提交意见不会让它重新排队。
    pub async fn stop_queued_model_run(
        &self,
        run_id: RunId,
        error: Value,
    ) -> Result<bool, StoreError> {
        self.finish_queued_model_run(run_id, error, QueuedRunTerminal::Stopped)
            .await
    }

    async fn finish_queued_model_run(
        &self,
        run_id: RunId,
        error: Value,
        terminal: QueuedRunTerminal,
    ) -> Result<bool, StoreError> {
        validate_json_object("scheduler_error", &error)?;
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                let error = error.clone();
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some() || run_model.status != "queued" {
                        return Ok(false);
                    }
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    if workflow.active_run_id != Some(run_id)
                        || workflow.execution != ExecutionStatus::Queued
                    {
                        return Ok(false);
                    }
                    let next_workflow = match terminal {
                        QueuedRunTerminal::Failed => fail_queued_workflow_run(&workflow, run_id)?,
                        QueuedRunTerminal::Stopped => stop_queued_workflow_run(&workflow, run_id)?,
                    };
                    let run_kind = run_model.kind.clone();
                    let request_count = run_model.request_count;
                    let mut checkpoint: Value = serde_json::from_str(&run_model.checkpoint_json)?;
                    let checkpoint = checkpoint.as_object_mut().ok_or_else(|| {
                        StoreError::CorruptRecord("Run 检查点必须是 JSON object".to_owned())
                    })?;
                    checkpoint.insert("scheduler_error".to_owned(), error.clone());

                    let mut active_run = run_model.into_active_model();
                    active_run.status = Set(terminal.status().to_owned());
                    active_run.checkpoint_json = Set(serde_json::to_string(&checkpoint)?);
                    active_run.ended_at = Set(Some(now));
                    active_run.updated_at = Set(now);
                    active_run.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({
                            "kind": run_kind,
                            "status": terminal.status(),
                            "request_count": request_count.to_string(),
                            "error": error,
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "execution": next_workflow.execution,
                            "run_id": run_id,
                        }),
                        now,
                    )
                    .await?;
                    Ok(true)
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok(changed) => {
                if changed {
                    self.notify_event_listeners();
                }
                Ok(changed)
            }
            Err(error) => Err(error),
        }
    }

    /// 将一次已验证的准备阶段模型响应、原始 assistant 消息、产物修订、审核和
    /// 请求终态作为同一个 SQLite 事务提交。
    ///
    /// 该入口只接受当前顶层 preparation Run 的 `dispatched` attempt。它跳过短暂的
    /// `tool_boundary`：模型已经给出最终阶段提交时，工作流在同一提交中直接进入
    /// `waiting_review`，不会暴露“请求已成功但产物尚未落库”的可恢复中间状态。
    ///
    /// # Errors
    ///
    /// 请求不属于当前准备 Run、响应重复但诊断不一致、产物不满足阶段守卫，或任一
    /// 持久化动作失败时返回错误；失败事务不会留下 assistant 消息或部分产物。
    pub async fn commit_preparation_model_submission(
        &self,
        request_id: Uuid,
        provider_request_id: Option<String>,
        usage: Value,
        raw_response: String,
        artifacts: Vec<PreparationArtifactInput>,
        chapter_plan: Option<Vec<ChapterPlanDraft>>,
    ) -> Result<PreparationSnapshot, StoreError> {
        if raw_response.trim().is_empty() {
            return Err(StoreError::InvalidModelRequest(
                "准备阶段模型响应不能为空".to_owned(),
            ));
        }
        let resolution = ModelRequestResolution::Succeeded {
            provider_request_id,
            usage,
        };
        validate_resolution(&resolution)?;
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    let request = model_request::Entity::find_by_id(db_id(request_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ModelRequestNotFound)?;
                    let current_status = model_request_status_from_db(&request.status)?;
                    let run_model = run::Entity::find_by_id(request.run_id)
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    let run_id: RunId = run_model.id.into();
                    let version_id = run_model.version_id.into();
                    if current_status.is_terminal() {
                        if current_status == ModelRequestStatus::Succeeded
                            && resolution_matches(&request, &resolution)?
                        {
                            return Ok((
                                preparation_snapshot(transaction, version_id).await?,
                                false,
                            ));
                        }
                        return Err(StoreError::InvalidModelRequestState(
                            "模型请求已有不同终态，拒绝覆盖准备阶段提交".to_owned(),
                        ));
                    }
                    if current_status != ModelRequestStatus::Dispatched {
                        return Err(StoreError::InvalidModelRequestState(format!(
                            "只有 dispatched 请求可以提交准备产物，当前为 {}",
                            request.status
                        )));
                    }
                    if run_model.parent_run_id.is_some() || run_model.kind != "preparation" {
                        return Err(StoreError::InvalidModelRequestState(
                            "该模型请求不属于顶层准备阶段 Run".to_owned(),
                        ));
                    }

                    let workflow = workflow_for_version(transaction, version_id).await?;
                    if workflow.active_run_id != Some(run_id)
                        || !matches!(
                            workflow.stage,
                            WorkflowStage::PreparingGenerating | WorkflowStage::PreparingRevising
                        )
                        || workflow.execution != ExecutionStatus::RequestInFlight
                    {
                        return Err(StoreError::InvalidModelRequestState(
                            "准备阶段工作流已不接受此模型响应".to_owned(),
                        ));
                    }

                    append_agent_messages_for_run(
                        transaction,
                        &run_model,
                        vec![NewAgentMessage {
                            role: AgentMessageRole::Assistant,
                            content: json!({
                                "kind": "text",
                                "role": "assistant",
                                "content": raw_response,
                            }),
                            request_id: Some(request_id),
                            tool_call_id: None,
                        }],
                        now,
                    )
                    .await?;
                    let next_workflow = persist_preparation_submission(
                        transaction,
                        version_id,
                        &workflow,
                        &artifacts,
                        chapter_plan.as_deref(),
                        "agent",
                        now,
                    )
                    .await?;

                    let mut active_request = request.into_active_model();
                    active_request.status = Set(STATUS_SUCCEEDED.to_owned());
                    active_request.provider_request_id =
                        Set(resolution.provider_request_id().map(str::to_owned));
                    active_request.usage_json =
                        Set(resolution.usage().map(serde_json::to_string).transpose()?);
                    active_request.error_json = Set(None);
                    active_request.updated_at = Set(now);
                    active_request.completed_at = Set(Some(now));
                    active_request.update(transaction).await?;
                    update_preparation_run_status(transaction, run_id, "waiting", None, None, now)
                        .await?;
                    reset_run_protocol_errors(transaction, run_id, now).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({
                            "kind": "preparation",
                            "status": "waiting",
                            "request_id": request_id,
                            "request_count": run_model.request_count.to_string(),
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "execution": next_workflow.execution,
                            "run_id": run_id,
                            "request_id": request_id,
                        }),
                        now,
                    )
                    .await?;
                    Ok((preparation_snapshot(transaction, version_id).await?, true))
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok((snapshot, changed)) => {
                if changed {
                    self.notify_event_listeners();
                }
                Ok(snapshot)
            }
            Err(error) => Err(error),
        }
    }

    /// 将一份已验证的章节细纲模型响应与细纲修订、审核和请求终态原子提交。
    ///
    /// 细纲最终提交已到达时，工作流在同一事务内直接进入 `waiting_review`；不会向
    /// 其他读取者暴露“请求成功但细纲尚未成为正式审核对象”的中间状态。
    pub async fn commit_chapter_outline_model_submission(
        &self,
        request_id: Uuid,
        chapter_id: ChapterId,
        provider_request_id: Option<String>,
        usage: Value,
        raw_response: String,
        input: NewChapterOutline,
    ) -> Result<ChapterOutlineSnapshot, StoreError> {
        if raw_response.trim().is_empty() {
            return Err(StoreError::InvalidModelRequest(
                "章节细纲模型响应不能为空".to_owned(),
            ));
        }
        let resolution = ModelRequestResolution::Succeeded {
            provider_request_id,
            usage,
        };
        validate_resolution(&resolution)?;
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    let request = model_request::Entity::find_by_id(db_id(request_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ModelRequestNotFound)?;
                    let current_status = model_request_status_from_db(&request.status)?;
                    let run_model = run::Entity::find_by_id(request.run_id)
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    let run_id: RunId = run_model.id.into();
                    let version_id = run_model.version_id.into();
                    if current_status.is_terminal() {
                        if current_status == ModelRequestStatus::Succeeded
                            && resolution_matches(&request, &resolution)?
                        {
                            return Ok((
                                chapter_outline_snapshot(transaction, chapter_id).await?,
                                false,
                            ));
                        }
                        return Err(StoreError::InvalidModelRequestState(
                            "模型请求已有不同终态，拒绝覆盖章节细纲提交".to_owned(),
                        ));
                    }
                    if current_status != ModelRequestStatus::Dispatched {
                        return Err(StoreError::InvalidModelRequestState(format!(
                            "只有 dispatched 请求可以提交章节细纲，当前为 {}",
                            request.status
                        )));
                    }
                    if run_model.parent_run_id.is_some() || run_model.kind != "chapter_outline" {
                        return Err(StoreError::InvalidModelRequestState(
                            "该模型请求不属于顶层章节细纲 Run".to_owned(),
                        ));
                    }

                    let workflow = workflow_for_version(transaction, version_id).await?;
                    if workflow.active_run_id != Some(run_id)
                        || workflow.active_chapter_id != Some(chapter_id)
                        || workflow.stage != WorkflowStage::WritingOutlineGenerating
                        || workflow.execution != ExecutionStatus::RequestInFlight
                    {
                        return Err(StoreError::InvalidModelRequestState(
                            "章节细纲工作流已不接受此模型响应".to_owned(),
                        ));
                    }

                    append_agent_messages_for_run(
                        transaction,
                        &run_model,
                        vec![NewAgentMessage {
                            role: AgentMessageRole::Assistant,
                            content: json!({
                                "kind": "text",
                                "role": "assistant",
                                "content": raw_response,
                            }),
                            request_id: Some(request_id),
                            tool_call_id: None,
                        }],
                        now,
                    )
                    .await?;
                    persist_chapter_outline_submission(
                        transaction,
                        chapter_id,
                        &workflow,
                        &input,
                        "agent",
                        now,
                    )
                    .await?;
                    reset_run_protocol_errors(transaction, run_id, now).await?;

                    let mut active_request = request.into_active_model();
                    active_request.status = Set(STATUS_SUCCEEDED.to_owned());
                    active_request.provider_request_id =
                        Set(resolution.provider_request_id().map(str::to_owned));
                    active_request.usage_json =
                        Set(resolution.usage().map(serde_json::to_string).transpose()?);
                    active_request.error_json = Set(None);
                    active_request.updated_at = Set(now);
                    active_request.completed_at = Set(Some(now));
                    active_request.update(transaction).await?;
                    Ok((
                        chapter_outline_snapshot(transaction, chapter_id).await?,
                        true,
                    ))
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok((snapshot, changed)) => {
                if changed {
                    self.notify_event_listeners();
                }
                Ok(snapshot)
            }
            Err(error) => Err(error),
        }
    }

    /// 在模型网络调用前原子标记 dispatch 并消耗一次请求额度。
    ///
    /// # Errors
    ///
    /// 请求不处于 prepared 状态、Run 不存在、额度耗尽或数据库操作失败时返回错误。
    /// 对同一已 dispatched 请求重复调用会返回原记录而不会再次增加计数。
    pub async fn dispatch_model_request(
        &self,
        request_id: Uuid,
    ) -> Result<PersistedModelRequest, StoreError> {
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    let request = model_request::Entity::find_by_id(db_id(request_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ModelRequestNotFound)?;
                    let status = model_request_status_from_db(&request.status)?;
                    if status == ModelRequestStatus::Dispatched {
                        return Ok((persisted_model_request_from_model(&request)?, false));
                    }
                    if status != ModelRequestStatus::Prepared {
                        return Err(StoreError::InvalidModelRequestState(format!(
                            "只有 prepared 请求可以 dispatch，当前为 {}",
                            request.status
                        )));
                    }
                    let run_model = run::Entity::find_by_id(request.run_id)
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    ensure_run_accepts_model_request(&run_model)?;
                    if run_model.request_count >= run_model.request_allowance_end {
                        return Err(StoreError::RequestAllowanceExhausted {
                            request_count: run_model.request_count,
                            request_allowance_end: run_model.request_allowance_end,
                        });
                    }
                    let next_count = run_model.request_count.checked_add(1).ok_or_else(|| {
                        StoreError::InvalidModelRequest(
                            "Run request_count 已达到 i64 上限".to_owned(),
                        )
                    })?;
                    let run_id: RunId = run_model.id.into();
                    let version_id = run_model.version_id.into();
                    let run_kind = run_model.kind.clone();
                    let next_workflow = if run_model.parent_run_id.is_none() {
                        let workflow = workflow_for_version(transaction, version_id).await?;
                        Some(dispatch_workflow_model_request(&workflow, run_id)?)
                    } else {
                        None
                    };
                    let mut active_request = request.into_active_model();
                    active_request.status = Set(STATUS_DISPATCHED.to_owned());
                    active_request.updated_at = Set(now);
                    active_request.dispatched_at = Set(Some(now));
                    let request = active_request.update(transaction).await?;

                    let mut active_run = run_model.into_active_model();
                    active_run.request_count = Set(next_count);
                    active_run.status = Set("running".to_owned());
                    active_run.updated_at = Set(now);
                    active_run.update(transaction).await?;
                    if let Some(workflow) = &next_workflow {
                        persist_workflow(transaction, workflow).await?;
                    }
                    let event_revision = next_workflow
                        .as_ref()
                        .map_or(request.attempt_no, |workflow| workflow.state_revision);
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        event_revision,
                        json!({
                            "kind": run_kind,
                            "status": "running",
                            "request_id": request_id,
                            "request_count": next_count.to_string(),
                        }),
                        now,
                    )
                    .await?;
                    if let Some(workflow) = next_workflow {
                        insert_event(
                            transaction,
                            version_id,
                            "workflow.changed",
                            version_id,
                            workflow.state_revision,
                            json!({
                                "stage": workflow.stage,
                                "execution": workflow.execution,
                                "run_id": run_id,
                                "request_id": request_id,
                            }),
                            now,
                        )
                        .await?;
                    }
                    Ok((persisted_model_request_from_model(&request)?, true))
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok((request, changed)) => {
                if changed {
                    self.notify_event_listeners();
                }
                Ok(request)
            }
            Err(error) => Err(error),
        }
    }

    /// 将一条完整但不合规的模型响应保存为可纠正的历史，并决定是否继续同一 Run。
    ///
    /// 提供商已经成功返回，因此请求本身标记为 `succeeded` 并保留 usage；`error_json`
    /// 记录本地协议校验失败。若响应带有 tool calls，会先写入每个 call 的配对错误
    /// result，再追加一条用户角色的纠正消息，保证下一轮请求仍满足模型消息协议。
    /// 前两次连续错误重新排队；第 [`MAX_CONSECUTIVE_PROTOCOL_ERRORS`] 次停止，必须由
    /// 用户通过原 Run 的显式 continue 重新授予额度。
    ///
    /// # Errors
    ///
    /// 请求或工作流不再处于当前 `request_in_flight`、输入 envelope 无效、或数据库
    /// 写入失败时返回错误。重复的同一回调返回 `newly_applied = false`，不重复写入。
    pub async fn record_model_protocol_correction(
        &self,
        request_id: Uuid,
        correction: ModelProtocolCorrection,
    ) -> Result<ModelProtocolCorrectionResult, StoreError> {
        validate_model_protocol_correction(request_id, &correction)?;
        let mut messages = vec![correction.assistant_message.clone()];
        messages.extend(protocol_error_tool_result_messages(
            request_id,
            &correction.assistant_message,
            &correction.correction,
            &correction.tool_result_code,
        )?);
        messages.push(NewAgentMessage {
            role: AgentMessageRole::User,
            content: json!({
                "kind": "protocol_correction",
                "content": correction.correction,
            }),
            request_id: Some(request_id),
            tool_call_id: None,
        });
        validate_new_messages(&messages)?;

        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                let correction = correction.clone();
                let messages = messages.clone();
                Box::pin(async move {
                    let request = model_request::Entity::find_by_id(db_id(request_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ModelRequestNotFound)?;
                    let run_model = run::Entity::find_by_id(request.run_id)
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    let run_id: RunId = run_model.id.into();
                    let version_id: VersionId = run_model.version_id.into();

                    let current_status = model_request_status_from_db(&request.status)?;
                    if current_status.is_terminal() {
                        if protocol_correction_matches(&request, &correction)? {
                            return Ok((
                                ModelProtocolCorrectionResult {
                                    workflow: workflow_for_version(transaction, version_id).await?,
                                    newly_applied: false,
                                    requeue: false,
                                    consecutive_protocol_errors: run_model
                                        .consecutive_protocol_errors,
                                },
                                false,
                            ));
                        }
                        return Err(StoreError::InvalidModelRequestState(
                            "模型请求已有不同终态，拒绝覆盖协议纠正记录".to_owned(),
                        ));
                    }
                    if current_status != ModelRequestStatus::Dispatched {
                        return Err(StoreError::InvalidModelRequestState(format!(
                            "只有 dispatched 请求可以写入协议纠正结果，当前为 {}",
                            request.status
                        )));
                    }
                    if run_model.parent_run_id.is_some() {
                        return Err(StoreError::InvalidModelRequestState(
                            "协议纠正只能用于顶层写作 Run".to_owned(),
                        ));
                    }
                    if run_model.consecutive_protocol_errors < 0 {
                        return Err(StoreError::CorruptRecord(
                            "Run consecutive_protocol_errors 不能为负数".to_owned(),
                        ));
                    }
                    let next_error_count = run_model
                        .consecutive_protocol_errors
                        .checked_add(1)
                        .ok_or_else(|| {
                            StoreError::InvalidModelRequest(
                                "Run consecutive_protocol_errors 已达到 i64 上限".to_owned(),
                            )
                        })?;
                    let stop = next_error_count >= MAX_CONSECUTIVE_PROTOCOL_ERRORS;
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    let next_workflow = if stop {
                        stop_writing_after_protocol_error(&workflow, run_id)?
                    } else {
                        continue_after_agent_tool(&workflow, run_id)?
                    };

                    append_agent_messages_for_run(transaction, &run_model, messages, now).await?;

                    let mut active_request = request.into_active_model();
                    active_request.status = Set(STATUS_SUCCEEDED.to_owned());
                    active_request.provider_request_id = Set(correction.provider_request_id);
                    active_request.usage_json = Set(Some(serde_json::to_string(&correction.usage)?));
                    active_request.error_json = Set(Some(serde_json::to_string(&correction.error)?));
                    active_request.updated_at = Set(now);
                    active_request.completed_at = Set(Some(now));
                    active_request.update(transaction).await?;

                    let run_kind = run_model.kind.clone();
                    let request_count = run_model.request_count;
                    let mut active_run = run_model.into_active_model();
                    active_run.status = Set((if stop { "stopped" } else { "queued" }).to_owned());
                    active_run.consecutive_protocol_errors = Set(next_error_count);
                    active_run.ended_at = Set(if stop { Some(now) } else { None });
                    active_run.updated_at = Set(now);
                    active_run.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;

                    let run_status = if stop { "stopped" } else { "queued" };
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({
                            "kind": run_kind,
                            "status": run_status,
                            "request_id": request_id,
                            "request_count": request_count.to_string(),
                            "consecutive_protocol_errors": next_error_count.to_string(),
                            "wait_reason": if stop { Some("protocol_error_limit") } else { None::<&str> },
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "execution": next_workflow.execution,
                            "run_id": run_id,
                            "request_id": request_id,
                            "protocol_correction": true,
                            "consecutive_protocol_errors": next_error_count.to_string(),
                        }),
                        now,
                    )
                    .await?;
                    Ok((
                        ModelProtocolCorrectionResult {
                            workflow: next_workflow,
                            newly_applied: true,
                            requeue: !stop,
                            consecutive_protocol_errors: next_error_count,
                        },
                        true,
                    ))
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok((result, changed)) => {
                if changed {
                    self.notify_event_listeners();
                }
                Ok(result)
            }
            Err(error) => Err(error),
        }
    }

    /// 将一个已发出的请求解析为 succeeded、failed 或 unknown；同一回调重复到达时
    /// 只接受完全相同的终态并返回原记录。
    ///
    /// # Errors
    ///
    /// 请求不存在、尚未 dispatch、已有不同终态，或结果 JSON 不符合对象合同则返回
    /// 错误。此操作从不修改 Run 的请求计数。
    pub async fn resolve_model_request(
        &self,
        request_id: Uuid,
        resolution: ModelRequestResolution,
    ) -> Result<PersistedModelRequest, StoreError> {
        self.resolve_model_request_inner(request_id, resolution, None)
            .await
    }

    /// 将模型的完整 assistant 响应与 attempt 终态作为同一个事务提交。
    ///
    /// 这用于协议错误、工具调用被拒绝等没有正式阶段产物的回调，确保原始响应不会
    /// 因为随后将 Run 标为 failed/unknown 而丢失。重复回调命中既有终态时不会再次
    /// 追加消息。
    pub async fn resolve_model_request_with_assistant_message(
        &self,
        request_id: Uuid,
        resolution: ModelRequestResolution,
        assistant_message: NewAgentMessage,
    ) -> Result<PersistedModelRequest, StoreError> {
        self.resolve_model_request_inner(request_id, resolution, Some(assistant_message))
            .await
    }

    async fn resolve_model_request_inner(
        &self,
        request_id: Uuid,
        resolution: ModelRequestResolution,
        assistant_message: Option<NewAgentMessage>,
    ) -> Result<PersistedModelRequest, StoreError> {
        validate_resolution(&resolution)?;
        if let Some(message) = &assistant_message {
            if message.role != AgentMessageRole::Assistant {
                return Err(StoreError::InvalidAgentMessage(
                    "模型响应消息必须使用 assistant 角色".to_owned(),
                ));
            }
            if message.request_id != Some(request_id) {
                return Err(StoreError::InvalidAgentMessage(
                    "模型响应消息必须关联当前 model_request".to_owned(),
                ));
            }
            validate_new_messages(std::slice::from_ref(message))?;
        }
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                let assistant_message = assistant_message.clone();
                Box::pin(async move {
                    let request = model_request::Entity::find_by_id(db_id(request_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ModelRequestNotFound)?;
                    let target_status = resolution.status();
                    let current_status = model_request_status_from_db(&request.status)?;
                    if current_status.is_terminal() {
                        if resolution_matches(&request, &resolution)? {
                            return Ok((persisted_model_request_from_model(&request)?, false));
                        }
                        return Err(StoreError::InvalidModelRequestState(
                            "模型请求已有不同终态，拒绝覆盖诊断记录".to_owned(),
                        ));
                    }
                    if current_status != ModelRequestStatus::Dispatched {
                        return Err(StoreError::InvalidModelRequestState(format!(
                            "只有 dispatched 请求可以写入终态，当前为 {}",
                            request.status
                        )));
                    }

                    let run_model = run::Entity::find_by_id(request.run_id)
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    let run_id: RunId = run_model.id.into();
                    let version_id = run_model.version_id.into();
                    let run_kind = run_model.kind.clone();
                    let request_count = run_model.request_count;
                    let completion = workflow_completion(&resolution);
                    let next_workflow = if run_model.parent_run_id.is_none() {
                        let workflow = workflow_for_version(transaction, version_id).await?;
                        Some(complete_workflow_model_request(
                            &workflow, run_id, completion,
                        )?)
                    } else {
                        None
                    };

                    if let Some(assistant_message) = assistant_message {
                        append_agent_messages_for_run(
                            transaction,
                            &run_model,
                            vec![assistant_message],
                            now,
                        )
                        .await?;
                    }

                    let mut active_request = request.into_active_model();
                    active_request.status =
                        Set(model_request_status_name(target_status).to_owned());
                    active_request.provider_request_id =
                        Set(resolution.provider_request_id().map(str::to_owned));
                    active_request.usage_json =
                        Set(resolution.usage().map(serde_json::to_string).transpose()?);
                    active_request.error_json =
                        Set(resolution.error().map(serde_json::to_string).transpose()?);
                    active_request.updated_at = Set(now);
                    active_request.completed_at = Set(Some(now));
                    let request = active_request.update(transaction).await?;
                    let mut active_run = run_model.into_active_model();
                    active_run.status = Set(run_status_after_resolution(&resolution).to_owned());
                    if matches!(&resolution, ModelRequestResolution::Succeeded { .. }) {
                        active_run.consecutive_protocol_errors = Set(0);
                    }
                    active_run.ended_at = Set(match completion {
                        ModelRequestCompletion::Succeeded => None,
                        ModelRequestCompletion::Failed | ModelRequestCompletion::Unknown => {
                            Some(now)
                        }
                    });
                    active_run.updated_at = Set(now);
                    active_run.update(transaction).await?;
                    if let Some(workflow) = &next_workflow {
                        persist_workflow(transaction, workflow).await?;
                    }
                    let event_revision = next_workflow
                        .as_ref()
                        .map_or(request.attempt_no, |workflow| workflow.state_revision);
                    let run_status = run_status_after_resolution(&resolution);
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        event_revision,
                        json!({
                            "kind": run_kind,
                            "status": run_status,
                            "request_id": request_id,
                            "request_count": request_count.to_string(),
                        }),
                        now,
                    )
                    .await?;
                    if let Some(workflow) = next_workflow {
                        insert_event(
                            transaction,
                            version_id,
                            "workflow.changed",
                            version_id,
                            workflow.state_revision,
                            json!({
                                "stage": workflow.stage,
                                "execution": workflow.execution,
                                "run_id": run_id,
                                "request_id": request_id,
                            }),
                            now,
                        )
                        .await?;
                    }
                    Ok((persisted_model_request_from_model(&request)?, true))
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok((request, changed)) => {
                if changed {
                    self.notify_event_listeners();
                }
                Ok(request)
            }
            Err(error) => Err(error),
        }
    }

    /// 作废尚未发送的 prepared 快照；不会消耗请求额度。
    ///
    /// # Errors
    ///
    /// 请求不存在、已 dispatch 或数据库操作失败时返回错误。重复取消已取消请求会返回
    /// 原记录，便于恢复流程幂等执行。
    pub async fn cancel_prepared_model_request(
        &self,
        request_id: Uuid,
    ) -> Result<PersistedModelRequest, StoreError> {
        let now = utc_now_millis();
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    let request = model_request::Entity::find_by_id(db_id(request_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ModelRequestNotFound)?;
                    let status = model_request_status_from_db(&request.status)?;
                    if status == ModelRequestStatus::Cancelled {
                        return persisted_model_request_from_model(&request);
                    }
                    if status != ModelRequestStatus::Prepared {
                        return Err(StoreError::InvalidModelRequestState(format!(
                            "只有 prepared 请求可以取消，当前为 {}",
                            request.status
                        )));
                    }
                    let mut active_request = request.into_active_model();
                    active_request.status = Set(STATUS_CANCELLED.to_owned());
                    active_request.updated_at = Set(now);
                    active_request.completed_at = Set(Some(now));
                    let request = active_request.update(transaction).await?;
                    persisted_model_request_from_model(&request)
                })
            })
            .await
            .map_err(transaction_error)
    }

    /// 读取一个 Run 的所有模型请求尝试，按 attempt 编号稳定排序。
    ///
    /// # Errors
    ///
    /// Run 不存在、记录 JSON 损坏或数据库读取失败时返回错误。
    pub async fn list_model_requests(
        &self,
        run_id: RunId,
    ) -> Result<Vec<PersistedModelRequest>, StoreError> {
        if run::Entity::find_by_id(db_id(run_id))
            .one(&self.database)
            .await?
            .is_none()
        {
            return Err(StoreError::RunNotFound);
        }
        model_request::Entity::find()
            .filter(model_request::Column::RunId.eq(db_id(run_id)))
            .order_by_asc(model_request::Column::AttemptNo)
            .all(&self.database)
            .await?
            .iter()
            .map(persisted_model_request_from_model)
            .collect()
    }
}

impl ModelRequestStatus {
    const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Unknown
        )
    }
}

impl ModelRequestResolution {
    const fn status(&self) -> ModelRequestStatus {
        match self {
            Self::Succeeded { .. } => ModelRequestStatus::Succeeded,
            Self::Failed { .. } => ModelRequestStatus::Failed,
            Self::Unknown { .. } => ModelRequestStatus::Unknown,
        }
    }

    fn provider_request_id(&self) -> Option<&str> {
        match self {
            Self::Succeeded {
                provider_request_id,
                ..
            }
            | Self::Failed {
                provider_request_id,
                ..
            }
            | Self::Unknown {
                provider_request_id,
                ..
            } => provider_request_id.as_deref(),
        }
    }

    const fn usage(&self) -> Option<&Value> {
        match self {
            Self::Succeeded { usage, .. } => Some(usage),
            Self::Failed { .. } | Self::Unknown { .. } => None,
        }
    }

    const fn error(&self) -> Option<&Value> {
        match self {
            Self::Succeeded { .. } => None,
            Self::Failed { error, .. } | Self::Unknown { error, .. } => Some(error),
        }
    }
}

fn workflow_completion(resolution: &ModelRequestResolution) -> ModelRequestCompletion {
    match resolution {
        ModelRequestResolution::Succeeded { .. } => ModelRequestCompletion::Succeeded,
        ModelRequestResolution::Failed { .. } => ModelRequestCompletion::Failed,
        ModelRequestResolution::Unknown { .. } => ModelRequestCompletion::Unknown,
    }
}

fn run_status_after_resolution(resolution: &ModelRequestResolution) -> &'static str {
    match resolution {
        ModelRequestResolution::Succeeded { .. } => "running",
        ModelRequestResolution::Failed { .. } | ModelRequestResolution::Unknown { .. } => "failed",
    }
}

fn validate_new_request(input: &NewModelRequest) -> Result<(), StoreError> {
    for (name, value) in [
        ("settings_snapshot", &input.settings_snapshot),
        ("prompt_snapshot", &input.prompt_snapshot),
        ("context_snapshot", &input.context_snapshot),
    ] {
        validate_json_object(name, value)?;
    }
    Ok(())
}

fn validate_resolution(resolution: &ModelRequestResolution) -> Result<(), StoreError> {
    if resolution
        .provider_request_id()
        .is_some_and(|request_id| request_id.trim().is_empty())
    {
        return Err(StoreError::InvalidModelRequest(
            "provider_request_id 不能是空字符串".to_owned(),
        ));
    }
    if let Some(usage) = resolution.usage() {
        validate_json_object("usage", usage)?;
    }
    if let Some(error) = resolution.error() {
        validate_json_object("error", error)?;
    }
    Ok(())
}

fn validate_model_protocol_correction(
    request_id: Uuid,
    correction: &ModelProtocolCorrection,
) -> Result<(), StoreError> {
    if correction
        .provider_request_id
        .as_deref()
        .is_some_and(|provider_request_id| provider_request_id.trim().is_empty())
    {
        return Err(StoreError::InvalidModelRequest(
            "provider_request_id 不能是空字符串".to_owned(),
        ));
    }
    validate_json_object("usage", &correction.usage)?;
    validate_json_object("error", &correction.error)?;
    if correction.correction.trim().is_empty() {
        return Err(StoreError::InvalidModelRequest(
            "protocol correction 不能为空".to_owned(),
        ));
    }
    if correction.tool_result_code.trim().is_empty() {
        return Err(StoreError::InvalidModelRequest(
            "protocol correction 的 tool_result_code 不能为空".to_owned(),
        ));
    }
    if correction.assistant_message.role != AgentMessageRole::Assistant {
        return Err(StoreError::InvalidAgentMessage(
            "协议纠正必须保存 assistant 原始响应".to_owned(),
        ));
    }
    if correction.assistant_message.request_id != Some(request_id) {
        return Err(StoreError::InvalidAgentMessage(
            "协议纠正 assistant 消息必须关联当前 model_request".to_owned(),
        ));
    }
    Ok(())
}

/// 为一条未被实际执行的 assistant tool call 生成协议错误结果。
///
/// 这不能复用正文工具调用表：调用还没有通过阶段白名单、单调用和参数校验，因而没有
/// 可安全应用的业务 ToolCall。消息账本仍必须保持 Chat Completions 的一对一配对。
fn protocol_error_tool_result_messages(
    request_id: Uuid,
    assistant_message: &NewAgentMessage,
    correction: &str,
    code: &str,
) -> Result<Vec<NewAgentMessage>, StoreError> {
    let kind = assistant_message
        .content
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            StoreError::InvalidAgentMessage("协议纠正 assistant 消息缺少字符串 kind".to_owned())
        })?;
    if kind == "text" {
        return Ok(Vec::new());
    }
    if kind != "assistant_tool_calls" {
        return Err(StoreError::InvalidAgentMessage(format!(
            "协议纠正 assistant 消息 kind 不支持：{kind}"
        )));
    }
    let calls = assistant_message
        .content
        .get("toolCalls")
        .and_then(Value::as_array)
        .filter(|calls| !calls.is_empty())
        .ok_or_else(|| {
            StoreError::InvalidAgentMessage(
                "assistant_tool_calls 缺少非空 toolCalls 数组".to_owned(),
            )
        })?;
    let content = serde_json::to_string(&json!({
        "status": "error",
        "code": code,
        "message": correction,
    }))?;
    let mut provider_call_ids = HashSet::with_capacity(calls.len());
    calls
        .iter()
        .map(|call| {
            let provider_call_id = call
                .get("id")
                .and_then(Value::as_str)
                .filter(|provider_call_id| !provider_call_id.trim().is_empty())
                .ok_or_else(|| {
                    StoreError::InvalidAgentMessage(
                        "assistant_tool_calls 的工具调用缺少非空 id".to_owned(),
                    )
                })?;
            if !provider_call_ids.insert(provider_call_id.to_owned()) {
                return Err(StoreError::InvalidAgentMessage(
                    "assistant_tool_calls 中存在重复的工具调用 id".to_owned(),
                ));
            }
            Ok(NewAgentMessage {
                role: AgentMessageRole::Tool,
                content: json!({
                    "kind": "tool_result",
                    "content": content,
                }),
                request_id: Some(request_id),
                tool_call_id: Some(provider_call_id.to_owned()),
            })
        })
        .collect()
}

fn protocol_correction_matches(
    request: &model_request::Model,
    correction: &ModelProtocolCorrection,
) -> Result<bool, StoreError> {
    if model_request_status_from_db(&request.status)? != ModelRequestStatus::Succeeded
        || request.provider_request_id != correction.provider_request_id
    {
        return Ok(false);
    }
    let same_usage = request
        .usage_json
        .as_deref()
        .map(|usage| parse_json_object(usage, "usage"))
        .transpose()?
        .is_some_and(|usage| usage == correction.usage);
    let same_error = request
        .error_json
        .as_deref()
        .map(|error| parse_json_object(error, "error"))
        .transpose()?
        .is_some_and(|error| error == correction.error);
    Ok(same_usage && same_error)
}

fn validate_json_object(name: &str, value: &Value) -> Result<(), StoreError> {
    if value.is_object() {
        Ok(())
    } else {
        Err(StoreError::InvalidModelRequest(format!(
            "{name} 必须是 JSON object"
        )))
    }
}

fn model_request_status_name(status: ModelRequestStatus) -> &'static str {
    match status {
        ModelRequestStatus::Prepared => STATUS_PREPARED,
        ModelRequestStatus::Dispatched => STATUS_DISPATCHED,
        ModelRequestStatus::Succeeded => STATUS_SUCCEEDED,
        ModelRequestStatus::Failed => STATUS_FAILED,
        ModelRequestStatus::Cancelled => STATUS_CANCELLED,
        ModelRequestStatus::Unknown => STATUS_UNKNOWN,
    }
}

pub(crate) fn model_request_status_from_db(status: &str) -> Result<ModelRequestStatus, StoreError> {
    match status {
        STATUS_PREPARED => Ok(ModelRequestStatus::Prepared),
        STATUS_DISPATCHED => Ok(ModelRequestStatus::Dispatched),
        STATUS_SUCCEEDED => Ok(ModelRequestStatus::Succeeded),
        STATUS_FAILED => Ok(ModelRequestStatus::Failed),
        STATUS_CANCELLED => Ok(ModelRequestStatus::Cancelled),
        STATUS_UNKNOWN => Ok(ModelRequestStatus::Unknown),
        other => Err(StoreError::CorruptRecord(format!(
            "model_request.status 不是已知状态：{other}"
        ))),
    }
}

fn persisted_model_request_from_model(
    model: &model_request::Model,
) -> Result<PersistedModelRequest, StoreError> {
    let request = PersistedModelRequest {
        id: model.id.into(),
        run_id: model.run_id.into(),
        attempt_no: model.attempt_no,
        status: model_request_status_from_db(&model.status)?,
        settings_snapshot: parse_json_object(&model.settings_snapshot_json, "settings_snapshot")?,
        prompt_snapshot: parse_json_object(&model.prompt_snapshot_json, "prompt_snapshot")?,
        context_snapshot: parse_json_object(&model.context_snapshot_json, "context_snapshot")?,
        provider_request_id: model.provider_request_id.clone(),
        usage: model
            .usage_json
            .as_deref()
            .map(|json| parse_json_object(json, "usage"))
            .transpose()?,
        error: model
            .error_json
            .as_deref()
            .map(|json| parse_json_object(json, "error"))
            .transpose()?,
        created_at_ms: model.created_at,
        updated_at_ms: model.updated_at,
        dispatched_at_ms: model.dispatched_at,
        completed_at_ms: model.completed_at,
    };
    validate_persisted_model_request(&request)?;
    Ok(request)
}

fn ensure_run_accepts_model_request(run_model: &run::Model) -> Result<(), StoreError> {
    if matches!(run_model.status.as_str(), "queued" | "running") {
        return Ok(());
    }
    Err(StoreError::InvalidModelRequestState(format!(
        "Run {} 当前为 {}，不能准备或发送模型请求",
        run_model.id, run_model.status
    )))
}

/// 从正文请求快照中读取将要在本次发送时封存的自由意见 ID。
///
/// 该字段只属于持久化诊断快照，不会作为模型协议字段发送；它让 Store 能在写入
/// `model_requests` 的同一事务内验证“本轮构建看到的完整反馈批次”仍未变化。
fn feedback_binding_from_context_snapshot(
    context_snapshot: &Value,
) -> Result<Option<Vec<Uuid>>, StoreError> {
    let Some(binding) = context_snapshot.get("feedbackBinding") else {
        return Ok(None);
    };
    let ids = binding
        .get("ids")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            StoreError::InvalidModelRequest("feedbackBinding.ids 必须是 UUID 字符串数组".to_owned())
        })?;
    let mut seen = HashSet::with_capacity(ids.len());
    let mut parsed = Vec::with_capacity(ids.len());
    for value in ids {
        let id = value.as_str().ok_or_else(|| {
            StoreError::InvalidModelRequest("feedbackBinding.ids 必须是 UUID 字符串数组".to_owned())
        })?;
        let id = Uuid::parse_str(id).map_err(|_| {
            StoreError::InvalidModelRequest("feedbackBinding.ids 包含无效 UUID".to_owned())
        })?;
        if !seen.insert(id) {
            return Err(StoreError::InvalidModelRequest(
                "feedbackBinding.ids 不能重复".to_owned(),
            ));
        }
        parsed.push(id);
    }
    Ok(Some(parsed))
}

/// 从正文请求诊断快照中读取本轮首次封存的工具结果调用 ID。
///
/// 这不是模型协议字段。它只让 Store 在同一事务中把“调度器确实放进请求消息数组的
/// 未封存结果”与数据库状态精确比对，随后标记为 included。
fn tool_result_binding_from_context_snapshot(
    context_snapshot: &Value,
) -> Result<Option<Vec<String>>, StoreError> {
    let Some(binding) = context_snapshot.get("toolResultBinding") else {
        return Ok(None);
    };
    let provider_call_ids = binding
        .get("providerCallIds")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            StoreError::InvalidModelRequest(
                "toolResultBinding.providerCallIds 必须是字符串数组".to_owned(),
            )
        })?;
    let mut seen = HashSet::with_capacity(provider_call_ids.len());
    let mut parsed = Vec::with_capacity(provider_call_ids.len());
    for value in provider_call_ids {
        let provider_call_id = value.as_str().ok_or_else(|| {
            StoreError::InvalidModelRequest(
                "toolResultBinding.providerCallIds 必须是字符串数组".to_owned(),
            )
        })?;
        if provider_call_id.trim().is_empty() {
            return Err(StoreError::InvalidModelRequest(
                "toolResultBinding.providerCallIds 不能包含空字符串".to_owned(),
            ));
        }
        if !seen.insert(provider_call_id.to_owned()) {
            return Err(StoreError::InvalidModelRequest(
                "toolResultBinding.providerCallIds 不能重复".to_owned(),
            ));
        }
        parsed.push(provider_call_id.to_owned());
    }
    Ok(Some(parsed))
}

/// 校验并读取本次请求要封存的完整 `bound` 反馈批次。
///
/// 请求构建到 SQLite commit 之间有新的意见加入同一批次时，调用方必须重新构建
/// 上下文；否则 A 已经绑定、B 在封存前到达的竞态会把 B 错投到下一轮。
async fn feedback_models_for_capture<C: sea_orm::ConnectionTrait>(
    connection: &C,
    run_model: &run::Model,
    expected_ids: &[Uuid],
) -> Result<Vec<feedback::Model>, StoreError> {
    let models = feedback::Entity::find()
        .filter(feedback::Column::RunId.eq(run_model.id))
        .filter(feedback::Column::Status.eq("bound"))
        .order_by_asc(feedback::Column::Sequence)
        .all(connection)
        .await?;
    let actual_ids = models
        .iter()
        .map(|model| {
            if model.bound_tool_call_id.is_none() || model.included_request_id.is_some() {
                Err(StoreError::CorruptRecord(
                    "bound 自由意见的工具或请求锚点无效".to_owned(),
                ))
            } else {
                Ok::<Uuid, StoreError>(model.id.into())
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    if actual_ids != expected_ids {
        return Err(StoreError::FeedbackSnapshotStale);
    }
    Ok(models)
}

/// 校验并读取本次请求会首次携带的全部写作工具结果。
async fn tool_result_models_for_capture<C: sea_orm::ConnectionTrait>(
    connection: &C,
    run_model: &run::Model,
    expected_provider_call_ids: &[String],
) -> Result<Vec<tool_call::Model>, StoreError> {
    let models = tool_call::Entity::find()
        .filter(tool_call::Column::RunId.eq(run_model.id))
        .filter(tool_call::Column::Status.is_in(["resolved", "error"]))
        .filter(tool_call::Column::ResultJson.is_not_null())
        .filter(tool_call::Column::IncludedRequestId.is_null())
        .order_by_asc(tool_call::Column::Sequence)
        .all(connection)
        .await?;
    let actual_provider_call_ids = models
        .iter()
        .map(|model| {
            if model.provider_call_id.trim().is_empty()
                || model.result_json.is_none()
                || model.included_request_id.is_some()
            {
                Err(StoreError::CorruptRecord(
                    "未封存写作工具结果缺少必要字段".to_owned(),
                ))
            } else {
                Ok(model.provider_call_id.clone())
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    if actual_provider_call_ids.as_slice() != expected_provider_call_ids {
        return Err(StoreError::FeedbackSnapshotStale);
    }
    Ok(models)
}

/// 已绑定的反馈必须随其基础工具结果一起进入同一个不可变请求快照。
fn ensure_bound_feedback_tools_are_captured(
    feedback_models: &[feedback::Model],
    tool_models: &[tool_call::Model],
) -> Result<(), StoreError> {
    if feedback_models.is_empty() {
        return Ok(());
    }
    let included_tool_ids = tool_models
        .iter()
        .map(|tool| tool.id.to_string())
        .collect::<HashSet<_>>();
    for feedback_model in feedback_models {
        let bound_tool_call_id = feedback_model
            .bound_tool_call_id
            .as_deref()
            .ok_or_else(|| StoreError::CorruptRecord("bound 自由意见缺少工具锚点".to_owned()))?;
        if !included_tool_ids.contains(bound_tool_call_id) {
            return Err(StoreError::FeedbackSnapshotStale);
        }
    }
    Ok(())
}

fn validate_persisted_model_request(request: &PersistedModelRequest) -> Result<(), StoreError> {
    if request.attempt_no <= 0 {
        return Err(corrupt_request("attempt_no 必须为正整数"));
    }
    if request.updated_at_ms < request.created_at_ms {
        return Err(corrupt_request("updated_at 不能早于 created_at"));
    }
    if request
        .provider_request_id
        .as_deref()
        .is_some_and(|request_id| request_id.trim().is_empty())
    {
        return Err(corrupt_request("provider_request_id 不能是空字符串"));
    }
    if let Some(dispatched_at_ms) = request.dispatched_at_ms
        && (dispatched_at_ms < request.created_at_ms || dispatched_at_ms > request.updated_at_ms)
    {
        return Err(corrupt_request("dispatched_at 超出请求生命周期"));
    }
    if let Some(completed_at_ms) = request.completed_at_ms
        && (completed_at_ms < request.created_at_ms || completed_at_ms > request.updated_at_ms)
    {
        return Err(corrupt_request("completed_at 超出请求生命周期"));
    }
    if let (Some(dispatched_at_ms), Some(completed_at_ms)) =
        (request.dispatched_at_ms, request.completed_at_ms)
        && completed_at_ms < dispatched_at_ms
    {
        return Err(corrupt_request("completed_at 不能早于 dispatched_at"));
    }

    let has_provider_request_id = request.provider_request_id.is_some();
    let has_usage = request.usage.is_some();
    let has_error = request.error.is_some();
    let valid_shape = match request.status {
        ModelRequestStatus::Prepared => {
            request.dispatched_at_ms.is_none()
                && request.completed_at_ms.is_none()
                && !has_provider_request_id
                && !has_usage
                && !has_error
        }
        ModelRequestStatus::Dispatched => {
            request.dispatched_at_ms.is_some()
                && request.completed_at_ms.is_none()
                && !has_provider_request_id
                && !has_usage
                && !has_error
        }
        ModelRequestStatus::Succeeded => {
            request.dispatched_at_ms.is_some() && request.completed_at_ms.is_some() && has_usage
        }
        ModelRequestStatus::Failed | ModelRequestStatus::Unknown => {
            request.dispatched_at_ms.is_some()
                && request.completed_at_ms.is_some()
                && !has_usage
                && has_error
        }
        ModelRequestStatus::Cancelled => {
            request.completed_at_ms.is_some()
                && !has_provider_request_id
                && !has_usage
                && match request.dispatched_at_ms {
                    // 许可取得前作废的 prepared 请求没有外部副作用，也不需要错误诊断。
                    None => !has_error,
                    // 已发送的请求必须保留取消原因，避免迟到回调被误读为普通失败。
                    Some(_) => has_error,
                }
        }
    };
    if valid_shape {
        Ok(())
    } else {
        Err(corrupt_request(format!(
            "状态 {} 与请求生命周期字段不一致",
            model_request_status_name(request.status)
        )))
    }
}

fn corrupt_request(reason: impl Into<String>) -> StoreError {
    StoreError::CorruptRecord(format!("model_request 记录损坏：{}", reason.into()))
}

fn parse_json_object(json: &str, field: &str) -> Result<Value, StoreError> {
    let value = serde_json::from_str::<Value>(json)?;
    validate_json_object(field, &value)?;
    Ok(value)
}

fn resolution_matches(
    model: &model_request::Model,
    resolution: &ModelRequestResolution,
) -> Result<bool, StoreError> {
    if model_request_status_from_db(&model.status)? != resolution.status()
        || model.provider_request_id.as_deref() != resolution.provider_request_id()
    {
        return Ok(false);
    }
    let same_usage = match (model.usage_json.as_deref(), resolution.usage()) {
        (None, None) => true,
        (Some(stored), Some(expected)) => parse_json_object(stored, "usage")? == *expected,
        _ => false,
    };
    let same_error = match (model.error_json.as_deref(), resolution.error()) {
        (None, None) => true,
        (Some(stored), Some(expected)) => parse_json_object(stored, "error")? == *expected,
        _ => false,
    };
    Ok(same_usage && same_error)
}

mod optional_decimal_i64 {
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    /// # Errors
    ///
    /// 当调用方的序列化器拒绝可空字符串值时返回其错误。
    #[allow(clippy::ref_option)] // Serde `with` 模块固定要求 `&Option<T>` 签名。
    pub fn serialize<S>(value: &Option<i64>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(value) => serializer.serialize_some(&value.to_string()),
            None => serializer.serialize_none(),
        }
    }

    /// # Errors
    ///
    /// 非空 JSON 值不是十进制字符串，或数值超出 `i64` 范围时返回反序列化错误。
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(deserializer)?
            .map(|value| {
                value.parse::<i64>().map_err(|error| {
                    D::Error::custom(format!("64 位整数必须是十进制字符串：{error}"))
                })
            })
            .transpose()
    }
}
