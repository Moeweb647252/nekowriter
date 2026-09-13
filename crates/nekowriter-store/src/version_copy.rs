//! 稿本切换与已通过内容复制。
//!
//! 复制不是导出/导入，也不复用源稿本的 Run。它只重建当前已通过内容的文档修订、
//! 依赖、审核依据和上下文折叠；新稿本随后可以从第一个尚未完成的计划章节继续。

use std::collections::{BTreeMap, HashMap};

use nekowriter_core::{
    ApprovedPreparationArtifact, ChapterPlan, ChapterPlanItem, ContextFold, CopiedContextFold,
    CopiedContextFoldStage, ExecutionStatus, NewCopiedNovelVersion, PreparationArtifactKind,
    ReviewStatus, RevisionId, VersionId, WorkflowSnapshot, WorkflowStage,
    non_whitespace_char_count, render_preparation_artifact_plaintext, validate_document_content,
};
use sea_orm::{
    ActiveModelTrait, DatabaseTransaction, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    Set, TransactionTrait,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    CHAPTER_BODY_ROLE, CHAPTER_GUIDANCE_ROLE, CHAPTER_OUTLINE_ROLE, NOVEL_BRIEF_ROLE, Store,
    StoreError, db_id,
    entity::{
        chapter, document, document_revision, novel_version, project, review, revision_dependency,
        run, tool_call,
    },
    insert_document_slots, insert_novel_version, insert_receipt, insert_workflow,
    parse_optional_id, payload_hash,
    preparation::{insert_chapter_document_slots, preparation_snapshot, workflow_for_version},
    project_from_model, replay_or_conflict, transaction_error,
};

const COPY_VERSION_METHOD: &str = "POST";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CopyVersionPayload<'a> {
    label: &'a str,
}

#[derive(Clone)]
struct CopiedRevision {
    source_revision_id: RevisionId,
    target_revision_id: RevisionId,
    content: String,
}

struct CopiedPreparationArtifact {
    copied_revision: CopiedRevision,
    source_review_id: Uuid,
}

struct CopiedChapter {
    order_index: i64,
    guidance: CopiedRevision,
    outline: CopiedRevision,
    body: CopiedRevision,
    source_outline_review_id: Uuid,
    source_body_review_id: Uuid,
}

