//! 回收站：列出软删除对象、恢复它们，或对可以物理移除的对象彻底删除。
//!
//! 恢复只把 `deleted_at` 清回去，因此章节的正文、修订 id 与在作品里的位置都还是
//! 原来那一份——不会为了「恢复」再写一条新修订（那会让历史、引用与统计跟现实对不上）。
//! 父对象仍在回收站里、或对象已经被彻底删除时，恢复必须明确报错而不是半恢复。

use std::collections::HashMap;

use nekowriter_core::recycle::{RecycleBin, RecycleItem, RecycleItemKind, RecycleParent};
use nekowriter_core::{
    ChapterId, PreparationSnapshot, VersionId, utc_now_millis, withdraw_extra_chapter,
};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    Set, TransactionTrait,
};
use uuid::Uuid;

use serde::Serialize;

use super::entity::{
    chapter, document, document_revision, knowledge_entry, knowledge_index_task, knowledge_space,
    novel_version, project, prompt_template, review, run,
};
use super::preparation::{
    active_top_level_run_exists, ensure_expected_state_revision, insert_app_event, insert_event,
    open_review_exists, persist_workflow, preparation_snapshot, workflow_for_version,
};
use super::{
    ActiveModelTrait, ProjectId, Store, StoreError, TextUuid, db_id, finish_write, insert_receipt,
    parse_id, payload_hash, replay_or_conflict, transaction_error,
};

const RESTORE_METHOD: &str = "POST";
const PURGE_METHOD: &str = "POST";

/// 稿本与正文内容的物理删除留给后续清理任务，见 docs/02 §8.4。
const MANUSCRIPT_PURGE_REASON: &str =
    "正文与它的事件、审核和修订引用共同构成审计链，本版本只保留在回收站里，不做物理删除";

/// 项目下的稿本、正文与审核记录同样构成审计链，因此项目也不做物理删除。
const PROJECT_PURGE_REASON: &str = "项目下的稿本、正文、事件、审核与修订引用共同构成审计链，本版本只把项目留在回收站里，不做物理删除";

const DELETE_EXTRA_CHAPTER_METHOD: &str = "DELETE";
const DELETE_VERSION_METHOD: &str = "DELETE";
const DELETE_PROJECT_METHOD: &str = "DELETE";

#[derive(Serialize)]
struct ExpectedStateRevisionPayload {
    expected_state_revision: i64,
}

#[derive(Serialize)]
struct ExpectedRevisionPayload {
    expected_revision: i64,
}

