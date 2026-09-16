//! 项目与稿本的读写：查询、创建、切换选中稿本，以及数据库行与域类型之间的转换。
//!
//! 从 `lib.rs` 拆出（P2.2）。`impl Store` 里与项目/稿本有关的方法、三个幂等命令
//! （`create_project` / `create_version` / `select_version`）与它们的 payload、以及
//! `insert_*` / `*_from_model` 这组行↔域转换都在这里；`lib.rs` 用
//! `pub(crate) use projects::{...}` 让兄弟模块原有的 `use super::{...}` 路径保持不变。
//! `Store::open`/`close`/访问器、`CredentialPatch` 与两个设置 helper 留在 `lib.rs`。

use nekowriter_core::{
    ChapterId, DocumentId, NewNovelVersion, NewProject, NovelVersion, PreparationArtifactKind,
    Project, ProjectBundle, ProjectId, ProjectListItem, ProjectProgress, ReviewId, RunId,
    VersionId, VersionSnapshot, WorkflowSnapshot, utc_now_millis,
};
use sea_orm::prelude::TextUuid;
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
    TransactionTrait,
};
use serde::Serialize;
use uuid::Uuid;

use super::entity::{
    self, chapter, command_receipt, document, document_revision, novel_version, project,
    workflow_instance,
};
use super::{
    CHAPTER_BODY_ROLE, CREATE_PROJECT_METHOD, CREATE_PROJECT_TARGET, CREATE_VERSION_METHOD,
    Command, CommandReceipt, NOVEL_BRIEF_ROLE, Notify, SELECT_VERSION_METHOD, Store, StoreError,
    approval_mode_name, db_id, insert_receipt, parse_id, parse_optional_id, payload_hash,
    transaction_error,
};
use super::{event_log, preparation};

impl Store {
    /// 返回某个稿本所属的项目。
    ///
    /// 项目级提示词覆盖以项目为作用域；调度器在构建请求时必须能由稿本解析出它
    /// 所属的项目，才能应用项目覆盖。
    ///
    /// # Errors
    ///
    /// 稿本不存在或已删除时返回 [`StoreError::VersionNotFound`]。
    pub async fn project_id_for_version(
        &self,
        version_id: VersionId,
    ) -> Result<ProjectId, StoreError> {
        novel_version::Entity::find_by_id(db_id(version_id))
            .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
            .one(&self.database)
            .await?
            .map(|model| model.project_id.into())
            .ok_or(StoreError::VersionNotFound)
    }

    /// 返回某个运行所属的项目。
    ///
    /// 命令前置检查需要按项目解析有效模型，而运行行直接记录 `project_id`。
    ///
    /// # Errors
    ///
    /// 运行不存在时返回 [`StoreError::RunNotFound`]。
    pub async fn project_id_for_run(&self, run_id: RunId) -> Result<ProjectId, StoreError> {
        entity::run::Entity::find_by_id(db_id(run_id))
            .one(&self.database)
            .await?
            .map(|model| model.project_id.into())
            .ok_or(StoreError::RunNotFound)
    }

    /// 返回某章所属的项目：先由章节解析稿本，再由稿本解析项目。
    ///
    /// # Errors
    ///
    /// 章节不存在或已删除时返回 [`StoreError::ChapterNotFound`]，稿本不存在时返回
    /// [`StoreError::VersionNotFound`]。
    pub async fn project_id_for_chapter(
        &self,
        chapter_id: ChapterId,
    ) -> Result<ProjectId, StoreError> {
        let version_id = entity::chapter::Entity::find_by_id(db_id(chapter_id))
            .filter(entity::chapter::Entity::COLUMN.deleted_at.is_null())
            .one(&self.database)
            .await?
            .map(|model| model.version_id.into())
            .ok_or(StoreError::ChapterNotFound)?;
        self.project_id_for_version(version_id).await
    }

    /// 返回某次审核所属的项目。
    ///
    /// # Errors
    ///
    /// 审核不存在时返回 [`StoreError::ReviewNotFound`]，稿本不存在时返回
    /// [`StoreError::VersionNotFound`]。
    pub async fn project_id_for_review(
        &self,
        review_id: ReviewId,
    ) -> Result<ProjectId, StoreError> {
        let version_id = entity::review::Entity::find_by_id(db_id(review_id))
            .one(&self.database)
            .await?
            .map(|model| model.version_id.into())
            .ok_or(StoreError::ReviewNotFound)?;
        self.project_id_for_version(version_id).await
    }

