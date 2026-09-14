//! 被拒模型回合（协议纠正）的唯一归属地。
//!
//! 一个阶段回合只有两种结局：被接受，或作为「被拒的一轮」回到模型。后者要做的事在
//! 写作与委托审核两条 lane 上完全一样——保留模型给出的原文、写一条配对的错误工具
//! 结果与一条纠正提示、把连续协议错误计数加一、未达上限时重新排队——差别只有「这次
//! 纠正记在哪个 Run 头上」和「达到上限时收束谁」。所以这里只有一个事务入口、一套
//! 结果类型和同一份计数策略：lane 的差别集中在 [`ProtocolCorrectionTarget`] 与两个
//! `settle_*` 上。
//!
//! 不属于这里的两件事：什么算一轮合法提交（`nekowriter-agent` 的阶段守卫），以及
//! 纠正提示怎么写（调度器的 `protocol_correction`）。它们回答的是「这一轮为什么被
//! 拒」，而这里回答的是「被拒之后写什么、记在谁头上」。

use nekowriter_core::{
    MAX_CONSECUTIVE_PROTOCOL_ERRORS, ReviewId, RunId, VersionId, WorkflowSnapshot,
    continue_after_agent_tool, stop_writing_after_protocol_error, utc_now_millis,
};
use sea_orm::{
    ActiveModelTrait, DatabaseTransaction, EntityTrait, IntoActiveModel, QueryFilter, Set,
    TransactionTrait,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::agent_messages::{NewAgentMessage, append_agent_messages_for_run};
use crate::entity::{model_request, run};
use crate::model_requests::{
    ModelRequestStatus, model_request_status_from_db, next_protocol_error_count,
    protocol_correction_matches, protocol_correction_messages, validate_model_protocol_correction,
};
use crate::preparation::{
    delegated_review_attempt, fail_delegated_review_child_run, insert_event, persist_workflow,
    reopen_delegated_review, workflow_for_version,
};
use crate::{Store, StoreError, db_id, transaction_error};

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

/// 这次纠正记在哪个 Run 头上：两条 lane 的全部差别都从这里分叉。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolCorrectionTarget {
    /// 顶层写作 Run：达上限时工作流收束为 `stopped`，等待用户显式继续。
    WritingRun,
    /// 委托审核的子 Run：达上限时子 Run 失败、Review 重新开放。
    DelegatedReview { review_id: ReviewId },
}

/// 协议纠正事务的结果。
///
/// 同一 request 的重复回调返回 `newly_applied = false`，绝不会再次追加纠正消息、
/// 增加连续错误次数或重新入队。`workflow` 是纠正之后的工作流快照；委托审核不推进
/// 工作流，因此它返回的是当前快照。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolCorrectionOutcome {
    pub workflow: WorkflowSnapshot,
    pub newly_applied: bool,
    pub requeue: bool,
    pub consecutive_protocol_errors: i64,
}

