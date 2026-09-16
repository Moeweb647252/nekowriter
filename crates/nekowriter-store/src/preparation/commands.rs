//! 准备与细纲命令：快照读取、小说梗概、准备阶段启动与审核、章节细纲与指导。
//!
//! 从 `preparation/mod.rs` 的这一层原样搬出（`docs/08` 的 P3.7）：方法体逐字不变，本层自用的
//! `*_METHOD` 常量与私有 payload DTO 随命令一起搬出。这些名字都是 `Store` 的固有方法，
//! `mod.rs` 不必再导出任何东西：兄弟模块与 app 侧原本就按方法语法调用，一行都不用改。

use nekowriter_core::{
    ChapterId, ChapterOutlineSnapshot, ChapterPlanDraft, CurrentChapterGuidance, CurrentNovelBrief,
    ExecutionStatus, FeedbackCitation, HumanReviewDecision, NewChapterGuidance, NewNovelBrief,
    PreparationArtifactInput, PreparationReviewCompletion, PreparationSnapshot, ReviewId,
    ReviewStatus, WorkflowSnapshot, WorkflowStage, begin_chapter_outline, begin_preparation,
    finish_preparation_review, non_whitespace_char_count, record_chapter_guidance,
    record_preparation_review_decision,
};
use sea_orm::{ActiveModelTrait, IntoActiveModel, Set, TransactionTrait};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    Command, NOVEL_BRIEF_ROLE, Notify, Store, StoreError, VersionId, insert_receipt, payload_hash,
    transaction_error, utc_now_millis,
};

use super::{
    CompletedRunContextFold, DECIDE_PREPARATION_REVIEW_METHOD, DecidePreparationReviewPayload,
    RunCheckpoint, build_preparation_context_fold, chapter_and_guidance_document,
    chapter_outline_snapshot, completed_run_context_folds, current_chapter_guidance,
    current_novel_brief, current_preparation_artifacts, current_preparation_is_approved,
    ensure_expected_state_revision, ensure_no_active_top_level_write_run,
    insert_chapter_outline_run, insert_document_revision, insert_document_revision_with_payload,
    insert_event, insert_preparation_run, materialize_chapter_plan, persist_human_review_decision,
    persist_preparation_submission, persist_workflow, preparation_review_target,
    preparation_snapshot, project_id_for_version, update_preparation_run_status,
    validate_review_decision_citations, validate_review_decision_feedback, workflow_for_version,
};

const SAVE_NOVEL_BRIEF_METHOD: &str = "PUT";
const START_PREPARATION_METHOD: &str = "POST";
const SUBMIT_PREPARATION_ARTIFACTS_METHOD: &str = "POST";
const SAVE_CHAPTER_GUIDANCE_METHOD: &str = "PUT";
const START_CHAPTER_OUTLINE_METHOD: &str = "POST";

#[derive(Serialize)]
struct SaveNovelBriefPayload<'a> {
    content: &'a str,
}

#[derive(Serialize)]
struct StartPreparationPayload {
    expected_state_revision: i64,
}

#[derive(Serialize)]
struct SubmitPreparationArtifactsPayload<'a> {
    expected_state_revision: i64,
    artifacts: &'a [PreparationArtifactInput],
    chapter_plan: Option<&'a [ChapterPlanDraft]>,
}

#[derive(Serialize)]
struct SaveChapterGuidancePayload<'a> {
    expected_state_revision: i64,
    content: &'a str,
}

#[derive(Serialize)]
struct StartChapterOutlinePayload {
    expected_state_revision: i64,
}

