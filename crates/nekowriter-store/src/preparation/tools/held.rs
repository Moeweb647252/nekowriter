//! 正文工具的暂停与恢复：工具/提交两种暂停边界的解析、落库与结果构造。
//!
//! 从 `preparation/mod.rs` 的这一层原样搬出（`docs/08` 的 P3.6），只移动：函数体逐字不变，
//! 只改了可见性前缀（被块外引用的项改 `pub(crate)`）。`preparation/mod.rs` 用
//! `pub use tools::held::{…}`（两个公开结果类型）与 `pub(crate) use tools::held::{…}` 把名字接回，
//! 所以兄弟模块、app 侧与 mod.rs 里其它批次的调用点一行都不用改。

use nekowriter_core::{
    ApprovalMode, ChapterId, ContextFold, HeldChapterBodySubmission, PendingChapterBodyProposal,
    ReviewId, RevisionId, RunId, WorkflowSnapshot, non_whitespace_char_count,
};
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, IntoActiveModel,
    QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::agent_messages::{
    append_agent_messages_for_run, phase_submission_call_id, phase_submission_messages,
};
use crate::entity::{feedback, model_request, run, tool_call};
use crate::event_log::insert_event;
use crate::model_requests::protocol_error_tool_result_messages;
use crate::{AgentMessageRole, CHAPTER_BODY_ROLE, NewAgentMessage, StoreError, VersionId, db_id};

use super::super::citations::{
    bind_pending_feedback_to_tool, resolve_pending_candidate_feedback_citations,
};
use super::super::{
    CHAPTER_BODY_FEEDBACK_PROTOCOL_CORRECTION, CHAPTER_BODY_SUBMISSION_PROTOCOL_CORRECTION,
    RunCheckpoint, chapter_and_document, chapter_body_outline_dependency_revision_id,
    chapter_body_revision_dependency_ids, current_chapter_body, insert_chapter_edit_review,
    insert_chapter_final_review, insert_document_revision_with_payload_and_source,
    insert_revision_dependency, persist_workflow, revision_dependency_ids,
};
use super::model::{
    awaiting_body_proposal_result, candidate_content_hash,
    held_chapter_body_tool_payload_from_model,
};

/// 模型直接提交章节时命中暂停边界的 Run 检查点。
///
/// 提交工具调用直到恢复后才进入 Agent 消息账本；这样暂停不会把“已提交章节”
/// 提前作为已完成事实注入下一轮上下文。原始工具参数属于该 assistant 消息，
/// 恢复时与它的配对工具结果一起写回。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct HeldChapterBodySubmissionCheckpoint {
    pub(crate) held: HeldChapterBodySubmission,
    pub(crate) assistant_message: NewAgentMessage,
}

/// 一个已挂起正文工具调用恢复后的持久化结果。
///
/// 该结构只在同一 Store 事务内传递；它让“显式继续”和“提交意见后自动继续”共享
/// 同一条正文/提案/反馈线性化路径，而不是各自复制一套候选应用逻辑。
pub(crate) struct ResolvedHeldChapterBodyTool {
    tool_call_id: Uuid,
    provider_call_id: String,
    tool_name: String,
    tool_result: Value,
    pub(crate) next_workflow: WorkflowSnapshot,
    run_status: String,
    request_count: i64,
    committed_revision: Option<(RevisionId, i64, String)>,
    awaiting_review: bool,
    pub(crate) bound_feedback_ids: Vec<Uuid>,
}