impl Store {
    /// 复制源稿本当前已通过的内容，并选中新稿本。
    ///
    /// 源稿本必须没有活跃写作或开放审核。未通过的正文、番外、Run、工具调用、
    /// 自由意见与模型用量都不会进入新稿本。
    pub async fn copy_version(
        &self,
        source_version_id: VersionId,
        command_id: Uuid,
        input: NewCopiedNovelVersion,
    ) -> Result<nekowriter_core::ProjectBundle, StoreError> {
        let target = format!("/api/versions/{source_version_id}/copy");
        let payload_hash = payload_hash(&CopyVersionPayload {
            label: &input.label,
        })?;
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(receipt, COPY_VERSION_METHOD, &target, &payload_hash);
        }

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let source_version =
                        novel_version::Entity::find_by_id(db_id(source_version_id))
                            .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
                            .one(transaction)
                            .await?
                            .ok_or(StoreError::VersionNotFound)?;
                    let project_id: Uuid = source_version.project_id.into();
                    let project_model = project::Entity::find_by_id(source_version.project_id)
                        .filter(project::Entity::COLUMN.deleted_at.is_null())
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ProjectNotFound)?;
                    let source_workflow =
                        workflow_for_version(transaction, source_version_id).await?;
                    let source_snapshot =
                        preparation_snapshot(transaction, source_version_id).await?;
                    ensure_source_is_copyable(
                        transaction,
                        source_version_id,
                        &source_workflow,
                        &source_snapshot,
                    )
                    .await?;

                    let (target_plan, plan_item_id_map) = copied_chapter_plan(&source_snapshot)?;
                    let source_chapters = chapter::Entity::find()
                        .filter(
                            chapter::Entity::COLUMN
                                .version_id
                                .eq(db_id(source_version_id)),
                        )
                        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
                        .order_by_asc(chapter::Entity::COLUMN.order_index)
                        .order_by_asc(chapter::Entity::COLUMN.id)
                        .all(transaction)
                        .await?;
                    let now = nekowriter_core::utc_now_millis();
                    let target_version_id = Uuid::new_v4();
                    let target_version = nekowriter_core::NovelVersion {
                        id: target_version_id,
                        project_id,
                        label: input.label,
                        novel_title: source_version.novel_title.clone(),
                        source_version_id: Some(source_version_id),
                        body_completed_at_ms: source_version.body_completed_at,
                        revision: 1,
                        created_at_ms: now,
                        updated_at_ms: now,
                    };
                    insert_novel_version(transaction, &target_version).await?;
                    insert_document_slots(transaction, target_version_id, now).await?;

                    let mut source_main_by_plan_item = HashMap::new();
                    let mut source_extras = Vec::new();
                    for source_chapter in source_chapters {
                        match source_chapter.kind.as_str() {
                            "main" => {
                                let plan_item_id = parse_optional_id(
                                    source_chapter.plan_item_id.clone(),
                                    "chapter.plan_item_id",
                                )?
                                .ok_or_else(|| {
                                    StoreError::CorruptRecord("正文章节缺少计划项标识".to_owned())
                                })?;
                                if source_main_by_plan_item
                                    .insert(plan_item_id, source_chapter)
                                    .is_some()
                                {
                                    return Err(StoreError::CorruptRecord(
                                        "稿本存在重复的正文章节计划项".to_owned(),
                                    ));
                                }
                            }
                            "extra" => source_extras.push(source_chapter),
                            kind => {
                                return Err(StoreError::CorruptRecord(format!(
                                    "章节使用了未知类型：{kind}"
                                )));
                            }
                        }
                    }

                    let mut target_chapter_ids = HashMap::new();
                    let mut first_unfinished_main_chapter_id = None;
                    let mut last_approved_main_chapter_id = None;
                    let mut seen_unfinished_main = false;
                    for item in &target_plan.items {
                        let source_item_id = plan_item_id_map
                            .iter()
                            .find_map(|(source_id, target_id)| {
                                (*target_id == item.plan_item_id).then_some(*source_id)
                            })
                            .ok_or_else(|| {
                                StoreError::CorruptRecord("复制章节计划缺少源计划项映射".to_owned())
                            })?;
                        let source_chapter = source_main_by_plan_item
                            .get(&source_item_id)
                            .ok_or_else(|| {
                                StoreError::CorruptRecord(
                                    "已通过粗纲的计划项没有对应正文章节".to_owned(),
                                )
                            })?;
                        let source_chapter_id: Uuid = source_chapter.id.into();
                        let target_chapter_id = Uuid::new_v4();
                        let status = match source_chapter.status.as_str() {
                            "approved" if !seen_unfinished_main => {
                                last_approved_main_chapter_id = Some(target_chapter_id);
                                "approved"
                            }
                            "planned" | "in_progress" => {
                                seen_unfinished_main = true;
                                first_unfinished_main_chapter_id.get_or_insert(target_chapter_id);
                                "planned"
                            }
                            "approved" => {
                                return Err(StoreError::CorruptRecord(
                                    "源稿本越过未完成章节提前通过了后续正文章节".to_owned(),
                                ));
                            }
                            status => {
                                return Err(StoreError::CorruptRecord(format!(
                                    "正文章节使用了未知状态：{status}"
                                )));
                            }
                        };
                        chapter::ActiveModel {
                            id: Set(db_id(target_chapter_id)),
                            version_id: Set(db_id(target_version_id)),
                            kind: Set("main".to_owned()),
                            plan_item_id: Set(Some(item.plan_item_id.to_string())),
                            order_index: Set(item.order_index),
                            title: Set(item.title.clone()),
                            status: Set(status.to_owned()),
                            approved_revision_id: Set(None),
                            revision: Set(0),
                            created_at: Set(now),
                            updated_at: Set(now),
                            deleted_at: Set(None),
                        }
                        .insert(transaction)
                        .await?;
                        insert_chapter_document_slots(
                            transaction,
                            target_version_id,
                            target_chapter_id,
                            now,
                        )
                        .await?;
                        target_chapter_ids.insert(source_chapter_id, target_chapter_id);
                    }
                    if source_main_by_plan_item.len() != target_plan.items.len() {
                        return Err(StoreError::CorruptRecord(
                            "源稿本存在不属于当前粗纲的正文章节".to_owned(),
                        ));
                    }

                    let mut seen_unfinished_extra = false;
                    for source_extra in source_extras {
                        let source_chapter_id: Uuid = source_extra.id.into();
                        match source_extra.status.as_str() {
                            "approved" if !seen_unfinished_extra => {
                                let target_chapter_id = Uuid::new_v4();
                                chapter::ActiveModel {
                                    id: Set(db_id(target_chapter_id)),
                                    version_id: Set(db_id(target_version_id)),
                                    kind: Set("extra".to_owned()),
                                    plan_item_id: Set(None),
                                    order_index: Set(source_extra.order_index),
                                    title: Set(source_extra.title.clone()),
                                    status: Set("approved".to_owned()),
                                    approved_revision_id: Set(None),
                                    revision: Set(0),
                                    created_at: Set(now),
                                    updated_at: Set(now),
                                    deleted_at: Set(None),
                                }
                                .insert(transaction)
                                .await?;
                                insert_chapter_document_slots(
                                    transaction,
                                    target_version_id,
                                    target_chapter_id,
                                    now,
                                )
                                .await?;
                                target_chapter_ids.insert(source_chapter_id, target_chapter_id);
                            }
                            "planned" | "in_progress" => seen_unfinished_extra = true,
                            "approved" => {
                                return Err(StoreError::CorruptRecord(
                                    "源稿本越过未完成番外提前通过了后续番外".to_owned(),
                                ));
                            }
                            status => {
                                return Err(StoreError::CorruptRecord(format!(
                                    "番外使用了未知状态：{status}"
                                )));
                            }
                        }
                    }

                    let source_brief_document =
                        active_version_document(transaction, source_version_id, NOVEL_BRIEF_ROLE)
                            .await?;
                    let target_brief_document =
                        active_version_document(transaction, target_version_id, NOVEL_BRIEF_ROLE)
                            .await?;
                    let copied_brief = copy_document_head(
                        transaction,
                        &source_brief_document,
                        &target_brief_document,
                        None,
                        now,
                        "小说目标",
                    )
                    .await?;
                    if source_snapshot.novel_brief.revision_id
                        != Some(copied_brief.source_revision_id)
                    {
                        return Err(StoreError::CorruptRecord(
                            "小说目标当前修订与复制来源不一致".to_owned(),
                        ));
                    }

                    let mut copied_artifacts = BTreeMap::new();
                    for kind in PreparationArtifactKind::ALL {
                        let source_artifact = source_snapshot
                            .artifacts
                            .iter()
                            .find(|artifact| artifact.kind == kind)
                            .ok_or_else(|| {
                                StoreError::CorruptRecord(format!(
                                    "复制时缺少 {} 产物",
                                    kind.stable_key()
                                ))
                            })?;
                        let source_document =
                            document::Entity::find_by_id(db_id(source_artifact.document_id))
                                .filter(
                                    document::Entity::COLUMN
                                        .version_id
                                        .eq(db_id(source_version_id)),
                                )
                                .filter(document::Entity::COLUMN.role.eq(kind.stable_key()))
                                .one(transaction)
                                .await?
                                .ok_or_else(|| {
                                    StoreError::CorruptRecord(format!(
                                        "复制时缺少 {} 源文档",
                                        kind.stable_key()
                                    ))
                                })?;
                        let target_document = active_version_document(
                            transaction,
                            target_version_id,
                            kind.stable_key(),
                        )
                        .await?;
                        let structured_payload = if kind == PreparationArtifactKind::Outline {
                            Some(serde_json::to_string(&target_plan)?)
                        } else {
                            None
                        };
                        let copied_revision = copy_document_head(
                            transaction,
                            &source_document,
                            &target_document,
                            structured_payload,
                            now,
                            kind.label(),
                        )
                        .await?;
                        if source_artifact.revision_id != Some(copied_revision.source_revision_id) {
                            return Err(StoreError::CorruptRecord(format!(
                                "{} 当前修订与复制来源不一致",
                                kind.label()
                            )));
                        }
                        let source_review_id = source_artifact
                            .review
                            .as_ref()
                            .map(|review| review.id)
                            .ok_or_else(|| {
                                StoreError::CorruptRecord(format!("{} 缺少通过审核", kind.label()))
                            })?;
                        copied_artifacts.insert(
                            kind,
                            CopiedPreparationArtifact {
                                copied_revision,
                                source_review_id,
                            },
                        );
                    }

                    let mut copied_chapters = Vec::new();
                    for (source_chapter_id, target_chapter_id) in &target_chapter_ids {
                        let source_chapter = chapter::Entity::find_by_id(db_id(*source_chapter_id))
                            .one(transaction)
                            .await?
                            .ok_or(StoreError::ChapterNotFound)?;
                        if source_chapter.status != "approved" {
                            continue;
                        }
                        let source_guidance = active_chapter_document(
                            transaction,
                            *source_chapter_id,
                            CHAPTER_GUIDANCE_ROLE,
                        )
                        .await?;
                        let source_outline = active_chapter_document(
                            transaction,
                            *source_chapter_id,
                            CHAPTER_OUTLINE_ROLE,
                        )
                        .await?;
                        let source_body = active_chapter_document(
                            transaction,
                            *source_chapter_id,
                            CHAPTER_BODY_ROLE,
                        )
                        .await?;
                        let target_guidance = active_chapter_document(
                            transaction,
                            *target_chapter_id,
                            CHAPTER_GUIDANCE_ROLE,
                        )
                        .await?;
                        let target_outline = active_chapter_document(
                            transaction,
                            *target_chapter_id,
                            CHAPTER_OUTLINE_ROLE,
                        )
                        .await?;
                        let target_body = active_chapter_document(
                            transaction,
                            *target_chapter_id,
                            CHAPTER_BODY_ROLE,
                        )
                        .await?;
                        let guidance = copy_document_head(
                            transaction,
                            &source_guidance,
                            &target_guidance,
                            None,
                            now,
                            "章节指导",
                        )
                        .await?;
                        let outline = copy_document_head(
                            transaction,
                            &source_outline,
                            &target_outline,
                            None,
                            now,
                            "章节细纲",
                        )
                        .await?;
                        let body = copy_document_head(
                            transaction,
                            &source_body,
                            &target_body,
                            None,
                            now,
                            "章节正文",
                        )
                        .await?;
                        if parse_optional_id(
                            source_chapter.approved_revision_id.clone(),
                            "chapter.approved_revision_id",
                        )? != Some(body.source_revision_id)
                        {
                            return Err(StoreError::CorruptRecord(
                                "已通过章节的当前正文与通过修订不一致".to_owned(),
                            ));
                        }
                        let source_outline_review = approved_review_for_revision(
                            transaction,
                            source_version_id,
                            "chapter_outline",
                            outline.source_revision_id,
                        )
                        .await?;
                        let source_body_review = approved_review_for_revision(
                            transaction,
                            source_version_id,
                            "chapter_final",
                            body.source_revision_id,
                        )
                        .await?;
                        let mut target_chapter =
                            chapter::Entity::find_by_id(db_id(*target_chapter_id))
                                .one(transaction)
                                .await?
                                .ok_or(StoreError::ChapterNotFound)?
                                .into_active_model();
                        target_chapter.status = Set("approved".to_owned());
                        target_chapter.approved_revision_id =
                            Set(Some(body.target_revision_id.to_string()));
                        target_chapter.revision = Set(1);
                        target_chapter.updated_at = Set(now);
                        target_chapter.update(transaction).await?;
                        copied_chapters.push(CopiedChapter {
                            order_index: source_chapter.order_index,
                            guidance,
                            outline,
                            body,
                            source_outline_review_id: source_outline_review.id.into(),
                            source_body_review_id: source_body_review.id.into(),
                        });
                    }

                    for kind in PreparationArtifactKind::ALL {
                        let copied = copied_artifacts.get(&kind).ok_or_else(|| {
                            StoreError::CorruptRecord("复制时缺少准备产物映射".to_owned())
                        })?;
                        let dependencies = preparation_dependency_ids(kind, &copied_artifacts)?;
                        for dependency_revision_id in &dependencies {
                            insert_revision_dependency(
                                transaction,
                                copied.copied_revision.target_revision_id,
                                *dependency_revision_id,
                                "copied_preparation",
                            )
                            .await?;
                        }
                        copy_approved_review(
                            transaction,
                            copied.source_review_id,
                            source_version_id,
                            target_version_id,
                            "preparation",
                            copied.copied_revision.source_revision_id,
                            copied.copied_revision.target_revision_id,
                            &dependencies,
                            now,
                        )
                        .await?;
                    }

                    let preparation_revision_ids = PreparationArtifactKind::ALL
                        .into_iter()
                        .map(|kind| {
                            copied_artifacts
                                .get(&kind)
                                .map(|artifact| artifact.copied_revision.target_revision_id)
                                .ok_or_else(|| {
                                    StoreError::CorruptRecord("复制时缺少准备产物修订".to_owned())
                                })
                        })
                        .collect::<Result<Vec<_>, StoreError>>()?;
                    for copied in &copied_chapters {
                        let mut outline_dependencies =
                            Vec::with_capacity(preparation_revision_ids.len() + 1);
                        outline_dependencies.push(copied.guidance.target_revision_id);
                        outline_dependencies.extend(preparation_revision_ids.iter().copied());
                        for dependency_revision_id in &outline_dependencies {
                            insert_revision_dependency(
                                transaction,
                                copied.outline.target_revision_id,
                                *dependency_revision_id,
                                "copied_chapter_outline",
                            )
                            .await?;
                        }
                        copy_approved_review(
                            transaction,
                            copied.source_outline_review_id,
                            source_version_id,
                            target_version_id,
                            "chapter_outline",
                            copied.outline.source_revision_id,
                            copied.outline.target_revision_id,
                            &outline_dependencies,
                            now,
                        )
                        .await?;
                        let body_dependencies = [copied.outline.target_revision_id];
                        insert_revision_dependency(
                            transaction,
                            copied.body.target_revision_id,
                            body_dependencies[0],
                            "copied_chapter_body",
                        )
                        .await?;
                        copy_approved_review(
                            transaction,
                            copied.source_body_review_id,
                            source_version_id,
                            target_version_id,
                            "chapter_final",
                            copied.body.source_revision_id,
                            copied.body.target_revision_id,
                            &body_dependencies,
                            now,
                        )
                        .await?;
                    }

                    let mut copied_context_folds =
                        Vec::with_capacity(1 + copied_chapters.len() * 2);
                    let approved_artifacts = PreparationArtifactKind::ALL
                        .into_iter()
                        .map(|kind| {
                            let copied = copied_artifacts.get(&kind).ok_or_else(|| {
                                StoreError::CorruptRecord("复制时缺少准备产物映射".to_owned())
                            })?;
                            Ok(ApprovedPreparationArtifact {
                                kind,
                                revision_id: copied.copied_revision.target_revision_id,
                                content: copied.copied_revision.content.clone(),
                                chapter_plan: (kind == PreparationArtifactKind::Outline)
                                    .then_some(target_plan.clone()),
                            })
                        })
                        .collect::<Result<Vec<_>, StoreError>>()?;
                    let preparation_plain_text =
                        render_preparation_artifact_plaintext(&approved_artifacts)
                            .map_err(|error| StoreError::CorruptRecord(error.to_string()))?;
                    copied_context_folds.push(CopiedContextFold {
                        id: Uuid::new_v4(),
                        stage: CopiedContextFoldStage::Preparation,
                        chapter_order: None,
                        fold: context_fold(
                            preparation_plain_text,
                            approved_artifacts
                                .iter()
                                .map(|artifact| artifact.revision_id)
                                .collect(),
                        ),
                    });
                    copied_chapters.sort_by_key(|chapter| chapter.order_index);
                    for copied in &copied_chapters {
                        let outline_plain_text = validate_document_content(
                            "章节细纲",
                            &copied.outline.content,
                            nekowriter_core::MAX_ARTIFACT_CONTENT_LEN,
                        )
                        .map_err(|error| StoreError::CorruptRecord(error.to_string()))?;
                        let body_plain_text = validate_document_content(
                            "章节正文",
                            &copied.body.content,
                            nekowriter_core::MAX_ARTIFACT_CONTENT_LEN,
                        )
                        .map_err(|error| StoreError::CorruptRecord(error.to_string()))?;
                        copied_context_folds.push(CopiedContextFold {
                            id: Uuid::new_v4(),
                            stage: CopiedContextFoldStage::ChapterOutline,
                            chapter_order: Some(copied.order_index),
                            fold: context_fold(
                                outline_plain_text,
                                vec![copied.outline.target_revision_id],
                            ),
                        });
                        copied_context_folds.push(CopiedContextFold {
                            id: Uuid::new_v4(),
                            stage: CopiedContextFoldStage::ChapterBody,
                            chapter_order: Some(copied.order_index),
                            fold: context_fold(
                                body_plain_text,
                                vec![copied.body.target_revision_id],
                            ),
                        });
                    }

                    if source_version.body_completed_at.is_some()
                        && first_unfinished_main_chapter_id.is_some()
                    {
                        return Err(StoreError::CorruptRecord(
                            "已确认完本的源稿本仍有未完成正文章节".to_owned(),
                        ));
                    }
                    let (stage, active_chapter_id) = match first_unfinished_main_chapter_id {
                        Some(chapter_id) => (WorkflowStage::WritingChapterReady, Some(chapter_id)),
                        None if source_version.body_completed_at.is_some() => {
                            let active_chapter_id = source_workflow
                                .active_chapter_id
                                .and_then(|source_id| target_chapter_ids.get(&source_id).copied())
                                .or(last_approved_main_chapter_id);
                            (WorkflowStage::Completed, active_chapter_id)
                        }
                        None => (
                            WorkflowStage::WritingChapterDone,
                            last_approved_main_chapter_id,
                        ),
                    };
                    let active_chapter_id = active_chapter_id.ok_or_else(|| {
                        StoreError::CorruptRecord("复制稿本缺少可继续的正文章节".to_owned())
                    })?;
                    let target_workflow = WorkflowSnapshot {
                        version_id: target_version_id,
                        stage,
                        execution: ExecutionStatus::Idle,
                        state_revision: 0,
                        active_run_id: None,
                        active_chapter_id: Some(active_chapter_id),
                        approval_mode: source_workflow.approval_mode,
                        pause_requested: false,
                        wait_reason: None,
                        resume_execution: None,
                        copied_context_folds,
                    };
                    insert_workflow(transaction, &target_workflow).await?;

                    let next_project_revision =
                        project_model.revision.checked_add(1).ok_or_else(|| {
                            StoreError::CorruptRecord("项目修订已达到最大值".to_owned())
                        })?;
                    let mut active_project = project_model.into_active_model();
                    active_project.selected_version_id = Set(db_id(target_version_id));
                    active_project.revision = Set(next_project_revision);
                    active_project.updated_at = Set(now);
                    let updated_project = active_project.update(transaction).await?;
                    let bundle = nekowriter_core::ProjectBundle {
                        project: project_from_model(&updated_project),
                        version: target_version,
                        workflow: target_workflow,
                    };
                    let response_json = serde_json::to_string(&bundle)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        COPY_VERSION_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        201,
                        now,
                    )
                    .await?;
                    Ok(bundle)
                })
            })
            .await
            .map_err(transaction_error);

        match result {
            Ok(bundle) => {
                self.notify_event_listeners();
                Ok(bundle)
            }
            Err(error) => {
                self.replay_after_receipt_conflict(
                    command_id,
                    COPY_VERSION_METHOD,
                    &target,
                    &payload_hash,
                    error,
                )
                .await
            }
        }
    }
}

