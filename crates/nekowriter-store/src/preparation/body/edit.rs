//! 正文写入与提交：正文与修订读取、追加与替换、正文 Agent 工具调用的落库、待审提案的决定。
//!
//! 从 `preparation/mod.rs` 的这一层原样搬出（`docs/08` 的 P3.9）：函数体逐字不变，本层自用的
//! `*_METHOD` 常量与私有 payload DTO 随命令一起搬出。六个方法都是 `Store` 的固有方法，兄弟模块与
//! app 侧原本就按方法语法调用，一行都不用改，也不需要任何再导出。

use nekowriter_core::{
    AppendChapterContent, ApprovalMode, ChapterBodySnapshot, ChapterId, DocumentId,
    DocumentRevisionView, ExecutionStatus, FeedbackCitation, HeldChapterBodyTool,
    MAX_CONSECUTIVE_PROTOCOL_ERRORS, PendingChapterBodyProposal, ReplaceChapterContent, ReviewId,
    RevisionId, RunId, WorkflowStage, continue_chapter_body_after_proposal,
    continue_chapter_body_after_tool, hold_chapter_body_proposal, hold_chapter_body_tool_for_pause,
    non_whitespace_char_count, replace_unique_chapter_content, stop_writing_after_protocol_error,
};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, QueryFilter, Set, TransactionTrait};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::agent_messages::append_agent_messages_for_run;
use crate::entity::{document, document_revision, model_request, review, run, tool_call};
use crate::event_log::insert_event;
use crate::model_requests::next_protocol_error_count;
use crate::{
    AgentMessageRole, CHAPTER_BODY_ROLE, Command, NewAgentMessage, Notify, Store, StoreError,
    VersionId, db_id, insert_receipt, parse_optional_id, payload_hash, transaction_error,
    utc_now_millis,
};

// 下面这些名字由 `preparation/mod.rs` 从 `tools::model` / `tools::free` / `citations` 再导出，
// 按 `super::super::…` 引用就能同时让那些再导出行继续有消费者。
use super::super::{
    AgentChapterBodyMutation, AppliedChapterBodyToolCall, CandidateCitationScope,
    ChapterBodyAgentToolCall, ChapterBodyMutation, HeldChapterBodyToolPayload,
    agent_body_edit_error_result, agent_tool_error_result, awaiting_body_proposal_result,
    bind_pending_feedback_to_tool, candidate_content_hash, chapter_and_document,
    chapter_body_revision_dependency_ids, chapter_body_run, chapter_body_snapshot,
    current_chapter_body, ensure_expected_state_revision, held_body_tool_result,
    held_chapter_body_tool_payload_from_model, insert_chapter_edit_review,
    insert_document_revision_with_payload_and_source, insert_revision_dependency,
    next_agent_tool_sequence, parse_agent_body_mutation,
    pending_chapter_body_proposal_from_tool_call, persist_workflow,
    resolve_pending_candidate_feedback_citations, update_chapter_body_run_status,
    validate_agent_body_tool_call, validate_chapter_body_feedback_citations, workflow_for_version,
};

const APPEND_CHAPTER_BODY_METHOD: &str = "POST";
const REPLACE_CHAPTER_BODY_METHOD: &str = "POST";
const DECIDE_CHAPTER_BODY_PROPOSAL_METHOD: &str = "POST";

#[derive(Serialize)]
struct AppendChapterBodyPayload<'a> {
    expected_state_revision: i64,
    content: &'a str,
}

#[derive(Serialize)]
struct ReplaceChapterBodyPayload<'a> {
    expected_state_revision: i64,
    old_str: &'a str,
    new_str: &'a str,
}

#[derive(Serialize)]
struct DecideChapterBodyProposalPayload<'a> {
    expected_state_revision: i64,
    feedback: &'a str,
    citations: &'a [FeedbackCitation],
}