    pub async fn list_projects(&self) -> Result<Vec<ProjectListItem>, StoreError> {
        let projects = project::Entity::find()
            .filter(project::Entity::COLUMN.deleted_at.is_null())
            .order_by_desc(project::Entity::COLUMN.updated_at)
            .order_by_asc(project::Entity::COLUMN.id)
            .all(&self.database)
            .await?;
        if projects.is_empty() {
            return Ok(Vec::new());
        }

        let selected_version_ids = projects
            .iter()
            .map(|project| project.selected_version_id)
            .collect::<Vec<_>>();
        let versions = novel_version::Entity::find()
            .filter(novel_version::Entity::COLUMN.id.is_in(selected_version_ids))
            .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
            .all(&self.database)
            .await?;
        let version_ids = versions
            .iter()
            .map(|version| version.id)
            .collect::<Vec<_>>();
        let workflows = workflow_instance::Entity::find()
            .filter(
                workflow_instance::Entity::COLUMN
                    .version_id
                    .is_in(version_ids.clone()),
            )
            .all(&self.database)
            .await?;

        let versions_by_id = versions
            .into_iter()
            .map(|version| (version.id, version))
            .collect::<std::collections::HashMap<_, _>>();
        let workflows_by_version_id = workflows
            .into_iter()
            .map(|workflow| (workflow.version_id, workflow))
            .collect::<std::collections::HashMap<_, _>>();
        let progress_by_version_id = self.project_progress_by_version(&version_ids).await?;

        let mut items = Vec::with_capacity(projects.len());
        for project_model in projects {
            let Some(version_model) = versions_by_id.get(&project_model.selected_version_id) else {
                continue;
            };
            let Some(workflow_model) = workflows_by_version_id.get(&version_model.id) else {
                continue;
            };
            let bundle = project_bundle_from_models(&project_model, version_model, workflow_model)?;
            items.push(ProjectListItem {
                project: bundle.project,
                selected_version: bundle.version,
                workflow: bundle.workflow,
                progress: progress_by_version_id
                    .get(&version_model.id)
                    .cloned()
                    .unwrap_or_default(),
            });
        }
        Ok(items)
    }

    /// 批量回算作品库所需的当前稿本进度；不读取正文内容，也不会为每张卡片单独查库。
    async fn project_progress_by_version(
        &self,
        version_ids: &[TextUuid],
    ) -> Result<std::collections::HashMap<TextUuid, ProjectProgress>, StoreError> {
        let mut progress_by_version_id = version_ids
            .iter()
            .copied()
            .map(|version_id| (version_id, ProjectProgress::default()))
            .collect::<std::collections::HashMap<_, _>>();
        if version_ids.is_empty() {
            return Ok(progress_by_version_id);
        }

        let chapters = chapter::Entity::find()
            .filter(
                chapter::Entity::COLUMN
                    .version_id
                    .is_in(version_ids.to_vec()),
            )
            .filter(chapter::Entity::COLUMN.deleted_at.is_null())
            .all(&self.database)
            .await?;
        for chapter in chapters {
            let progress = progress_by_version_id
                .get_mut(&chapter.version_id)
                .expect("章节查询只会返回当前选中稿本");
            match chapter.kind.as_str() {
                "main" => {
                    progress.main_chapter_total += 1;
                    match chapter.status.as_str() {
                        "approved" => progress.main_chapter_approved += 1,
                        "planned" | "in_progress" => {}
                        other => {
                            return Err(StoreError::CorruptRecord(format!(
                                "未知章节状态：{other}"
                            )));
                        }
                    }
                }
                "extra" => {
                    progress.extra_chapter_total += 1;
                    if !matches!(
                        chapter.status.as_str(),
                        "planned" | "in_progress" | "approved"
                    ) {
                        return Err(StoreError::CorruptRecord(format!(
                            "未知章节状态：{}",
                            chapter.status
                        )));
                    }
                }
                other => {
                    return Err(StoreError::CorruptRecord(format!("未知章节类型：{other}")));
                }
            }
        }

        let body_documents = document::Entity::find()
            .filter(
                document::Entity::COLUMN
                    .version_id
                    .is_in(version_ids.to_vec()),
            )
            .filter(document::Entity::COLUMN.deleted_at.is_null())
            .filter(document::Entity::COLUMN.role.eq(CHAPTER_BODY_ROLE))
            .all(&self.database)
            .await?;
        let current_revision_ids = body_documents
            .iter()
            .filter_map(|document| document.current_revision_id.as_deref())
            .map(|revision_id| parse_id(revision_id, "document.current_revision_id"))
            .collect::<Result<Vec<_>, _>>()?;
        let revisions = document_revision::Entity::find()
            .filter(
                document_revision::Entity::COLUMN.id.is_in(
                    current_revision_ids
                        .into_iter()
                        .map(db_id)
                        .collect::<Vec<_>>(),
                ),
            )
            .all(&self.database)
            .await?;
        let word_count_by_revision_id = revisions
            .into_iter()
            .map(|revision| (revision.id, revision.word_count))
            .collect::<std::collections::HashMap<_, _>>();
        for document in body_documents {
            let Some(revision_id) = document.current_revision_id.as_deref() else {
                continue;
            };
            let revision_id = db_id(parse_id(revision_id, "document.current_revision_id")?);
            let word_count = word_count_by_revision_id
                .get(&revision_id)
                .copied()
                .ok_or_else(|| {
                    StoreError::CorruptRecord("正文槽指向不存在的当前修订".to_owned())
                })?;
            let progress = progress_by_version_id
                .get_mut(&document.version_id)
                .expect("正文槽查询只会返回当前选中稿本");
            progress.current_word_count += word_count;
        }

        Ok(progress_by_version_id)
    }

