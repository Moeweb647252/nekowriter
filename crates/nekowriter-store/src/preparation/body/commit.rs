//! 正文终稿提交：把正文 Agent 的阶段提交、模型请求终态与整章终稿审核原子落库。
//!
//! 从 `preparation/mod.rs` 的这一层原样搬出（`docs/08` 的 P3.10）：函数体逐字不变。两个协议纠正
//! 常量仍留在 `preparation/mod.rs`（`tools::held.rs` 也在用），本模块按 `super::super::…` 引用；
//! 它是 `Store` 的固有方法，调用方按方法语法调用，不需要任何再导出。

use nekowriter_core::{
    ChapterBodySnapshot, ChapterId, ExecutionStatus, HeldChapterBodySubmission, RunId,
    WorkflowStage, continue_chapter_body_after_protocol_correction,
    hold_chapter_body_submission_for_pause, submit_chapter_for_review,
};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, QueryFilter, Set, TransactionTrait};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::agent_messages::{append_agent_messages_for_run, phase_submission_messages};
use crate::entity::{feedback, model_request, run};
use crate::event_log::insert_event;
use crate::model_requests::protocol_error_tool_result_messages;
use crate::{
    AgentMessageRole, CHAPTER_BODY_ROLE, NewAgentMessage, Store, StoreError, VersionId, db_id,
    transaction_error, utc_now_millis,
};

// 下面这些名字都由 `preparation/mod.rs` 再导出（含它自己的项与 `tools::held` 的再导出），
// 按 `super::super::…` 引用就能同时让那些再导出行与还在原地的常量继续有消费者。
use super::super::{
    CHAPTER_BODY_FEEDBACK_PROTOCOL_CORRECTION, HeldChapterBodySubmissionCheckpoint, RunCheckpoint,
    chapter_and_document, chapter_body_outline_dependency_revision_id, chapter_body_run,
    chapter_body_snapshot, current_chapter_body, insert_chapter_final_review, persist_workflow,
    revision_dependency_ids, workflow_for_version,
};

