//! 自由工具与提问工具：调用上下文校验、落库与结果回送。
//!
//! 从 `preparation/mod.rs` 的这一层原样搬出（`docs/08` 的 P3.5），只移动：函数体逐字不变，
//! 只改了可见性前缀（被块外引用的项改 `pub(crate)`）。`preparation/mod.rs` 用
//! `pub use tools::free::{…}`（两个公开 DTO）与 `pub(crate) use tools::free::next_agent_tool_sequence`
//! 把名字接回，所以兄弟模块与 app 侧的既有引用一行都不用改。

use nekowriter_core::{
    ExecutionStatus, MAX_CONSECUTIVE_PROTOCOL_ERRORS, PendingUserQuestion, RunId, WorkflowSnapshot,
    continue_after_agent_tool, stop_writing_after_protocol_error, wait_for_user_answer,
};
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, Set, TransactionTrait,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::agent_messages::append_agent_messages_for_run;
use crate::entity::{model_request, question, run, tool_call};
use crate::event_log::insert_event;
use crate::model_requests::next_protocol_error_count;
use crate::{
    AgentMessageRole, NewAgentMessage, Store, StoreError, VersionId, db_id, transaction_error,
    utc_now_millis,
};

use super::super::citations::bind_pending_feedback_to_tool;
use super::super::{persist_workflow, workflow_for_version};
use super::model::{
    AgentQuestionToolCall, AppliedAgentQuestionToolCall, parse_agent_question_schema,
    pending_user_question_from_model, question_stage_matches_run_kind, question_tool_error_result,
    validate_agent_question_tool_call,
};

/// 一次由调用方算好结果的自由工具调用。
#[derive(Clone, Debug)]
pub struct FreeAgentToolCall {
    pub provider_call_id: String,
    pub name: String,
    pub arguments: Value,
    /// 已算好的工具结果，会被原样回送给模型。
    pub result: Value,
    /// `true` 表示这是一个稳定错误结果，落库状态为 `error`。
    pub failed: bool,
    pub error_code: Option<String>,
}

/// 自由工具调用经事务落库后的结果。
///
/// `newly_resolved` 用来抑制供应商重放；`requeue` 则表达这次新调用是否仍可继续
/// 同一 Run。两者必须区分，否则第三次连续工具错误仍会被调度器错误地再次投递。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedFreeAgentToolCall {
    pub result: Value,
    pub newly_resolved: bool,
    pub requeue: bool,
}

/// 一次顶层写作 Run 中、由调用方提供结果的自由工具调用的落库上下文。
///
/// 校验只存在这一处：只有 `dispatched` 请求、顶层 `preparation`/`chapter_outline`/
/// `chapter_body` Run、工作流正在等待该阶段模型响应且 Run 为 `running` 时才允许落库。
struct FreeAgentToolContext {
    request: model_request::Model,
    run_model: run::Model,
    run_id: RunId,
    version_id: VersionId,
    run_kind: String,
    workflow: WorkflowSnapshot,
    sequence: i64,
    tool_call_id: Uuid,
    provider_call_id: String,
    args_json: String,
    assistant_message: NewAgentMessage,
}

async fn prepare_free_agent_tool_call(
    transaction: &DatabaseTransaction,
    request_id: Uuid,
    provider_call_id: &str,
    name: &str,
    arguments: &Value,
) -> Result<FreeAgentToolContext, StoreError> {
    let request = model_request::Entity::find_by_id(db_id(request_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::ModelRequestNotFound)?;
    if request.status != "dispatched" {
        return Err(StoreError::InvalidModelRequestState(format!(
            "只有 dispatched 请求可以执行 {name}，当前为 {}",
            request.status
        )));
    }
    let run_model = run::Entity::find_by_id(request.run_id)
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
            "该模型请求不属于顶层写作 Run".to_owned(),
        ));
    }
    let run_id: RunId = run_model.id.into();
    let version_id = run_model.version_id.into();
    let run_kind = run_model.kind.clone();
    let workflow = workflow_for_version(transaction, version_id).await?;
    if workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::RequestInFlight
        || !question_stage_matches_run_kind(&workflow.stage, &run_kind)
    {
        return Err(StoreError::InvalidModelRequestState(format!(
            "当前工作流不接受 {name} 工具调用"
        )));
    }
    if run_model.status != "running" {
        return Err(StoreError::CorruptRecord(format!(
            "{name} 在途请求的 Run 未处于 running 状态"
        )));
    }
    let sequence = next_agent_tool_sequence(transaction, &run_model.id).await?;
    let tool_call_id = Uuid::new_v4();
    let assistant_message = NewAgentMessage {
        role: AgentMessageRole::Assistant,
        content: json!({
            "kind": "assistant_tool_calls",
            "content": Value::Null,
            "toolCalls": [{
                "id": provider_call_id,
                "name": name,
                "arguments": arguments,
            }],
        }),
        request_id: Some(request_id),
        tool_call_id: None,
    };
    Ok(FreeAgentToolContext {
        request,
        run_model,
        run_id,
        version_id,
        run_kind,
        workflow,
        sequence,
        tool_call_id,
        provider_call_id: provider_call_id.to_owned(),
        args_json: serde_json::to_string(arguments)?,
        assistant_message,
    })
}