impl Store {
    pub async fn get_preparation_snapshot(
        &self,
        version_id: VersionId,
    ) -> Result<PreparationSnapshot, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move { preparation_snapshot(transaction, version_id).await })
            })
            .await
            .map_err(transaction_error)
    }

    /// 读取当前稿本所有已通过阶段的折叠来源，供 App 在构建下一模型请求时按固定
    /// 顺序注入。原始消息仍保存在 `agent_messages`，不会因为此查询而被删除。
    pub async fn list_completed_run_context_folds(
        &self,
        version_id: VersionId,
    ) -> Result<Vec<CompletedRunContextFold>, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move { completed_run_context_folds(transaction, version_id).await })
            })
            .await
            .map_err(transaction_error)
    }

    pub async fn save_novel_brief(
        &self,
        version_id: VersionId,
        command_id: Uuid,
        input: NewNovelBrief,
    ) -> Result<CurrentNovelBrief, StoreError> {
        let target = format!("/api/versions/{version_id}/brief");
        let payload_hash = payload_hash(&SaveNovelBriefPayload {
            content: &input.content,
        })?;
        let command = Command::new(command_id, SAVE_NOVEL_BRIEF_METHOD, &target, &payload_hash);

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    if workflow.stage != WorkflowStage::Initializing {
                        return Err(StoreError::InvalidPreparationState(
                            "小说目标只能在准备阶段开始前修改".to_owned(),
                        ));
                    }
                    let brief = current_novel_brief(transaction, version_id).await?;
                    let now = utc_now_millis();
                    let (revision_id, revision_no) = insert_document_revision(
                        transaction,
                        brief.document_id,
                        &input.content,
                        "user",
                        now,
                    )
                    .await?;
                    let response = CurrentNovelBrief {
                        document_id: brief.document_id,
                        revision_id: Some(revision_id),
                        content: Some(input.content),
                    };
                    insert_event(
                        transaction,
                        version_id,
                        "document.committed",
                        revision_id,
                        revision_no,
                        json!({ "role": NOVEL_BRIEF_ROLE, "word_count": non_whitespace_char_count(response.content.as_deref().unwrap_or_default()) }),
                        now,
                    )
                    .await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SAVE_NOVEL_BRIEF_METHOD,
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

    pub async fn start_preparation(
        &self,
        version_id: VersionId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<PreparationSnapshot, StoreError> {
        let target = format!("/api/versions/{version_id}/preparation/start");
        let payload_hash = payload_hash(&StartPreparationPayload {
            expected_state_revision,
        })?;
        let command = Command::new(command_id, START_PREPARATION_METHOD, &target, &payload_hash);

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.stage != WorkflowStage::Initializing {
                        return Err(StoreError::InvalidPreparationState(
                            "准备阶段已经开始或稿本不再处于初始化状态".to_owned(),
                        ));
                    }
                    let brief = current_novel_brief(transaction, version_id).await?;
                    if brief
                        .content
                        .as_deref()
                        .is_none_or(|content| content.trim().is_empty())
                    {
                        return Err(StoreError::NovelBriefRequired);
                    }

                    let now = utc_now_millis();
                    let run_id = Uuid::new_v4();
                    let project_id = project_id_for_version(transaction, version_id).await?;
                    ensure_no_active_top_level_write_run(transaction, project_id).await?;
                    insert_preparation_run(transaction, run_id, project_id, version_id, now)
                        .await?;
                    let next_workflow = begin_preparation(&workflow, run_id)?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({ "kind": "preparation", "status": "queued" }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({ "stage": next_workflow.stage, "run_id": run_id }),
                        now,
                    )
                    .await?;
                    let response = preparation_snapshot(transaction, version_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        START_PREPARATION_METHOD,
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

    pub async fn submit_preparation_artifacts(
        &self,
        version_id: VersionId,
        command_id: Uuid,
        expected_state_revision: i64,
        artifacts: Vec<PreparationArtifactInput>,
        chapter_plan: Option<Vec<ChapterPlanDraft>>,
    ) -> Result<PreparationSnapshot, StoreError> {
        let target = format!("/api/versions/{version_id}/preparation/artifacts");
        let payload_hash = payload_hash(&SubmitPreparationArtifactsPayload {
            expected_state_revision,
            artifacts: &artifacts,
            chapter_plan: chapter_plan.as_deref(),
        })?;
        let command = Command::new(
            command_id,
            SUBMIT_PREPARATION_ARTIFACTS_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.execution == ExecutionStatus::RequestInFlight {
                        return Err(StoreError::ResourceBusy(
                            "模型正在生成准备产物，不能同时手动提交".to_owned(),
                        ));
                    }
                    let now = utc_now_millis();
                    let next_workflow = persist_preparation_submission(
                        transaction,
                        version_id,
                        &workflow,
                        &artifacts,
                        chapter_plan.as_deref(),
                        "user",
                        now,
                    )
                    .await?;
                    let active_run_id = next_workflow.active_run_id.ok_or_else(|| {
                        StoreError::CorruptRecord("准备阶段缺少活跃 Run".to_owned())
                    })?;
                    update_preparation_run_status(
                        transaction,
                        active_run_id,
                        "waiting",
                        None,
                        None,
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        active_run_id,
                        next_workflow.state_revision,
                        json!({ "kind": "preparation", "status": "waiting" }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({ "stage": next_workflow.stage }),
                        now,
                    )
                    .await?;

                    let response = preparation_snapshot(transaction, version_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SUBMIT_PREPARATION_ARTIFACTS_METHOD,
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

    pub async fn decide_preparation_review(
        &self,
        review_id: ReviewId,
        command_id: Uuid,
        expected_state_revision: i64,
        decision: HumanReviewDecision,
        citations: Vec<FeedbackCitation>,
    ) -> Result<PreparationSnapshot, StoreError> {
        let target = format!("/api/reviews/{review_id}/decision");
        let payload_hash = payload_hash(&DecidePreparationReviewPayload {
            expected_state_revision,
            decision: &decision,
            citations: &citations,
        })?;
        let command = Command::new(
            command_id,
            DECIDE_PREPARATION_REVIEW_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let review_target = preparation_review_target(transaction, review_id).await?;
                    if review_target.status != ReviewStatus::Open {
                        return Err(StoreError::ReviewAlreadyResolved);
                    }
                    let workflow =
                        workflow_for_version(transaction, review_target.version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.stage != WorkflowStage::PreparingReviewing {
                        return Err(StoreError::InvalidPreparationState(
                            "只有准备审核中状态可以作出准备产物审核决定".to_owned(),
                        ));
                    }
                    if review_target.current_revision_id != review_target.target_revision_id {
                        return Err(StoreError::InvalidPreparationState(
                            "审核目标已不是当前修订，不能对旧修订作出决定".to_owned(),
                        ));
                    }
                    // 准备阶段审核可以引用本稿本任意已持久化文档。
                    validate_review_decision_citations(
                        transaction,
                        &citations,
                        review_target.version_id,
                        None,
                    )
                    .await?;
                    validate_review_decision_feedback(&decision, &citations)?;

                    let now = utc_now_millis();
                    persist_human_review_decision(
                        transaction,
                        review_id,
                        &decision,
                        &citations,
                        now,
                    )
                    .await?;
                    let current_artifacts =
                        current_preparation_artifacts(transaction, review_target.version_id)
                            .await?;
                    let has_open_reviews = current_artifacts.iter().any(|artifact| {
                        matches!(
                            artifact.review.as_ref().map(|review| &review.status),
                            Some(ReviewStatus::Open | ReviewStatus::Delegated)
                        )
                    });

                    let (next_workflow, run_status) = if has_open_reviews {
                        (record_preparation_review_decision(&workflow)?, None)
                    } else if current_preparation_is_approved(transaction, &current_artifacts)
                        .await?
                    {
                        let active_run_id = workflow.active_run_id.ok_or_else(|| {
                            StoreError::CorruptRecord("准备审核缺少对应的活跃 Run".to_owned())
                        })?;
                        let context_fold = build_preparation_context_fold(
                            transaction,
                            active_run_id,
                            &current_artifacts,
                        )
                        .await?;
                        let first_chapter_id = materialize_chapter_plan(
                            transaction,
                            review_target.version_id,
                            &current_artifacts,
                            now,
                        )
                        .await?;
                        let checkpoint = RunCheckpoint {
                            chapter_id: None,
                            context_fold: Some(context_fold),
                            held_chapter_body_submission: None,
                        };
                        update_preparation_run_status(
                            transaction,
                            active_run_id,
                            "succeeded",
                            Some(&checkpoint),
                            Some(now),
                            now,
                        )
                        .await?;
                        let mut next_workflow = finish_preparation_review(
                            &workflow,
                            PreparationReviewCompletion::AllApproved,
                        )?;
                        next_workflow.active_chapter_id = Some(first_chapter_id);
                        (next_workflow, Some((active_run_id, "succeeded")))
                    } else {
                        let active_run_id = workflow.active_run_id.ok_or_else(|| {
                            StoreError::CorruptRecord("准备审核缺少对应的活跃 Run".to_owned())
                        })?;
                        update_preparation_run_status(
                            transaction,
                            active_run_id,
                            "queued",
                            None,
                            None,
                            now,
                        )
                        .await?;
                        (
                            finish_preparation_review(
                                &workflow,
                                PreparationReviewCompletion::RevisionsRequired,
                            )?,
                            Some((active_run_id, "queued")),
                        )
                    };
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        review_target.version_id,
                        "review.changed",
                        review_id,
                        next_workflow.state_revision,
                        json!({
                            "role": review_target.kind.stable_key(),
                            "passed": decision.passed,
                            "status": if decision.passed { "approved" } else { "rejected" },
                        }),
                        now,
                    )
                    .await?;
                    if let Some((run_id, status)) = run_status {
                        insert_event(
                            transaction,
                            review_target.version_id,
                            "run.changed",
                            run_id,
                            next_workflow.state_revision,
                            json!({ "kind": "preparation", "status": status }),
                            now,
                        )
                        .await?;
                    }
                    insert_event(
                        transaction,
                        review_target.version_id,
                        "workflow.changed",
                        review_target.version_id,
                        next_workflow.state_revision,
                        json!({ "stage": next_workflow.stage }),
                        now,
                    )
                    .await?;

                    let response =
                        preparation_snapshot(transaction, review_target.version_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        DECIDE_PREPARATION_REVIEW_METHOD,
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

    pub async fn get_chapter_guidance(
        &self,
        chapter_id: ChapterId,
    ) -> Result<CurrentChapterGuidance, StoreError> {
        current_chapter_guidance(&self.database, chapter_id).await
    }

    pub async fn save_chapter_guidance(
        &self,
        chapter_id: ChapterId,
        command_id: Uuid,
        expected_state_revision: i64,
        input: NewChapterGuidance,
    ) -> Result<CurrentChapterGuidance, StoreError> {
        let target = format!("/api/chapters/{chapter_id}/guidance");
        let payload_hash = payload_hash(&SaveChapterGuidancePayload {
            expected_state_revision,
            content: &input.content,
        })?;
        let command = Command::new(
            command_id,
            SAVE_CHAPTER_GUIDANCE_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let (chapter_model, guidance_document) =
                        chapter_and_guidance_document(transaction, chapter_id).await?;
                    let version_id: VersionId = chapter_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let next_workflow = record_chapter_guidance(&workflow, chapter_id)?;
                    let now = utc_now_millis();
                    let (revision_id, revision_no) = insert_document_revision_with_payload(
                        transaction,
                        guidance_document.id.into(),
                        &input.content,
                        None,
                        "user",
                        now,
                    )
                    .await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "document.committed",
                        revision_id,
                        revision_no,
                        json!({
                            "chapter_id": chapter_id,
                            "role": "chapter_guidance",
                            "word_count": non_whitespace_char_count(&input.content),
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
                            "chapter_id": chapter_id,
                        }),
                        now,
                    )
                    .await?;
                    let response = CurrentChapterGuidance {
                        document_id: guidance_document.id.into(),
                        revision_id: Some(revision_id),
                        revision_no: Some(revision_no),
                        content: Some(input.content),
                    };
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SAVE_CHAPTER_GUIDANCE_METHOD,
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

    pub async fn start_chapter_outline(
        &self,
        chapter_id: ChapterId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<WorkflowSnapshot, StoreError> {
        let target = format!("/api/chapters/{chapter_id}/outline/start");
        let payload_hash = payload_hash(&StartChapterOutlinePayload {
            expected_state_revision,
        })?;
        let command = Command::new(
            command_id,
            START_CHAPTER_OUTLINE_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let (chapter_model, guidance_document) =
                        chapter_and_guidance_document(transaction, chapter_id).await?;
                    if chapter_model.status != "planned" {
                        return Err(StoreError::InvalidChapterState(
                            "章节已启动或不再可开始细纲".to_owned(),
                        ));
                    }
                    let version_id: VersionId = chapter_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let run_id = Uuid::new_v4();
                    let next_workflow = begin_chapter_outline(&workflow, chapter_id, run_id)?;
                    let now = utc_now_millis();
                    let empty_guidance_revision = if guidance_document.current_revision_id.is_none()
                    {
                        Some(
                            insert_document_revision_with_payload(
                                transaction,
                                guidance_document.id.into(),
                                "",
                                None,
                                "user",
                                now,
                            )
                            .await?,
                        )
                    } else {
                        None
                    };
                    let project_id = project_id_for_version(transaction, version_id).await?;
                    ensure_no_active_top_level_write_run(transaction, project_id).await?;
                    insert_chapter_outline_run(
                        transaction,
                        run_id,
                        project_id,
                        version_id,
                        chapter_id,
                        now,
                    )
                    .await?;
                    let chapter_next_revision = chapter_model.revision + 1;
                    let mut active_chapter = chapter_model.into_active_model();
                    active_chapter.status = Set("in_progress".to_owned());
                    active_chapter.revision = Set(chapter_next_revision);
                    active_chapter.updated_at = Set(now);
                    active_chapter.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    if let Some((revision_id, revision_no)) = empty_guidance_revision {
                        insert_event(
                            transaction,
                            version_id,
                            "document.committed",
                            revision_id,
                            revision_no,
                            json!({
                                "chapter_id": chapter_id,
                                "role": "chapter_guidance",
                                "word_count": 0,
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
                            "kind": "chapter_outline",
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
                            "chapter_id": chapter_id,
                            "run_id": run_id,
                        }),
                        now,
                    )
                    .await?;
                    let response_json = serde_json::to_string(&next_workflow)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        START_CHAPTER_OUTLINE_METHOD,
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

    /// 读取当前章节的细纲文档、审核状态和已通过时的阶段折叠。
    pub async fn get_chapter_outline(
        &self,
        chapter_id: ChapterId,
    ) -> Result<ChapterOutlineSnapshot, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move { chapter_outline_snapshot(transaction, chapter_id).await })
            })
            .await
            .map_err(transaction_error)
    }
}
