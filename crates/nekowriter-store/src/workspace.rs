//! The workspace view: everything the UI shows for one version (`docs/product-spec.md`
//! §18.2).

use nekowriter_core::{
    chapter::{ChapterKind, ChapterStatus},
    document::DocumentRole,
    ids::{ChapterId, ModelRequestId, ReviewId, RunId, VersionId},
    preparation::{ArtifactKind, ChapterPlan},
    review::ReviewStatus,
    run::{RunHalt, RunKind, RunStatus},
    time::Timestamp,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Store, StoreError,
    documents::{RevisionView, current_revision, version_document},
    entity::{chapter, model_request, review, run, version},
    projects::{ProjectView, VersionSummary, project_view, version_summaries},
    workflow::load_version,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkspaceView {
    pub project: ProjectView,
    pub version: VersionSummary,
    /// Every version of the project that is not deleted, in order.
    pub versions: Vec<VersionSummary>,
    /// The workflow revision; workspace commands quote it.
    pub revision: i64,
    /// The run of the current stage, if one exists.
    pub run: Option<RunView>,
    pub brief: Option<RevisionView>,
    /// The four preparation artifacts, in tab order.
    pub artifacts: Vec<ArtifactView>,
    pub chapters: Vec<ChapterView>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunView {
    pub id: RunId,
    pub kind: RunKind,
    pub status: RunStatus,
    pub halt: Option<RunHalt>,
    pub request_count: i32,
    pub request_allowance_end: i32,
    pub consecutive_protocol_errors: i32,
    /// The request in flight, whose live preview the event stream carries.
    pub request_in_flight: Option<ModelRequestId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArtifactView {
    pub kind: ArtifactKind,
    pub revision: Option<RevisionView>,
    /// The chapter plan, for the outline.
    pub chapter_plan: Option<ChapterPlan>,
    /// The latest review of the artifact.
    pub review: Option<ReviewView>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewView {
    pub id: ReviewId,
    pub status: ReviewStatus,
    /// The review round, counting the agent's submissions from 1.
    pub round: i32,
    pub feedback: Option<String>,
    pub decided_at: Option<Timestamp>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChapterView {
    pub id: ChapterId,
    pub kind: ChapterKind,
    pub number: i32,
    pub title: String,
    pub goal: String,
    pub status: ChapterStatus,
}

impl Store {
    pub async fn workspace(&self, version_id: VersionId) -> Result<WorkspaceView, StoreError> {
        let txn = sea_orm::TransactionTrait::begin(&self.db).await?;
        let view = workspace(&txn, version_id).await?;
        txn.commit().await?;
        Ok(view)
    }
}

/// Reads the view in one transaction, so it is a consistent snapshot.
async fn workspace<C: ConnectionTrait>(
    db: &C,
    version_id: VersionId,
) -> Result<WorkspaceView, StoreError> {
    let context = load_version(db, version_id).await?;
    let version_ids: Vec<VersionId> = version::Entity::find()
        .filter(version::Column::ProjectId.eq(context.project.id))
        .filter(version::Column::DeletedAt.is_null())
        .order_by_asc(version::Column::Number)
        .all(db)
        .await?
        .into_iter()
        .map(|version| version.id)
        .collect();
    let mut summaries = version_summaries(db, version_ids.clone()).await?;
    let versions: Vec<VersionSummary> = version_ids
        .iter()
        .filter_map(|id| summaries.remove(id))
        .collect();
    let version = versions
        .iter()
        .find(|summary| summary.id == version_id)
        .cloned()
        .ok_or_else(|| StoreError::Corrupt(format!("version {version_id} has no summary")))?;

    let run = match context.workflow.active_run_id {
        Some(run_id) => Some(run_view(db, run_id).await?),
        None => None,
    };
    let brief_document = version_document(db, version_id, DocumentRole::NovelBrief).await?;
    let brief = current_revision(db, &brief_document)
        .await?
        .map(|revision| RevisionView::from(&revision));

    let reviews = review::Entity::find()
        .filter(review::Column::VersionId.eq(version_id))
        .order_by_desc(review::Column::CreatedAt)
        .all(db)
        .await?;
    let mut artifacts = Vec::with_capacity(ArtifactKind::ALL.len());
    for kind in ArtifactKind::ALL {
        let document = version_document(db, version_id, kind.into()).await?;
        let current = current_revision(db, &document).await?;
        let review = reviews
            .iter()
            .find(|review| review.kind.artifact() == Some(kind))
            .map(|review| ReviewView {
                id: review.id,
                status: review.status,
                round: review.round,
                feedback: review.feedback.clone(),
                decided_at: review.decided_at,
            });
        artifacts.push(ArtifactView {
            kind,
            revision: current.as_ref().map(RevisionView::from),
            chapter_plan: current.and_then(|revision| revision.chapter_plan),
            review,
        });
    }

    let chapters = chapter::Entity::find()
        .filter(chapter::Column::VersionId.eq(version_id))
        .order_by_asc(chapter::Column::Kind)
        .order_by_asc(chapter::Column::Number)
        .all(db)
        .await?
        .into_iter()
        .map(|chapter| ChapterView {
            id: chapter.id,
            kind: chapter.kind,
            number: chapter.number,
            title: chapter.title,
            goal: chapter.goal,
            status: chapter.status,
        })
        .collect();

    Ok(WorkspaceView {
        project: project_view(db, context.project).await?,
        version,
        versions,
        revision: context.workflow.revision,
        run,
        brief,
        artifacts,
        chapters,
    })
}

async fn run_view<C: ConnectionTrait>(db: &C, run_id: RunId) -> Result<RunView, StoreError> {
    let run = run::Entity::find_by_id(run_id)
        .one(db)
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("the active run {run_id} is missing")))?;
    let request_in_flight = if run.status == RunStatus::Running {
        model_request::Entity::find()
            .filter(model_request::Column::RunId.eq(run_id))
            .filter(model_request::Column::Status.eq(model_request::ModelRequestStatus::Dispatched))
            .one(db)
            .await?
            .map(|request| request.id)
    } else {
        None
    };
    Ok(RunView {
        id: run.id,
        kind: run.kind,
        status: run.status,
        halt: run.halt,
        request_count: run.request_count,
        request_allowance_end: run.request_allowance_end,
        consecutive_protocol_errors: run.consecutive_protocol_errors,
        request_in_flight,
    })
}