impl Store {
    /// 将正文 Agent 的阶段提交、模型请求终态和整章终稿审核原子落库。
    ///
    /// 正文内容不从 `summary` 读取；它只能来自此前已经解析并执行的正文工具调用。
    /// 因此模型无法通过提交工具绕过正文修订、依赖或整章审核门槛。
    pub async fn commit_chapter_body_model_submission(
        &self,
        request_id: Uuid,
        chapter_id: ChapterId,
        provider_request_id: Option<String>,
        usage: Value,
        assistant_message: NewAgentMessage,
        summary: String,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        // 提交是一次工具调用：成功路径写配对的应用结果，被拒路径写配对的错误结果。
        let submission_messages = phase_submission_messages(request_id, assistant_message.clone())?;
        if !usage.is_object() {
            return Err(StoreError::InvalidModelRequest(
                "章节正文模型响应的 usage 必须是 JSON object".to_owned(),
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
        nekowriter_core::validate_document_content(
            "章节摘要",
            &summary,
            nekowriter_core::MAX_ARTIFACT_CONTENT_LEN,
        )
        .map_err(|error| StoreError::InvalidModelRequest(error.to_string()))?;

        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                let provider_request_id = provider_request_id.clone();
                let usage = usage.clone();
                let assistant_message = assistant_message.clone();
                let summary = summary.clone();
                Box::pin(async move {
                    let request = model_request::Entity::find_by_id(db_id(request_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ModelRequestNotFound)?;
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

                    if request.status == "succeeded" {
                        if workflow.active_run_id == Some(run_id)
                            && workflow.active_chapter_id == Some(chapter_id)
                            && ((workflow.stage == WorkflowStage::WritingChapterReviewing
                                && workflow.execution == ExecutionStatus::WaitingReview)
                                || (workflow.stage == WorkflowStage::WritingDrafting
                                    && workflow.execution == ExecutionStatus::Paused
                                    && workflow.pause_requested))
                        {
                            return Ok((
                                chapter_body_snapshot(transaction, chapter_id).await?,
                                false,
                            ));
                        }
                        return Err(StoreError::InvalidModelRequestState(
                            "已成功的正文请求不对应当前整章审核状态".to_owned(),
                        ));
                    }
                    if request.status != "dispatched" {
                        return Err(StoreError::InvalidModelRequestState(format!(
                            "只有 dispatched 请求可以提交章节正文，当前为 {}",
                            request.status
                        )));
                    }
                    if workflow.active_run_id != Some(run_id)
                        || workflow.active_chapter_id != Some(chapter_id)
                        || workflow.stage != WorkflowStage::WritingDrafting
                        || workflow.execution != ExecutionStatus::RequestInFlight
                    {
                        return Err(StoreError::InvalidModelRequestState(
                            "章节正文工作流已不接受此模型提交".to_owned(),
                        ));
                    }

                    let (chapter_model, _) = chapter_and_document(
                        transaction,
                        chapter_id,
                        CHAPTER_BODY_ROLE,
                        "章节正文",
                    )
                    .await?;
                    let chapter_version_id: VersionId = chapter_model.version_id.into();
                    if chapter_model.status != "in_progress" || chapter_version_id != version_id {
                        return Err(StoreError::InvalidChapterState(
                            "当前章节不属于可提交正文的活跃稿本".to_owned(),
                        ));
                    }
                    let body_run = chapter_body_run(transaction, run_id, chapter_id).await?;
                    if workflow.pause_requested {
                        let mut checkpoint: RunCheckpoint =
                            serde_json::from_str(&body_run.checkpoint_json)?;
                        if checkpoint.chapter_id != Some(chapter_id)
                            || checkpoint.held_chapter_body_submission.is_some()
                        {
                            return Err(StoreError::CorruptRecord(
                                "正文暂停提交的 Run 检查点无效".to_owned(),
                            ));
                        }
                        let next_workflow =
                            hold_chapter_body_submission_for_pause(&workflow, chapter_id, run_id)?;
                        checkpoint.held_chapter_body_submission =
                            Some(HeldChapterBodySubmissionCheckpoint {
                                held: HeldChapterBodySubmission {
                                    request_id,
                                    summary: summary.clone(),
                                },
                                assistant_message: assistant_message.clone(),
                            });
                        let mut active_request = request.into_active_model();
                        active_request.status = Set("succeeded".to_owned());
                        active_request.provider_request_id = Set(provider_request_id);
                        active_request.usage_json = Set(Some(serde_json::to_string(&usage)?));
                        active_request.error_json = Set(None);
                        active_request.updated_at = Set(now);
                        active_request.completed_at = Set(Some(now));
                        active_request.update(transaction).await?;
                        let request_count = body_run.request_count;
                        let mut active_run = body_run.into_active_model();
                        active_run.status = Set("paused".to_owned());
                        active_run.consecutive_protocol_errors = Set(0);
                        active_run.checkpoint_json = Set(serde_json::to_string(&checkpoint)?);
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
                                "held_submission": true,
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
                                "pause_requested": true,
                            }),
                            now,
                        )
                        .await?;
                        return Ok((chapter_body_snapshot(transaction, chapter_id).await?, true));
                    }
                    let has_pending_feedback = feedback::Entity::find()
                        .filter(feedback::Entity::COLUMN.run_id.eq(body_run.id))
                        .filter(feedback::Entity::COLUMN.status.is_in(["queued", "bound"]))
                        .one(transaction)
                        .await?
                        .is_some();
                    if has_pending_feedback {
                        // 这是业务守卫的拒绝，不是协议错误：§36.1 已把四条阶段提交路径统一成
                        // 「被拒回合」——立即重排、不推进 `consecutive_protocol_errors`、永不触发
                        // 上限，非进展回合的上限由请求额度承担。反例现场（2026-09-15 第二章正文
                        // Run）：两个纯文本回合已占 2/3 计数，这次守卫拒绝把计数推到 3，Run 当场
                        // 停下，模型刚收到的「请用 append_content / replace_content 投递意见」再没有
                        // 执行机会，未投递意见永远留在队列里。
                        let next_workflow = continue_chapter_body_after_protocol_correction(
                            &workflow, chapter_id, run_id,
                        )?;
                        let mut messages = vec![assistant_message.clone()];
                        messages.extend(protocol_error_tool_result_messages(
                            request_id,
                            &assistant_message,
                            CHAPTER_BODY_FEEDBACK_PROTOCOL_CORRECTION,
                            "PHASE_SUBMISSION_REJECTED",
                        )?);
                        messages.push(NewAgentMessage {
                            role: AgentMessageRole::User,
                            content: json!({
                                "kind": "protocol_correction",
                                "content": CHAPTER_BODY_FEEDBACK_PROTOCOL_CORRECTION,
                            }),
                            request_id: Some(request_id),
                            tool_call_id: None,
                        });
                        append_agent_messages_for_run(transaction, &body_run, messages, now)
                            .await?;
                        let mut active_request = request.into_active_model();
                        active_request.status = Set("succeeded".to_owned());
                        active_request.provider_request_id = Set(provider_request_id);
                        active_request.usage_json = Set(Some(serde_json::to_string(&usage)?));
                        active_request.error_json = Set(None);
                        active_request.updated_at = Set(now);
                        active_request.completed_at = Set(Some(now));
                        active_request.update(transaction).await?;
                        let request_count = body_run.request_count;
                        let mut active_run = body_run.into_active_model();
                        active_run.status = Set("queued".to_owned());
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
                                "status": "queued",
                                "request_id": request_id,
                                "request_count": request_count.to_string(),
                                "reason": "pending_feedback_requires_tool_result",
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
                        return Ok((chapter_body_snapshot(transaction, chapter_id).await?, true));
                    }
                    let body = current_chapter_body(transaction, chapter_id).await?;
                    let body_revision_id = body.revision_id.ok_or_else(|| {
                        StoreError::InvalidChapterBodyEdit("章节正文不能为空".to_owned())
                    })?;
                    let body_content = body.content.ok_or_else(|| {
                        StoreError::CorruptRecord("章节正文当前修订缺少内容".to_owned())
                    })?;
                    nekowriter_core::validate_document_content(
                        "章节正文",
                        &body_content,
                        nekowriter_core::MAX_ARTIFACT_CONTENT_LEN,
                    )
                    .map_err(|error| StoreError::InvalidChapterBodyEdit(error.to_string()))?;
                    let outline_revision_id = chapter_body_outline_dependency_revision_id(
                        transaction,
                        version_id,
                        chapter_id,
                    )
                    .await?;
                    let dependency_revision_ids =
                        revision_dependency_ids(transaction, body_revision_id).await?;
                    if !dependency_revision_ids.contains(&outline_revision_id) {
                        return Err(StoreError::InvalidChapterState(
                            "当前正文未依赖已通过的章节细纲".to_owned(),
                        ));
                    }

                    let next_workflow = submit_chapter_for_review(&workflow, chapter_id)?;
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
                        &run_model,
                        submission_messages.clone(),
                        now,
                    )
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
                    active_run.updated_at = Set(now);
                    active_run.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "review.changed",
                        review_id,
                        next_workflow.state_revision,
                        json!({
                            "chapter_id": chapter_id,
                            "role": CHAPTER_BODY_ROLE,
                            "status": "open",
                            "source": "agent_submission",
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
                            "chapter_id": chapter_id,
                            "kind": "chapter_body",
                            "status": "waiting",
                            "request_id": request_id,
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
                            "chapter_id": chapter_id,
                            "run_id": run_id,
                            "request_id": request_id,
                        }),
                        now,
                    )
                    .await?;
                    Ok((chapter_body_snapshot(transaction, chapter_id).await?, true))
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
}