impl Store {
    /// 软删除一份番外章。
    ///
    /// 删除只把 `deleted_at` 写回去：章节行、文档槽与全部修订都不动，因此从回收站
    /// 恢复时正文、修订 id 与在作品里的位置都还是原来那一份。正文计划章不提供
    /// 单独删除（见 docs/02 §8.4）；有开放审核或活跃写作任务时返回 [`StoreError::ResourceBusy`]。
    ///
    /// 工作流在同一事务里从被删的那一章收回：退回已完本，并让活跃章节落到删除前
    /// 仍然可见的最后一章。
    pub async fn delete_extra_chapter(
        &self,
        chapter_id: ChapterId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<PreparationSnapshot, StoreError> {
        let target = format!("/api/chapters/{chapter_id}");
        let payload_hash = payload_hash(&ExpectedStateRevisionPayload {
            expected_state_revision,
        })?;
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(
                receipt,
                DELETE_EXTRA_CHAPTER_METHOD,
                &target,
                &payload_hash,
            );
        }
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let now = utc_now_millis();
                    let model = chapter::Entity::find_by_id(db_id(chapter_id))
                        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ChapterNotFound)?;
                    if model.kind != "extra" {
                        return Err(StoreError::InvalidChapterState(
                            "正文计划章不支持单独删除".to_owned(),
                        ));
                    }
                    let version_id = model.version_id.0;
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if open_review_exists(transaction, version_id).await?
                        || active_top_level_run_exists(transaction, version_id).await?
                    {
                        return Err(StoreError::ResourceBusy(
                            "仍有审核或写作任务，不能删除番外".to_owned(),
                        ));
                    }
                    let version_model = novel_version::Entity::find_by_id(db_id(version_id))
                        .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::VersionNotFound)?;

                    let next_chapter_revision = model.revision + 1;
                    let mut active_chapter = model.into_active_model();
                    active_chapter.deleted_at = Set(Some(now));
                    active_chapter.revision = Set(next_chapter_revision);
                    active_chapter.updated_at = Set(now);
                    active_chapter.update(transaction).await?;

                    let remaining_active = last_visible_chapter(transaction, version_id).await?;
                    let next_workflow =
                        withdraw_extra_chapter(&workflow, chapter_id, remaining_active)?;
                    let next_version_revision = version_model.revision + 1;
                    let mut active_version = version_model.into_active_model();
                    active_version.revision = Set(next_version_revision);
                    active_version.updated_at = Set(now);
                    active_version.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        serde_json::json!({
                            "stage": next_workflow.stage,
                            "chapter_id": chapter_id,
                            "kind": "extra",
                            "deleted": true,
                            "run_id": serde_json::Value::Null,
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "recycle.changed",
                        chapter_id,
                        next_version_revision,
                        serde_json::json!({ "kind": "chapter", "action": "deleted" }),
                        now,
                    )
                    .await?;
                    let response = preparation_snapshot(transaction, version_id).await?;
                    insert_receipt(
                        transaction,
                        command_id,
                        DELETE_EXTRA_CHAPTER_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&response)?,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
            .map_err(transaction_error);
        finish_write(
            self,
            command_id,
            DELETE_EXTRA_CHAPTER_METHOD,
            &target,
            &payload_hash,
            result,
        )
        .await
    }

    /// 软删除一份稿本。
    ///
    /// 稿本下的章节、文档与修订都不改写：它们随祖先不可见，也在祖先恢复时一起回来。
    /// 项目当前选中的稿本不能删除（界面上要先切到别的稿本），有活跃写作任务或开放
    /// 审核时返回 [`StoreError::ResourceBusy`]。
    pub async fn delete_version(
        &self,
        version_id: VersionId,
        command_id: Uuid,
        expected_revision: i64,
    ) -> Result<RecycleBin, StoreError> {
        let target = format!("/api/versions/{version_id}");
        let payload_hash = payload_hash(&ExpectedRevisionPayload { expected_revision })?;
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(receipt, DELETE_VERSION_METHOD, &target, &payload_hash);
        }
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let now = utc_now_millis();
                    let model = novel_version::Entity::find_by_id(db_id(version_id))
                        .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::VersionNotFound)?;
                    if model.revision != expected_revision {
                        return Err(StoreError::VersionRevisionConflict {
                            expected: expected_revision,
                            actual: model.revision,
                        });
                    }
                    let project = live_project(transaction, model.project_id.0).await?;
                    if project.selected_version_id.0 == version_id {
                        return Err(StoreError::InvalidVersionState(
                            "这是当前选中的稿本，请先切到别的稿本再删除".to_owned(),
                        ));
                    }
                    if open_review_exists(transaction, version_id).await?
                        || active_top_level_run_exists(transaction, version_id).await?
                    {
                        return Err(StoreError::ResourceBusy(
                            "仍有审核或写作任务，不能删除稿本".to_owned(),
                        ));
                    }
                    let next_revision = model.revision + 1;
                    let mut active = model.into_active_model();
                    active.deleted_at = Set(Some(now));
                    active.revision = Set(next_revision);
                    active.updated_at = Set(now);
                    active.update(transaction).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "recycle.changed",
                        version_id,
                        next_revision,
                        serde_json::json!({ "kind": "version", "action": "deleted" }),
                        now,
                    )
                    .await?;
                    let bin = recycle_bin(transaction).await?;
                    insert_receipt(
                        transaction,
                        command_id,
                        DELETE_VERSION_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&bin)?,
                        200,
                        now,
                    )
                    .await?;
                    Ok(bin)
                })
            })
            .await
            .map_err(transaction_error);
        finish_write(
            self,
            command_id,
            DELETE_VERSION_METHOD,
            &target,
            &payload_hash,
            result,
        )
        .await
    }

    /// 软删除一个项目。
    ///
    /// 只把项目的 `deleted_at` 写回去，不批量改写它下面任何孩子的状态：查询逐层检查
    /// 祖先，因此项目在回收站里时它的稿本、章节与正文都不可见，而恢复项目会原样把它
    /// 们带回来（此前单独删除的孩子不会被误恢复）。项目下仍有开放审核或活跃写作任务
    /// 时返回 [`StoreError::ResourceBusy`]。
    pub async fn delete_project(
        &self,
        project_id: ProjectId,
        command_id: Uuid,
        expected_revision: i64,
    ) -> Result<RecycleBin, StoreError> {
        let target = format!("/api/projects/{project_id}");
        let payload_hash = payload_hash(&ExpectedRevisionPayload { expected_revision })?;
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(receipt, DELETE_PROJECT_METHOD, &target, &payload_hash);
        }
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let now = utc_now_millis();
                    let model = project::Entity::find_by_id(db_id(project_id))
                        .filter(project::Entity::COLUMN.deleted_at.is_null())
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ProjectNotFound)?;
                    if model.revision != expected_revision {
                        return Err(StoreError::StateRevisionConflict {
                            expected: expected_revision,
                            actual: model.revision,
                        });
                    }
                    if project_has_active_work(transaction, project_id).await? {
                        return Err(StoreError::ResourceBusy(
                            "项目下仍有审核或写作任务，不能删除项目".to_owned(),
                        ));
                    }
                    let next_revision = model.revision + 1;
                    let mut active = model.into_active_model();
                    active.deleted_at = Set(Some(now));
                    active.revision = Set(next_revision);
                    active.updated_at = Set(now);
                    active.update(transaction).await?;
                    insert_app_event(
                        transaction,
                        "recycle.changed",
                        project_id,
                        next_revision,
                        serde_json::json!({ "kind": "project", "action": "deleted" }),
                        now,
                    )
                    .await?;
                    let bin = recycle_bin(transaction).await?;
                    insert_receipt(
                        transaction,
                        command_id,
                        DELETE_PROJECT_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&bin)?,
                        200,
                        now,
                    )
                    .await?;
                    Ok(bin)
                })
            })
            .await
            .map_err(transaction_error);
        finish_write(
            self,
            command_id,
            DELETE_PROJECT_METHOD,
            &target,
            &payload_hash,
            result,
        )
        .await
    }

    /// 回收站快照：所有软删除的对象，按删除时间倒序。
    ///
    /// 每条都带上「现在能不能恢复 / 能不能彻底删除」的结论与原因，界面因此可以直接
    /// 解释前提，而不是等用户点下去才失败。
    pub async fn get_recycle_bin(&self) -> Result<RecycleBin, StoreError> {
        recycle_bin(&self.database).await
    }

    /// 恢复一条回收站记录。
    ///
    /// # Errors
    ///
    /// 对象不在回收站里（不存在或已被彻底删除）时返回 [`StoreError::RecycleItemNotFound`]；
    /// 所属父对象仍在回收站里时返回 [`StoreError::RecycleAncestorDeleted`]。
    pub async fn restore_recycle_item(
        &self,
        kind: RecycleItemKind,
        id: Uuid,
        command_id: Uuid,
    ) -> Result<RecycleBin, StoreError> {
        let target = format!("/api/recycle-bin/{}/{id}/restore", kind.as_str());
        let payload_hash = payload_hash(&kind.as_str())?;
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(receipt, RESTORE_METHOD, &target, &payload_hash);
        }
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let now = utc_now_millis();
                    match kind {
                        RecycleItemKind::Project => restore_project(transaction, id, now).await?,
                        RecycleItemKind::Version => restore_version(transaction, id, now).await?,
                        RecycleItemKind::Chapter => restore_chapter(transaction, id, now).await?,
                        RecycleItemKind::PromptTemplate => {
                            restore_prompt_template(transaction, id, now).await?;
                        }
                        RecycleItemKind::KnowledgeEntry => {
                            restore_knowledge_entry(transaction, id, now).await?;
                        }
                        RecycleItemKind::KnowledgeSpace => {
                            restore_knowledge_space(transaction, id, now).await?;
                        }
                    }
                    let bin = recycle_bin(transaction).await?;
                    insert_receipt(
                        transaction,
                        command_id,
                        RESTORE_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&bin)?,
                        200,
                        now,
                    )
                    .await?;
                    Ok(bin)
                })
            })
            .await
            .map_err(transaction_error);
        finish_write(
            self,
            command_id,
            RESTORE_METHOD,
            &target,
            &payload_hash,
            result,
        )
        .await
    }

    /// 彻底删除一条回收站记录。
    ///
    /// # Errors
    ///
    /// 对象不在回收站里时返回 [`StoreError::RecycleItemNotFound`]；对象类别不支持物理
    /// 删除（稿本与正文）时返回 [`StoreError::RecycleNotPurgeable`]。
    pub async fn purge_recycle_item(
        &self,
        kind: RecycleItemKind,
        id: Uuid,
        command_id: Uuid,
    ) -> Result<RecycleBin, StoreError> {
        let target = format!("/api/recycle-bin/{}/{id}/purge", kind.as_str());
        let payload_hash = payload_hash(&kind.as_str())?;
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(receipt, PURGE_METHOD, &target, &payload_hash);
        }
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let now = utc_now_millis();
                    match kind {
                        RecycleItemKind::PromptTemplate => {
                            purge_prompt_template(transaction, id, now).await?;
                        }
                        RecycleItemKind::KnowledgeEntry => {
                            purge_knowledge_entry(transaction, id, now).await?;
                        }
                        RecycleItemKind::KnowledgeSpace => {
                            purge_knowledge_space(transaction, id, now).await?;
                        }
                        RecycleItemKind::Version | RecycleItemKind::Chapter => {
                            if !stays_in_bin(transaction, kind, id).await? {
                                return Err(StoreError::RecycleItemNotFound);
                            }
                            return Err(StoreError::RecycleNotPurgeable(
                                MANUSCRIPT_PURGE_REASON.to_owned(),
                            ));
                        }
                        RecycleItemKind::Project => {
                            if !stays_in_bin(transaction, kind, id).await? {
                                return Err(StoreError::RecycleItemNotFound);
                            }
                            return Err(StoreError::RecycleNotPurgeable(
                                PROJECT_PURGE_REASON.to_owned(),
                            ));
                        }
                    }
                    let bin = recycle_bin(transaction).await?;
                    insert_receipt(
                        transaction,
                        command_id,
                        PURGE_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&bin)?,
                        200,
                        now,
                    )
                    .await?;
                    Ok(bin)
                })
            })
            .await
            .map_err(transaction_error);
        finish_write(
            self,
            command_id,
            PURGE_METHOD,
            &target,
            &payload_hash,
            result,
        )
        .await
    }
}

