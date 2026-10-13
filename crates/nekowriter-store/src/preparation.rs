//! Preparation (`docs/product-spec.md` §6): the brief, the start, the artifacts the agent
//! submits, their reviews, and the fold that ends the stage.

use std::collections::BTreeSet;

use nekowriter_core::{
    chapter::ChapterKind,
    conflict::StateConflict,
    document::DocumentRole,
    ids::{ChapterId, CommandId, ReviewId, VersionId},
    preparation::{ArtifactKind, NovelBrief, PreparationSubmission, accept_preparation},
    review::{Decision, DecisionSource, ReviewKind, ReviewStatus},
    run::{Fold, RunKind, RunStatus},
    run_message::RunMessageBody,
    submission::SubmissionProblem,
    workflow::{ChapterStep, PreparationStep, WorkflowStage},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, Set,
};

use crate::{
    Store, StoreError,
    chapters::{NewChapter, create_chapter_rows},
    command::{Begun, WriteTx},
    documents::{NewRevision, add_revision, current_revision, version_document},
    entity::{review, revision, revision_dependency, run},
    execution::{append_message, check_model, create_run, queue_for_revision},
    reviews::{decidable, record_decision},
    workflow::{
        VersionContext, WorkspaceAck, advance, load_version, project_has_other_active_run, touch,
    },
};

impl Store {
    /// Saves the novel brief. It can change only until preparation starts.
    pub async fn save_brief(
        &self,
        command: CommandId,
        version_id: VersionId,
        expected_revision: i64,
        brief: NovelBrief,
    ) -> Result<WorkspaceAck, StoreError> {
        let input = (version_id, expected_revision, &brief);
        let mut tx = match self.begin_command(command, "brief.save", &input).await? {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let context = load_version(tx.db(), version_id).await?;
        context.check_revision(expected_revision)?;
        if context.workflow.stage != WorkflowStage::NEW_VERSION {
            return Err(StoreError::StateConflict(StateConflict::BriefLocked));
        }
        let document = version_document(tx.db(), version_id, DocumentRole::NovelBrief).await?;
        let unchanged = current_revision(tx.db(), &document)
            .await?
            .is_some_and(|current| current.content == brief.as_str());
        if !unchanged {
            let now = tx.now();
            add_revision(
                tx.db(),
                document,
                NewRevision::text(brief.as_str().to_owned(), None),
                now,
            )
            .await?;
        }
        let workflow = touch(tx.write(), version_id, context.project.id).await?;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }

    /// Starts preparation: creates the run that writes the four artifacts.
    pub async fn start_preparation(
        &self,
        command: CommandId,
        version_id: VersionId,
        expected_revision: i64,
    ) -> Result<WorkspaceAck, StoreError> {
        let input = (version_id, expected_revision);
        let mut tx = match self
            .begin_command(command, "preparation.start", &input)
            .await?
        {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let context = load_version(tx.db(), version_id).await?;
        context.check_revision(expected_revision)?;
        if context.workflow.stage != WorkflowStage::NEW_VERSION {
            return Err(StoreError::StateConflict(StateConflict::WrongStep));
        }
        let brief = version_document(tx.db(), version_id, DocumentRole::NovelBrief).await?;
        if brief.current_revision_id.is_none() {
            return Err(StoreError::StateConflict(StateConflict::BriefMissing));
        }
        if project_has_other_active_run(tx.db(), context.project.id, None).await? {
            return Err(StoreError::StateConflict(StateConflict::ProjectBusy));
        }
        check_model(tx.db(), &context.project).await?;
        let run = create_run(tx.write(), &context, RunKind::Preparation, None).await?;
        let workflow = advance(
            tx.write(),
            context.workflow,
            context.project.id,
            WorkflowStage::Preparation {
                step: PreparationStep::Generating,
            },
            Some(run.id),
        )
        .await?;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }
}

/// Approves or rejects a preparation artifact. When the round's last open review is
/// decided, the round closes: the stage folds if everything is approved, otherwise the run
/// revises what was rejected or became stale.
pub(crate) async fn decide_preparation_review(
    tx: &mut WriteTx,
    context: VersionContext,
    review: review::Model,
    decision: Decision,
) -> Result<crate::entity::workflow::Model, StoreError> {
    let run = current_review_run(tx.db(), &context, &review, decision.source).await?;
    let round = review.round;
    record_decision(tx, review, decision).await?;

    let still_open = review::Entity::find()
        .filter(review::Column::RunId.eq(run.id))
        .filter(review::Column::Round.eq(round))
        .filter(review::Column::Status.is_in([ReviewStatus::Open, ReviewStatus::Delegated]))
        .one(tx.db())
        .await?
        .is_some();
    if still_open {
        touch(tx, context.version.id, context.project.id).await
    } else {
        close_round(tx, context, run).await
    }
}

/// The preparation run whose current round contains `review`, if the review is still open.
async fn current_review_run<C: ConnectionTrait>(
    db: &C,
    context: &VersionContext,
    review: &review::Model,
    source: DecisionSource,
) -> Result<run::Model, StoreError> {
    let not_open = || StoreError::StateConflict(StateConflict::ReviewNotOpen);
    if !decidable(review, source)
        || context.workflow.stage
            != (WorkflowStage::Preparation {
                step: PreparationStep::Reviewing,
            })
        || context.workflow.active_run_id != Some(review.run_id)
    {
        return Err(not_open());
    }
    let run = run::Entity::find_by_id(review.run_id)
        .one(db)
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("review {} has no run", review.id)))?;
    if run.round != review.round || run.status != RunStatus::WaitingReview {
        return Err(not_open());
    }
    Ok(run)
}

