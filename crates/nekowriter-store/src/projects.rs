//! Projects: create, rename, delete, restore, list, and their settings.
//!
//! Creating a project also creates its first version, which becomes the selected one.

use std::collections::HashMap;

use nekowriter_core::{
    conflict::{Resource, StateConflict},
    event::Event,
    ids::{CommandId, ProjectId, VersionId},
    model::{ModelProfile, ThinkingLevel},
    project::{NovelTitle, ProjectName, WritingLanguage},
    time::Timestamp,
    workflow::WorkflowStage,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Store, StoreError,
    command::{Begun, CommandTx},
    entity::{project, version, workflow},
    versions::create_version_rows,
    workflow::project_has_other_active_run,
};

/// A project as clients see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectView {
    pub id: ProjectId,
    pub name: ProjectName,
    pub writing_language: WritingLanguage,
    /// The project's own model; `None` uses the system default.
    pub model: Option<ModelProfile>,
    pub thinking_level: ThinkingLevel,
    pub selected_version: VersionSummary,
    /// Increases with every change; commands that modify the project must quote it.
    pub revision: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub deleted_at: Option<Timestamp>,
}

/// The essentials of a version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VersionSummary {
    pub id: VersionId,
    /// Position of the version inside its project, starting at 1.
    pub number: i32,
    pub novel_title: NovelTitle,
    pub stage: WorkflowStage,
}

/// Which projects to list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProjectListFilter {
    /// Projects in use, most recently updated first.
    #[default]
    Active,
    /// Deleted projects that can be restored, most recently deleted first.
    Deleted,
}

/// The input of `project.create`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NewProject {
    pub name: ProjectName,
    pub novel_title: NovelTitle,
    pub writing_language: WritingLanguage,
}

/// The input of `project.update_settings`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectSettings {
    pub writing_language: WritingLanguage,
    pub model: Option<ModelProfile>,
    pub thinking_level: ThinkingLevel,
}

impl Store {
    pub async fn list_projects(
        &self,
        filter: ProjectListFilter,
    ) -> Result<Vec<ProjectView>, StoreError> {
        let query = match filter {
            ProjectListFilter::Active => project::Entity::find()
                .filter(project::Column::DeletedAt.is_null())
                .order_by_desc(project::Column::UpdatedAt),
            ProjectListFilter::Deleted => project::Entity::find()
                .filter(project::Column::DeletedAt.is_not_null())
                .order_by_desc(project::Column::DeletedAt),
        };
        let projects = query.all(&self.db).await?;
        let selected: Vec<VersionId> = projects
            .iter()
            .map(|project| project.selected_version_id)
            .collect();
        let mut summaries = version_summaries(&self.db, selected).await?;
        projects
            .into_iter()
            .map(|project| {
                let summary = summaries
                    .remove(&project.selected_version_id)
                    .ok_or_else(|| missing_selected_version(project.id))?;
                Ok(view(project, summary))
            })
            .collect()
    }

    /// One project, unless it does not exist or is deleted.
    pub async fn project(&self, id: ProjectId) -> Result<ProjectView, StoreError> {
        let project = active_project(&self.db, id).await?;
        project_view(&self.db, project).await
    }