/// 在暂停命中的完整工具边界上恢复正文调用。
///
/// 调用方已经验证该 Run 是当前正文 Run 且工作流确实处于 `paused`。此函数负责余下
/// 的全部原子事实：基础修订复验、候选正文或提案、工具结果、反馈绑定、消息账本、
/// Run 状态与工作流。事件和命令回执仍由外层命令写入，以保留各命令自己的审计语义。
pub(crate) async fn resolve_held_chapter_body_tool(
    transaction: &DatabaseTransaction,
    workflow: &WorkflowSnapshot,
    body_run: run::Model,
    chapter_id: ChapterId,
    now: i64,
) -> Result<ResolvedHeldChapterBodyTool, StoreError> {
    let run_id: RunId = body_run.id.into();
    let version_id: VersionId = body_run.version_id.into();
    let (chapter_model, body_document) =
        chapter_and_document(transaction, chapter_id, CHAPTER_BODY_ROLE, "章节正文").await?;
    if chapter_model.status != "in_progress" {
        return Err(StoreError::InvalidChapterState(
            "章节尚未处于可编辑正文状态".to_owned(),
        ));
    }
    let chapter_version_id: VersionId = chapter_model.version_id.into();
    if chapter_version_id != version_id {
        return Err(StoreError::CorruptRecord(
            "已暂停正文工具调用章节与 Run 稿本不一致".to_owned(),
        ));
    }
    let held_call = tool_call::Entity::find()
        .filter(tool_call::Entity::COLUMN.run_id.eq(body_run.id))
        .filter(tool_call::Entity::COLUMN.status.eq("held"))
        .one(transaction)
        .await?
        .ok_or_else(|| {
            StoreError::InvalidChapterState("暂停状态缺少可恢复的完整工具调用".to_owned())
        })?;
    let held_payload = held_chapter_body_tool_payload_from_model(&held_call)?;
    let held = held_payload.held.clone();
    let current_body = current_chapter_body(transaction, chapter_id).await?;
    if current_body.revision_id != held.base_revision_id {
        return Err(StoreError::ProposalBaseRevisionConflict {
            expected: held.base_revision_id,
            actual: current_body.revision_id,
        });
    }
    let tool_call_id: Uuid = held_call.id.into();
    let request_id: Uuid = held_call.request_id.into();
    let held_sequence = held_call.sequence;
    let (mut tool_result, committed_revision, proposal) = match (
        held.candidate_content.clone(),
        held_payload.result_if_resumed,
    ) {
        (Some(candidate_content), None)
            if workflow.approval_mode == ApprovalMode::ApproveRequired =>
        {
            if tool_call::Entity::find()
                .filter(tool_call::Entity::COLUMN.run_id.eq(body_run.id))
                .filter(tool_call::Entity::COLUMN.status.eq("awaiting_review"))
                .one(transaction)
                .await?
                .is_some()
            {
                return Err(StoreError::InvalidChapterState(
                    "当前正文 Run 已有待审编辑提案".to_owned(),
                ));
            }
            let proposal = PendingChapterBodyProposal {
                tool_call_id,
                provider_call_id: held.provider_call_id.clone(),
                tool_name: held.tool_name.clone(),
                base_revision_id: held.base_revision_id,
                base_revision_no: held.base_revision_no,
                candidate_hash: candidate_content_hash(&candidate_content),
                candidate_content,
            };
            (
                awaiting_body_proposal_result(&proposal),
                None,
                Some(proposal),
            )
        }
        (Some(candidate_content), None) => {
            let dependency_revision_ids = chapter_body_revision_dependency_ids(
                transaction,
                version_id,
                chapter_id,
                held.base_revision_id,
            )
            .await?;
            let (revision_id, revision_no) = insert_document_revision_with_payload_and_source(
                transaction,
                body_document.id.into(),
                &candidate_content,
                None,
                "agent",
                held.base_revision_id,
                now,
            )
            .await?;
            for dependency_revision_id in &dependency_revision_ids {
                insert_revision_dependency(
                    transaction,
                    revision_id,
                    *dependency_revision_id,
                    "chapter_body_input",
                )
                .await?;
            }
            resolve_pending_candidate_feedback_citations(
                transaction,
                run_id,
                tool_call_id,
                &candidate_content,
                body_document.id.into(),
                revision_id,
                chapter_id,
                now,
            )
            .await?;
            (
                json!({
                    "status": "applied",
                    "tool_call_id": held.provider_call_id,
                    "base_revision_id": held.base_revision_id.map(|revision_id| revision_id.to_string()),
                    "result_revision_id": revision_id.to_string(),
                    "data": {
                        "revision_no": revision_no.to_string(),
                        "word_count": non_whitespace_char_count(&candidate_content).to_string(),
                    },
                    "feedback": [],
                }),
                Some((revision_id, revision_no, candidate_content)),
                None,
            )
        }
        (None, Some(result)) => (result, None, None),
        _ => {
            return Err(StoreError::CorruptRecord(
                "已暂停正文工具调用缺少候选内容或稳定错误结果".to_owned(),
            ));
        }
    };
    let result_object = tool_result
        .as_object_mut()
        .ok_or_else(|| StoreError::CorruptRecord("正文工具结果必须是 JSON object".to_owned()))?;
    result_object
        .entry("tool_call_id".to_owned())
        .or_insert_with(|| Value::String(held.provider_call_id.clone()));
    result_object
        .entry("base_revision_id".to_owned())
        .or_insert_with(|| {
            held.base_revision_id.map_or(Value::Null, |revision_id| {
                Value::String(revision_id.to_string())
            })
        });
    result_object
        .entry("feedback".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    let awaiting_review = proposal.is_some();
    let next_workflow = if awaiting_review {
        nekowriter_core::resume_chapter_body_to_proposal(workflow, chapter_id, run_id)?
    } else {
        nekowriter_core::resume_chapter_body_after_pause(workflow, chapter_id, run_id)?
    };
    let mut active_call = held_call.into_active_model();
    active_call.proposal_json = Set(proposal.as_ref().map(serde_json::to_string).transpose()?);
    active_call.held_json = Set(None);
    active_call.status = Set(if awaiting_review {
        "awaiting_review"
    } else if tool_result["status"] == "applied" {
        "resolved"
    } else {
        "error"
    }
    .to_owned());
    active_call.result_json = Set((!awaiting_review)
        .then(|| serde_json::to_string(&tool_result))
        .transpose()?);
    active_call.included_request_id = Set(None);
    active_call.resolved_at = Set((!awaiting_review).then_some(now));
    active_call.update(transaction).await?;
    if awaiting_review {
        let dependency_revision_ids = chapter_body_revision_dependency_ids(
            transaction,
            version_id,
            chapter_id,
            held.base_revision_id,
        )
        .await?;
        insert_chapter_edit_review(
            transaction,
            version_id,
            tool_call_id,
            held.base_revision_id,
            &dependency_revision_ids,
            now,
        )
        .await?;
    }
    let bound_feedback_ids = if awaiting_review {
        Vec::new()
    } else {
        bind_pending_feedback_to_tool(transaction, run_id, tool_call_id, held_sequence, now).await?
    };
    if !awaiting_review {
        append_agent_messages_for_run(
            transaction,
            &body_run,
            vec![NewAgentMessage {
                role: AgentMessageRole::Tool,
                content: json!({
                    "kind": "tool_result",
                    "content": serde_json::to_string(&tool_result)?,
                }),
                request_id: Some(request_id),
                tool_call_id: Some(held.provider_call_id.clone()),
            }],
            now,
        )
        .await?;
    }
    let run_status = if awaiting_review { "waiting" } else { "queued" }.to_owned();
    let request_count = body_run.request_count;
    let mut active_run = body_run.into_active_model();
    active_run.status = Set(run_status.clone());
    active_run.ended_at = Set(None);
    active_run.updated_at = Set(now);
    active_run.update(transaction).await?;
    persist_workflow(transaction, &next_workflow).await?;
    Ok(ResolvedHeldChapterBodyTool {
        tool_call_id,
        provider_call_id: held.provider_call_id,
        tool_name: held.tool_name,
        tool_result,
        next_workflow,
        run_status,
        request_count,
        committed_revision,
        awaiting_review,
        bound_feedback_ids,
    })
}

pub(crate) async fn insert_resolved_held_chapter_body_tool_events(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    chapter_id: ChapterId,
    run_id: RunId,
    resolved: &ResolvedHeldChapterBodyTool,
    source: &'static str,
    now: i64,
) -> Result<(), StoreError> {
    if let Some((revision_id, revision_no, content)) = &resolved.committed_revision {
        insert_event(
            transaction,
            version_id,
            "document.committed",
            *revision_id,
            *revision_no,
            json!({
                "chapter_id": chapter_id,
                "role": CHAPTER_BODY_ROLE,
                "word_count": non_whitespace_char_count(content),
                "source": source,
            }),
            now,
        )
        .await?;
    }
    insert_event(
        transaction,
        version_id,
        if resolved.awaiting_review {
            "tool.proposed"
        } else {
            "tool.resolved"
        },
        resolved.tool_call_id,
        resolved.next_workflow.state_revision,
        json!({
            "chapter_id": chapter_id,
            "name": resolved.tool_name,
            "provider_call_id": resolved.provider_call_id,
            "status": resolved.tool_result["status"],
            "resumed": true,
            "source": source,
        }),
        now,
    )
    .await?;
    for feedback_id in &resolved.bound_feedback_ids {
        insert_event(
            transaction,
            version_id,
            "feedback.changed",
            *feedback_id,
            resolved.next_workflow.state_revision,
            json!({
                "chapter_id": chapter_id,
                "run_id": run_id,
                "status": "bound",
                "bound_tool_call_id": resolved.tool_call_id,
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
        resolved.next_workflow.state_revision,
        json!({
            "chapter_id": chapter_id,
            "kind": "chapter_body",
            "status": resolved.run_status,
            "request_count": resolved.request_count.to_string(),
        }),
        now,
    )
    .await?;
    insert_event(
        transaction,
        version_id,
        "workflow.changed",
        version_id,
        resolved.next_workflow.state_revision,
        json!({
            "stage": resolved.next_workflow.stage,
            "execution": resolved.next_workflow.execution,
            "chapter_id": chapter_id,
            "run_id": run_id,
            "pause_requested": false,
            "source": source,
        }),
        now,
    )
    .await?;
    Ok(())
}

/// 已挂起结构化章节提交恢复后的终态。
pub(crate) struct ResolvedHeldChapterBodySubmission {
    pub(crate) next_workflow: WorkflowSnapshot,
    run_status: String,
    request_id: Uuid,
    request_count: i64,
    review_id: Option<ReviewId>,
    protocol_correction: bool,
}

/// 恢复一个在“提交章节”边界命中的暂停。
///
/// 如果仍有未投递意见，或当前没有可审核的正文修订，恢复不会假装阶段已完成：它把
/// 原始助手文本与协议纠正消息写回消息账本，清除暂停并重新排队，等待新的合法正文
/// 工具结果。只有提交实际可应用时才创建终稿审核。两条路径都会清除 Run 检查点中的
/// 挂起提交。
pub(crate) async fn resolve_held_chapter_body_submission(
    transaction: &DatabaseTransaction,
    workflow: &WorkflowSnapshot,
    body_run: run::Model,
    chapter_id: ChapterId,
    now: i64,
) -> Result<ResolvedHeldChapterBodySubmission, StoreError> {
    let run_id: RunId = body_run.id.into();
    let version_id: VersionId = body_run.version_id.into();
    let mut checkpoint: RunCheckpoint = serde_json::from_str(&body_run.checkpoint_json)?;
    if checkpoint.chapter_id != Some(chapter_id) {
        return Err(StoreError::CorruptRecord(
            "已暂停章节提交的 Run 检查点与当前章节不匹配".to_owned(),
        ));
    }
    let held = checkpoint
        .held_chapter_body_submission
        .take()
        .ok_or_else(|| {
            StoreError::InvalidChapterState("暂停状态缺少可恢复的章节提交".to_owned())
        })?;
    if held.held.summary.trim().is_empty() {
        return Err(StoreError::CorruptRecord(
            "已暂停章节提交缺少摘要".to_owned(),
        ));
    }
    phase_submission_call_id(held.held.request_id, &held.assistant_message)?;
    let request = model_request::Entity::find_by_id(db_id(held.held.request_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::ModelRequestNotFound)?;
    if request.run_id != body_run.id || request.status != "succeeded" {
        return Err(StoreError::CorruptRecord(
            "已暂停章节提交的请求状态与 Run 不一致".to_owned(),
        ));
    }
    let (chapter_model, _) =
        chapter_and_document(transaction, chapter_id, CHAPTER_BODY_ROLE, "章节正文").await?;
    let chapter_version_id: VersionId = chapter_model.version_id.into();
    if chapter_model.status != "in_progress" || chapter_version_id != version_id {
        return Err(StoreError::InvalidChapterState(
            "当前章节不属于可恢复提交的活跃稿本".to_owned(),
        ));
    }
    let has_pending_feedback = feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(body_run.id))
        .filter(feedback::Entity::COLUMN.status.is_in(["queued", "bound"]))
        .one(transaction)
        .await?
        .is_some();
    let mut submission_problem = None;
    let mut applicable_submission = None;
    if !has_pending_feedback {
        let body = current_chapter_body(transaction, chapter_id).await?;
        if let Some(body_revision_id) = body.revision_id {
            let body_content = body
                .content
                .ok_or_else(|| StoreError::CorruptRecord("章节正文当前修订缺少内容".to_owned()))?;
            if let Err(error) = nekowriter_core::validate_document_content(
                "章节正文",
                &body_content,
                nekowriter_core::MAX_ARTIFACT_CONTENT_LEN,
            ) {
                submission_problem = Some(error.to_string());
            }
            if submission_problem.is_none() {
                let outline_revision_id = match chapter_body_outline_dependency_revision_id(
                    transaction,
                    version_id,
                    chapter_id,
                )
                .await
                {
                    Ok(revision_id) => Some(revision_id),
                    Err(StoreError::InvalidChapterState(message)) => {
                        submission_problem = Some(message);
                        None
                    }
                    Err(error) => return Err(error),
                };
                if let Some(outline_revision_id) = outline_revision_id {
                    let dependency_revision_ids =
                        revision_dependency_ids(transaction, body_revision_id).await?;
                    if dependency_revision_ids.contains(&outline_revision_id) {
                        applicable_submission = Some((body_revision_id, dependency_revision_ids));
                    } else {
                        submission_problem = Some("当前正文未依赖已通过的章节细纲".to_owned());
                    }
                }
            }
        } else {
            submission_problem = Some("章节正文不能为空".to_owned());
        }
    }
    let correction = if has_pending_feedback {
        Some(CHAPTER_BODY_FEEDBACK_PROTOCOL_CORRECTION.to_owned())
    } else {
        submission_problem.map(|reason| {
            format!("{CHAPTER_BODY_SUBMISSION_PROTOCOL_CORRECTION}\n本次无法应用的原因：{reason}")
        })
    };
    let (next_workflow, run_status, review_id, protocol_correction) = if let Some(correction) =
        correction
    {
        let next_workflow =
            nekowriter_core::resume_chapter_body_after_pause(workflow, chapter_id, run_id)?;
        let mut messages = vec![held.assistant_message.clone()];
        messages.extend(protocol_error_tool_result_messages(
            held.held.request_id,
            &held.assistant_message,
            &correction,
            "PHASE_SUBMISSION_REJECTED",
        )?);
        messages.push(NewAgentMessage {
            role: AgentMessageRole::User,
            content: json!({
                "kind": "protocol_correction",
                "content": correction,
            }),
            request_id: Some(held.held.request_id),
            tool_call_id: None,
        });
        append_agent_messages_for_run(transaction, &body_run, messages, now).await?;
        (next_workflow, "queued".to_owned(), None, true)
    } else {
        let (body_revision_id, dependency_revision_ids) = applicable_submission
            .ok_or_else(|| StoreError::CorruptRecord("暂停章节提交缺少可应用性判定".to_owned()))?;
        let next_workflow = nekowriter_core::resume_chapter_body_submission_after_pause(
            workflow, chapter_id, run_id,
        )?;
        let review_id = insert_chapter_final_review(
            transaction,
            version_id,
            body_revision_id,
            &dependency_revision_ids,
            now,
        )
        .await?;
        append_agent_messages_for_run(
            transaction,
            &body_run,
            phase_submission_messages(held.held.request_id, held.assistant_message.clone())?,
            now,
        )
        .await?;
        (next_workflow, "waiting".to_owned(), Some(review_id), false)
    };
    let request_count = body_run.request_count;
    let mut active_run = body_run.into_active_model();
    active_run.status = Set(run_status.clone());
    active_run.checkpoint_json = Set(serde_json::to_string(&checkpoint)?);
    active_run.ended_at = Set(None);
    active_run.updated_at = Set(now);
    active_run.update(transaction).await?;
    persist_workflow(transaction, &next_workflow).await?;
    Ok(ResolvedHeldChapterBodySubmission {
        next_workflow,
        run_status,
        request_id: held.held.request_id,
        request_count,
        review_id,
        protocol_correction,
    })
}

pub(crate) async fn insert_resolved_held_chapter_body_submission_events(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    chapter_id: ChapterId,
    run_id: RunId,
    resolved: &ResolvedHeldChapterBodySubmission,
    source: &'static str,
    now: i64,
) -> Result<(), StoreError> {
    if let Some(review_id) = resolved.review_id {
        insert_event(
            transaction,
            version_id,
            "review.changed",
            review_id,
            resolved.next_workflow.state_revision,
            json!({
                "chapter_id": chapter_id,
                "role": CHAPTER_BODY_ROLE,
                "status": "open",
                "source": source,
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
        resolved.next_workflow.state_revision,
        json!({
            "chapter_id": chapter_id,
            "kind": "chapter_body",
            "status": resolved.run_status,
            "request_id": resolved.request_id,
            "request_count": resolved.request_count.to_string(),
            "protocol_correction": resolved.protocol_correction,
        }),
        now,
    )
    .await?;
    insert_event(
        transaction,
        version_id,
        "workflow.changed",
        version_id,
        resolved.next_workflow.state_revision,
        json!({
            "stage": resolved.next_workflow.stage,
            "execution": resolved.next_workflow.execution,
            "chapter_id": chapter_id,
            "run_id": run_id,
            "request_id": resolved.request_id,
            "pause_requested": false,
            "source": source,
        }),
        now,
    )
    .await?;
    Ok(())
}

/// 暂停的正文 Run 在任一时刻只能保留一种边界事实。
///
/// 工具调用和直接阶段提交都可能成为暂停的线性化点。将判定收敛在这里，避免恢复
/// 命令与“提交意见后恢复”各自假定暂停总是由工具调用造成，并使损坏的双重检查点
/// 在写入任何新状态前失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HeldChapterBodyPauseBoundary {
    Tool,
    Submission,
}

pub(crate) async fn held_chapter_body_pause_boundary<C: ConnectionTrait>(
    connection: &C,
    body_run: &run::Model,
    chapter_id: ChapterId,
) -> Result<HeldChapterBodyPauseBoundary, StoreError> {
    let checkpoint: RunCheckpoint = serde_json::from_str(&body_run.checkpoint_json)?;
    if checkpoint.chapter_id != Some(chapter_id) {
        return Err(StoreError::CorruptRecord(
            "正文暂停 Run 检查点与当前章节不匹配".to_owned(),
        ));
    }
    let held_tools = tool_call::Entity::find()
        .filter(tool_call::Entity::COLUMN.run_id.eq(body_run.id))
        .filter(tool_call::Entity::COLUMN.status.eq("held"))
        .all(connection)
        .await?;
    match (
        held_tools.len(),
        checkpoint.held_chapter_body_submission.is_some(),
    ) {
        (1, false) => Ok(HeldChapterBodyPauseBoundary::Tool),
        (0, true) => Ok(HeldChapterBodyPauseBoundary::Submission),
        (0, false) => Err(StoreError::InvalidChapterState(
            "暂停状态缺少可恢复的正文工具调用或章节提交".to_owned(),
        )),
        _ => Err(StoreError::CorruptRecord(
            "正文暂停状态同时保留了多个恢复边界".to_owned(),
        )),
    }
}

/// 已完成阶段可供后续 Agent 请求使用的纯文本折叠来源。
///
/// Store 只描述持久化 Run、章节顺序和折叠本身；App 将其映射为 agent crate 的
/// `CompletedStageContext`，避免 store 反向依赖模型协议层。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletedRunContextStage {
    Preparation,
    ChapterOutline,
    ChapterBody,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedRunContextFold {
    pub stage: CompletedRunContextStage,
    pub chapter_order: Option<i64>,
    pub source_run_id: RunId,
    pub fold: ContextFold,
}