async fn recycle_bin<C: ConnectionTrait>(connection: &C) -> Result<RecycleBin, StoreError> {
    let mut items = Vec::new();
    items.extend(deleted_projects(connection).await?);
    items.extend(deleted_versions(connection).await?);
    items.extend(deleted_extra_chapters(connection).await?);
    items.extend(deleted_prompt_templates(connection).await?);
    items.extend(deleted_knowledge_entries(connection).await?);
    items.extend(deleted_knowledge_spaces(connection).await?);
    items.sort_by(|left, right| {
        right
            .deleted_at_ms
            .cmp(&left.deleted_at_ms)
            .then_with(|| left.title.cmp(&right.title))
    });
    Ok(RecycleBin { items })
}

/// 软删除的项目。项目没有祖先，因此在回收站里总是可以恢复。
async fn deleted_projects<C: ConnectionTrait>(
    connection: &C,
) -> Result<Vec<RecycleItem>, StoreError> {
    let models = project::Entity::find()
        .filter(project::Entity::COLUMN.deleted_at.is_not_null())
        .all(connection)
        .await?;
    let mut items = Vec::with_capacity(models.len());
    for model in models {
        items.push(RecycleItem {
            kind: RecycleItemKind::Project,
            id: model.id.0,
            title: model.name.clone(),
            detail: project_detail(connection, &model.id).await?,
            deleted_at_ms: model.deleted_at.unwrap_or_default(),
            parent: None,
            restorable: true,
            restore_blocked_reason: None,
            purgeable: false,
            purge_blocked_reason: Some(PROJECT_PURGE_REASON.to_owned()),
        });
    }
    Ok(items)
}