impl Store {
    /// 读取当前章节的正文、终稿审核状态以及两个阶段的纯文本折叠。
    pub async fn get_chapter_body(
        &self,
        chapter_id: ChapterId,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move { chapter_body_snapshot(transaction, chapter_id).await })
            })
            .await
            .map_err(transaction_error)
    }

    /// 读取一条不可变文档修订，用于引用回看与历史核对。
    ///
    /// 引用锚定在提交时的具体修订上，因此该修订即使已经不是文档当前修订，也必须
    /// 仍可读取；这里只做只读查询，不修改任何文档状态。
    ///
    /// # Errors
    ///
    /// 修订不存在、与 `document_id` 不匹配、或所属文档已删除时返回
    /// [`StoreError::DocumentRevisionNotFound`]。
    pub async fn get_document_revision(
        &self,
        document_id: DocumentId,
        revision_id: RevisionId,
    ) -> Result<DocumentRevisionView, StoreError> {
        let Some(revision) = document_revision::Entity::find_by_id(db_id(revision_id))
            .one(&self.database)
            .await?
        else {
            return Err(StoreError::DocumentRevisionNotFound);
        };
        if revision.document_id != db_id(document_id) {
            return Err(StoreError::DocumentRevisionNotFound);
        }
        let document = document::Entity::find_by_id(db_id(document_id))
            .filter(document::Entity::COLUMN.deleted_at.is_null())
            .one(&self.database)
            .await?
            .ok_or(StoreError::DocumentRevisionNotFound)?;
        let is_current = document
            .current_revision_id
            .as_deref()
            .and_then(|value| Uuid::parse_str(value).ok())
            == Some(revision_id);
        Ok(DocumentRevisionView {
            document_id,
            revision_id,
            revision_no: revision.revision_no,
            role: document.role,
            chapter_id: parse_optional_id(document.chapter_id.as_deref(), "document.chapter_id")?,
            word_count: revision.word_count,
            content: revision.content,
            is_current,
            created_at_ms: revision.created_at,
        })
    }

    /// 以自动接受模式追加当前章节正文。
    pub async fn append_chapter_body(
        &self,
        chapter_id: ChapterId,
        command_id: Uuid,
        expected_state_revision: i64,
        input: AppendChapterContent,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        let target = format!("/api/chapters/{chapter_id}/body/append");
        let payload_hash = payload_hash(&AppendChapterBodyPayload {
            expected_state_revision,
            content: &input.content,
        })?;
        self.mutate_chapter_body(
            chapter_id,
            command_id,
            expected_state_revision,
            target,
            payload_hash,
            APPEND_CHAPTER_BODY_METHOD,
            ChapterBodyMutation::Append(input),
        )
        .await
    }

    /// 以自动接受模式精确替换当前章节正文的一段文本。
    pub async fn replace_chapter_body(
        &self,
        chapter_id: ChapterId,
        command_id: Uuid,
        expected_state_revision: i64,
        input: ReplaceChapterContent,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        let target = format!("/api/chapters/{chapter_id}/body/replace");
        let payload_hash = payload_hash(&ReplaceChapterBodyPayload {
            expected_state_revision,
            old_str: &input.old_str,
            new_str: &input.new_str,
        })?;
        self.mutate_chapter_body(
            chapter_id,
            command_id,
            expected_state_revision,
            target,
            payload_hash,
            REPLACE_CHAPTER_BODY_METHOD,
            ChapterBodyMutation::Replace(input),
        )
        .await
    }

    /// 在同一事务中执行一条正文 Agent 工具调用。
    ///
    /// 这不是 HTTP 命令的包装：`request_id + provider_call_id` 是模型回调的幂等键。
    /// 一次成功的事务会同时完成模型请求、assistant/tool 消息、工具调用审计、可选
    /// 正文修订或待审提案、Run 状态、工作流状态和持久事件。自动接受或业务校验
    /// 失败后会为下一轮请求恢复排队；逐次确认模式的实际正文变更则停在待审提案。
    pub async fn execute_chapter_body_agent_tool_call(
        &self,
        request_id: Uuid,
        chapter_id: ChapterId,
        provider_request_id: Option<String>,
        usage: Value,
        call: ChapterBodyAgentToolCall,
    ) -> Result<AppliedChapterBodyToolCall, StoreError> {
        validate_agent_body_tool_call(&call)?;
        if !usage.is_object() {
            return Err(StoreError::InvalidModelRequest(
                "正文工具调用的模型 usage 必须是 JSON object".to_owned(),
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
                        if existing.name != call.name || existing.args_json != args_json {
                            return Err(StoreError::InvalidModelRequestState(
                                "同一 provider tool call ID 的名称或参数发生变化".to_owned(),
                            ));
                        }
                        let result = if existing.status == "awaiting_review" {
                            let proposal = pending_chapter_body_proposal_from_tool_call(&existing)?;
                            awaiting_body_proposal_result(&proposal)
                        } else if existing.status == "held" {
                            let held = held_chapter_body_tool_payload_from_model(&existing)?;
                            held_body_tool_result(&held.held)
                        } else {
                            let result_json = existing.result_json.ok_or_else(|| {
                                StoreError::CorruptRecord(
                                    "已解析正文工具调用缺少 result_json".to_owned(),
                                )
                            })?;
                            let result: Value = serde_json::from_str(&result_json)?;
                            if !result.is_object() {
                                return Err(StoreError::CorruptRecord(
                                    "已解析正文工具调用的 result_json 必须是 JSON object"
                                        .to_owned(),
                                ));
                            }
                            result
                        };
                        return Ok((
                            AppliedChapterBodyToolCall {
                                provider_call_id: call.provider_call_id,
                                result,
                                newly_resolved: false,
                                requeue: false,
                            },
                            false,
                        ));
                    }

                    if request.status != "dispatched" {
                        return Err(StoreError::InvalidModelRequestState(format!(
                            "只有 dispatched 请求可以执行正文工具，当前为 {}",
                            request.status
                        )));
                    }
                    let run_model = run::Entity::find_by_id(request.run_id)
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some() || run_model.kind != "chapter_body" {
                        return Err(StoreError::InvalidModelRequestState(
                            "该模型请求不属于顶层章节正文 Run".to_owned(),
                        ));
                    }
                    let run_id: RunId = run_model.id.into();
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    if workflow.active_run_id != Some(run_id)
                        || workflow.active_chapter_id != Some(chapter_id)
                        || workflow.stage != WorkflowStage::WritingDrafting
                        || workflow.execution != ExecutionStatus::RequestInFlight
                    {
                        return Err(StoreError::InvalidModelRequestState(
                            "章节正文工作流已不接受此工具调用".to_owned(),
                        ));
                    }

                    let (chapter_model, body_document) = chapter_and_document(
                        transaction,
                        chapter_id,
                        CHAPTER_BODY_ROLE,
                        "章节正文",
                    )
                    .await?;
                    if chapter_model.status != "in_progress" {
                        return Err(StoreError::InvalidChapterState(
                            "章节尚未处于可编辑正文状态".to_owned(),
                        ));
                    }
                    let chapter_version_id: VersionId = chapter_model.version_id.into();
                    if chapter_version_id != version_id {
                        return Err(StoreError::CorruptRecord(
                            "正文工具调用章节与 Run 稿本不一致".to_owned(),
                        ));
                    }
                    chapter_body_run(transaction, run_id, chapter_id).await?;

                    let current_body = current_chapter_body(transaction, chapter_id).await?;
                    let base_revision_id = current_body.revision_id;
                    let base_revision_no = current_body.revision_no;
                    let current_content = current_body.content.unwrap_or_default();
                    let candidate = match parse_agent_body_mutation(&call) {
                        Ok(AgentChapterBodyMutation::Append(input)) => {
                            Ok(format!("{current_content}{}", input.content))
                        }
                        Ok(AgentChapterBodyMutation::Replace(input)) => replace_unique_chapter_content(
                            &current_content,
                            &input.old_str,
                            &input.new_str,
                        )
                        .map_err(|error| agent_body_edit_error_result(&error)),
                        Err(error) => Err(error.into_result()),
                    };
                    let tool_call_id = Uuid::new_v4();
                    let sequence =
                        next_agent_tool_sequence(transaction, &run_model.id).await?;
                    if workflow.pause_requested {
                        if tool_call::Entity::find()
                            .filter(tool_call::Entity::COLUMN.run_id.eq(run_model.id))
                            .filter(tool_call::Entity::COLUMN.status.eq("held"))
                            .one(transaction)
                            .await?
                            .is_some()
                        {
                            return Err(StoreError::InvalidModelRequestState(
                                "当前正文 Run 已有挂起的工具调用".to_owned(),
                            ));
                        }
                        let (candidate_content, result_if_resumed) = match candidate {
                            Ok(next_content)
                                if next_content.chars().count()
                                    > nekowriter_core::MAX_ARTIFACT_CONTENT_LEN =>
                            {
                                (
                                    None,
                                    Some(agent_tool_error_result(
                                        "VALIDATION_ERROR",
                                        &format!(
                                            "章节正文超过 {} 个字符",
                                            nekowriter_core::MAX_ARTIFACT_CONTENT_LEN
                                        ),
                                    )),
                                )
                            }
                            Ok(next_content) => (Some(next_content), None),
                            Err(error) => (None, Some(error)),
                        };
                        let candidate_hash = candidate_content
                            .as_deref()
                            .map(candidate_content_hash);
                        let held = HeldChapterBodyTool {
                            tool_call_id,
                            provider_call_id: call.provider_call_id.clone(),
                            tool_name: call.name.clone(),
                            base_revision_id,
                            base_revision_no,
                            candidate_content,
                            candidate_hash,
                        };
                        let held_payload = HeldChapterBodyToolPayload {
                            held: held.clone(),
                            result_if_resumed,
                        };
                        let next_workflow =
                            hold_chapter_body_tool_for_pause(&workflow, chapter_id, run_id)?;
                        append_agent_messages_for_run(
                            transaction,
                            &run_model,
                            vec![NewAgentMessage {
                                role: AgentMessageRole::Assistant,
                                content: json!({
                                    "kind": "assistant_tool_calls",
                                    "content": Value::Null,
                                    "toolCalls": [{
                                        "id": call.provider_call_id.clone(),
                                        "name": call.name.clone(),
                                        "arguments": call.arguments.clone(),
                                    }],
                                }),
                                request_id: Some(request_id),
                                tool_call_id: None,
                            }],
                            now,
                        )
                        .await?;
                        tool_call::ActiveModel {
                            id: Set(db_id(tool_call_id)),
                            request_id: Set(request.id),
                            run_id: Set(run_model.id),
                            provider_call_id: Set(call.provider_call_id.clone()),
                            sequence: Set(sequence),
                            name: Set(call.name.clone()),
                            args_json: Set(args_json),
                            base_revision_id: Set(base_revision_id.map(|id| id.to_string())),
                            proposal_json: Set(None),
                            held_json: Set(Some(serde_json::to_string(&held_payload)?)),
                            status: Set("held".to_owned()),
                            result_json: Set(None),
                            included_request_id: Set(None),
                            created_at: Set(now),
                            resolved_at: Set(None),
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
                        active_run.status = Set("paused".to_owned());
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
                                "chapter_id": chapter_id,
                                "kind": "chapter_body",
                                "status": "paused",
                                "request_id": request_id,
                                "request_count": request_count.to_string(),
                                "held_tool_call_id": tool_call_id,
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
                                "chapter_id": chapter_id,
                                "run_id": run_id,
                                "request_id": request_id,
                            }),
                            now,
                        )
                        .await?;
                        return Ok((
                            AppliedChapterBodyToolCall {
                                provider_call_id: call.provider_call_id,
                                result: held_body_tool_result(&held),
                                newly_resolved: false,
                                requeue: false,
                            },
                            true,
                        ));
                    }
                    let mut proposal = None;
                    let (mut tool_result, committed_revision) = match candidate {
                        Ok(next_content)
                            if next_content.chars().count()
                                > nekowriter_core::MAX_ARTIFACT_CONTENT_LEN =>
                        {
                            (
                                agent_tool_error_result(
                                    "VALIDATION_ERROR",
                                    &format!(
                                        "章节正文超过 {} 个字符",
                                        nekowriter_core::MAX_ARTIFACT_CONTENT_LEN
                                    ),
                                ),
                                None,
                            )
                        }
                        Ok(next_content) if workflow.approval_mode == ApprovalMode::ApproveRequired => {
                            if tool_call::Entity::find()
                                .filter(tool_call::Entity::COLUMN.run_id.eq(run_model.id))
                                .filter(
                                    tool_call::Entity::COLUMN
                                        .status
                                        .eq("awaiting_review"),
                                )
                                .one(transaction)
                                .await?
                                .is_some()
                            {
                                return Err(StoreError::InvalidModelRequestState(
                                    "当前正文 Run 已有待审编辑提案".to_owned(),
                                ));
                            }
                            let pending = PendingChapterBodyProposal {
                                tool_call_id,
                                provider_call_id: call.provider_call_id.clone(),
                                tool_name: call.name.clone(),
                                base_revision_id,
                                base_revision_no,
                                candidate_hash: candidate_content_hash(&next_content),
                                candidate_content: next_content,
                            };
                            let result = awaiting_body_proposal_result(&pending);
                            proposal = Some(pending);
                            (result, None)
                        }
                        Ok(next_content) => {
                            let dependency_revision_ids = chapter_body_revision_dependency_ids(
                                transaction,
                                version_id,
                                chapter_id,
                                base_revision_id,
                            )
                            .await?;
                            let (revision_id, revision_no) =
                                insert_document_revision_with_payload_and_source(
                                    transaction,
                                    body_document.id.into(),
                                    &next_content,
                                    None,
                                    "agent",
                                    base_revision_id,
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
                            (
                                json!({
                                    "status": "applied",
                                    "tool_call_id": call.provider_call_id,
                                    "base_revision_id": base_revision_id.map(|revision_id| revision_id.to_string()),
                                    "result_revision_id": revision_id.to_string(),
                                    "data": {
                                        "revision_no": revision_no.to_string(),
                                        "word_count": non_whitespace_char_count(&next_content).to_string(),
                                    },
                                    "feedback": [],
                                }),
                                Some((revision_id, revision_no, next_content)),
                            )
                        }
                        Err(error) => (error, None),
                    };
                    let result_object = tool_result.as_object_mut().ok_or_else(|| {
                        StoreError::CorruptRecord("正文工具结果必须是 JSON object".to_owned())
                    })?;
                    result_object
                        .entry("tool_call_id".to_owned())
                        .or_insert_with(|| Value::String(call.provider_call_id.clone()));
                    result_object
                        .entry("base_revision_id".to_owned())
                        .or_insert_with(|| {
                            base_revision_id
                                .map_or(Value::Null, |revision_id| Value::String(revision_id.to_string()))
                        });
                    result_object
                        .entry("feedback".to_owned())
                        .or_insert_with(|| Value::Array(Vec::new()));
                    let awaiting_review = proposal.is_some();
                    if run_model.consecutive_protocol_errors < 0 {
                        return Err(StoreError::CorruptRecord(
                            "Run consecutive_protocol_errors 不能为负数".to_owned(),
                        ));
                    }
                    // `awaiting_review` 是一个已形成的业务边界；其余非 applied 工具
                    // 结果都没有推进正文，和非法 ask_user 一样必须受同一工程上限约束。
                    let is_protocol_error = !awaiting_review && tool_result["status"] != "applied";
                    let next_error_count = if is_protocol_error {
                        next_protocol_error_count(run_model.consecutive_protocol_errors)?
                    } else {
                        0
                    };
                    let stop = is_protocol_error
                        && next_error_count >= MAX_CONSECUTIVE_PROTOCOL_ERRORS;
                    let next_workflow = if awaiting_review {
                        hold_chapter_body_proposal(&workflow, chapter_id, run_id)?
                    } else if stop {
                        stop_writing_after_protocol_error(&workflow, run_id)?
                    } else {
                        continue_chapter_body_after_tool(&workflow, chapter_id, run_id)?
                    };
                    let assistant_message = NewAgentMessage {
                        role: AgentMessageRole::Assistant,
                        content: json!({
                            "kind": "assistant_tool_calls",
                            "content": Value::Null,
                            "toolCalls": [{
                                "id": call.provider_call_id.clone(),
                                "name": call.name.clone(),
                                "arguments": call.arguments.clone(),
                            }],
                        }),
                        request_id: Some(request_id),
                        tool_call_id: None,
                    };
                    let mut messages = vec![assistant_message];
                    if !awaiting_review {
                        let model_visible_result = serde_json::to_string(&tool_result)?;
                        messages.push(NewAgentMessage {
                            role: AgentMessageRole::Tool,
                            content: json!({
                                "kind": "tool_result",
                                "content": model_visible_result,
                            }),
                            request_id: Some(request_id),
                            tool_call_id: Some(call.provider_call_id.clone()),
                        });
                    }
                    append_agent_messages_for_run(
                        transaction,
                        &run_model,
                        messages,
                        now,
                    )
                    .await?;

                    tool_call::ActiveModel {
                        id: Set(db_id(tool_call_id)),
                        request_id: Set(request.id),
                        run_id: Set(run_model.id),
                        provider_call_id: Set(call.provider_call_id.clone()),
                        sequence: Set(sequence),
                        name: Set(call.name.clone()),
                        args_json: Set(args_json),
                        base_revision_id: Set(base_revision_id.map(|id| id.to_string())),
                        proposal_json: Set(
                            proposal
                                .as_ref()
                                .map(serde_json::to_string)
                                .transpose()?,
                        ),
                        held_json: Set(None),
                        status: Set(
                            if awaiting_review {
                                "awaiting_review"
                            } else if tool_result["status"] == "applied" {
                                "resolved"
                            } else {
                                "error"
                            }
                            .to_owned(),
                        ),
                        result_json: Set((!awaiting_review)
                            .then(|| serde_json::to_string(&tool_result))
                            .transpose()?),
                        included_request_id: Set(None),
                        created_at: Set(now),
                        resolved_at: Set((!awaiting_review).then_some(now)),
                    }
                    .insert(transaction)
                    .await?;

                    if awaiting_review {
                        let dependency_revision_ids = chapter_body_revision_dependency_ids(
                            transaction,
                            version_id,
                            chapter_id,
                            base_revision_id,
                        )
                        .await?;
                        insert_chapter_edit_review(
                            transaction,
                            version_id,
                            tool_call_id,
                            base_revision_id,
                            &dependency_revision_ids,
                            now,
                        )
                        .await?;
                    }

                    let bound_feedback_ids = if awaiting_review {
                        Vec::new()
                    } else {
                        bind_pending_feedback_to_tool(
                            transaction,
                            run_id,
                            tool_call_id,
                            sequence,
                            now,
                        )
                        .await?
                    };

                    let mut active_request = request.into_active_model();
                    active_request.status = Set("succeeded".to_owned());
                    active_request.provider_request_id = Set(provider_request_id);
                    active_request.usage_json = Set(Some(serde_json::to_string(&usage)?));
                    active_request.error_json = Set(None);
                    active_request.updated_at = Set(now);
                    active_request.completed_at = Set(Some(now));
                    active_request.update(transaction).await?;
                    let request_count = run_model.request_count;
                    let run_status = if awaiting_review {
                        "waiting"
                    } else if stop {
                        "stopped"
                    } else {
                        "queued"
                    };
                    let mut active_run = run_model.into_active_model();
                    active_run.status = Set(run_status.to_owned());
                    active_run.consecutive_protocol_errors = Set(next_error_count);
                    active_run.ended_at = Set(if stop { Some(now) } else { None });
                    active_run.updated_at = Set(now);
                    active_run.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;

                    if let Some((revision_id, revision_no, content)) = committed_revision {
                        insert_event(
                            transaction,
                            version_id,
                            "document.committed",
                            revision_id,
                            revision_no,
                            json!({
                                "chapter_id": chapter_id,
                                "role": CHAPTER_BODY_ROLE,
                                "word_count": non_whitespace_char_count(&content),
                                "source": "agent_tool",
                            }),
                            now,
                        )
                        .await?;
                    }
                    let mut tool_event = json!({
                        "chapter_id": chapter_id,
                        "name": call.name.clone(),
                        "provider_call_id": call.provider_call_id.clone(),
                        "status": tool_result["status"],
                        "proposal_id": proposal.as_ref().map(|proposal| proposal.tool_call_id),
                    });
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
                        if awaiting_review {
                            "tool.proposed"
                        } else {
                            "tool.resolved"
                        },
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
                                "chapter_id": chapter_id,
                                "run_id": run_id,
                                "status": "bound",
                                "bound_tool_call_id": tool_call_id,
                            }),
                            now,
                        )
                        .await?;
                    }
                    let mut run_event = json!({
                        "chapter_id": chapter_id,
                        "kind": "chapter_body",
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
                            "chapter_id": chapter_id,
                            "run_id": run_id,
                            "request_id": request_id,
                            "protocol_correction": is_protocol_error,
                            "consecutive_protocol_errors": next_error_count.to_string(),
                        }),
                        now,
                    )
                    .await?;
                    Ok((
                        AppliedChapterBodyToolCall {
                            provider_call_id: call.provider_call_id,
                            result: tool_result,
                            newly_resolved: !awaiting_review,
                            requeue: !awaiting_review && !stop,
                        },
                        true,
                    ))
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok((execution, changed)) => {
                if changed {
                    self.notify_event_listeners();
                }
                Ok(execution)
            }
            Err(error) => Err(error),
        }
    }

    /// 对一条待审正文提案作出决定。
    ///
    /// 空白意见表示接受，非空意见表示拒绝并作为下一轮工具结果反馈给 Agent。两种
    /// 决定都在同一事务里完成基础修订复核、工具结果、消息账本、Run/工作流重新
    /// 排队、事件和命令回执；因此同一 command ID 的网络重试不会重复创建正文修订。
    pub async fn decide_chapter_body_proposal(
        &self,
        chapter_id: ChapterId,
        tool_call_id: Uuid,
        command_id: Uuid,
        expected_state_revision: i64,
        feedback: String,
        citations: Vec<FeedbackCitation>,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        if feedback.chars().count() > nekowriter_core::MAX_REVIEW_ISSUES_LEN {
            return Err(StoreError::InvalidChapterBodyEdit(format!(
                "编辑反馈超过 {} 个字符",
                nekowriter_core::MAX_REVIEW_ISSUES_LEN
            )));
        }
        let target = format!("/api/chapters/{chapter_id}/body/proposals/{tool_call_id}/decision");
        let payload_hash = payload_hash(&DecideChapterBodyProposalPayload {
            expected_state_revision,
            feedback: &feedback,
            citations: &citations,
        })?;
        let command = Command::new(
            command_id,
            DECIDE_CHAPTER_BODY_PROPOSAL_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                let feedback = feedback.clone();
                let citations = citations.clone();
                Box::pin(async move {
                    let (chapter_model, body_document) = chapter_and_document(
                        transaction,
                        chapter_id,
                        CHAPTER_BODY_ROLE,
                        "章节正文",
                    )
                    .await?;
                    if chapter_model.status != "in_progress" {
                        return Err(StoreError::InvalidChapterState(
                            "章节尚未处于可编辑正文状态".to_owned(),
                        ));
                    }
                    let version_id: VersionId = chapter_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let run_id = workflow.active_run_id.ok_or_else(|| {
                        StoreError::InvalidChapterState("正文提案缺少活跃 Run".to_owned())
                    })?;
                    if workflow.active_chapter_id != Some(chapter_id)
                        || workflow.stage != WorkflowStage::WritingDrafting
                        || workflow.execution != ExecutionStatus::WaitingReview
                    {
                        return Err(StoreError::InvalidChapterState(
                            "当前工作流不在正文编辑提案审核状态".to_owned(),
                        ));
                    }
                    let run_model = chapter_body_run(transaction, run_id, chapter_id).await?;
                    if run_model.status != "waiting" {
                        return Err(StoreError::CorruptRecord(
                            "待审正文提案的 Run 未处于 waiting 状态".to_owned(),
                        ));
                    }
                    let proposal_call = tool_call::Entity::find_by_id(db_id(tool_call_id))
                        .filter(tool_call::Entity::COLUMN.run_id.eq(run_model.id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ProposalNotFound)?;
                    if proposal_call.status != "awaiting_review" {
                        return Err(StoreError::ProposalAlreadyResolved);
                    }
                    let proposal = pending_chapter_body_proposal_from_tool_call(&proposal_call)?;
                    let chapter_edit_review = review::Entity::find()
                        .filter(review::Entity::COLUMN.version_id.eq(db_id(version_id)))
                        .filter(review::Entity::COLUMN.review_type.eq("chapter_edit"))
                        .filter(
                            review::Entity::COLUMN
                                .target_tool_call_id
                                .eq(tool_call_id.to_string()),
                        )
                        .filter(review::Entity::COLUMN.status.eq("open"))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ReviewAlreadyResolved)?;
                    validate_chapter_body_feedback_citations(
                        transaction,
                        &citations,
                        version_id,
                        chapter_id,
                        run_id,
                        CandidateCitationScope::Proposal {
                            tool_call_id,
                        },
                    )
                    .await?;
                    let current_body = current_chapter_body(transaction, chapter_id).await?;
                    if current_body.revision_id != proposal.base_revision_id {
                        return Err(StoreError::ProposalBaseRevisionConflict {
                            expected: proposal.base_revision_id,
                            actual: current_body.revision_id,
                        });
                    }
                    let accepted = feedback.trim().is_empty() && citations.is_empty();
                    let (tool_result, committed_revision) = if accepted {
                        let dependency_revision_ids = chapter_body_revision_dependency_ids(
                            transaction,
                            version_id,
                            chapter_id,
                            proposal.base_revision_id,
                        )
                        .await?;
                        let (revision_id, revision_no) =
                            insert_document_revision_with_payload_and_source(
                                transaction,
                                body_document.id.into(),
                                &proposal.candidate_content,
                                None,
                                "agent",
                                proposal.base_revision_id,
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
                            &proposal.candidate_content,
                            body_document.id.into(),
                            revision_id,
                            chapter_id,
                            now,
                        )
                        .await?;
                        (
                            json!({
                                "status": "applied",
                                "tool_call_id": proposal.provider_call_id,
                                "base_revision_id": proposal.base_revision_id.map(|revision_id| revision_id.to_string()),
                                "result_revision_id": revision_id.to_string(),
                                "data": {
                                    "word_count": non_whitespace_char_count(&proposal.candidate_content).to_string(),
                                },
                                "feedback": [],
                            }),
                            Some((revision_id, revision_no)),
                        )
                    } else {
                        (
                            json!({
                                "status": "rejected",
                                "tool_call_id": proposal.provider_call_id,
                                "base_revision_id": proposal.base_revision_id.map(|revision_id| revision_id.to_string()),
                                "feedback": [{
                                    "text": feedback.clone(),
                                    "citations": citations.clone(),
                                }],
                            }),
                            None,
                        )
                    };
                    let request_id: Uuid = proposal_call.request_id.into();
                    let proposal_sequence = proposal_call.sequence;
                    let mut active_call = proposal_call.into_active_model();
                    // 调用生命周期到这里都已完成；“rejected”是返回给模型的业务
                    // 结果，而不是把 ToolCall 留在另一种未完成的数据库状态。
                    active_call.status = Set("resolved".to_owned());
                    active_call.result_json = Set(Some(serde_json::to_string(&tool_result)?));
                    active_call.resolved_at = Set(Some(now));
                    active_call.update(transaction).await?;
                    let bound_feedback_ids = bind_pending_feedback_to_tool(
                        transaction,
                        run_id,
                        tool_call_id,
                        proposal_sequence,
                        now,
                    )
                    .await?;
                    append_agent_messages_for_run(
                        transaction,
                        &run_model,
                        vec![NewAgentMessage {
                            role: AgentMessageRole::Tool,
                            content: json!({
                                "kind": "tool_result",
                                "content": serde_json::to_string(&tool_result)?,
                            }),
                            request_id: Some(request_id),
                            tool_call_id: Some(proposal.provider_call_id.clone()),
                        }],
                        now,
                    )
                    .await?;
                    let next_workflow =
                        continue_chapter_body_after_proposal(&workflow, chapter_id, run_id)?;
                    update_chapter_body_run_status(
                        transaction,
                        run_id,
                        chapter_id,
                        "queued",
                        None,
                        None,
                        now,
                    )
                    .await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    let chapter_edit_review_id: ReviewId = chapter_edit_review.id.into();
                    let mut active_review = chapter_edit_review.into_active_model();
                    active_review.status = Set(
                        if accepted { "approved" } else { "rejected" }.to_owned(),
                    );
                    active_review.decision_source = Set(Some("human".to_owned()));
                    active_review.passed = Set(Some(accepted));
                    active_review.confidence = Set(None);
                    active_review.issues_json = Set(
                        (!feedback.is_empty())
                            .then(|| serde_json::to_string(&feedback))
                            .transpose()?,
                    );
                    active_review.user_input_json = Set(Some(serde_json::to_string(&json!({
                        "source": "user",
                        "feedback": feedback,
                        "citations": citations,
                    }))?));
                    active_review.updated_at = Set(now);
                    active_review.update(transaction).await?;
                    if let Some((revision_id, revision_no)) = committed_revision {
                        insert_event(
                            transaction,
                            version_id,
                            "document.committed",
                            revision_id,
                            revision_no,
                            json!({
                                "chapter_id": chapter_id,
                                "role": CHAPTER_BODY_ROLE,
                                "word_count": non_whitespace_char_count(&proposal.candidate_content),
                                "source": "agent_proposal",
                            }),
                            now,
                        )
                        .await?;
                    }
                    insert_event(
                        transaction,
                        version_id,
                        "tool.resolved",
                        tool_call_id,
                        next_workflow.state_revision,
                        json!({
                            "chapter_id": chapter_id,
                            "name": proposal.tool_name,
                            "provider_call_id": proposal.provider_call_id,
                            "status": tool_result["status"],
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "review.changed",
                        chapter_edit_review_id,
                        next_workflow.state_revision,
                        json!({
                            "status": if accepted { "approved" } else { "rejected" },
                            "source": "human",
                            "tool_call_id": tool_call_id,
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
                                "chapter_id": chapter_id,
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
                            "chapter_id": chapter_id,
                            "kind": "chapter_body",
                            "status": "queued",
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
                            "chapter_id": chapter_id,
                            "run_id": run_id,
                        }),
                        now,
                    )
                    .await?;
                    let response = chapter_body_snapshot(transaction, chapter_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        DECIDE_CHAPTER_BODY_PROPOSAL_METHOD,
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