async fn ensure_source_is_copyable(
    transaction: &DatabaseTransaction,
    source_version_id: VersionId,
    workflow: &WorkflowSnapshot,
    snapshot: &nekowriter_core::PreparationSnapshot,
) -> Result<(), StoreError> {
    if workflow.execution != ExecutionStatus::Idle || workflow.active_run_id.is_some() {
        return Err(StoreError::InvalidChapterState(
            "源稿本仍有活跃写作任务，不能复制".to_owned(),
        ));
    }
    if snapshot.novel_brief.revision_id.is_none() || snapshot.novel_brief.content.is_none() {
        return Err(StoreError::InvalidPreparationState(
            "源稿本缺少已保存的小说目标，不能复制".to_owned(),
        ));
    }
    if snapshot.context_fold.is_none()
        || snapshot.artifacts.len() != PreparationArtifactKind::ALL.len()
        || snapshot.artifacts.iter().any(|artifact| {
            artifact.revision_id.is_none()
                || artifact.review.as_ref().map(|review| review.status.clone())
                    != Some(ReviewStatus::Approved)
                || artifact.review.as_ref().and_then(|review| review.passed) != Some(true)
        })
    {
        return Err(StoreError::InvalidPreparationState(
            "源稿本准备产物尚未全部审核通过，不能复制".to_owned(),
        ));
    }
    if review::Entity::find()
        .filter(
            review::Entity::COLUMN
                .version_id
                .eq(db_id(source_version_id)),
        )
        .filter(review::Entity::COLUMN.status.is_in(["open", "delegated"]))
        .one(transaction)
        .await?
        .is_some()
    {
        return Err(StoreError::InvalidChapterState(
            "源稿本仍有开放审核，不能复制".to_owned(),
        ));
    }
    let source_run_ids = run::Entity::find()
        .filter(run::Entity::COLUMN.version_id.eq(db_id(source_version_id)))
        .all(transaction)
        .await?
        .into_iter()
        .map(|run| run.id)
        .collect::<Vec<_>>();
    if !source_run_ids.is_empty()
        && tool_call::Entity::find()
            .filter(tool_call::Entity::COLUMN.run_id.is_in(source_run_ids))
            .filter(
                tool_call::Entity::COLUMN
                    .status
                    .is_in(["awaiting_review", "held"]),
            )
            .one(transaction)
            .await?
            .is_some()
    {
        return Err(StoreError::InvalidChapterState(
            "源稿本仍有待审正文提案或暂停工具调用，不能复制".to_owned(),
        ));
    }
    Ok(())
}