/// Ends a review round: fold the stage if every artifact is approved, otherwise queue the
/// run to revise the rejected and stale artifacts.
async fn close_round(
    tx: &mut WriteTx,
    context: VersionContext,
    run: run::Model,
) -> Result<crate::entity::workflow::Model, StoreError> {
    let needs_work = artifacts_needing_work(tx.db(), &run).await?;
    let project_id = context.project.id;
    if needs_work.is_empty() {
        let fold = approved_revisions(tx.db(), &run).await?;
        let first_chapter = create_chapters(tx, context.version.id, &fold).await?;
        let mut done = run.into_active_model();
        done.status = Set(RunStatus::Succeeded);
        done.fold = Set(Some(Fold(fold)));
        done.queued_at = Set(None);
        done.updated_at = Set(tx.now());
        done.update(tx.db()).await?;
        advance(
            tx,
            context.workflow,
            project_id,
            WorkflowStage::Chapter {
                chapter_id: first_chapter,
                step: ChapterStep::Ready,
            },
            None,
        )
        .await
    } else {
        append_message(
            tx,
            run.id,
            RunMessageBody::PreparationReviewOutcome { round: run.round },
        )
        .await?;
        let active_run = queue_for_revision(tx, run).await?;
        advance(
            tx,
            context.workflow,
            project_id,
            WorkflowStage::Preparation {
                step: PreparationStep::Revising,
            },
            Some(active_run),
        )
        .await
    }
}

/// The artifacts the run's current round rejected or marked stale.
async fn artifacts_needing_work<C: ConnectionTrait>(
    db: &C,
    run: &run::Model,
) -> Result<BTreeSet<ArtifactKind>, StoreError> {
    let reviews = review::Entity::find()
        .filter(review::Column::RunId.eq(run.id))
        .filter(review::Column::Round.eq(run.round))
        .filter(review::Column::Status.is_in([ReviewStatus::Rejected, ReviewStatus::Stale]))
        .all(db)
        .await?;
    Ok(reviews
        .iter()
        .filter_map(|review| review.kind.artifact())
        .collect())
}

/// The artifacts a preparation run must submit next: all four in the first round, then the
/// ones the last round rejected or marked stale.
pub(crate) async fn required_artifacts<C: ConnectionTrait>(
    db: &C,
    run: &run::Model,
) -> Result<BTreeSet<ArtifactKind>, StoreError> {
    if run.round == 0 {
        Ok(ArtifactKind::ALL.into_iter().collect())
    } else {
        artifacts_needing_work(db, run).await
    }
}

/// The current revision of every artifact, checked to be approved, in fold order.
async fn approved_revisions<C: ConnectionTrait>(
    db: &C,
    run: &run::Model,
) -> Result<Vec<nekowriter_core::ids::RevisionId>, StoreError> {
    let mut revisions = Vec::with_capacity(ArtifactKind::ALL.len());
    for kind in ArtifactKind::ALL {
        let document = version_document(db, run.version_id, kind.into()).await?;
        let latest = review::Entity::find()
            .filter(review::Column::RunId.eq(run.id))
            .filter(review::Column::Kind.eq(ReviewKind::from(kind)))
            .order_by_desc(review::Column::Round)
            .one(db)
            .await?;
        match (latest, document.current_revision_id) {
            (Some(review), Some(current))
                if review.status == ReviewStatus::Approved
                    && review.target_revision_id == Some(current) =>
            {
                revisions.push(current);
            }
            _ => {
                return Err(StoreError::Corrupt(format!(
                    "the {kind:?} artifact of run {} is not approved at the end of preparation",
                    run.id
                )));
            }
        }
    }
    Ok(revisions)
}