    pub async fn get_project(&self, project_id: ProjectId) -> Result<ProjectBundle, StoreError> {
        let project_model = project::Entity::find_by_id(db_id(project_id))
            .filter(project::Entity::COLUMN.deleted_at.is_null())
            .one(&self.database)
            .await?
            .ok_or(StoreError::ProjectNotFound)?;
        let version_model = novel_version::Entity::find_by_id(project_model.selected_version_id)
            .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
            .one(&self.database)
            .await?
            .ok_or(StoreError::ProjectNotFound)?;
        let workflow_model = workflow_instance::Entity::find_by_id(version_model.id)
            .one(&self.database)
            .await?
            .ok_or(StoreError::ProjectNotFound)?;
        project_bundle_from_models(&project_model, &version_model, &workflow_model)
    }

    pub async fn list_versions(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<NovelVersion>, StoreError> {
        if !self.project_exists(project_id).await? {
            return Err(StoreError::ProjectNotFound);
        }
        novel_version::Entity::find()
            .filter(
                novel_version::Entity::COLUMN
                    .project_id
                    .eq(db_id(project_id)),
            )
            .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
            .order_by_asc(novel_version::Entity::COLUMN.created_at)
            .order_by_asc(novel_version::Entity::COLUMN.id)
            .all(&self.database)
            .await?
            .iter()
            .map(version_from_model)
            .collect::<Result<Vec<_>, _>>()
    }

    pub async fn get_version_snapshot(
        &self,
        version_id: VersionId,
    ) -> Result<VersionSnapshot, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move { version_snapshot(transaction, version_id).await })
            })
            .await
            .map_err(transaction_error)
    }

    pub async fn create_project(
        &self,
        command_id: Uuid,
        input: NewProject,
    ) -> Result<ProjectBundle, StoreError> {
        let payload_hash = payload_hash(&CreateProjectPayload::from(&input))?;
        let command = Command::new(
            command_id,
            CREATE_PROJECT_METHOD,
            CREATE_PROJECT_TARGET,
            &payload_hash,
        );

        let now = utc_now_millis();
        let project_id = Uuid::new_v4();
        let version_id = Uuid::new_v4();
        let workflow = WorkflowSnapshot::initial(version_id);
        let project = Project {
            id: project_id,
            name: input.name,
            selected_version_id: version_id,
            revision: 1,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let version = NovelVersion {
            id: version_id,
            project_id,
            label: input.version_label,
            novel_title: input.novel_title,
            source_version_id: None,
            body_completed_at_ms: None,
            revision: 1,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let bundle = ProjectBundle {
            project,
            version,
            workflow,
        };

        let bundle_for_transaction = bundle.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    insert_project_bundle(transaction, &bundle_for_transaction).await?;
                    preparation::insert_app_event(
                        transaction,
                        "project.changed",
                        bundle_for_transaction.project.id,
                        bundle_for_transaction.project.revision,
                        serde_json::json!({
                            "action": "created",
                            "project_id": bundle_for_transaction.project.id,
                            "version_id": bundle_for_transaction.version.id,
                        }),
                        now,
                    )
                    .await?;
                    let response_json = serde_json::to_string(&bundle_for_transaction)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        CREATE_PROJECT_METHOD,
                        CREATE_PROJECT_TARGET,
                        &payload_hash_for_transaction,
                        &response_json,
                        201,
                        now,
                    )
                    .await?;
                    Ok(bundle_for_transaction)
                })
            })
            .await
    }

    pub async fn create_version(
        &self,
        project_id: ProjectId,
        command_id: Uuid,
        input: NewNovelVersion,
    ) -> Result<ProjectBundle, StoreError> {
        let target = format!("/api/projects/{project_id}/versions");
        let payload_hash = payload_hash(&CreateVersionPayload::from(&input))?;
        let command = Command::new(command_id, CREATE_VERSION_METHOD, &target, &payload_hash);

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let project_model = project::Entity::find_by_id(db_id(project_id))
                        .filter(project::Entity::COLUMN.deleted_at.is_null())
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ProjectNotFound)?;
                    let mut project = project_from_model(&project_model);
                    let now = utc_now_millis();
                    let version_id = Uuid::new_v4();
                    let workflow = WorkflowSnapshot::initial(version_id);
                    let version = NovelVersion {
                        id: version_id,
                        project_id,
                        label: input.label,
                        novel_title: input.novel_title,
                        source_version_id: None,
                        body_completed_at_ms: None,
                        revision: 1,
                        created_at_ms: now,
                        updated_at_ms: now,
                    };
                    project.selected_version_id = version_id;
                    project.revision += 1;
                    project.updated_at_ms = now;

                    insert_novel_version(transaction, &version).await?;
                    insert_workflow(transaction, &workflow).await?;
                    insert_document_slots(transaction, version_id, now).await?;
                    let mut active_project = project_model.into_active_model();
                    active_project.selected_version_id = Set(db_id(project.selected_version_id));
                    active_project.revision = Set(project.revision);
                    active_project.updated_at = Set(project.updated_at_ms);
                    active_project.update(transaction).await?;

                    let bundle = ProjectBundle {
                        project,
                        version,
                        workflow,
                    };
                    preparation::insert_app_event(
                        transaction,
                        "project.changed",
                        bundle.project.id,
                        bundle.project.revision,
                        serde_json::json!({
                            "action": "version_created",
                            "project_id": bundle.project.id,
                            "version_id": bundle.version.id,
                        }),
                        now,
                    )
                    .await?;
                    let response_json = serde_json::to_string(&bundle)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        CREATE_VERSION_METHOD,
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
    }

    /// 切换项目工作台当前选中的稿本。
    pub async fn select_version(
        &self,
        project_id: ProjectId,
        version_id: VersionId,
        command_id: Uuid,
        expected_project_revision: i64,
    ) -> Result<ProjectBundle, StoreError> {
        let target = format!("/api/projects/{project_id}/selected-version");
        let payload_hash = payload_hash(&SelectVersionPayload {
            version_id,
            expected_project_revision,
        })?;
        let command = Command::new(command_id, SELECT_VERSION_METHOD, &target, &payload_hash);

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let project_model = project::Entity::find_by_id(db_id(project_id))
                        .filter(project::Entity::COLUMN.deleted_at.is_null())
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ProjectNotFound)?;
                    if project_model.revision != expected_project_revision {
                        return Err(StoreError::StateRevisionConflict {
                            expected: expected_project_revision,
                            actual: project_model.revision,
                        });
                    }
                    let version_model = novel_version::Entity::find_by_id(db_id(version_id))
                        .filter(
                            novel_version::Entity::COLUMN
                                .project_id
                                .eq(db_id(project_id)),
                        )
                        .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::VersionNotFound)?;
                    let workflow_model = workflow_instance::Entity::find_by_id(db_id(version_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::VersionNotFound)?;
                    let now = utc_now_millis();
                    let mut active_project = project_model.into_active_model();
                    active_project.selected_version_id = Set(db_id(version_id));
                    active_project.revision = Set(expected_project_revision + 1);
                    active_project.updated_at = Set(now);
                    let updated_project = active_project.update(transaction).await?;
                    let bundle = project_bundle_from_models(
                        &updated_project,
                        &version_model,
                        &workflow_model,
                    )?;
                    preparation::insert_app_event(
                        transaction,
                        "project.changed",
                        bundle.project.id,
                        bundle.project.revision,
                        serde_json::json!({
                            "action": "selected_version_changed",
                            "project_id": bundle.project.id,
                            "version_id": bundle.version.id,
                        }),
                        now,
                    )
                    .await?;
                    let response_json = serde_json::to_string(&bundle)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SELECT_VERSION_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(bundle)
                })
            })
            .await
    }

    async fn project_exists(&self, project_id: ProjectId) -> Result<bool, StoreError> {
        Ok(project::Entity::find_by_id(db_id(project_id))
            .filter(project::Entity::COLUMN.deleted_at.is_null())
            .one(&self.database)
            .await?
            .is_some())
    }

    pub(crate) async fn find_receipt(
        &self,
        command_id: Uuid,
    ) -> Result<Option<CommandReceipt>, StoreError> {
        let receipt = command_receipt::Entity::find_by_id(db_id(command_id))
            .one(&self.database)
            .await?;
        Ok(receipt.map(|receipt| CommandReceipt {
            method: receipt.method,
            target: receipt.target,
            payload_hash: receipt.payload_hash,
            response_json: receipt.response_json,
        }))
    }
}

