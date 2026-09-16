//! 正文流转与意见：编辑审核模式、暂停与恢复、停止与继续、意见提交与投递读取、用户提问回答。
//!
//! 从 `preparation/mod.rs` 的这一层原样搬出（`docs/08` 的 P3.8）：方法体逐字不变，本层自用的
//! `*_METHOD` 常量与私有 payload DTO 随命令一起搬出。五个公开结果类型由 `preparation/mod.rs` 用
//! `pub use body::control::{…}` 接回（`lib.rs` 也在转出它们）；九个方法都是 `Store` 的固有方法，
//! 兄弟模块与 app 侧原本就按方法语法调用，一行都不用改。

use std::collections::HashMap;

use nekowriter_core::{
    ApprovalMode, ChapterBodySnapshot, ExecutionStatus, FeedbackCitation, FeedbackStatus,
    QuestionId, RunId, UserQuestionAnswers, UserQuestionSchema, WorkflowSnapshot, WorkflowStage,
    cancel_chapter_body_pause_for_feedback, continue_after_user_answer, continue_writing_run,
    record_chapter_body_feedback, stop_writing_run as stop_writing_workflow_run,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
    TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::agent_messages::append_agent_messages_for_run;
use crate::entity::{feedback, model_request, question, run, tool_call};
use crate::event_log::insert_event;
use crate::{
    AgentMessageRole, Command, NewAgentMessage, Notify, Store, StoreError, VersionId, db_id,
    insert_receipt, payload_hash, transaction_error, utc_now_millis,
};

// 下面这些名字都由 `preparation/mod.rs` 再导出（它自己的项，或 `citations` / `tools/*` 的再导出），
// 按 `super::super::…` 引用就能同时让那些再导出行继续有消费者。
use super::super::{
    CandidateCitationScope, HeldChapterBodyPauseBoundary, QueuedChapterBodyFeedback,
    REQUEST_ALLOWANCE_STEP, bind_pending_feedback_to_existing_tool_for_pause,
    bind_pending_feedback_to_tool, chapter_body_run, chapter_body_run_status_matches_execution,
    chapter_body_snapshot, ensure_expected_state_revision, feedback_citations_from_json,
    feedback_status_from_db, feedback_status_name, held_chapter_body_pause_boundary,
    insert_resolved_held_chapter_body_submission_events,
    insert_resolved_held_chapter_body_tool_events, latest_unsealed_chapter_body_tool_result,
    persist_workflow, question_stage_matches_run_kind, question_tool_result,
    queue_chapter_body_feedback, resolve_held_chapter_body_submission,
    resolve_held_chapter_body_tool, validate_chapter_body_feedback_citations, workflow_for_version,
    writing_run_status_for_execution,
};

const SET_APPROVAL_MODE_METHOD: &str = "POST";
const SUBMIT_RUN_FEEDBACK_METHOD: &str = "POST";
const REQUEST_CHAPTER_BODY_PAUSE_METHOD: &str = "POST";
const RESUME_CHAPTER_BODY_METHOD: &str = "POST";
const ANSWER_QUESTION_METHOD: &str = "POST";
const CONTINUE_WRITING_RUN_METHOD: &str = "POST";
const STOP_WRITING_RUN_METHOD: &str = "POST";

#[derive(Serialize)]
struct SetApprovalModePayload {
    expected_state_revision: i64,
    mode: ApprovalMode,
}

#[derive(Serialize)]
struct SubmitRunFeedbackPayload<'a> {
    expected_state_revision: i64,
    text: &'a str,
    citations: &'a [FeedbackCitation],
}

#[derive(Serialize)]
struct AnswerQuestionPayload<'a> {
    expected_state_revision: i64,
    answers: &'a UserQuestionAnswers,
}

#[derive(Serialize)]
struct RunWorkflowControlPayload {
    expected_state_revision: i64,
}

/// 回答问题后的最小命令结果。
///
/// 前端依据其中的工作流刷新所属阶段快照；HTTP 层同时把 queued Run 交还给调度器。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswerResult {
    pub question_id: QuestionId,
    pub run_id: RunId,
    pub workflow: WorkflowSnapshot,
}

/// 显式停止一条顶层写作 Run 的结果。
///
/// `cancelled_request_ids` 是已经在同一 SQLite 事务中作废的 prepared/dispatched
/// attempt；应用层据此中止仍在内存中执行的 HTTP 任务。回执保留这些 ID，使响应
/// 丢失后的重试仍可再次发出相同的本地取消信号。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopWritingRunResult {
    pub workflow: WorkflowSnapshot,
    pub cancelled_request_ids: Vec<Uuid>,
}

/// 为什么要收束自动执行中的 Run。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterruptionSource {
    /// 上次进程以非正常方式结束，本次启动时收束。
    StartupRecovery,
    /// 本次退出服务，收尾在途请求。
    ProcessShutdown,
}

impl InterruptionSource {
    /// 持久事件里的 `reason`，也写在请求错误的 `kind` 上。
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::StartupRecovery => "startup_recovery",
            Self::ProcessShutdown => "process_shutdown",
        }
    }

    /// 用户可见原因文案的前缀。
    ///
    /// 收束命令在 `preparation/mod.rs` 里拼中文文案，所以这个前缀对它也必须可见。
    pub(crate) const fn prefix(self) -> &'static str {
        match self {
            Self::StartupRecovery => "服务重启",
            Self::ProcessShutdown => "退出服务",
        }
    }
}