impl Store {
    /// 保存模型已经完整返回、但不满足本阶段协议的响应，并在工程上限内要求模型纠正。
    ///
    /// 这与网络或提供商失败不同：完整原始响应、任何 tool-call 的配对错误结果以及
    /// 纠正提示都进入同一 Run 的消息账本，因此下一次请求可以在不破坏调用协议的前提
    /// 下继续。连续计数与达上限后的收束由 `target` 决定，两条 lane 共用同一份策略。
    ///
    /// 请求 id 由 `correction.assistant_message.request_id` 给出：纠正必须写在模型
    /// 那次响应真正归属的请求上，多一个入参只会多一种写错的可能。
    ///
    /// # Errors
    ///
    /// 请求不存在、不在 `dispatched`、已有不同终态、目标 Run 与 lane 不匹配、消息形状
    /// 非法或数据库写入失败时返回错误。
    pub async fn record_model_protocol_correction(
        &self,
        target: ProtocolCorrectionTarget,
        correction: ModelProtocolCorrection,
    ) -> Result<ProtocolCorrectionOutcome, StoreError> {
        let request_id = correction.assistant_message.request_id.ok_or_else(|| {
            StoreError::InvalidAgentMessage(
                "协议纠正 assistant 消息必须关联当前 model_request".to_owned(),
            )
        })?;
        validate_model_protocol_correction(request_id, &correction)?;
        let messages = protocol_correction_messages(request_id, &correction)?;
        let outcome = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let request = model_request::Entity::find_by_id(db_id(request_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ModelRequestNotFound)?;
                    let run_model = load_target_run(transaction, &request, target).await?;
                    let run_id: RunId = run_model.id.into();
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;

                    let current_status = model_request_status_from_db(&request.status)?;
                    if current_status.is_terminal() {
                        if protocol_correction_matches(&request, &correction)? {
                            return Ok(ProtocolCorrectionOutcome {
                                workflow,
                                newly_applied: false,
                                requeue: false,
                                consecutive_protocol_errors: run_model.consecutive_protocol_errors,
                            });
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

                    let attempt_no = request.attempt_no;
                    let next_error_count = apply_rejected_round(
                        transaction,
                        request,
                        &run_model,
                        &correction,
                        messages,
                    )
                    .await?;
                    let round = RejectedRound {
                        request_id,
                        run_id,
                        version_id,
                        attempt_no,
                        workflow,
                        protocol_error: &correction.error,
                        next_error_count,
                        stop: next_error_count >= MAX_CONSECUTIVE_PROTOCOL_ERRORS,
                        now: utc_now_millis(),
                    };
                    let workflow = match target {
                        ProtocolCorrectionTarget::WritingRun => {
                            settle_writing_run(transaction, &round, run_model).await?
                        }
                        ProtocolCorrectionTarget::DelegatedReview { review_id } => {
                            settle_delegated_review(transaction, &round, review_id, run_model)
                                .await?;
                            round.workflow.clone()
                        }
                    };
                    Ok(ProtocolCorrectionOutcome {
                        workflow,
                        newly_applied: true,
                        requeue: !round.stop,
                        consecutive_protocol_errors: next_error_count,
                    })
                })
            })
            .await
            .map_err(transaction_error)?;
        if outcome.newly_applied {
            self.notify_event_listeners();
        }
        Ok(outcome)
    }
}

/// 一轮被拒回合的事实：两条 lane 的收束函数都要用它，因此只在一处组装。
struct RejectedRound<'a> {
    request_id: Uuid,
    run_id: RunId,
    version_id: VersionId,
    /// 本阶段的请求序号：子 Run 用它作事件版本（子 Run 不推进工作流）。
    attempt_no: i64,
    /// 纠正之前的工作流快照。
    workflow: WorkflowSnapshot,
    /// 写进请求错误字段的那份原因，收束时也用它解释重新开放。
    protocol_error: &'a Value,
    next_error_count: i64,
    stop: bool,
    now: i64,
}

/// 载入这次纠正要记在头上的 Run，并做 lane 自己的匹配校验。
///
/// 写作 lane 只接受顶层写作 Run；委托审核 lane 只接受当前 attempt 占有的审核子 Run。
/// 在错误的 actor 上写纠正会让计数与收束各自落到别处，所以两者都在这里拒绝。
async fn load_target_run(
    transaction: &DatabaseTransaction,
    request: &model_request::Model,
    target: ProtocolCorrectionTarget,
) -> Result<run::Model, StoreError> {
    match target {
        ProtocolCorrectionTarget::WritingRun => {
            let run_model = run::Entity::find_by_id(request.run_id)
                .one(transaction)
                .await?
                .ok_or(StoreError::RunNotFound)?;
            if run_model.parent_run_id.is_some() {
                return Err(StoreError::InvalidModelRequestState(
                    "协议纠正只能用于顶层写作 Run".to_owned(),
                ));
            }
            Ok(run_model)
        }
        ProtocolCorrectionTarget::DelegatedReview { review_id } => {
            let attempt = delegated_review_attempt(transaction, review_id).await?;
            let request_run_id: RunId = request.run_id.into();
            if attempt.run_id != request_run_id {
                return Err(StoreError::InvalidModelRequestState(
                    "审核 Agent 回调不属于当前委托尝试".to_owned(),
                ));
            }
            let child_run = run::Entity::find_by_id(request.run_id)
                .filter(run::Entity::COLUMN.kind.eq("review"))
                .filter(run::Entity::COLUMN.version_id.eq(db_id(attempt.version_id)))
                .one(transaction)
                .await?
                .ok_or(StoreError::RunNotFound)?;
            if child_run.status != "running" {
                return Err(StoreError::InvalidModelRequestState(
                    "审核 Agent 子 Run 不在运行状态".to_owned(),
                ));
            }
            Ok(child_run)
        }
    }
}