/// Creates the main chapters from the approved outline's plan and returns the first one.
async fn create_chapters(
    tx: &mut WriteTx,
    version_id: VersionId,
    fold: &[nekowriter_core::ids::RevisionId],
) -> Result<ChapterId, StoreError> {
    let outline_id = fold
        .last()
        .copied()
        .ok_or_else(|| StoreError::Corrupt("an empty preparation fold".to_owned()))?;
    let outline = revision::Entity::find_by_id(outline_id)
        .one(tx.db())
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("revision {outline_id} is missing")))?;
    let plan = outline.chapter_plan.ok_or_else(|| {
        StoreError::Corrupt(format!("outline revision {outline_id} has no chapter plan"))
    })?;
    let mut first = None;
    for (index, item) in plan.0.into_iter().enumerate() {
        let now = tx.now();
        let new = NewChapter {
            kind: ChapterKind::Main,
            number: i32::try_from(index + 1).expect("the plan size is bounded"),
            plan_item_id: Some(item.id),
            title: item.title,
            goal: item.goal,
        };
        let id = create_chapter_rows(tx.db(), version_id, new, now).await?;
        first.get_or_insert(id);
    }
    first.ok_or_else(|| {
        StoreError::Corrupt(format!("outline revision {outline_id} plans no chapters"))
    })
}

/// Applies a preparation submission that passed the protocol checks. On success the new
/// revisions, their dependencies and the next round's reviews are written and the new
/// round number is returned; the caller moves the workflow to review.
pub(crate) async fn apply_submission(
    tx: &mut WriteTx,
    run: &run::Model,
    submission: PreparationSubmission,
) -> Result<Result<i32, SubmissionProblem>, StoreError> {
    let required = required_artifacts(tx.db(), run).await?;
    let accepted = match accept_preparation(&required, submission) {
        Ok(accepted) => accepted,
        Err(problem) => return Ok(Err(problem)),
    };
    let round = run.round + 1;
    let now = tx.now();

    let mut new_revisions = Vec::with_capacity(accepted.artifacts.len());
    let mut plan = accepted.chapter_plan;
    for (kind, content) in accepted.artifacts {
        let document = version_document(tx.db(), run.version_id, kind.into()).await?;
        let chapter_plan = if kind == ArtifactKind::Outline {
            plan.take()
        } else {
            None
        };
        let revision = add_revision(
            tx.db(),
            document,
            NewRevision {
                chapter_plan,
                ..NewRevision::text(content, Some(run.id))
            },
            now,
        )
        .await?;
        new_revisions.push((kind, revision));
    }

    // Dependencies are recorded after the whole batch exists, so the order inside the
    // batch cannot make an artifact depend on a revision that is about to be replaced.
    for (kind, revision) in &new_revisions {
        for upstream in kind.upstream() {
            let document = version_document(tx.db(), run.version_id, (*upstream).into()).await?;
            if let Some(source) = document.current_revision_id {
                revision_dependency::ActiveModel {
                    revision_id: Set(revision.id),
                    depends_on_revision_id: Set(source),
                }
                .insert(tx.db())
                .await?;
            }
        }
        insert_review(tx, run, round, *kind, revision.id, ReviewStatus::Open).await?;
    }

    // An approved artifact written from an artifact that just changed is stale (§6).
    let submitted: BTreeSet<ArtifactKind> = new_revisions.iter().map(|(kind, _)| *kind).collect();
    for kind in ArtifactKind::ALL {
        if submitted.contains(&kind) || !kind.upstream().iter().any(|up| submitted.contains(up)) {
            continue;
        }
        let document = version_document(tx.db(), run.version_id, kind.into()).await?;
        if let Some(current) = document.current_revision_id {
            insert_review(tx, run, round, kind, current, ReviewStatus::Stale).await?;
        }
    }

    Ok(Ok(round))
}

async fn insert_review(
    tx: &WriteTx,
    run: &run::Model,
    round: i32,
    kind: ArtifactKind,
    target: nekowriter_core::ids::RevisionId,
    status: ReviewStatus,
) -> Result<(), StoreError> {
    review::ActiveModel {
        id: Set(ReviewId::generate()),
        version_id: Set(run.version_id),
        chapter_id: Set(None),
        run_id: Set(run.id),
        round: Set(round),
        kind: Set(kind.into()),
        target_revision_id: Set(Some(target)),
        target_call_id: Set(None),
        status: Set(status),
        summary: Set(None),
        feedback: Set(None),
        decided_at: Set(None),
        created_at: Set(tx.now()),
        ..Default::default()
    }
    .insert(tx.db())
    .await?;
    Ok(())
}