/// 一次自由工具调用的可回送结果。
struct FreeAgentToolResult<'a> {
    name: &'a str,
    result: &'a Value,
    /// `resolved` 或 `error`。
    status: &'a str,
    /// 失败时的稳定错误码，正常结果时为 `None`。
    error_code: Option<&'a str>,
}

/// 提交一次自由工具调用：写入 assistant/tool 审计与工具调用记录，更新本次模型请求
/// 与 Run，推进工作流到下一轮并发出事件。
///
/// 返回值表示调度器是否可以继续投递同一 Run。工具返回稳定错误说明模型没有形成
/// 可推进动作，必须与其他协议纠正共用三次工程上限。
async fn commit_free_agent_tool_call(
    transaction: &DatabaseTransaction,
    context: FreeAgentToolContext,
    request_id: Uuid,
    provider_request_id: Option<String>,
    usage: &Value,
    outcome: FreeAgentToolResult<'_>,
    now: i64,
) -> Result<bool, StoreError> {
    let FreeAgentToolContext {
        request,
        run_model,
        run_id,
        version_id,
        run_kind,
        workflow,
        sequence,
        tool_call_id,
        provider_call_id,
        args_json,
        assistant_message,
    } = context;
    if run_model.consecutive_protocol_errors < 0 {
        return Err(StoreError::CorruptRecord(
            "Run consecutive_protocol_errors 不能为负数".to_owned(),
        ));
    }
    let is_protocol_error = outcome.status == "error";
    let next_error_count = if is_protocol_error {
        next_protocol_error_count(run_model.consecutive_protocol_errors)?
    } else {
        0
    };
    let stop = is_protocol_error && next_error_count >= MAX_CONSECUTIVE_PROTOCOL_ERRORS;
    let next_workflow = if stop {
        stop_writing_after_protocol_error(&workflow, run_id)?
    } else {
        continue_after_agent_tool(&workflow, run_id)?
    };
    let run_status = if stop { "stopped" } else { "queued" };
    let result_json = serde_json::to_string(outcome.result)?;
    let mut messages = vec![assistant_message];
    messages.push(NewAgentMessage {
        role: AgentMessageRole::Tool,
        content: json!({
            "kind": "tool_result",
            "content": result_json,
        }),
        request_id: Some(request_id),
        tool_call_id: Some(provider_call_id.clone()),
    });
    append_agent_messages_for_run(transaction, &run_model, messages, now).await?;
    tool_call::ActiveModel {
        id: Set(db_id(tool_call_id)),
        request_id: Set(request.id),
        run_id: Set(run_model.id),
        provider_call_id: Set(provider_call_id.clone()),
        sequence: Set(sequence),
        name: Set(outcome.name.to_owned()),
        args_json: Set(args_json),
        base_revision_id: Set(None),
        proposal_json: Set(None),
        held_json: Set(None),
        status: Set(outcome.status.to_owned()),
        result_json: Set(Some(result_json)),
        included_request_id: Set(None),
        created_at: Set(now),
        resolved_at: Set(Some(now)),
    }
    .insert(transaction)
    .await?;
    let bound_feedback_ids = if run_kind == "chapter_body" {
        bind_pending_feedback_to_tool(transaction, run_id, tool_call_id, sequence, now).await?
    } else {
        Vec::new()
    };
    let mut active_request = request.into_active_model();
    active_request.status = Set("succeeded".to_owned());
    active_request.provider_request_id = Set(provider_request_id);
    active_request.usage_json = Set(Some(serde_json::to_string(usage)?));
    active_request.error_json = Set(None);
    active_request.updated_at = Set(now);
    active_request.completed_at = Set(Some(now));
    active_request.update(transaction).await?;
    let request_count = run_model.request_count;
    let mut active_run = run_model.into_active_model();
    active_run.status = Set(run_status.to_owned());
    active_run.consecutive_protocol_errors = Set(next_error_count);
    active_run.ended_at = Set(if stop { Some(now) } else { None });
    active_run.updated_at = Set(now);
    active_run.update(transaction).await?;
    persist_workflow(transaction, &next_workflow).await?;
    let mut tool_event = json!({
        "run_id": run_id,
        "chapter_id": next_workflow.active_chapter_id,
        "name": outcome.name,
        "provider_call_id": provider_call_id,
        "status": outcome.status,
    });
    if let Some(code) = outcome.error_code {
        tool_event
            .as_object_mut()
            .expect("工具事件必须是 JSON object")
            .insert("code".to_owned(), Value::String(code.to_owned()));
    }
    if is_protocol_error {
        tool_event
            .as_object_mut()
            .expect("工具事件必须是 JSON object")
            .insert(
                "consecutive_protocol_errors".to_owned(),
                Value::String(next_error_count.to_string()),
            );
    }
    insert_event(
        transaction,
        version_id,
        "tool.resolved",
        tool_call_id,
        next_workflow.state_revision,
        tool_event,
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
    let mut run_event = json!({
        "kind": run_kind,
        "status": run_status,
        "request_id": request_id,
        "request_count": request_count.to_string(),
    });
    if is_protocol_error {
        let event = run_event
            .as_object_mut()
            .expect("Run 事件必须是 JSON object");
        event.insert("reason".to_owned(), Value::String("tool_error".to_owned()));
        event.insert(
            "consecutive_protocol_errors".to_owned(),
            Value::String(next_error_count.to_string()),
        );
        event.insert(
            "wait_reason".to_owned(),
            if stop {
                Value::String("protocol_error_limit".to_owned())
            } else {
                Value::Null
            },
        );
    }
    insert_event(
        transaction,
        version_id,
        "run.changed",
        run_id,
        next_workflow.state_revision,
        run_event,
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
            "request_id": request_id,
            "pause_requested": next_workflow.pause_requested,
            "protocol_correction": is_protocol_error,
            "consecutive_protocol_errors": next_error_count.to_string(),
        }),
        now,
    )
    .await?;
    Ok(!stop)
}