/// 被拒的一轮在校验通过后必须写下的东西：配对（被拒调用 + 错误工具结果）与纠正
/// 提示进账本，这条 `dispatched` 请求以 `succeeded` 收束并保留原始响应与错误，因此
/// Dashboard 的用量与失败原因都不会丢。返回新的连续协议错误计数。
async fn apply_rejected_round(
    transaction: &DatabaseTransaction,
    request: model_request::Model,
    run_model: &run::Model,
    correction: &ModelProtocolCorrection,
    messages: Vec<NewAgentMessage>,
) -> Result<i64, StoreError> {
    let next_error_count = next_protocol_error_count(run_model.consecutive_protocol_errors)?;
    let now = utc_now_millis();
    append_agent_messages_for_run(transaction, run_model, messages, now).await?;
    let mut active_request = request.into_active_model();
    active_request.status = Set("succeeded".to_owned());
    active_request.provider_request_id = Set(correction.provider_request_id.clone());
    active_request.usage_json = Set(Some(serde_json::to_string(&correction.usage)?));
    active_request.error_json = Set(Some(serde_json::to_string(&correction.error)?));
    active_request.updated_at = Set(now);
    active_request.completed_at = Set(Some(now));
    active_request.update(transaction).await?;
    Ok(next_error_count)
}

/// 写作 lane 的收束：工作流按上限继续或停止，Run 状态与连续错误计数一起落库，并
/// 广播与派发时同形的两条事件（`run.changed` 带 `wait_reason`，`workflow.changed`
/// 带 `protocol_correction`）。返回纠正之后的工作流快照。
async fn settle_writing_run(
    transaction: &DatabaseTransaction,
    round: &RejectedRound<'_>,
    run_model: run::Model,
) -> Result<WorkflowSnapshot, StoreError> {
    let next_workflow = if round.stop {
        stop_writing_after_protocol_error(&round.workflow, round.run_id)?
    } else {
        continue_after_agent_tool(&round.workflow, round.run_id)?
    };
    let run_kind = run_model.kind.clone();
    let request_count = run_model.request_count;
    let run_status = if round.stop { "stopped" } else { "queued" };
    let mut active_run = run_model.into_active_model();
    active_run.status = Set(run_status.to_owned());
    active_run.consecutive_protocol_errors = Set(round.next_error_count);
    active_run.ended_at = Set(if round.stop { Some(round.now) } else { None });
    active_run.updated_at = Set(round.now);
    active_run.update(transaction).await?;
    persist_workflow(transaction, &next_workflow).await?;
    insert_event(
        transaction,
        round.version_id,
        "run.changed",
        round.run_id,
        next_workflow.state_revision,
        json!({
            "kind": run_kind,
            "status": run_status,
            "request_id": round.request_id,
            "request_count": request_count.to_string(),
            "consecutive_protocol_errors": round.next_error_count.to_string(),
            "wait_reason": if round.stop { Some("protocol_error_limit") } else { None::<&str> },
        }),
        round.now,
    )
    .await?;
    insert_event(
        transaction,
        round.version_id,
        "workflow.changed",
        round.version_id,
        next_workflow.state_revision,
        json!({
            "stage": next_workflow.stage,
            "execution": next_workflow.execution,
            "run_id": round.run_id,
            "request_id": round.request_id,
            "protocol_correction": true,
            "consecutive_protocol_errors": round.next_error_count.to_string(),
        }),
        round.now,
    )
    .await?;
    Ok(next_workflow)
}