/// 软删除的稿本。所属项目必须仍然可见，否则恢复会先把用户挡在门外。
async fn deleted_versions<C: ConnectionTrait>(
    connection: &C,
) -> Result<Vec<RecycleItem>, StoreError> {
    let models = novel_version::Entity::find()
        .filter(novel_version::Entity::COLUMN.deleted_at.is_not_null())
        .all(connection)
        .await?;
    let projects = projects_by_id(connection, models.iter().map(|model| &model.project_id)).await?;
    let mut items = Vec::with_capacity(models.len());
    for model in models {
        let parent = projects
            .get(&model.project_id.0)
            .filter(|project| project.deleted_at.is_none());
        let detail = chapter_count_detail(connection, &model.id).await?;
        items.push(RecycleItem {
            kind: RecycleItemKind::Version,
            id: model.id.0,
            title: format!("{}（{}）", model.novel_title, model.label),
            detail,
            deleted_at_ms: model.deleted_at.unwrap_or_default(),
            parent: parent.map(|project| RecycleParent {
                kind: "项目".to_owned(),
                id: project.id.0,
                title: project.name.clone(),
            }),
            restorable: parent.is_some(),
            restore_blocked_reason: parent
                .is_none()
                .then(|| "所属项目仍在回收站里，请先恢复它".to_owned()),
            purgeable: false,
            purge_blocked_reason: Some(MANUSCRIPT_PURGE_REASON.to_owned()),
        });
    }
    Ok(items)
}

/// 软删除的番外章。正文计划章不提供独立删除，因此这里只会看到番外。
async fn deleted_extra_chapters<C: ConnectionTrait>(
    connection: &C,
) -> Result<Vec<RecycleItem>, StoreError> {
    let models = chapter::Entity::find()
        .filter(chapter::Entity::COLUMN.deleted_at.is_not_null())
        .filter(chapter::Entity::COLUMN.kind.eq("extra"))
        .all(connection)
        .await?;
    let versions = versions_by_id(connection, models.iter().map(|model| &model.version_id)).await?;
    let project_ids: Vec<&TextUuid> = versions
        .values()
        .map(|version| &version.project_id)
        .collect();
    let projects = projects_by_id(connection, project_ids).await?;
    let mut items = Vec::with_capacity(models.len());
    for model in models {
        // 祖先可见性逐层判断：稿本或项目任一在回收站里就不放行。
        let version = versions
            .get(&model.version_id.0)
            .filter(|version| version.deleted_at.is_none());
        let project_visible = version
            .and_then(|version| projects.get(&version.project_id.0))
            .is_some_and(|project| project.deleted_at.is_none());
        let restorable = version.is_some() && project_visible;
        let blocked = if version.is_none() {
            Some("所属稿本仍在回收站里，请先恢复稿本".to_owned())
        } else if !project_visible {
            Some("所属项目仍在回收站里，请先恢复项目".to_owned())
        } else {
            None
        };
        let word_count = chapter_word_count(connection, model.version_id.0, model.id.0).await?;
        let detail = match version {
            Some(version) => format!(
                "{word_count} 字正文 · 所属稿本 {}（{}）",
                version.novel_title, version.label
            ),
            None => format!("{word_count} 字正文"),
        };
        items.push(RecycleItem {
            kind: RecycleItemKind::Chapter,
            id: model.id.0,
            title: model.title.clone(),
            detail: Some(detail),
            deleted_at_ms: model.deleted_at.unwrap_or_default(),
            parent: version.map(|version| RecycleParent {
                kind: "稿本".to_owned(),
                id: version.id.0,
                title: format!("{}（{}）", version.novel_title, version.label),
            }),
            restorable,
            restore_blocked_reason: blocked,
            purgeable: false,
            purge_blocked_reason: Some(MANUSCRIPT_PURGE_REASON.to_owned()),
        });
    }
    Ok(items)
}

async fn deleted_prompt_templates<C: ConnectionTrait>(
    connection: &C,
) -> Result<Vec<RecycleItem>, StoreError> {
    let models = prompt_template::Entity::find()
        .filter(prompt_template::Entity::COLUMN.deleted_at.is_not_null())
        .all(connection)
        .await?;
    Ok(models
        .into_iter()
        .map(|model| RecycleItem {
            kind: RecycleItemKind::PromptTemplate,
            id: model.id.0,
            title: model.title,
            detail: None,
            deleted_at_ms: model.deleted_at.unwrap_or_default(),
            parent: None,
            restorable: true,
            restore_blocked_reason: None,
            purgeable: true,
            purge_blocked_reason: None,
        })
        .collect())
}