    pub async fn create_project(
        &self,
        command: CommandId,
        input: NewProject,
    ) -> Result<ProjectView, StoreError> {
        let mut tx = match self
            .begin_command(command, "project.create", &input)
            .await?
        {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let project_id = ProjectId::generate();
        let version_id = VersionId::generate();
        let now = tx.now();
        let project = project::ActiveModel {
            id: Set(project_id),
            name: Set(input.name),
            writing_language: Set(input.writing_language),
            model: Set(None),
            thinking_level: Set(ThinkingLevel::Off),
            selected_version_id: Set(version_id),
            revision: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        }
        .insert(tx.db())
        .await?;
        create_version_rows(tx.db(), project_id, version_id, 1, input.novel_title, now).await?;
        tx.emit(Event::ProjectChanged { project_id });
        let view = project_view(tx.db(), project).await?;
        tx.commit(view).await
    }

    pub async fn rename_project(
        &self,
        command: CommandId,
        id: ProjectId,
        expected_revision: i64,
        name: ProjectName,
    ) -> Result<ProjectView, StoreError> {
        let input = (id, expected_revision, &name);
        let tx = match self
            .begin_command(command, "project.rename", &input)
            .await?
        {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let project = active_project(tx.db(), id).await?;
        check_revision(Resource::Project, expected_revision, project.revision)?;
        update_project(tx, project, |model, _| model.name = Set(name)).await
    }

    pub async fn update_project_settings(
        &self,
        command: CommandId,
        id: ProjectId,
        expected_revision: i64,
        settings: ProjectSettings,
    ) -> Result<ProjectView, StoreError> {
        let input = (id, expected_revision, &settings);
        let tx = match self
            .begin_command(command, "project.update_settings", &input)
            .await?
        {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let project = active_project(tx.db(), id).await?;
        check_revision(Resource::Project, expected_revision, project.revision)?;
        update_project(tx, project, |model, _| {
            model.writing_language = Set(settings.writing_language);
            model.model = Set(settings.model);
            model.thinking_level = Set(settings.thinking_level);
        })
        .await
    }

    pub async fn delete_project(
        &self,
        command: CommandId,
        id: ProjectId,
        expected_revision: i64,
    ) -> Result<ProjectView, StoreError> {
        let input = (id, expected_revision);
        let tx = match self
            .begin_command(command, "project.delete", &input)
            .await?
        {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let project = active_project(tx.db(), id).await?;
        check_revision(Resource::Project, expected_revision, project.revision)?;
        if project_has_other_active_run(tx.db(), id, None).await? {
            return Err(StoreError::StateConflict(StateConflict::RunActive));
        }
        update_project(tx, project, |model, now| model.deleted_at = Set(Some(now))).await
    }

    pub async fn restore_project(
        &self,
        command: CommandId,
        id: ProjectId,
        expected_revision: i64,
    ) -> Result<ProjectView, StoreError> {
        let input = (id, expected_revision);
        let tx = match self
            .begin_command(command, "project.restore", &input)
            .await?
        {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let project = project::Entity::find_by_id(id)
            .one(tx.db())
            .await?
            .ok_or(StoreError::NotFound(Resource::Project))?;
        if project.deleted_at.is_none() {
            return Err(StoreError::StateConflict(StateConflict::ProjectNotDeleted));
        }
        check_revision(Resource::Project, expected_revision, project.revision)?;
        update_project(tx, project, |model, _| model.deleted_at = Set(None)).await
    }
}

/// Applies `change` to the project, advances its revision, records the event and commits.
pub(crate) async fn update_project(
    mut tx: CommandTx,
    project: project::Model,
    change: impl FnOnce(&mut project::ActiveModel, Timestamp),
) -> Result<ProjectView, StoreError> {
    let now = tx.now();
    let revision = project.revision + 1;
    let mut model = project.into_active_model();
    change(&mut model, now);
    model.revision = Set(revision);
    model.updated_at = Set(now);
    let project = model.update(tx.db()).await?;
    tx.emit(Event::ProjectChanged {
        project_id: project.id,
    });
    let view = project_view(tx.db(), project).await?;
    tx.commit(view).await
}

/// The project, unless it does not exist or is deleted.
pub(crate) async fn active_project<C: ConnectionTrait>(
    db: &C,
    id: ProjectId,
) -> Result<project::Model, StoreError> {
    project::Entity::find_by_id(id)
        .filter(project::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or(StoreError::NotFound(Resource::Project))
}

/// Fails with [`StoreError::RevisionConflict`] unless the client quoted the current
/// revision.
pub(crate) fn check_revision(
    resource: Resource,
    expected: i64,
    actual: i64,
) -> Result<(), StoreError> {
    if expected == actual {
        Ok(())
    } else {
        Err(StoreError::RevisionConflict {
            resource,
            expected,
            actual,
        })
    }
}

/// Summaries of the given versions, keyed by ID.
pub(crate) async fn version_summaries<C: ConnectionTrait>(
    db: &C,
    ids: Vec<VersionId>,
) -> Result<HashMap<VersionId, VersionSummary>, StoreError> {
    let versions = version::Entity::find()
        .filter(version::Column::Id.is_in(ids.clone()))
        .all(db)
        .await?;
    let mut stages: HashMap<VersionId, WorkflowStage> = workflow::Entity::find()
        .filter(workflow::Column::VersionId.is_in(ids))
        .all(db)
        .await?
        .into_iter()
        .map(|workflow| (workflow.version_id, workflow.stage))
        .collect();
    versions
        .into_iter()
        .map(|version| {
            let stage = stages.remove(&version.id).ok_or_else(|| {
                StoreError::Corrupt(format!("version {} has no workflow", version.id))
            })?;
            Ok((
                version.id,
                VersionSummary {
                    id: version.id,
                    number: version.number,
                    novel_title: version.novel_title,
                    stage,
                },
            ))
        })
        .collect()
}

pub(crate) async fn project_view<C: ConnectionTrait>(
    db: &C,
    project: project::Model,
) -> Result<ProjectView, StoreError> {
    let summary = version_summaries(db, vec![project.selected_version_id])
        .await?
        .remove(&project.selected_version_id)
        .ok_or_else(|| missing_selected_version(project.id))?;
    Ok(view(project, summary))
}

fn missing_selected_version(project: ProjectId) -> StoreError {
    StoreError::Corrupt(format!("project {project} has no selected version"))
}

fn view(project: project::Model, selected_version: VersionSummary) -> ProjectView {
    ProjectView {
        id: project.id,
        name: project.name,
        writing_language: project.writing_language,
        model: project.model,
        thinking_level: project.thinking_level,
        selected_version,
        revision: project.revision,
        created_at: project.created_at,
        updated_at: project.updated_at,
        deleted_at: project.deleted_at,
    }
}