pub(crate) async fn next_agent_tool_sequence<C: ConnectionTrait>(
    connection: &C,
    run_id: &sea_orm::prelude::TextUuid,
) -> Result<i64, StoreError> {
    tool_call::Entity::find()
        .filter(tool_call::Entity::COLUMN.run_id.eq(*run_id))
        .order_by_desc(tool_call::Entity::COLUMN.sequence)
        .one(connection)
        .await?
        .map_or(Ok(1), |tool| {
            tool.sequence.checked_add(1).ok_or_else(|| {
                StoreError::InvalidModelRequest("正文工具调用 sequence 已达到 i64 上限".to_owned())
            })
        })
}

// 三个 `ask_user` / 自由工具的执行入口也搬到这里；它们原来在 `impl Store` 的中间，
// 现在自带一个 `impl Store` 块。
impl Store {
    /// 执行一条完整到达的 `ask_user` 调用。
    ///
    /// 合法问题会把模型请求收束为 `waiting_answer`，并原子写入 assistant/tool 审计、
    /// Question、Run 和工作流。参数无效时不会让模型调用消失：同样持久化一条稳定
    /// 错误工具结果并重新排队，让 Agent 能在下一轮自行修正。
    pub async fn execute_agent_question_tool_call(
        &self,
        request_id: Uuid,
        provider_request_id: Option<String>,
        usage: Value,
        call: AgentQuestionToolCall,
    ) -> Result<AppliedAgentQuestionToolCall, StoreError> {
        validate_agent_question_tool_call(&call)?;
        if !usage.is_object() {
            return Err(StoreError::InvalidModelRequest(
                "ask_user 模型响应的 usage 必须是 JSON object".to_owned(),
            ));
        }
        if provider_request_id
            .as_deref()
            .is_some_and(|provider_request_id| provider_request_id.trim().is_empty())
        {
            return Err(StoreError::InvalidModelRequest(
                "provider_request_id 不能是空字符串".to_owned(),
            ));
        }
        let args_json = serde_json::to_string(&call.arguments)?;
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                let call = call.clone();
                let args_json = args_json.clone();
                let provider_request_id = provider_request_id.clone();
                let usage = usage.clone();
                Box::pin(async move {
                    let request = model_request::Entity::find_by_id(db_id(request_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ModelRequestNotFound)?;
                    if let Some(existing) = tool_call::Entity::find()
                        .filter(tool_call::Entity::COLUMN.request_id.eq(request.id))
                        .filter(
                            tool_call::Entity::COLUMN
                                .provider_call_id
                                .eq(call.provider_call_id.clone()),
                        )
                        .one(transaction)
                        .await?
                    {
                        if existing.name != "ask_user" || existing.args_json != args_json {
                            return Err(StoreError::InvalidModelRequestState(
                                "同一 provider tool call ID 的名称或参数发生变化".to_owned(),
                            ));
                        }
                        let provider_call_id = existing.provider_call_id.clone();
                        return match existing.status.as_str() {
                            "awaiting_answer" => {
                                let question_model = question::Entity::find()
                                    .filter(question::Entity::COLUMN.tool_call_id.eq(existing.id))
                                    .one(transaction)
                                    .await?
                                    .ok_or_else(|| {
                                        StoreError::CorruptRecord(
                                            "待答 ask_user 工具调用缺少问题记录".to_owned(),
                                        )
                                    })?;
                                Ok(AppliedAgentQuestionToolCall::AwaitingAnswer {
                                    provider_call_id,
                                    question: pending_user_question_from_model(&question_model)?,
                                    newly_created: false,
                                })
                            }
                            "resolved" | "error" | "included" => {
                                let result_json = existing.result_json.ok_or_else(|| {
                                    StoreError::CorruptRecord(
                                        "已完成 ask_user 工具调用缺少 result_json".to_owned(),
                                    )
                                })?;
                                let result: Value = serde_json::from_str(&result_json)?;
                                if !result.is_object() {
                                    return Err(StoreError::CorruptRecord(
                                        "ask_user 工具结果必须是 JSON object".to_owned(),
                                    ));
                                }
                                Ok(AppliedAgentQuestionToolCall::Resolved {
                                    provider_call_id,
                                    result,
                                    newly_resolved: false,
                                    requeue: false,
                                })
                            }
                            _ => Err(StoreError::CorruptRecord(
                                "ask_user 工具调用处于未知状态".to_owned(),
                            )),
                        };
                    }
                    let FreeAgentToolContext {
                        request,
                        run_model,
                        run_id,
                        version_id,
                        run_kind,
                        workflow,
                        sequence,
                        tool_call_id,
                        provider_call_id,
                        args_json,
                        assistant_message,
                    } = prepare_free_agent_tool_call(
                        transaction,
                        request_id,
                        &call.provider_call_id,
                        "ask_user",
                        &call.arguments,
                    )
                    .await?;
                    match parse_agent_question_schema(&call.arguments) {
                        Ok(schema) => {
                            let next_workflow = wait_for_user_answer(&workflow, run_id)?;
                            let question_id = Uuid::new_v4();
                            let pending_question = PendingUserQuestion {
                                id: question_id,
                                tool_call_id,
                                schema: schema.clone(),
                                created_at_ms: now,
                            };
                            append_agent_messages_for_run(
                                transaction,
                                &run_model,
                                vec![assistant_message],
                                now,
                            )
                            .await?;
                            tool_call::ActiveModel {
                                id: Set(db_id(tool_call_id)),
                                request_id: Set(request.id),
                                run_id: Set(run_model.id),
                                provider_call_id: Set(call.provider_call_id.clone()),
                                sequence: Set(sequence),
                                name: Set("ask_user".to_owned()),
                                args_json: Set(args_json),
                                base_revision_id: Set(None),
                                proposal_json: Set(None),
                                held_json: Set(None),
                                status: Set("awaiting_answer".to_owned()),
                                result_json: Set(None),
                                included_request_id: Set(None),
                                created_at: Set(now),
                                resolved_at: Set(None),
                            }
                            .insert(transaction)
                            .await?;
                            question::ActiveModel {
                                id: Set(db_id(question_id)),
                                tool_call_id: Set(db_id(tool_call_id)),
                                schema_json: Set(serde_json::to_string(&schema)?),
                                answer_json: Set(None),
                                status: Set("open".to_owned()),
                                created_at: Set(now),
                                updated_at: Set(now),
                            }
                            .insert(transaction)
                            .await?;
                            let mut active_request = request.into_active_model();
                            active_request.status = Set("succeeded".to_owned());
                            active_request.provider_request_id = Set(provider_request_id);
                            active_request.usage_json = Set(Some(serde_json::to_string(&usage)?));
                            active_request.error_json = Set(None);
                            active_request.updated_at = Set(now);
                            active_request.completed_at = Set(Some(now));
                            active_request.update(transaction).await?;
                            let request_count = run_model.request_count;
                            let mut active_run = run_model.into_active_model();
                            active_run.status = Set("waiting".to_owned());
                            active_run.consecutive_protocol_errors = Set(0);
                            active_run.ended_at = Set(None);
                            active_run.updated_at = Set(now);
                            active_run.update(transaction).await?;
                            persist_workflow(transaction, &next_workflow).await?;
                            insert_event(
                                transaction,
                                version_id,
                                "question.created",
                                question_id,
                                next_workflow.state_revision,
                                json!({
                                    "run_id": run_id,
                                    "tool_call_id": tool_call_id,
                                    "provider_call_id": call.provider_call_id,
                                    "question_count": schema.questions.len(),
                                }),
                                now,
                            )
                            .await?;
                            insert_event(
                                transaction,
                                version_id,
                                "run.changed",
                                run_id,
                                next_workflow.state_revision,
                                json!({
                                    "kind": run_kind.clone(),
                                    "status": "waiting",
                                    "request_id": request_id,
                                    "request_count": request_count.to_string(),
                                    "wait_reason": "ask_user",
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
                                    "request_id": request_id,
                                    "pause_requested": next_workflow.pause_requested,
                                }),
                                now,
                            )
                            .await?;
                            Ok(AppliedAgentQuestionToolCall::AwaitingAnswer {
                                provider_call_id: call.provider_call_id,
                                question: pending_question,
                                newly_created: true,
                            })
                        }
                        Err(StoreError::InvalidQuestion(message)) => {
                            let tool_result = question_tool_error_result(
                                &call.provider_call_id,
                                "TOOL_ARGUMENTS_INVALID",
                                &message,
                            );
                            let requeue = commit_free_agent_tool_call(
                                transaction,
                                FreeAgentToolContext {
                                    request,
                                    run_model,
                                    run_id,
                                    version_id,
                                    run_kind,
                                    workflow,
                                    sequence,
                                    tool_call_id,
                                    provider_call_id,
                                    args_json,
                                    assistant_message,
                                },
                                request_id,
                                provider_request_id,
                                &usage,
                                FreeAgentToolResult {
                                    name: "ask_user",
                                    result: &tool_result,
                                    status: "error",
                                    error_code: Some("TOOL_ARGUMENTS_INVALID"),
                                },
                                now,
                            )
                            .await?;
                            Ok(AppliedAgentQuestionToolCall::Resolved {
                                provider_call_id: call.provider_call_id,
                                result: tool_result,
                                newly_resolved: true,
                                requeue,
                            })
                        }
                        Err(error) => Err(error),
                    }
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok(execution) => {
                if matches!(
                    execution,
                    AppliedAgentQuestionToolCall::AwaitingAnswer {
                        newly_created: true,
                        ..
                    } | AppliedAgentQuestionToolCall::Resolved {
                        newly_resolved: true,
                        ..
                    }
                ) {
                    self.notify_event_listeners();
                }
                Ok(execution)
            }
            Err(error) => Err(error),
        }
    }

    /// 读取一次自由工具调用已经落库的结果。
    ///
    /// 供应商回调重放时调用方先用它跳过重复的检索与重复扣费。
    ///
    /// # Errors
    ///
    /// 工具结果不是 JSON object 或数据库读取失败时返回错误。
    pub async fn free_agent_tool_result(
        &self,
        request_id: Uuid,
        provider_call_id: &str,
    ) -> Result<Option<Value>, StoreError> {
        let request = model_request::Entity::find_by_id(db_id(request_id))
            .one(&self.database)
            .await?
            .ok_or(StoreError::ModelRequestNotFound)?;
        let Some(existing) = tool_call::Entity::find()
            .filter(tool_call::Entity::COLUMN.request_id.eq(request.id))
            .filter(
                tool_call::Entity::COLUMN
                    .provider_call_id
                    .eq(provider_call_id.to_owned()),
            )
            .one(&self.database)
            .await?
        else {
            return Ok(None);
        };
        let Some(result_json) = existing.result_json else {
            return Ok(None);
        };
        let result: Value = serde_json::from_str(&result_json)?;
        if !result.is_object() {
            return Err(StoreError::CorruptRecord(
                "自由工具结果必须是 JSON object".to_owned(),
            ));
        }
        Ok(Some(result))
    }

    /// 提交一次由调用方算好结果的自由工具调用（目前只有 `query_knowledge`）。
    ///
    /// 结果落库与工作流推进在一个事务里完成；同一 `provider_call_id` 的重放只返回
    /// 已提交结果，不重复写入、不重复推进。
    ///
    /// # Errors
    ///
    /// 参数无效、请求或 Run 不存在、工作流不接受该工具、同一调用 ID 的名称或参数
    /// 发生变化，或数据库操作失败时返回错误。
    pub async fn execute_free_agent_tool_call(
        &self,
        request_id: Uuid,
        provider_request_id: Option<String>,
        usage: Value,
        call: FreeAgentToolCall,
    ) -> Result<AppliedFreeAgentToolCall, StoreError> {
        if call.provider_call_id.trim().is_empty() {
            return Err(StoreError::InvalidModelRequest(
                "自由工具 provider_call_id 不能为空".to_owned(),
            ));
        }
        if !usage.is_object() {
            return Err(StoreError::InvalidModelRequest(
                "自由工具调用的模型 usage 必须是 JSON object".to_owned(),
            ));
        }
        if provider_request_id
            .as_deref()
            .is_some_and(|provider_request_id| provider_request_id.trim().is_empty())
        {
            return Err(StoreError::InvalidModelRequest(
                "provider_request_id 不能是空字符串".to_owned(),
            ));
        }
        let args_json = serde_json::to_string(&call.arguments)?;
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                let call = call.clone();
                let args_json = args_json.clone();
                let provider_request_id = provider_request_id.clone();
                let usage = usage.clone();
                Box::pin(async move {
                    if let Some(request) = model_request::Entity::find_by_id(db_id(request_id))
                        .one(transaction)
                        .await?
                        && let Some(existing) = tool_call::Entity::find()
                            .filter(tool_call::Entity::COLUMN.request_id.eq(request.id))
                            .filter(
                                tool_call::Entity::COLUMN
                                    .provider_call_id
                                    .eq(call.provider_call_id.clone()),
                            )
                            .one(transaction)
                            .await?
                    {
                        if existing.name != call.name || existing.args_json != args_json {
                            return Err(StoreError::InvalidModelRequestState(
                                "同一 provider tool call ID 的名称或参数发生变化".to_owned(),
                            ));
                        }
                        let result_json = existing.result_json.ok_or_else(|| {
                            StoreError::CorruptRecord(
                                "已完成自由工具调用缺少 result_json".to_owned(),
                            )
                        })?;
                        return Ok((
                            AppliedFreeAgentToolCall {
                                result: serde_json::from_str(&result_json)?,
                                newly_resolved: false,
                                requeue: false,
                            },
                            false,
                        ));
                    }
                    let context = prepare_free_agent_tool_call(
                        transaction,
                        request_id,
                        &call.provider_call_id,
                        &call.name,
                        &call.arguments,
                    )
                    .await?;
                    let requeue = commit_free_agent_tool_call(
                        transaction,
                        context,
                        request_id,
                        provider_request_id,
                        &usage,
                        FreeAgentToolResult {
                            name: &call.name,
                            result: &call.result,
                            status: if call.failed { "error" } else { "resolved" },
                            error_code: call.error_code.as_deref(),
                        },
                        now,
                    )
                    .await?;
                    Ok((
                        AppliedFreeAgentToolCall {
                            result: call.result.clone(),
                            newly_resolved: true,
                            requeue,
                        },
                        true,
                    ))
                })
            })
            .await
            .map_err(transaction_error)?;
        if result.1 {
            self.notify_event_listeners();
        }
        Ok(result.0)
    }
}