async fn deleted_knowledge_entries<C: ConnectionTrait>(
    connection: &C,
) -> Result<Vec<RecycleItem>, StoreError> {
    let models = knowledge_entry::Entity::find()
        .filter(knowledge_entry::Entity::COLUMN.deleted_at.is_not_null())
        .all(connection)
        .await?;
    let spaces = spaces_by_id(connection, models.iter().map(|model| &model.space_id)).await?;
    Ok(models
        .into_iter()
        .map(|model| {
            let space = spaces
                .get(&model.space_id.0)
                .filter(|space| space.deleted_at.is_none());
            RecycleItem {
                kind: RecycleItemKind::KnowledgeEntry,
                id: model.id.0,
                title: model.title,
                detail: space.map(|space| format!("所属 Space：{}", space.title)),
                deleted_at_ms: model.deleted_at.unwrap_or_default(),
                parent: space.map(|space| RecycleParent {
                    kind: "知识 Space".to_owned(),
                    id: space.id.0,
                    title: space.title.clone(),
                }),
                restorable: space.is_some(),
                restore_blocked_reason: space
                    .is_none()
                    .then(|| "所属 Space 仍在回收站里，请先恢复它".to_owned()),
                purgeable: true,
                purge_blocked_reason: None,
            }
        })
        .collect())
}

async fn deleted_knowledge_spaces<C: ConnectionTrait>(
    connection: &C,
) -> Result<Vec<RecycleItem>, StoreError> {
    let models = knowledge_space::Entity::find()
        .filter(knowledge_space::Entity::COLUMN.deleted_at.is_not_null())
        .all(connection)
        .await?;
    Ok(models
        .into_iter()
        .map(|model| RecycleItem {
            kind: RecycleItemKind::KnowledgeSpace,
            id: model.id.0,
            title: model.title,
            detail: None,
            deleted_at_ms: model.deleted_at.unwrap_or_default(),
            parent: None,
            restorable: true,
            restore_blocked_reason: None,
            purgeable: true,
            purge_blocked_reason: None,
        })
        .collect())
}