async fn version_snapshot<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<VersionSnapshot, StoreError> {
    let version_model = novel_version::Entity::find_by_id(db_id(version_id))
        .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .ok_or(StoreError::VersionNotFound)?;
    let project_model = project::Entity::find_by_id(version_model.project_id)
        .filter(project::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .ok_or(StoreError::VersionNotFound)?;
    let workflow_model = workflow_instance::Entity::find_by_id(version_model.id)
        .one(connection)
        .await?
        .ok_or(StoreError::VersionNotFound)?;
    let bundle = project_bundle_from_models(&project_model, &version_model, &workflow_model)?;
    let watermark = event_log::retained_event_watermark(connection).await?;

    Ok(VersionSnapshot {
        project: bundle.project,
        version: bundle.version,
        workflow: bundle.workflow,
        event_watermark: watermark.to_string(),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateProjectPayload<'a> {
    name: &'a str,
    novel_title: &'a str,
    version_label: &'a str,
}

impl<'a> From<&'a NewProject> for CreateProjectPayload<'a> {
    fn from(value: &'a NewProject) -> Self {
        Self {
            name: &value.name,
            novel_title: &value.novel_title,
            version_label: &value.version_label,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateVersionPayload<'a> {
    label: &'a str,
    novel_title: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SelectVersionPayload {
    version_id: VersionId,
    expected_project_revision: i64,
}

impl<'a> From<&'a NewNovelVersion> for CreateVersionPayload<'a> {
    fn from(value: &'a NewNovelVersion) -> Self {
        Self {
            label: &value.label,
            novel_title: &value.novel_title,
        }
    }
}

async fn insert_project_bundle<C: ConnectionTrait>(
    connection: &C,
    bundle: &ProjectBundle,
) -> Result<(), StoreError> {
    project::ActiveModel {
        id: Set(db_id(bundle.project.id)),
        name: Set(bundle.project.name.clone()),
        selected_version_id: Set(db_id(bundle.project.selected_version_id)),
        settings_json: Set("{}".to_owned()),
        settings_revision: Set(0),
        revision: Set(bundle.project.revision),
        created_at: Set(bundle.project.created_at_ms),
        updated_at: Set(bundle.project.updated_at_ms),
        deleted_at: Set(None),
    }
    .insert(connection)
    .await?;
    insert_novel_version(connection, &bundle.version).await?;
    insert_workflow(connection, &bundle.workflow).await?;
    insert_document_slots(connection, bundle.version.id, bundle.version.created_at_ms).await
}

pub(crate) async fn insert_novel_version<C: ConnectionTrait>(
    connection: &C,
    version: &NovelVersion,
) -> Result<(), StoreError> {
    novel_version::ActiveModel {
        id: Set(db_id(version.id)),
        project_id: Set(db_id(version.project_id)),
        label: Set(version.label.clone()),
        novel_title: Set(version.novel_title.clone()),
        source_version_id: Set(version.source_version_id.map(|id| id.to_string())),
        body_completed_at: Set(version.body_completed_at_ms),
        revision: Set(version.revision),
        created_at: Set(version.created_at_ms),
        updated_at: Set(version.updated_at_ms),
        deleted_at: Set(None),
    }
    .insert(connection)
    .await?;
    Ok(())
}

pub(crate) async fn insert_document_slots<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
    created_at_ms: i64,
) -> Result<(), StoreError> {
    let roles = std::iter::once(NOVEL_BRIEF_ROLE).chain(
        PreparationArtifactKind::ALL
            .into_iter()
            .map(PreparationArtifactKind::stable_key),
    );
    for role in roles {
        document::ActiveModel {
            id: Set(db_id(DocumentId::new_v4())),
            version_id: Set(db_id(version_id)),
            chapter_id: Set(None),
            role: Set(role.to_owned()),
            current_revision_id: Set(None),
            revision: Set(0),
            created_at: Set(created_at_ms),
            updated_at: Set(created_at_ms),
            deleted_at: Set(None),
        }
        .insert(connection)
        .await?;
    }
    Ok(())
}

pub(crate) async fn insert_workflow<C: ConnectionTrait>(
    connection: &C,
    workflow: &WorkflowSnapshot,
) -> Result<(), StoreError> {
    workflow_instance::ActiveModel {
        version_id: Set(db_id(workflow.version_id)),
        state_json: Set(serde_json::to_string(workflow)?),
        state_revision: Set(workflow.state_revision),
        active_run_id: Set(workflow.active_run_id.map(|run_id| run_id.to_string())),
        approval_mode: Set(approval_mode_name(workflow.approval_mode).to_owned()),
        pause_requested: Set(workflow.pause_requested),
    }
    .insert(connection)
    .await?;
    Ok(())
}

fn project_bundle_from_models(
    project_model: &project::Model,
    version_model: &novel_version::Model,
    workflow_model: &workflow_instance::Model,
) -> Result<ProjectBundle, StoreError> {
    let project = project_from_model(project_model);
    let version = version_from_model(version_model)?;
    let workflow = workflow_from_model(workflow_model)?;
    // `selected_version_id` 只是项目工作台当前展示的稿本，并不限定可读取的
    // 稿本；运行中的旧稿必须仍能单独取得其工作流快照。
    if workflow.version_id != version.id || version.project_id != project.id {
        return Err(StoreError::CorruptRecord(
            "项目、稿本和工作流的归属不一致".to_owned(),
        ));
    }
    Ok(ProjectBundle {
        project,
        version,
        workflow,
    })
}

fn version_from_model(model: &novel_version::Model) -> Result<NovelVersion, StoreError> {
    Ok(NovelVersion {
        id: model.id.into(),
        project_id: model.project_id.into(),
        label: model.label.clone(),
        novel_title: model.novel_title.clone(),
        source_version_id: parse_optional_id(
            model.source_version_id.as_deref(),
            "source_version_id",
        )?,
        body_completed_at_ms: model.body_completed_at,
        revision: model.revision,
        created_at_ms: model.created_at,
        updated_at_ms: model.updated_at,
    })
}

pub(crate) fn project_from_model(model: &project::Model) -> Project {
    Project {
        id: model.id.into(),
        name: model.name.clone(),
        selected_version_id: model.selected_version_id.into(),
        revision: model.revision,
        created_at_ms: model.created_at,
        updated_at_ms: model.updated_at,
    }
}

pub(crate) fn workflow_from_model(
    model: &workflow_instance::Model,
) -> Result<WorkflowSnapshot, StoreError> {
    let workflow = serde_json::from_str::<WorkflowSnapshot>(&model.state_json)?;
    let version_id: Uuid = model.version_id.into();
    let active_run_id = parse_optional_id(model.active_run_id.as_deref(), "active_run_id")?;
    if workflow.version_id != version_id || workflow.active_run_id != active_run_id {
        return Err(StoreError::CorruptRecord(
            "工作流与稿本的归属不一致".to_owned(),
        ));
    }
    Ok(workflow)
}