/// 一次收束的结果：被中断的写作 Run 与审核委托数量。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InterruptionOutcome {
    pub writing_runs: usize,
    pub review_delegations: usize,
}

/// 一条已经绑定到正文工具结果的自由意见，供调度器把基础工具结果物化为实际模型
/// 消息。`included` 记录也会返回，因为后续请求重放历史 tool 消息时必须保持首次
/// 发出时的同一反馈内容。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedChapterBodyFeedback {
    pub provider_tool_call_id: String,
    pub id: Uuid,
    pub sequence: i64,
    pub text: String,
    pub citations: Vec<FeedbackCitation>,
    pub status: FeedbackStatus,
}
impl Store {
    /// 切换正文 Agent 的编辑审核模式。
    ///
    /// 模式是稿本级工作流控制数据。既有待审提案不会因切换而被接受、拒绝或删除；
    /// 只有之后完整接收的新编辑调用读取新的模式。
    pub async fn set_approval_mode(
        &self,
        version_id: VersionId,
        command_id: Uuid,
        expected_state_revision: i64,
        approval_mode: ApprovalMode,
    ) -> Result<WorkflowSnapshot, StoreError> {
        let target = format!("/api/versions/{version_id}/approval-mode");
        let payload_hash = payload_hash(&SetApprovalModePayload {
            expected_state_revision,
            mode: approval_mode,
        })?;
        let command = Command::new(command_id, SET_APPROVAL_MODE_METHOD, &target, &payload_hash);
        // 两段式站点：通知与否要看事务结果，收尾走 `finish`，而 `finish` 不做预查，
        // 所以事务外的预查仍由这里承担。
        if let Some(replayed) = command.replay_if_recorded(self).await? {
            return Ok(replayed);
        }

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let next_workflow =
                        nekowriter_core::set_approval_mode(&workflow, approval_mode);
                    let changed = next_workflow != workflow;
                    if changed {
                        persist_workflow(transaction, &next_workflow).await?;
                        insert_event(
                            transaction,
                            version_id,
                            "workflow.changed",
                            version_id,
                            next_workflow.state_revision,
                            json!({
                                "stage": next_workflow.stage,
                                "execution": next_workflow.execution,
                                "approval_mode": next_workflow.approval_mode,
                            }),
                            now,
                        )
                        .await?;
                    }
                    let response_json = serde_json::to_string(&next_workflow)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SET_APPROVAL_MODE_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok((next_workflow, changed))
                })
            })
            .await
            .map_err(transaction_error);
        // no-op 命令不通知：审批模式没变就不该让界面重读。
        let notify = Notify::on_change(matches!(&result, Ok((_, changed)) if *changed));
        let result = result.map(|(workflow, _)| workflow);
        command.finish(self, notify, result).await
    }

    /// 请求在当前正文 Agent 的下一个完整工具调用边界暂停。
    ///
    /// 该命令不取消模型网络调用，也不会先写入半截工具参数。实际进入 `paused` 的
    /// 线性化点位于 `execute_chapter_body_agent_tool_call` 收到完整调用时。
    pub async fn request_chapter_body_pause(
        &self,
        run_id: RunId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        let target = format!("/api/runs/{run_id}/pause");
        let payload_hash = payload_hash(&RunWorkflowControlPayload {
            expected_state_revision,
        })?;
        let command = Command::new(
            command_id,
            REQUEST_CHAPTER_BODY_PAUSE_METHOD,
            &target,
            &payload_hash,
        );
        // 两段式站点：通知与否要看事务结果，收尾走 `finish`，而 `finish` 不做预查，
        // 所以事务外的预查仍由这里承担。
        if let Some(replayed) = command.replay_if_recorded(self).await? {
            return Ok(replayed);
        }
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some() || run_model.kind != "chapter_body" {
                        return Err(StoreError::InvalidChapterState(
                            "只有当前章节正文 Run 可以请求暂停".to_owned(),
                        ));
                    }
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let chapter_id = workflow.active_chapter_id.ok_or_else(|| {
                        StoreError::InvalidChapterState("当前正文 Run 缺少活动章节".to_owned())
                    })?;
                    if workflow.active_run_id != Some(run_id)
                        || workflow.stage != WorkflowStage::WritingDrafting
                    {
                        return Err(StoreError::InvalidChapterState(
                            "当前工作流不接受正文暂停请求".to_owned(),
                        ));
                    }
                    let body_run = chapter_body_run(transaction, run_id, chapter_id).await?;
                    if !chapter_body_run_status_matches_execution(
                        &body_run.status,
                        &workflow.execution,
                    ) {
                        return Err(StoreError::CorruptRecord(
                            "正文 Run 状态与工作流执行状态不一致".to_owned(),
                        ));
                    }
                    let next_workflow =
                        nekowriter_core::request_chapter_body_pause(&workflow, chapter_id, run_id)?;
                    let changed = next_workflow != workflow;
                    if changed {
                        persist_workflow(transaction, &next_workflow).await?;
                        insert_event(
                            transaction,
                            version_id,
                            "workflow.changed",
                            version_id,
                            next_workflow.state_revision,
                            json!({
                                "stage": next_workflow.stage,
                                "execution": next_workflow.execution,
                                "chapter_id": chapter_id,
                                "run_id": run_id,
                                "pause_requested": true,
                            }),
                            now,
                        )
                        .await?;
                    }
                    let response = chapter_body_snapshot(transaction, chapter_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        REQUEST_CHAPTER_BODY_PAUSE_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok((response, changed))
                })
            })
            .await
            .map_err(transaction_error);
        // no-op 命令不通知：暂停已经请求过就不该再让界面重读。
        let notify = Notify::on_change(matches!(&result, Ok((_, changed)) if *changed));
        let result = result.map(|(response, _)| response);
        command.finish(self, notify, result).await
    }

    /// 恢复一个已经命中工具边界的正文暂停。
    ///
    /// 挂起工具调用、正文修订或提案、工具消息、反馈绑定、Run/工作流与命令回执在
    /// 同一事务中完成。恢复时重新读取审核模式：自动接受会应用候选，逐次确认会转为
    /// 待审提案；两者都不会绕过原始基础修订检查。
    pub async fn resume_chapter_body_pause(
        &self,
        run_id: RunId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        let target = format!("/api/runs/{run_id}/resume");
        let payload_hash = payload_hash(&RunWorkflowControlPayload {
            expected_state_revision,
        })?;
        let command = Command::new(
            command_id,
            RESUME_CHAPTER_BODY_METHOD,
            &target,
            &payload_hash,
        );
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some() || run_model.kind != "chapter_body" {
                        return Err(StoreError::InvalidChapterState(
                            "只有当前章节正文 Run 可以恢复暂停".to_owned(),
                        ));
                    }
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let chapter_id = workflow.active_chapter_id.ok_or_else(|| {
                        StoreError::InvalidChapterState("当前正文 Run 缺少活动章节".to_owned())
                    })?;
                    if workflow.active_run_id != Some(run_id)
                        || workflow.stage != WorkflowStage::WritingDrafting
                        || workflow.execution != ExecutionStatus::Paused
                        || !workflow.pause_requested
                    {
                        return Err(StoreError::InvalidChapterState(
                            "当前工作流不在可恢复的正文暂停状态".to_owned(),
                        ));
                    }
                    let body_run = chapter_body_run(transaction, run_id, chapter_id).await?;
                    if body_run.status != "paused" {
                        return Err(StoreError::CorruptRecord(
                            "已暂停正文工具调用的 Run 未处于 paused 状态".to_owned(),
                        ));
                    }
                    match held_chapter_body_pause_boundary(transaction, &body_run, chapter_id)
                        .await?
                    {
                        HeldChapterBodyPauseBoundary::Tool => {
                            let resolved = resolve_held_chapter_body_tool(
                                transaction,
                                &workflow,
                                body_run,
                                chapter_id,
                                now,
                            )
                            .await?;
                            insert_resolved_held_chapter_body_tool_events(
                                transaction,
                                version_id,
                                chapter_id,
                                run_id,
                                &resolved,
                                "agent_tool_after_pause",
                                now,
                            )
                            .await?;
                        }
                        HeldChapterBodyPauseBoundary::Submission => {
                            let resolved = resolve_held_chapter_body_submission(
                                transaction,
                                &workflow,
                                body_run,
                                chapter_id,
                                now,
                            )
                            .await?;
                            insert_resolved_held_chapter_body_submission_events(
                                transaction,
                                version_id,
                                chapter_id,
                                run_id,
                                &resolved,
                                "chapter_submission_after_pause",
                                now,
                            )
                            .await?;
                        }
                    }
                    let response = chapter_body_snapshot(transaction, chapter_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        RESUME_CHAPTER_BODY_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 显式停止当前顶层写作 Run，并作废尚未形成稳定结果的请求 attempt。
    ///
    /// 完整停止事实、取消标记、Run/工作流、事件和命令回执在同一事务中提交。事务
    /// 成功后应用层才会根据返回的 request ID 中止本地 HTTP 任务；因此即使进程在
    /// 两步之间退出，迟到回调也会因 request 已取消而无法提交正文或推进工作流。
    pub async fn stop_writing_run(
        &self,
        run_id: RunId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<StopWritingRunResult, StoreError> {
        let target = format!("/api/runs/{run_id}/stop");
        let payload_hash = payload_hash(&RunWorkflowControlPayload {
            expected_state_revision,
        })?;
        let command = Command::new(command_id, STOP_WRITING_RUN_METHOD, &target, &payload_hash);
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some()
                        || !matches!(
                            run_model.kind.as_str(),
                            "preparation" | "chapter_outline" | "chapter_body"
                        )
                    {
                        return Err(StoreError::InvalidModelRequestState(
                            "只有顶层写作 Run 可以显式停止".to_owned(),
                        ));
                    }
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let expected_run_status = writing_run_status_for_execution(&workflow.execution)
                        .ok_or_else(|| {
                            StoreError::InvalidModelRequestState(
                                "当前 Run 不在可显式停止的执行状态".to_owned(),
                            )
                        })?;
                    if run_model.status != expected_run_status {
                        return Err(StoreError::CorruptRecord(
                            "Run 状态与工作流执行状态不一致".to_owned(),
                        ));
                    }
                    let stopped_from_execution = workflow.execution.clone();
                    let next_workflow = stop_writing_workflow_run(&workflow, run_id)?;

                    let outstanding_requests = model_request::Entity::find()
                        .filter(model_request::Column::RunId.eq(run_model.id))
                        .filter(
                            model_request::Column::Status.is_in(["prepared", "dispatched"]),
                        )
                        .all(transaction)
                        .await?;
                    if outstanding_requests.len() > 1 {
                        return Err(StoreError::CorruptRecord(
                            "一个顶层写作 Run 同时存在多个未终态模型请求".to_owned(),
                        ));
                    }
                    let mut cancelled_request_ids = Vec::with_capacity(outstanding_requests.len());
                    let mut cancelled_attempt_nos = Vec::with_capacity(outstanding_requests.len());
                    for request in outstanding_requests {
                        let request_id: Uuid = request.id.into();
                        let attempt_no = request.attempt_no;
                        let was_dispatched = request.status == "dispatched";
                        let mut active_request = request.into_active_model();
                        active_request.status = Set("cancelled".to_owned());
                        active_request.error_json = Set(was_dispatched.then(|| {
                            serde_json::to_string(&json!({
                                "kind": "user_stopped",
                                "message": "用户在模型响应完成前显式停止了当前 Run",
                            }))
                        }).transpose()?);
                        active_request.updated_at = Set(now);
                        active_request.completed_at = Set(Some(now));
                        active_request.update(transaction).await?;
                        cancelled_request_ids.push(request_id);
                        cancelled_attempt_nos.push(attempt_no);
                        insert_event(
                            transaction,
                            version_id,
                            "model_request.cancelled",
                            request_id,
                            next_workflow.state_revision,
                            json!({
                                "run_id": run_id,
                                "attempt_no": attempt_no.to_string(),
                                "was_dispatched": was_dispatched,
                                "reason": "user_stopped",
                            }),
                            now,
                        )
                        .await?;
                    }

                    let mut checkpoint: Value = serde_json::from_str(&run_model.checkpoint_json)?;
                    let checkpoint = checkpoint.as_object_mut().ok_or_else(|| {
                        StoreError::CorruptRecord("Run 检查点必须是 JSON object".to_owned())
                    })?;
                    checkpoint.insert(
                        "stop".to_owned(),
                        json!({
                            "reason": "user_stopped",
                            "stopped_from_execution": stopped_from_execution,
                            "cancelled_request_ids": &cancelled_request_ids,
                            "cancelled_attempt_nos": cancelled_attempt_nos.iter().map(ToString::to_string).collect::<Vec<_>>(),
                        }),
                    );

                    let run_kind = run_model.kind.clone();
                    let request_count = run_model.request_count;
                    let mut active_run = run_model.into_active_model();
                    active_run.status = Set("stopped".to_owned());
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
                            "status": "stopped",
                            "reason": "user_stopped",
                            "request_count": request_count.to_string(),
                            "cancelled_request_ids": &cancelled_request_ids,
                            "cancelled_attempt_nos": cancelled_attempt_nos.iter().map(ToString::to_string).collect::<Vec<_>>(),
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
                            "chapter_id": next_workflow.active_chapter_id,
                            "pause_requested": next_workflow.pause_requested,
                            "wait_reason": next_workflow.wait_reason,
                            "resume_execution": next_workflow.resume_execution,
                            "reason": "user_stopped",
                        }),
                        now,
                    )
                    .await?;
                    let response = StopWritingRunResult {
                        workflow: next_workflow,
                        cancelled_request_ids,
                    };
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        STOP_WRITING_RUN_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 显式继续一个失败、停止或因服务重启而中断的顶层写作 Run。
    ///
    /// 这不是正文暂停恢复：只有异常终态才能进入这里，并会在同一事务中重新授予
    /// 一轮请求额度、增加 `resume_count`、清零连续协议错误。用户停止在待答问题、
    /// 待审提案或已暂停边界时，Core 会恢复原等待状态而非重新排队，绝不借此入口
    /// 绕过原有的人机决定。
    pub async fn continue_writing_run(
        &self,
        run_id: RunId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<WorkflowSnapshot, StoreError> {
        let target = format!("/api/runs/{run_id}/continue");
        let payload_hash = payload_hash(&RunWorkflowControlPayload {
            expected_state_revision,
        })?;
        let command = Command::new(
            command_id,
            CONTINUE_WRITING_RUN_METHOD,
            &target,
            &payload_hash,
        );
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some()
                        || !matches!(
                            run_model.kind.as_str(),
                            "preparation" | "chapter_outline" | "chapter_body"
                        )
                    {
                        return Err(StoreError::InvalidModelRequestState(
                            "只有顶层写作 Run 可以显式继续".to_owned(),
                        ));
                    }
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let expected_run_status = match workflow.execution {
                        ExecutionStatus::Interrupted => "interrupted",
                        ExecutionStatus::Failed => "failed",
                        ExecutionStatus::Stopped => "stopped",
                        _ => {
                            return Err(StoreError::InvalidModelRequestState(
                                "当前 Run 不在可显式继续的异常终态".to_owned(),
                            ));
                        }
                    };
                    if run_model.status != expected_run_status {
                        return Err(StoreError::CorruptRecord(
                            "Run 状态与工作流异常终态不一致".to_owned(),
                        ));
                    }
                    let next_workflow = continue_writing_run(&workflow, run_id)?;
                    let next_run_status = writing_run_status_for_execution(
                        &next_workflow.execution,
                    )
                    .ok_or_else(|| {
                        StoreError::CorruptRecord(
                            "显式继续没有恢复到可持久化的写作 Run 状态".to_owned(),
                        )
                    })?;
                    let next_allowance_end = run_model
                        .request_count
                        .checked_add(REQUEST_ALLOWANCE_STEP)
                        .ok_or_else(|| {
                            StoreError::InvalidModelRequest(
                                "Run request_allowance_end 已达到 i64 上限".to_owned(),
                            )
                        })?;
                    let next_resume_count =
                        run_model.resume_count.checked_add(1).ok_or_else(|| {
                            StoreError::InvalidModelRequest(
                                "Run resume_count 已达到 i64 上限".to_owned(),
                            )
                        })?;
                    let run_kind = run_model.kind.clone();
                    let request_count = run_model.request_count;
                    let mut active_run = run_model.into_active_model();
                    active_run.status = Set(next_run_status.to_owned());
                    active_run.resume_count = Set(next_resume_count);
                    active_run.request_allowance_end = Set(next_allowance_end);
                    active_run.consecutive_protocol_errors = Set(0);
                    active_run.ended_at = Set(None);
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
                            "status": next_run_status,
                            "resumed": true,
                            "resume_count": next_resume_count.to_string(),
                            "request_count": request_count.to_string(),
                            "request_allowance_end": next_allowance_end.to_string(),
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
                            "chapter_id": next_workflow.active_chapter_id,
                            "pause_requested": next_workflow.pause_requested,
                            "resume_execution": next_workflow.resume_execution,
                        }),
                        now,
                    )
                    .await?;
                    let response_json = serde_json::to_string(&next_workflow)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        CONTINUE_WRITING_RUN_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(next_workflow)
                })
            })
            .await
    }

    /// 向当前正文 Run 追加一条自由意见。
    ///
    /// 意见以独立实体保存，而非改写已经完成的工具结果或已发出的模型请求。一个
    /// Run 同时只有一个未封存批次：后续意见继承它的工具锚点，等目标工具结果被
    /// 下一次请求封存时再一起交给模型。
    pub async fn submit_run_feedback(
        &self,
        run_id: RunId,
        command_id: Uuid,
        expected_state_revision: i64,
        text: String,
        citations: Vec<FeedbackCitation>,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        if text.trim().is_empty() && citations.is_empty() {
            return Err(StoreError::EmptyFeedback);
        }
        if text.chars().count() > nekowriter_core::MAX_REVIEW_ISSUES_LEN {
            return Err(StoreError::InvalidChapterBodyEdit(format!(
                "自由意见超过 {} 个字符",
                nekowriter_core::MAX_REVIEW_ISSUES_LEN
            )));
        }
        let target = format!("/api/runs/{run_id}/feedback");
        let payload_hash = payload_hash(&SubmitRunFeedbackPayload {
            expected_state_revision,
            text: &text,
            citations: &citations,
        })?;
        let command = Command::new(
            command_id,
            SUBMIT_RUN_FEEDBACK_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                let text = text.clone();
                let citations = citations.clone();
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some() || run_model.kind != "chapter_body" {
                        return Err(StoreError::InvalidChapterState(
                            "自由意见只能投递给当前章节正文 Run".to_owned(),
                        ));
                    }
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let chapter_id = workflow.active_chapter_id.ok_or_else(|| {
                        StoreError::InvalidChapterState("当前正文 Run 缺少活动章节".to_owned())
                    })?;
                    if workflow.active_run_id != Some(run_id)
                        || workflow.stage != WorkflowStage::WritingDrafting
                    {
                        return Err(StoreError::InvalidChapterState(
                            "当前工作流不接受正文自由意见".to_owned(),
                        ));
                    }
                    validate_chapter_body_feedback_citations(
                        transaction,
                        &citations,
                        version_id,
                        chapter_id,
                        run_id,
                        CandidateCitationScope::HeldCurrentRun,
                    )
                    .await?;
                    let body_run = chapter_body_run(transaction, run_id, chapter_id).await?;
                    if !chapter_body_run_status_matches_execution(
                        &body_run.status,
                        &workflow.execution,
                    ) {
                        return Err(StoreError::CorruptRecord(
                            "正文 Run 状态与工作流执行状态不一致".to_owned(),
                        ));
                    }
                    let held_boundary = if workflow.execution == ExecutionStatus::Paused {
                        Some(
                            held_chapter_body_pause_boundary(transaction, &body_run, chapter_id)
                                .await?,
                        )
                    } else {
                        None
                    };
                    let resumes_held_boundary = held_boundary.is_some();
                    let cancels_pending_pause = workflow.pause_requested && !resumes_held_boundary;
                    let feedback_workflow = if resumes_held_boundary {
                        None
                    } else if cancels_pending_pause {
                        Some(cancel_chapter_body_pause_for_feedback(
                            &workflow, chapter_id, run_id,
                        )?)
                    } else {
                        Some(record_chapter_body_feedback(&workflow, chapter_id, run_id)?)
                    };

                    let QueuedChapterBodyFeedback {
                        id: feedback_id,
                        sequence,
                        status,
                        bound_tool_call_id,
                    } = queue_chapter_body_feedback(transaction, &body_run, text, &citations, now)
                        .await?;
                    match held_boundary {
                        Some(HeldChapterBodyPauseBoundary::Tool) => {
                            let resolved = resolve_held_chapter_body_tool(
                                transaction,
                                &workflow,
                                body_run.clone(),
                                chapter_id,
                                now,
                            )
                            .await?;
                            if !resolved.bound_feedback_ids.contains(&feedback_id) {
                                insert_event(
                                    transaction,
                                    version_id,
                                    "feedback.changed",
                                    feedback_id,
                                    resolved.next_workflow.state_revision,
                                    json!({
                                        "chapter_id": chapter_id,
                                        "run_id": run_id,
                                        "status": feedback_status_name(status),
                                        "sequence": sequence.to_string(),
                                        "bound_tool_call_id": bound_tool_call_id,
                                    }),
                                    now,
                                )
                                .await?;
                            }
                            insert_resolved_held_chapter_body_tool_events(
                                transaction,
                                version_id,
                                chapter_id,
                                run_id,
                                &resolved,
                                "feedback_resumes_held_tool",
                                now,
                            )
                            .await?;
                        }
                        Some(HeldChapterBodyPauseBoundary::Submission) => {
                            let resolved = resolve_held_chapter_body_submission(
                                transaction,
                                &workflow,
                                body_run.clone(),
                                chapter_id,
                                now,
                            )
                            .await?;
                            insert_event(
                                transaction,
                                version_id,
                                "feedback.changed",
                                feedback_id,
                                resolved.next_workflow.state_revision,
                                json!({
                                    "chapter_id": chapter_id,
                                    "run_id": run_id,
                                    "status": feedback_status_name(status),
                                    "sequence": sequence.to_string(),
                                    "bound_tool_call_id": bound_tool_call_id,
                                }),
                                now,
                            )
                            .await?;
                            insert_resolved_held_chapter_body_submission_events(
                                transaction,
                                version_id,
                                chapter_id,
                                run_id,
                                &resolved,
                                "feedback_resumes_held_submission",
                                now,
                            )
                            .await?;
                        }
                        None => {
                            let next_workflow = feedback_workflow.as_ref().ok_or_else(|| {
                                StoreError::CorruptRecord("自由意见缺少下一工作流状态".to_owned())
                            })?;
                            let bound_feedback_ids = if cancels_pending_pause {
                                match latest_unsealed_chapter_body_tool_result(
                                    transaction,
                                    &body_run.id,
                                )
                                .await?
                                {
                                    Some((tool_call_id, _)) => {
                                        bind_pending_feedback_to_existing_tool_for_pause(
                                            transaction,
                                            run_id,
                                            tool_call_id,
                                            now,
                                        )
                                        .await?
                                    }
                                    None => Vec::new(),
                                }
                            } else {
                                Vec::new()
                            };
                            persist_workflow(transaction, next_workflow).await?;
                            if !bound_feedback_ids.contains(&feedback_id) {
                                let final_status = if bound_tool_call_id.is_some() {
                                    FeedbackStatus::Bound
                                } else {
                                    status
                                };
                                insert_event(
                                    transaction,
                                    version_id,
                                    "feedback.changed",
                                    feedback_id,
                                    next_workflow.state_revision,
                                    json!({
                                        "chapter_id": chapter_id,
                                        "run_id": run_id,
                                        "status": feedback_status_name(final_status),
                                        "sequence": sequence.to_string(),
                                        "bound_tool_call_id": bound_tool_call_id,
                                    }),
                                    now,
                                )
                                .await?;
                            }
                            for bound_feedback_id in bound_feedback_ids {
                                insert_event(
                                    transaction,
                                    version_id,
                                    "feedback.changed",
                                    bound_feedback_id,
                                    next_workflow.state_revision,
                                    json!({
                                        "chapter_id": chapter_id,
                                        "run_id": run_id,
                                        "status": "bound",
                                    }),
                                    now,
                                )
                                .await?;
                            }
                            insert_event(
                                transaction,
                                version_id,
                                "workflow.changed",
                                version_id,
                                next_workflow.state_revision,
                                json!({
                                    "stage": next_workflow.stage,
                                    "execution": next_workflow.execution,
                                    "chapter_id": chapter_id,
                                    "run_id": run_id,
                                    "pause_requested": next_workflow.pause_requested,
                                }),
                                now,
                            )
                            .await?;
                        }
                    }
                    let response = chapter_body_snapshot(transaction, chapter_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SUBMIT_RUN_FEEDBACK_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 读取当前正文 Run 已经绑定到工具结果的自由意见。
    ///
    /// Store 返回原始工具调用 ID 而非数据库 UUID，使应用层只需在重放 `tool` 消息
    /// 时把这些条目附入对应 JSON 结果。实际“included”状态仍在请求封存事务中改变，
    /// 本读取不会提前声称意见已发出。
    pub async fn list_materialized_chapter_body_feedback(
        &self,
        run_id: RunId,
    ) -> Result<Vec<MaterializedChapterBodyFeedback>, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some() || run_model.kind != "chapter_body" {
                        return Err(StoreError::InvalidChapterState(
                            "只有章节正文 Run 可以读取自由意见".to_owned(),
                        ));
                    }
                    let tools = tool_call::Entity::find()
                        .filter(tool_call::Entity::COLUMN.run_id.eq(run_model.id))
                        .all(transaction)
                        .await?
                        .into_iter()
                        .map(|tool| (tool.id.clone().to_string(), tool))
                        .collect::<HashMap<_, _>>();
                    let models = feedback::Entity::find()
                        .filter(feedback::Entity::COLUMN.run_id.eq(run_model.id))
                        .filter(feedback::Entity::COLUMN.status.is_in(["bound", "included"]))
                        .order_by_asc(feedback::Entity::COLUMN.sequence)
                        .all(transaction)
                        .await?;
                    models
                        .iter()
                        .map(|model| {
                            let status = feedback_status_from_db(&model.status)?;
                            let bound_tool_call_id =
                                model.bound_tool_call_id.as_ref().ok_or_else(|| {
                                    StoreError::CorruptRecord(
                                        "已绑定自由意见缺少 bound_tool_call_id".to_owned(),
                                    )
                                })?;
                            if (status == FeedbackStatus::Bound
                                && model.included_request_id.is_some())
                                || (status == FeedbackStatus::Included
                                    && model.included_request_id.is_none())
                            {
                                return Err(StoreError::CorruptRecord(
                                    "自由意见状态与 included_request_id 不一致".to_owned(),
                                ));
                            }
                            let tool = tools.get(bound_tool_call_id).ok_or_else(|| {
                                StoreError::CorruptRecord(
                                    "自由意见绑定了不属于当前 Run 的工具调用".to_owned(),
                                )
                            })?;
                            if tool.result_json.is_none() {
                                return Err(StoreError::CorruptRecord(
                                    "自由意见绑定的工具调用尚无可物化结果".to_owned(),
                                ));
                            }
                            let citations = feedback_citations_from_json(&model.citations_json)?;
                            Ok(MaterializedChapterBodyFeedback {
                                provider_tool_call_id: tool.provider_call_id.clone(),
                                id: model.id.into(),
                                sequence: model.sequence,
                                text: model.text.clone(),
                                citations,
                                status,
                            })
                        })
                        .collect()
                })
            })
            .await
            .map_err(transaction_error)
    }

    /// 返回本轮即将首次纳入下一请求快照的写作工具结果调用 ID。
    ///
    /// 调度器将该列表写入不可变请求诊断快照；Store 在 capture 事务中再次精确校验
    /// 并标记结果已经 included，避免暂停特例把新意见回填到早已发出的历史结果。
    pub async fn list_unsealed_agent_tool_result_provider_ids(
        &self,
        run_id: RunId,
    ) -> Result<Vec<String>, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some()
                        || !matches!(
                            run_model.kind.as_str(),
                            "preparation" | "chapter_outline" | "chapter_body"
                        )
                    {
                        return Err(StoreError::InvalidChapterState(
                            "只有顶层写作 Run 可以读取未封存工具结果".to_owned(),
                        ));
                    }
                    tool_call::Entity::find()
                        .filter(tool_call::Entity::COLUMN.run_id.eq(run_model.id))
                        .filter(
                            tool_call::Entity::COLUMN
                                .status
                                .is_in(["resolved", "error"]),
                        )
                        .filter(tool_call::Entity::COLUMN.result_json.is_not_null())
                        .filter(tool_call::Entity::COLUMN.included_request_id.is_null())
                        .order_by_asc(tool_call::Entity::COLUMN.sequence)
                        .all(transaction)
                        .await?
                        .into_iter()
                        .map(|tool| {
                            if tool.provider_call_id.trim().is_empty() {
                                Err(StoreError::CorruptRecord(
                                    "未封存写作工具结果缺少 provider_call_id".to_owned(),
                                ))
                            } else {
                                Ok(tool.provider_call_id)
                            }
                        })
                        .collect()
                })
            })
            .await
            .map_err(transaction_error)
    }

    /// 验证并提交一组 `ask_user` 答案，形成同一工具调用的可回送结果。
    pub async fn answer_user_question(
        &self,
        question_id: QuestionId,
        command_id: Uuid,
        expected_state_revision: i64,
        answers: UserQuestionAnswers,
    ) -> Result<QuestionAnswerResult, StoreError> {
        let target = format!("/api/questions/{question_id}/answer");
        let payload_hash = payload_hash(&AnswerQuestionPayload {
            expected_state_revision,
            answers: &answers,
        })?;
        let command = Command::new(command_id, ANSWER_QUESTION_METHOD, &target, &payload_hash);
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                let answers = answers.clone();
                Box::pin(async move {
                    let question_model = question::Entity::find_by_id(db_id(question_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::QuestionNotFound)?;
                    let tool_model = tool_call::Entity::find_by_id(question_model.tool_call_id)
                        .one(transaction)
                        .await?
                        .ok_or_else(|| {
                            StoreError::CorruptRecord("问题引用的工具调用不存在".to_owned())
                        })?;
                    let run_model = run::Entity::find_by_id(tool_model.run_id)
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some()
                        || !matches!(
                            run_model.kind.as_str(),
                            "preparation" | "chapter_outline" | "chapter_body"
                        )
                    {
                        return Err(StoreError::QuestionNotFound);
                    }
                    let run_id: RunId = run_model.id.into();
                    let version_id = run_model.version_id.into();
                    let run_kind = run_model.kind.clone();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.active_run_id != Some(run_id)
                        || workflow.execution != ExecutionStatus::WaitingAnswer
                        || !question_stage_matches_run_kind(&workflow.stage, &run_kind)
                    {
                        return Err(StoreError::InvalidModelRequestState(
                            "当前工作流不接受该问题的答案".to_owned(),
                        ));
                    }
                    if run_model.status != "waiting" {
                        return Err(StoreError::CorruptRecord(
                            "待答问题的 Run 未处于 waiting 状态".to_owned(),
                        ));
                    }
                    if question_model.status == "answered" {
                        return Err(StoreError::QuestionAlreadyAnswered);
                    }
                    if question_model.status != "open" || question_model.answer_json.is_some() {
                        return Err(StoreError::CorruptRecord(
                            "待答问题的状态与答案字段不一致".to_owned(),
                        ));
                    }
                    if tool_model.status != "awaiting_answer" || tool_model.result_json.is_some() {
                        return Err(StoreError::CorruptRecord(
                            "待答问题的工具调用状态不一致".to_owned(),
                        ));
                    }
                    let schema: UserQuestionSchema =
                        serde_json::from_str(&question_model.schema_json)?;
                    let normalized_answers = schema
                        .validate_answers(&answers.answers)
                        .map_err(|error| StoreError::InvalidQuestion(error.to_string()))?;
                    let normalized = UserQuestionAnswers {
                        answers: normalized_answers.clone(),
                    };
                    let tool_result =
                        question_tool_result(&tool_model.provider_call_id, &normalized_answers);
                    let next_workflow = continue_after_user_answer(&workflow, run_id)?;
                    let mut active_question = question_model.into_active_model();
                    active_question.answer_json = Set(Some(serde_json::to_string(&normalized)?));
                    active_question.status = Set("answered".to_owned());
                    active_question.updated_at = Set(now);
                    active_question.update(transaction).await?;
                    let tool_call_id: Uuid = tool_model.id.into();
                    let tool_sequence = tool_model.sequence;
                    let provider_call_id = tool_model.provider_call_id.clone();
                    let tool_request_id: Uuid = tool_model.request_id.into();
                    let mut active_tool = tool_model.into_active_model();
                    active_tool.status = Set("resolved".to_owned());
                    active_tool.result_json = Set(Some(serde_json::to_string(&tool_result)?));
                    active_tool.included_request_id = Set(None);
                    active_tool.resolved_at = Set(Some(now));
                    active_tool.update(transaction).await?;
                    append_agent_messages_for_run(
                        transaction,
                        &run_model,
                        vec![NewAgentMessage {
                            role: AgentMessageRole::Tool,
                            content: json!({
                                "kind": "tool_result",
                                "content": serde_json::to_string(&tool_result)?,
                            }),
                            request_id: Some(tool_request_id),
                            tool_call_id: Some(provider_call_id.clone()),
                        }],
                        now,
                    )
                    .await?;
                    let bound_feedback_ids = if run_kind == "chapter_body" {
                        bind_pending_feedback_to_tool(
                            transaction,
                            run_id,
                            tool_call_id,
                            tool_sequence,
                            now,
                        )
                        .await?
                    } else {
                        Vec::new()
                    };
                    let request_count = run_model.request_count;
                    let mut active_run = run_model.into_active_model();
                    active_run.status = Set("queued".to_owned());
                    active_run.ended_at = Set(None);
                    active_run.updated_at = Set(now);
                    active_run.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "question.answered",
                        question_id,
                        next_workflow.state_revision,
                        json!({
                            "run_id": run_id,
                            "tool_call_id": tool_call_id,
                            "answer_count": normalized_answers.len(),
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "tool.resolved",
                        tool_call_id,
                        next_workflow.state_revision,
                        json!({
                            "run_id": run_id,
                            "chapter_id": next_workflow.active_chapter_id,
                            "name": "ask_user",
                            "provider_call_id": provider_call_id,
                            "status": "answered",
                        }),
                        now,
                    )
                    .await?;
                    for feedback_id in bound_feedback_ids {
                        insert_event(
                            transaction,
                            version_id,
                            "feedback.changed",
                            feedback_id,
                            next_workflow.state_revision,
                            json!({
                                "chapter_id": next_workflow.active_chapter_id,
                                "run_id": run_id,
                                "status": "bound",
                                "bound_tool_call_id": tool_call_id,
                            }),
                            now,
                        )
                        .await?;
                    }
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({
                            "kind": run_kind.clone(),
                            "status": "queued",
                            "request_count": request_count.to_string(),
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
                            "chapter_id": next_workflow.active_chapter_id,
                            "pause_requested": next_workflow.pause_requested,
                        }),
                        now,
                    )
                    .await?;
                    let response = QuestionAnswerResult {
                        question_id,
                        run_id,
                        workflow: next_workflow,
                    };
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        ANSWER_QUESTION_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }
}