/// 删除一章之后仍然可见的最后一章（按作品顺序），用作工作流新的活跃章节。
///
/// 番外只能加在已完本稿本上，因此这里通常返回一份正文章节；稿本一章不剩时返回
/// `None`，工作流仍然停在已完本，只是不再高亮任何一章。
async fn last_visible_chapter(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
) -> Result<Option<ChapterId>, StoreError> {
    let model = chapter::Entity::find()
        .filter(chapter::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
        .order_by_desc(chapter::Entity::COLUMN.order_index)
        .one(transaction)
        .await?;
    Ok(model.map(|model| model.id.0))
}

/// 对象是否还留在回收站里（用于区分「不能删」与「已经不在了」）。
async fn stays_in_bin(
    transaction: &DatabaseTransaction,
    kind: RecycleItemKind,
    id: Uuid,
) -> Result<bool, StoreError> {
    match kind {
        RecycleItemKind::Project => Ok(required_deleted_project(transaction, id).await.is_ok()),
        RecycleItemKind::Version => Ok(required_deleted_version(transaction, id).await.is_ok()),
        _ => Ok(required_deleted_chapter(transaction, id).await.is_ok()),
    }
}

/// 恢复一个项目：只清 `deleted_at`，不碰它下面任何孩子的状态。
///
/// 因此恢复项目会把它的稿本、章节与正文原样带回来，而删除项目之前单独删掉的稿本或
/// 番外仍然留在回收站里。
async fn restore_project(
    transaction: &DatabaseTransaction,
    id: Uuid,
    now: i64,
) -> Result<(), StoreError> {
    let model = required_deleted_project(transaction, id).await?;
    let next_revision = model.revision + 1;
    let mut active = model.into_active_model();
    active.deleted_at = Set(None);
    active.revision = Set(next_revision);
    active.updated_at = Set(now);
    active.update(transaction).await?;
    insert_app_event(
        transaction,
        "recycle.changed",
        id,
        next_revision,
        serde_json::json!({ "kind": "project", "action": "restored" }),
        now,
    )
    .await?;
    Ok(())
}

async fn restore_version(
    transaction: &DatabaseTransaction,
    id: Uuid,
    now: i64,
) -> Result<(), StoreError> {
    let model = required_deleted_version(transaction, id).await?;
    live_project(transaction, model.project_id.0).await?;
    let next_revision = model.revision + 1;
    let mut active = model.into_active_model();
    active.deleted_at = Set(None);
    active.revision = Set(next_revision);
    active.updated_at = Set(now);
    active.update(transaction).await?;
    insert_event(
        transaction,
        id,
        "recycle.changed",
        id,
        next_revision,
        serde_json::json!({ "kind": "version", "action": "restored" }),
        now,
    )
    .await?;
    Ok(())
}

/// 恢复一份番外：先把被它占用的顺序号让出来，再清 `deleted_at`。
///
/// 删除期间如果新增了番外，新番外会取「当前最大序号 + 1」，正好可能落在被删章节的
/// 序号上；因此恢复必须从高到低把同稿本、同类别、序号不小于它的行依次上移，
/// 让它回到删除前的那一格，而不是撞上唯一键或被排到末尾。
async fn restore_chapter(
    transaction: &DatabaseTransaction,
    id: Uuid,
    now: i64,
) -> Result<(), StoreError> {
    let model = required_deleted_chapter(transaction, id).await?;
    let version_id = model.version_id.0;
    let version = live_version(transaction, version_id).await?;
    live_project(transaction, version.project_id.0).await?;

    let taken = chapter::Entity::find()
        .filter(chapter::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(chapter::Entity::COLUMN.kind.eq(model.kind.clone()))
        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
        .filter(chapter::Entity::COLUMN.order_index.gte(model.order_index))
        .order_by_desc(chapter::Entity::COLUMN.order_index)
        .all(transaction)
        .await?;
    for row in taken {
        let next_index = row.order_index + 1;
        let mut active = row.into_active_model();
        active.order_index = Set(next_index);
        active.updated_at = Set(now);
        active.update(transaction).await?;
    }

    let next_chapter_revision = model.revision + 1;
    let next_version_revision = version.revision + 1;
    let mut active_chapter = model.into_active_model();
    active_chapter.deleted_at = Set(None);
    active_chapter.revision = Set(next_chapter_revision);
    active_chapter.updated_at = Set(now);
    active_chapter.update(transaction).await?;
    let mut active_version = version.into_active_model();
    active_version.revision = Set(next_version_revision);
    active_version.updated_at = Set(now);
    active_version.update(transaction).await?;

    insert_event(
        transaction,
        version_id,
        "recycle.changed",
        id,
        next_version_revision,
        serde_json::json!({ "kind": "chapter", "action": "restored" }),
        now,
    )
    .await?;
    Ok(())
}

async fn restore_prompt_template(
    transaction: &DatabaseTransaction,
    id: Uuid,
    now: i64,
) -> Result<(), StoreError> {
    let model = required_deleted_prompt_template(transaction, id).await?;
    let next_revision = model.revision + 1;
    let mut active = model.into_active_model();
    active.deleted_at = Set(None);
    active.revision = Set(next_revision);
    active.updated_at = Set(now);
    active.update(transaction).await?;
    insert_app_event(
        transaction,
        "prompt_template.changed",
        id,
        next_revision,
        serde_json::json!({ "action": "restored" }),
        now,
    )
    .await?;
    Ok(())
}

async fn restore_knowledge_entry(
    transaction: &DatabaseTransaction,
    id: Uuid,
    now: i64,
) -> Result<(), StoreError> {
    let model = required_deleted_knowledge_entry(transaction, id).await?;
    live_space(transaction, model.space_id.0).await?;
    let next_revision = model.revision + 1;
    let mut active = model.into_active_model();
    active.deleted_at = Set(None);
    active.revision = Set(next_revision);
    active.updated_at = Set(now);
    active.update(transaction).await?;
    insert_app_event(
        transaction,
        "knowledge.index_changed",
        id,
        next_revision,
        serde_json::json!({ "action": "restored" }),
        now,
    )
    .await?;
    Ok(())
}

async fn restore_knowledge_space(
    transaction: &DatabaseTransaction,
    id: Uuid,
    now: i64,
) -> Result<(), StoreError> {
    let model = required_deleted_knowledge_space(transaction, id).await?;
    let next_revision = model.revision + 1;
    let mut active = model.into_active_model();
    active.deleted_at = Set(None);
    active.revision = Set(next_revision);
    active.updated_at = Set(now);
    active.update(transaction).await?;
    insert_app_event(
        transaction,
        "knowledge.index_changed",
        id,
        next_revision,
        serde_json::json!({ "action": "restored" }),
        now,
    )
    .await?;
    Ok(())
}

async fn purge_prompt_template(
    transaction: &DatabaseTransaction,
    id: Uuid,
    now: i64,
) -> Result<(), StoreError> {
    let model = required_deleted_prompt_template(transaction, id).await?;
    prompt_template::Entity::delete_by_id(model.id)
        .exec(transaction)
        .await?;
    insert_app_event(
        transaction,
        "prompt_template.changed",
        id,
        model.revision + 1,
        serde_json::json!({ "action": "purged" }),
        now,
    )
    .await?;
    Ok(())
}

/// 彻底删除一条知识条目：条目行与它尚未执行的索引任务一起移除；已经写进向量库的
/// 块由检索时的存活校验过滤，物理清理属于后续清理任务（见 docs/02 §8.4）。
async fn purge_knowledge_entry(
    transaction: &DatabaseTransaction,
    id: Uuid,
    now: i64,
) -> Result<(), StoreError> {
    let model = required_deleted_knowledge_entry(transaction, id).await?;
    knowledge_index_task::Entity::delete_many()
        .filter(knowledge_index_task::Entity::COLUMN.entry_id.eq(db_id(id)))
        .exec(transaction)
        .await?;
    knowledge_entry::Entity::delete_by_id(model.id)
        .exec(transaction)
        .await?;
    insert_app_event(
        transaction,
        "knowledge.index_changed",
        id,
        model.revision + 1,
        serde_json::json!({ "action": "purged" }),
        now,
    )
    .await?;
    Ok(())
}

async fn purge_knowledge_space(
    transaction: &DatabaseTransaction,
    id: Uuid,
    now: i64,
) -> Result<(), StoreError> {
    let model = required_deleted_knowledge_space(transaction, id).await?;
    knowledge_space::Entity::delete_by_id(model.id)
        .exec(transaction)
        .await?;
    insert_app_event(
        transaction,
        "knowledge.index_changed",
        id,
        model.revision + 1,
        serde_json::json!({ "action": "purged" }),
        now,
    )
    .await?;
    Ok(())
}

/// 读取一条仍然留在回收站里的项目；不存在或并未删除都当作不在回收站。
async fn required_deleted_project(
    transaction: &DatabaseTransaction,
    id: Uuid,
) -> Result<project::Model, StoreError> {
    project::Entity::find_by_id(db_id(id))
        .filter(project::Entity::COLUMN.deleted_at.is_not_null())
        .one(transaction)
        .await?
        .ok_or(StoreError::RecycleItemNotFound)
}

/// 读取一条仍然留在回收站里的稿本；不存在或并未删除都当作不在回收站。
async fn required_deleted_version(
    transaction: &DatabaseTransaction,
    id: Uuid,
) -> Result<novel_version::Model, StoreError> {
    novel_version::Entity::find_by_id(db_id(id))
        .filter(novel_version::Entity::COLUMN.deleted_at.is_not_null())
        .one(transaction)
        .await?
        .ok_or(StoreError::RecycleItemNotFound)
}

/// 读取一条仍然留在回收站里的番外章。正文计划章不属于回收站。
async fn required_deleted_chapter(
    transaction: &DatabaseTransaction,
    id: Uuid,
) -> Result<chapter::Model, StoreError> {
    chapter::Entity::find_by_id(db_id(id))
        .filter(chapter::Entity::COLUMN.deleted_at.is_not_null())
        .filter(chapter::Entity::COLUMN.kind.eq("extra"))
        .one(transaction)
        .await?
        .ok_or(StoreError::RecycleItemNotFound)
}

async fn required_deleted_prompt_template(
    transaction: &DatabaseTransaction,
    id: Uuid,
) -> Result<prompt_template::Model, StoreError> {
    prompt_template::Entity::find_by_id(db_id(id))
        .filter(prompt_template::Entity::COLUMN.deleted_at.is_not_null())
        .one(transaction)
        .await?
        .ok_or(StoreError::RecycleItemNotFound)
}

async fn required_deleted_knowledge_entry(
    transaction: &DatabaseTransaction,
    id: Uuid,
) -> Result<knowledge_entry::Model, StoreError> {
    knowledge_entry::Entity::find_by_id(db_id(id))
        .filter(knowledge_entry::Entity::COLUMN.deleted_at.is_not_null())
        .one(transaction)
        .await?
        .ok_or(StoreError::RecycleItemNotFound)
}

async fn required_deleted_knowledge_space(
    transaction: &DatabaseTransaction,
    id: Uuid,
) -> Result<knowledge_space::Model, StoreError> {
    knowledge_space::Entity::find_by_id(db_id(id))
        .filter(knowledge_space::Entity::COLUMN.deleted_at.is_not_null())
        .one(transaction)
        .await?
        .ok_or(StoreError::RecycleItemNotFound)
}

async fn live_project(
    transaction: &DatabaseTransaction,
    id: Uuid,
) -> Result<project::Model, StoreError> {
    project::Entity::find_by_id(db_id(id))
        .filter(project::Entity::COLUMN.deleted_at.is_null())
        .one(transaction)
        .await?
        .ok_or(StoreError::RecycleAncestorDeleted("项目".to_owned()))
}

async fn live_version(
    transaction: &DatabaseTransaction,
    id: Uuid,
) -> Result<novel_version::Model, StoreError> {
    novel_version::Entity::find_by_id(db_id(id))
        .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
        .one(transaction)
        .await?
        .ok_or(StoreError::RecycleAncestorDeleted("稿本".to_owned()))
}

async fn live_space(
    transaction: &DatabaseTransaction,
    id: Uuid,
) -> Result<knowledge_space::Model, StoreError> {
    knowledge_space::Entity::find_by_id(db_id(id))
        .filter(knowledge_space::Entity::COLUMN.deleted_at.is_null())
        .one(transaction)
        .await?
        .ok_or(StoreError::RecycleAncestorDeleted("知识 Space".to_owned()))
}

async fn projects_by_id<'a, C, I>(
    connection: &C,
    ids: I,
) -> Result<HashMap<Uuid, project::Model>, StoreError>
where
    C: ConnectionTrait,
    I: IntoIterator<Item = &'a TextUuid>,
{
    let ids: Vec<TextUuid> = ids.into_iter().copied().collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(project::Entity::find()
        .filter(project::Entity::COLUMN.id.is_in(ids))
        .all(connection)
        .await?
        .into_iter()
        .map(|model| (model.id.0, model))
        .collect())
}

async fn versions_by_id<'a, C, I>(
    connection: &C,
    ids: I,
) -> Result<HashMap<Uuid, novel_version::Model>, StoreError>
where
    C: ConnectionTrait,
    I: IntoIterator<Item = &'a TextUuid>,
{
    let ids: Vec<TextUuid> = ids.into_iter().copied().collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(novel_version::Entity::find()
        .filter(novel_version::Entity::COLUMN.id.is_in(ids))
        .all(connection)
        .await?
        .into_iter()
        .map(|model| (model.id.0, model))
        .collect())
}

