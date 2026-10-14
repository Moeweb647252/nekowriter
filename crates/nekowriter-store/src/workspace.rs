//! The workspace view: everything the UI shows for one version (`docs/product-spec.md`
//! §18.2). Documents appear as revision summaries; their content is loaded by revision ID.

use std::collections::HashMap;

use nekowriter_core::{
    chapter::{ChapterKind, ChapterStatus},
    citation::Citation,
    document::DocumentRole,
    ids::{ChapterId, ModelRequestId, ReviewId, RevisionId, RunId, VersionId},
    preparation::ArtifactKind,
    review::{DecisionSource, ReviewKind, ReviewStatus},
    run::{RunHalt, RunKind, RunStatus},
    time::Timestamp,
    workflow::ApprovalMode,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Store, StoreError,
    documents::{RevisionSummary, RevisionView, current_revision, version_document},
    entity::{chapter, document, model_request, review, revision, run, version},
    feedback::{FeedbackView, HeldCallView, feedback_views, held_view},
    projects::{ProjectView, VersionSummary, project_view, version_summaries},
    proposals::{ProposalView, proposal_view},
    questions::{QuestionView, question_view},
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
    /// When the user first confirmed the novel complete.
    pub completed_at: Option<Timestamp>,
    /// The run of the current stage, if one exists.
    pub run: Option<RunView>,
    /// The user asked the run to pause at its next tool call (§9.3).
    pub pause_requested: bool,
    /// The run's feedback the dock shows (§9.2).
    pub feedback: Vec<FeedbackView>,
    /// The call the run holds while it is paused.
    pub held: Option<HeldCallView>,
    /// Whether body edits apply at once or wait for review (§9.5).
    pub approval_mode: ApprovalMode,
    /// The run's proposed edit waiting for review.
    pub proposal: Option<ProposalView>,
    /// The questions the run waits for answers to.
    pub question: Option<QuestionView>,
    /// The novel brief, with its content: it is edited in the workspace.
    pub brief: Option<RevisionView>,
    /// The four preparation artifacts, in tab order.
    pub artifacts: Vec<ArtifactView>,
    /// The main chapters in plan order, then the extra chapters in order.
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
    pub revision: Option<RevisionSummary>,
    /// The latest review of the artifact.
    pub review: Option<ReviewView>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewView {
    pub id: ReviewId,
    pub kind: ReviewKind,
    pub status: ReviewStatus,
    /// The review round, counting the writer's submissions from 1.
    pub round: i32,
    /// The revision under review; absent for an edit review.
    pub target: Option<RevisionId>,
    /// What the writer says a submitted chapter contains.
    pub summary: Option<String>,
    /// The reviewer's feedback on a rejection; a review agent may reject without issues.
    pub feedback: Option<String>,
    /// The passages a rejection quotes.
    pub citations: Vec<Citation>,
    pub decided_at: Option<Timestamp>,
    pub decided_by: Option<DecisionSource>,
    /// A review agent's confidence in thousandths; shown only (D08).
    pub confidence_permille: Option<i32>,
    /// Why the last delegation failed, until the review is delegated again or decided.
    pub delegation_failure: Option<RunHalt>,
}

impl From<&review::Model> for ReviewView {
    fn from(review: &review::Model) -> Self {
        Self {
            id: review.id,
            kind: review.kind,
            status: review.status,
            round: review.round,
            target: review.target_revision_id,
            summary: review.summary.clone(),
            feedback: review.feedback.clone(),
            citations: review
                .citations
                .as_ref()
                .map_or_else(Vec::new, |citations| {
                    citations
                        .0
                        .iter()
                        .map(|cited| cited.citation.clone())
                        .collect()
                }),
            decided_at: review.decided_at,
            decided_by: review.decided_by,
            confidence_permille: review.confidence_permille,
            delegation_failure: review.delegation_failure.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChapterView {
    pub id: ChapterId,
    pub kind: ChapterKind,
    /// The chapter's number among the chapters of its kind.
    pub number: i32,
    pub title: String,
    /// The plan item's goal; empty for extra chapters.
    pub goal: String,
    pub status: ChapterStatus,
    pub guidance: Option<RevisionSummary>,
    pub outline: Option<RevisionSummary>,
    pub body: Option<RevisionSummary>,
    /// The latest outline or whole-chapter review.
    pub review: Option<ReviewView>,
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
    let (version, versions) = versions(db, context.project.id, version_id).await?;
    let (run, feedback, held, proposal, question) = match context.workflow.active_run_id {
        Some(run_id) => (
            Some(run_view(db, run_id).await?),
            feedback_views(db, run_id).await?,
            held_view(db, run_id).await?,
            proposal_view(db, run_id).await?,
            question_view(db, run_id).await?,
        ),
        None => (None, Vec::new(), None, None, None),
    };
    let brief_document = version_document(db, version_id, DocumentRole::NovelBrief).await?;
    let brief = current_revision(db, &brief_document)
        .await?
        .map(RevisionView::from);

    let documents = document::Entity::find()
        .filter(document::Column::VersionId.eq(version_id))
        .all(db)
        .await?;
    let summaries = revision_summaries(
        db,
        documents
            .iter()
            .filter_map(|document| document.current_revision_id)
            .collect(),
    )
    .await?;
    let current = |chapter: Option<ChapterId>, role: DocumentRole| {
        documents
            .iter()
            .find(|document| document.chapter_id == chapter && document.role == role)
            .and_then(|document| document.current_revision_id)
            .and_then(|id| summaries.get(&id).copied())
    };
    let reviews = review::Entity::find()
        .filter(review::Column::VersionId.eq(version_id))
        .order_by_desc(review::Column::CreatedAt)
        .all(db)
        .await?;

    let artifacts = ArtifactKind::ALL
        .into_iter()
        .map(|kind| ArtifactView {
            kind,
            revision: current(None, kind.into()),
            review: reviews
                .iter()
                .find(|review| review.kind.artifact() == Some(kind))
                .map(ReviewView::from),
        })
        .collect();
    let mut chapters = chapter::Entity::find()
        .filter(chapter::Column::VersionId.eq(version_id))
        .filter(chapter::Column::DeletedAt.is_null())
        .all(db)
        .await?;
    chapters.sort_by_key(|chapter| (chapter.kind == ChapterKind::Extra, chapter.number));
    let chapters = chapters
        .into_iter()
        .map(|chapter| ChapterView {
            guidance: current(Some(chapter.id), DocumentRole::ChapterGuidance),
            outline: current(Some(chapter.id), DocumentRole::ChapterOutline),
            body: current(Some(chapter.id), DocumentRole::ChapterBody),
            // Edit reviews are shown as the run's proposal.
            review: reviews
                .iter()
                .find(|review| {
                    review.chapter_id == Some(chapter.id) && review.kind != ReviewKind::ChapterEdit
                })
                .map(ReviewView::from),
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
        completed_at: context.workflow.completed_at,
        pause_requested: context.workflow.pause_requested,
        run,
        feedback,
        held,
        approval_mode: context.workflow.approval_mode,
        proposal,
        question,
        brief,
        artifacts,
        chapters,
    })
}

/// The version's summary and those of all the project's versions, in order.
async fn versions<C: ConnectionTrait>(
    db: &C,
    project_id: nekowriter_core::ids::ProjectId,
    version_id: VersionId,
) -> Result<(VersionSummary, Vec<VersionSummary>), StoreError> {
    let version_ids: Vec<VersionId> = version::Entity::find()
        .filter(version::Column::ProjectId.eq(project_id))
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
    Ok((version, versions))
}

/// Summaries of the given revisions, read without their content.
async fn revision_summaries<C: ConnectionTrait>(
    db: &C,
    ids: Vec<RevisionId>,
) -> Result<HashMap<RevisionId, RevisionSummary>, StoreError> {
    let rows: Vec<(RevisionId, i32, Option<i32>, Timestamp)> = revision::Entity::find()
        .select_only()
        .columns([
            revision::Column::Id,
            revision::Column::Number,
            revision::Column::WordCount,
            revision::Column::CreatedAt,
        ])
        .filter(revision::Column::Id.is_in(ids))
        .into_tuple()
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(id, number, word_count, created_at)| {
            (
                id,
                RevisionSummary {
                    id,
                    number,
                    word_count,
                    created_at,
                },
            )
        })
        .collect())
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