/// 委托审核 lane 的收束：先把「这一轮被拒、模型将重提」写进事件流，再按上限决定
/// 重排还是收束。`consecutive_protocol_errors` 只通过事件到达客户端（快照与
/// Dashboard 的 DTO 都不含它），所以这两条事件是这条回路唯一的对外信号；达到上限
/// 时按取消/失败的同一路径收束——子 Run `failed`、Review 重新 `open`。
///
/// 子 Run 的行只写一次，且无论重排还是收束都带上这一轮的计数：行与它上面刚写的
/// 事件来自同一个 `round`，两者不可能出现「事件说 3、行里停在 2」。
async fn settle_delegated_review(
    transaction: &DatabaseTransaction,
    round: &RejectedRound<'_>,
    review_id: ReviewId,
    child_run: run::Model,
) -> Result<(), StoreError> {
    insert_event(
        transaction,
        round.version_id,
        "run.changed",
        round.run_id,
        round.attempt_no,
        review_correction_run_event(
            &child_run,
            review_id,
            round.request_id,
            round.next_error_count,
            round.stop,
        ),
        round.now,
    )
    .await?;
    insert_event(
        transaction,
        round.version_id,
        "review.changed",
        review_id,
        round.workflow.state_revision,
        review_correction_review_event(
            review_id,
            round.run_id,
            round.request_id,
            round.next_error_count,
        ),
        round.now,
    )
    .await?;
    // 计数与失败状态必须落在同一次写入里：上面两条事件已经宣布了这一轮的计数
    // 与上限原因，只在部分分支写计数会让行里的数字与它刚发出的事件互相矛盾。
    let mut active_child_run = child_run.into_active_model();
    active_child_run.consecutive_protocol_errors = Set(round.next_error_count);
    active_child_run.updated_at = Set(round.now);
    let active_child_run = if round.stop {
        fail_delegated_review_child_run(active_child_run, round.now)
    } else {
        active_child_run
    };
    active_child_run.update(transaction).await?;
    if round.stop {
        reopen_delegated_review(
            transaction,
            review_id,
            round.run_id,
            round.version_id,
            round.protocol_error,
            round.now,
        )
        .await?;
    }
    Ok(())
}

/// 被拒的一轮在子 Run 的 `run.changed` 里留下的信号：字段与写作 lane 的纠正事件
/// 一致（`status`/`request_id`/`request_count`/连续错误计数/`wait_reason`），订阅方
/// 不必按 lane 分叉。
fn review_correction_run_event(
    child_run: &run::Model,
    review_id: ReviewId,
    request_id: Uuid,
    next_error_count: i64,
    stop: bool,
) -> Value {
    json!({
        "kind": "review",
        "status": child_run.status,
        "review_id": review_id,
        "request_id": request_id,
        "request_count": child_run.request_count.to_string(),
        "consecutive_protocol_errors": next_error_count.to_string(),
        "protocol_correction": true,
        "wait_reason": if stop { Some("protocol_error_limit") } else { None::<&str> },
    })
}

/// 审核侧对应的事件：Review 本身仍是 `delegated`，变化的是「这一轮被拒、模型将
/// 重提」这个事实。
fn review_correction_review_event(
    review_id: ReviewId,
    run_id: RunId,
    request_id: Uuid,
    next_error_count: i64,
) -> Value {
    json!({
        "status": "delegated",
        "attempt_id": run_id,
        "review_id": review_id,
        "request_id": request_id,
        "protocol_correction": true,
        "consecutive_protocol_errors": next_error_count.to_string(),
    })
}