async fn spaces_by_id<'a, C, I>(
    connection: &C,
    ids: I,
) -> Result<HashMap<Uuid, knowledge_space::Model>, StoreError>
where
    C: ConnectionTrait,
    I: IntoIterator<Item = &'a TextUuid>,
{
    let ids: Vec<TextUuid> = ids.into_iter().copied().collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(knowledge_space::Entity::find()
        .filter(knowledge_space::Entity::COLUMN.id.is_in(ids))
        .all(connection)
        .await?
        .into_iter()
        .map(|model| (model.id.0, model))
        .collect())
}

/// 稿本在回收站里仍然带着多少章节，用来回答「恢复之后是不是空壳」。
/// 项目在回收站列表里的次行：它下面还剩多少稿本与章节。
///
/// 统计不区分这些稿本是否也被单独删除：它们都在同一个项目里，恢复项目会一并可见。
async fn project_detail<C: ConnectionTrait>(
    connection: &C,
    project_id: &TextUuid,
) -> Result<Option<String>, StoreError> {
    let versions = novel_version::Entity::find()
        .filter(novel_version::Entity::COLUMN.project_id.eq(*project_id))
        .all(connection)
        .await?;
    if versions.is_empty() {
        return Ok(Some("还没有稿本".to_owned()));
    }
    let version_ids: Vec<TextUuid> = versions.iter().map(|model| model.id).collect();
    let chapter_count = chapter::Entity::find()
        .filter(chapter::Entity::COLUMN.version_id.is_in(version_ids))
        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
        .all(connection)
        .await?
        .len();
    Ok(Some(format!(
        "{} 份稿本 · {chapter_count} 章",
        versions.len()
    )))
}

