//! Helpers for commands on a version's workspace: loading the version with its project and
//! workflow, and advancing the workflow.

use nekowriter_core::{
    conflict::Resource,
    event::Event,
    ids::{ProjectId, RunId, VersionId},
    workflow::WorkflowStage,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    StoreError,
    command::WriteTx,
    entity::{project, version, workflow},
    projects::check_revision,
};

/// What a workspace command returns: the workflow revision it produced. Clients reload the
/// workspace through the event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkspaceAck {
    pub revision: i64,
}

/// A version that is not deleted, in a project that is not deleted.
pub(crate) struct VersionContext {
    pub(crate) project: project::Model,
    pub(crate) version: version::Model,
    pub(crate) workflow: workflow::Model,
}

pub(crate) async fn load_version<C: ConnectionTrait>(
    db: &C,
    version_id: VersionId,
) -> Result<VersionContext, StoreError> {
    let not_found = || StoreError::NotFound(Resource::Version);
    let version = version::Entity::find_by_id(version_id)
        .filter(version::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or_else(not_found)?;
    let project = project::Entity::find_by_id(version.project_id)
        .filter(project::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or_else(not_found)?;
    let workflow = workflow::Entity::find_by_id(version_id)
        .one(db)
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("version {version_id} has no workflow")))?;
    Ok(VersionContext {
        project,
        version,
        workflow,
    })
}

impl VersionContext {
    /// Fails unless the client quoted the workflow's current revision.
    pub(crate) fn check_revision(&self, expected: i64) -> Result<(), StoreError> {
        check_revision(Resource::Workspace, expected, self.workflow.revision)
    }
}

/// Moves the workflow to `stage` with `active_run`, advances its revision and records the
/// change.
pub(crate) async fn advance(
    tx: &mut WriteTx,
    workflow: workflow::Model,
    project_id: ProjectId,
    stage: WorkflowStage,
    active_run: Option<RunId>,
) -> Result<workflow::Model, StoreError> {
    let revision = workflow.revision + 1;
    let version_id = workflow.version_id;
    let mut model = workflow.into_active_model();
    model.stage = Set(stage);
    model.active_run_id = Set(active_run);
    model.revision = Set(revision);
    model.updated_at = Set(tx.now());
    let workflow = model.update(tx.db()).await?;
    tx.emit(version_changed(project_id, version_id));
    Ok(workflow)
}

/// Records a change inside a version without moving its workflow (for example a run's
/// progress), still advancing the workflow revision so stale commands are refused.
pub(crate) async fn touch(
    tx: &mut WriteTx,
    version_id: VersionId,
    project_id: ProjectId,
) -> Result<workflow::Model, StoreError> {
    let workflow = workflow::Entity::find_by_id(version_id)
        .one(tx.db())
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("version {version_id} has no workflow")))?;
    let stage = workflow.stage.clone();
    let active_run = workflow.active_run_id;
    advance(tx, workflow, project_id, stage, active_run).await
}

pub(crate) fn version_changed(project_id: ProjectId, version_id: VersionId) -> Event {
    Event::VersionChanged {
        project_id,
        version_id,
    }
}

/// Whether another writer run of the project holds its single writing slot (§4
/// invariant 2). Review runs decide a writer's review and hold no slot of their own.
pub(crate) async fn project_has_other_active_run<C: ConnectionTrait>(
    db: &C,
    project_id: ProjectId,
    except: Option<RunId>,
) -> Result<bool, StoreError> {
    use nekowriter_core::run::{RunKind, RunStatus};
    use sea_orm::Iterable;

    use crate::entity::run;

    let active: Vec<RunStatus> = RunStatus::iter()
        .filter(|status| status.is_active())
        .collect();
    let mut query = run::Entity::find()
        .filter(run::Column::ProjectId.eq(project_id))
        .filter(run::Column::Kind.ne(RunKind::Review))
        .filter(run::Column::Status.is_in(active));
    if let Some(except) = except {
        query = query.filter(run::Column::Id.ne(except));
    }
    Ok(query.one(db).await?.is_some())
}

/// Whether the version has a writing run that is working, waiting or interrupted.
pub(crate) async fn version_has_active_run<C: ConnectionTrait>(
    db: &C,
    version_id: VersionId,
) -> Result<bool, StoreError> {
    use nekowriter_core::run::{RunKind, RunStatus};
    use sea_orm::Iterable;

    use crate::entity::run;

    let active: Vec<RunStatus> = RunStatus::iter()
        .filter(|status| status.is_active())
        .collect();
    Ok(run::Entity::find()
        .filter(run::Column::VersionId.eq(version_id))
        .filter(run::Column::Kind.ne(RunKind::Review))
        .filter(run::Column::Status.is_in(active))
        .one(db)
        .await?
        .is_some())
}
