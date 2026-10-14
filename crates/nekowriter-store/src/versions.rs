//! Versions: independent manuscripts inside a project (`docs/product-spec.md` §14).
//!
//! A new version is empty: a workflow at `preparation.not_started` and empty documents for
//! the brief and the four artifacts.

use nekowriter_core::{
    conflict::{Resource, StateConflict},
    document::DocumentRole,
    ids::{CommandId, DocumentId, ProjectId, VersionId},
    project::NovelTitle,
    time::Timestamp,
    workflow::{ApprovalMode, WorkflowStage},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, QuerySelect, Set,
};

use crate::{
    Store, StoreError,
    command::Begun,
    entity::{document, version, workflow},
    projects::{ProjectView, active_project, check_revision, update_project},
    workflow::version_has_active_run,
};

/// Inserts a version with its workflow and documents.
pub(crate) async fn create_version_rows<C: ConnectionTrait>(
    db: &C,
    project_id: ProjectId,
    version_id: VersionId,
    number: i32,
    novel_title: NovelTitle,
    now: Timestamp,
) -> Result<version::Model, StoreError> {
    let version = version::ActiveModel {
        id: Set(version_id),
        project_id: Set(project_id),
        number: Set(number),
        novel_title: Set(novel_title),
        revision: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
        deleted_at: Set(None),
    }
    .insert(db)
    .await?;
    workflow::ActiveModel {
        version_id: Set(version.id),
        stage: Set(WorkflowStage::NEW_VERSION),
        active_run_id: Set(None),
        revision: Set(1),
        completed_at: Set(None),
        pause_requested: Set(false),
        approval_mode: Set(ApprovalMode::AutoApprove),
        updated_at: Set(now),
    }
    .insert(db)
    .await?;
    for role in DocumentRole::VERSION_ROLES {
        document::ActiveModel {
            id: Set(DocumentId::generate()),
            version_id: Set(version.id),
            chapter_id: Set(None),
            role: Set(role),
            current_revision_id: Set(None),
        }
        .insert(db)
        .await?;
    }
    Ok(version)
}

impl Store {
    /// Creates the project's next version and selects it.
    pub async fn create_version(
        &self,
        command: CommandId,
        project_id: ProjectId,
        expected_revision: i64,
        novel_title: NovelTitle,
    ) -> Result<ProjectView, StoreError> {
        let input = (project_id, expected_revision, &novel_title);
        let tx = match self
            .begin_command(command, "version.create", &input)
            .await?
        {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let project = active_project(tx.db(), project_id).await?;
        check_revision(Resource::Project, expected_revision, project.revision)?;
        let last_number: Option<i32> = version::Entity::find()
            .select_only()
            .column(version::Column::Number)
            .filter(version::Column::ProjectId.eq(project_id))
            .order_by_desc(version::Column::Number)
            .into_tuple()
            .one(tx.db())
            .await?;
        let version = create_version_rows(
            tx.db(),
            project_id,
            VersionId::generate(),
            last_number.unwrap_or(0) + 1,
            novel_title,
            tx.now(),
        )
        .await?;
        update_project(tx, project, |model, _| {
            model.selected_version_id = Set(version.id);
        })
        .await
    }

    /// Deletes a version that the work does not show and that nothing is active in (§14).
    pub async fn delete_version(
        &self,
        command: CommandId,
        project_id: ProjectId,
        expected_revision: i64,
        version_id: VersionId,
    ) -> Result<ProjectView, StoreError> {
        let input = (project_id, expected_revision, version_id);
        let tx = match self
            .begin_command(command, "version.delete", &input)
            .await?
        {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let project = active_project(tx.db(), project_id).await?;
        check_revision(Resource::Project, expected_revision, project.revision)?;
        let version = version::Entity::find_by_id(version_id)
            .filter(version::Column::ProjectId.eq(project_id))
            .filter(version::Column::DeletedAt.is_null())
            .one(tx.db())
            .await?
            .ok_or(StoreError::NotFound(Resource::Version))?;
        if project.selected_version_id == version_id {
            return Err(StoreError::StateConflict(StateConflict::VersionSelected));
        }
        if version_has_active_run(tx.db(), version_id).await? {
            return Err(StoreError::StateConflict(StateConflict::RunActive));
        }
        let now = tx.now();
        let mut deleted = version.into_active_model();
        deleted.deleted_at = Set(Some(now));
        deleted.update(tx.db()).await?;
        update_project(tx, project, |_, _| {}).await
    }

    /// Makes `version_id` the version the workspace shows. It starts nothing.
    pub async fn select_version(
        &self,
        command: CommandId,
        project_id: ProjectId,
        expected_revision: i64,
        version_id: VersionId,
    ) -> Result<ProjectView, StoreError> {
        let input = (project_id, expected_revision, version_id);
        let tx = match self
            .begin_command(command, "version.select", &input)
            .await?
        {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let project = active_project(tx.db(), project_id).await?;
        check_revision(Resource::Project, expected_revision, project.revision)?;
        version::Entity::find_by_id(version_id)
            .filter(version::Column::ProjectId.eq(project_id))
            .filter(version::Column::DeletedAt.is_null())
            .one(tx.db())
            .await?
            .ok_or(StoreError::NotFound(Resource::Version))?;
        update_project(tx, project, |model, _| {
            model.selected_version_id = Set(version_id);
        })
        .await
    }
}