/// 项目下是否仍有开放审核或活跃写作任务。
///
/// 与单稿本的检查同一套判定：只要任一稿本还有未结束的审核或顶层 Run，整个项目就不
/// 允许删除——否则一个正在写作的项目会被从用户脚下抽走。
async fn project_has_active_work(
    transaction: &DatabaseTransaction,
    project_id: Uuid,
) -> Result<bool, StoreError> {
    let version_ids: Vec<TextUuid> = novel_version::Entity::find()
        .filter(
            novel_version::Entity::COLUMN
                .project_id
                .eq(db_id(project_id)),
        )
        .all(transaction)
        .await?
        .into_iter()
        .map(|model| model.id)
        .collect();
    if version_ids.is_empty() {
        return Ok(false);
    }
    let review_open = review::Entity::find()
        .filter(review::Entity::COLUMN.version_id.is_in(version_ids.clone()))
        .filter(review::Entity::COLUMN.status.is_in(["open", "delegated"]))
        .one(transaction)
        .await?
        .is_some();
    if review_open {
        return Ok(true);
    }
    Ok(run::Entity::find()
        .filter(run::Entity::COLUMN.version_id.is_in(version_ids))
        .filter(run::Entity::COLUMN.parent_run_id.is_null())
        .filter(
            run::Entity::COLUMN
                .kind
                .is_in(["preparation", "chapter_outline", "chapter_body"]),
        )
        .filter(run::Entity::COLUMN.status.is_in([
            "queued",
            "running",
            "waiting",
            "paused",
            "interrupted",
        ]))
        .one(transaction)
        .await?
        .is_some())
}

async fn chapter_count_detail<C: ConnectionTrait>(
    connection: &C,
    version_id: &TextUuid,
) -> Result<Option<String>, StoreError> {
    let chapters = chapter::Entity::find()
        .filter(chapter::Entity::COLUMN.version_id.eq(*version_id))
        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
        .all(connection)
        .await?;
    if chapters.is_empty() {
        return Ok(Some("还没有章节".to_owned()));
    }
    let approved = chapters
        .iter()
        .filter(|chapter| chapter.status == "approved")
        .count();
    Ok(Some(format!(
        "{} 章（{} 章已定稿）",
        chapters.len(),
        approved
    )))
}

/// 该章节当前正文的非空白字符数；没有正文时为 0。
async fn chapter_word_count<C: ConnectionTrait>(
    connection: &C,
    version_id: Uuid,
    chapter_id: Uuid,
) -> Result<i64, StoreError> {
    let document = document::Entity::find()
        .filter(document::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(
            document::Entity::COLUMN
                .chapter_id
                .eq(chapter_id.to_string()),
        )
        .filter(document::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?;
    let Some(document) = document else {
        return Ok(0);
    };
    let Some(revision_id) = document.current_revision_id else {
        return Ok(0);
    };
    let revision = document_revision::Entity::find_by_id(db_id(parse_id(
        &revision_id,
        "document.current_revision_id",
    )?))
    .one(connection)
    .await?;
    Ok(revision.map_or(0, |revision| {
        revision
            .content
            .chars()
            .filter(|character| !character.is_whitespace())
            .count() as i64
    }))
}