fn copied_chapter_plan(
    snapshot: &nekowriter_core::PreparationSnapshot,
) -> Result<(ChapterPlan, HashMap<Uuid, Uuid>), StoreError> {
    let source_plan = snapshot
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == PreparationArtifactKind::Outline)
        .and_then(|artifact| artifact.chapter_plan.as_ref())
        .ok_or_else(|| StoreError::CorruptRecord("源稿本审核通过的粗纲缺少章节计划".to_owned()))?;
    source_plan
        .validate()
        .map_err(|error| StoreError::CorruptRecord(error.to_string()))?;
    let mut plan_item_id_map = HashMap::with_capacity(source_plan.items.len());
    let items = source_plan
        .items
        .iter()
        .map(|item| {
            let target_plan_item_id = Uuid::new_v4();
            plan_item_id_map.insert(item.plan_item_id, target_plan_item_id);
            ChapterPlanItem {
                plan_item_id: target_plan_item_id,
                order_index: item.order_index,
                title: item.title.clone(),
                goal: item.goal.clone(),
            }
        })
        .collect();
    Ok((ChapterPlan { items }, plan_item_id_map))
}

async fn active_version_document(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    role: &str,
) -> Result<document::Model, StoreError> {
    document::Entity::find()
        .filter(document::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(document::Entity::COLUMN.chapter_id.is_null())
        .filter(document::Entity::COLUMN.role.eq(role))
        .filter(document::Entity::COLUMN.deleted_at.is_null())
        .one(transaction)
        .await?
        .ok_or_else(|| StoreError::CorruptRecord(format!("稿本缺少 {role} 文档槽")))
}

async fn active_chapter_document(
    transaction: &DatabaseTransaction,
    chapter_id: Uuid,
    role: &str,
) -> Result<document::Model, StoreError> {
    document::Entity::find()
        .filter(
            document::Entity::COLUMN
                .chapter_id
                .eq(chapter_id.to_string()),
        )
        .filter(document::Entity::COLUMN.role.eq(role))
        .filter(document::Entity::COLUMN.deleted_at.is_null())
        .one(transaction)
        .await?
        .ok_or_else(|| StoreError::CorruptRecord(format!("章节缺少 {role} 文档槽")))
}

async fn copy_document_head(
    transaction: &DatabaseTransaction,
    source_document: &document::Model,
    target_document: &document::Model,
    structured_payload_override: Option<String>,
    now: i64,
    label: &str,
) -> Result<CopiedRevision, StoreError> {
    let source_revision_id = parse_optional_id(
        source_document.current_revision_id.clone(),
        "source_document.current_revision_id",
    )?
    .ok_or_else(|| StoreError::CorruptRecord(format!("{label} 缺少当前修订")))?;
    let source_revision = document_revision::Entity::find_by_id(db_id(source_revision_id))
        .one(transaction)
        .await?
        .ok_or_else(|| StoreError::CorruptRecord(format!("{label} 当前修订不存在")))?;
    if source_revision.document_id != source_document.id {
        return Err(StoreError::CorruptRecord(format!(
            "{label} 当前修订不属于其文档"
        )));
    }
    if target_document.current_revision_id.is_some() || target_document.revision != 0 {
        return Err(StoreError::CorruptRecord(format!(
            "复制目标的 {label} 文档不是空槽"
        )));
    }
    let target_revision_id = Uuid::new_v4();
    document_revision::ActiveModel {
        id: Set(db_id(target_revision_id)),
        document_id: Set(target_document.id),
        revision_no: Set(1),
        content: Set(source_revision.content.clone()),
        structured_payload: Set(
            structured_payload_override.or(source_revision.structured_payload.clone())
        ),
        word_count: Set(non_whitespace_char_count(&source_revision.content) as i64),
        created_by: Set("copied".to_owned()),
        source_revision_id: Set(Some(source_revision_id.to_string())),
        created_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    let mut target_document_active = target_document.clone().into_active_model();
    target_document_active.current_revision_id = Set(Some(target_revision_id.to_string()));
    target_document_active.revision = Set(1);
    target_document_active.updated_at = Set(now);
    target_document_active.update(transaction).await?;
    Ok(CopiedRevision {
        source_revision_id,
        target_revision_id,
        content: source_revision.content,
    })
}

async fn approved_review_for_revision(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    review_type: &str,
    revision_id: RevisionId,
) -> Result<review::Model, StoreError> {
    review::Entity::find()
        .filter(review::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(review::Entity::COLUMN.review_type.eq(review_type))
        .filter(
            review::Entity::COLUMN
                .target_revision_id
                .eq(revision_id.to_string()),
        )
        .filter(review::Entity::COLUMN.status.eq("approved"))
        .filter(review::Entity::COLUMN.passed.eq(true))
        .order_by_desc(review::Entity::COLUMN.updated_at)
        .order_by_desc(review::Entity::COLUMN.id)
        .one(transaction)
        .await?
        .ok_or_else(|| StoreError::CorruptRecord(format!("{review_type} 缺少通过审核依据")))
}

fn preparation_dependency_kinds(
    kind: PreparationArtifactKind,
) -> &'static [PreparationArtifactKind] {
    match kind {
        PreparationArtifactKind::Research => &[],
        PreparationArtifactKind::Background => &[PreparationArtifactKind::Research],
        PreparationArtifactKind::Characters => &[
            PreparationArtifactKind::Research,
            PreparationArtifactKind::Background,
        ],
        PreparationArtifactKind::Outline => &[
            PreparationArtifactKind::Research,
            PreparationArtifactKind::Background,
            PreparationArtifactKind::Characters,
        ],
    }
}

fn preparation_dependency_ids(
    kind: PreparationArtifactKind,
    copied_artifacts: &BTreeMap<PreparationArtifactKind, CopiedPreparationArtifact>,
) -> Result<Vec<RevisionId>, StoreError> {
    preparation_dependency_kinds(kind)
        .iter()
        .map(|dependency_kind| {
            copied_artifacts
                .get(dependency_kind)
                .map(|artifact| artifact.copied_revision.target_revision_id)
                .ok_or_else(|| StoreError::CorruptRecord("复制时缺少准备产物依赖修订".to_owned()))
        })
        .collect()
}

async fn insert_revision_dependency(
    transaction: &DatabaseTransaction,
    revision_id: RevisionId,
    dependency_revision_id: RevisionId,
    purpose: &str,
) -> Result<(), StoreError> {
    revision_dependency::ActiveModel {
        revision_id: Set(db_id(revision_id)),
        dependency_revision_id: Set(db_id(dependency_revision_id)),
        purpose: Set(purpose.to_owned()),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn copy_approved_review(
    transaction: &DatabaseTransaction,
    source_review_id: Uuid,
    source_version_id: VersionId,
    target_version_id: VersionId,
    review_type: &str,
    source_revision_id: RevisionId,
    target_revision_id: RevisionId,
    dependency_revision_ids: &[RevisionId],
    now: i64,
) -> Result<(), StoreError> {
    let source_review = review::Entity::find_by_id(db_id(source_review_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewNotFound)?;
    if source_review.version_id != db_id(source_version_id)
        || source_review.review_type != review_type
        || source_review.target_revision_id.as_deref() != Some(&source_revision_id.to_string())
        || source_review.status != "approved"
        || source_review.passed != Some(true)
    {
        return Err(StoreError::CorruptRecord(
            "复制来源的审核依据不再是当前通过审核".to_owned(),
        ));
    }
    review::ActiveModel {
        id: Set(db_id(Uuid::new_v4())),
        version_id: Set(db_id(target_version_id)),
        batch_id: Set(None),
        review_type: Set(review_type.to_owned()),
        target_revision_id: Set(Some(target_revision_id.to_string())),
        target_tool_call_id: Set(None),
        dependencies_json: Set(serde_json::to_string(
            &dependency_revision_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        )?),
        status: Set("approved".to_owned()),
        attempt_id: Set(None),
        decision_source: Set(Some("copied".to_owned())),
        source_review_id: Set(Some(source_review_id.to_string())),
        passed: Set(Some(true)),
        confidence: Set(source_review.confidence),
        issues_json: Set(None),
        user_input_json: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(())
}

fn context_fold(plain_text: String, artifact_revision_ids: Vec<RevisionId>) -> ContextFold {
    ContextFold {
        artifact_revision_ids,
        through_message_sequence: 0,
        content_hash: format!("{:x}", Sha256::digest(plain_text.as_bytes())),
        plain_text,
    }
}
