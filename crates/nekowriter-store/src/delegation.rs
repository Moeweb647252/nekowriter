//! Delegating a review to a review agent (`docs/product-spec.md` §10).
//!
//! A delegated review gets a child run that reads the review's subject and submits one
//! verdict. The verdict is applied only while the review is still delegated to that run;
//! cancelling the delegation, or the run failing, returns the review to the user.

use nekowriter_core::{
    call::CallRequest,
    citation::Citation,
    conflict::{Resource, StateConflict},
    document::DocumentRole,
    ids::{CommandId, ReviewId, RunId},
    review::{Decision, DecisionSource, ReviewFeedback, ReviewKind, ReviewStatus},
    run::{RunHalt, RunKind, RunStatus},
    run_message::FeedbackItem,
    stage::{ReviewSubject, StageInput},
    submission::{SubmissionProblem, VerdictSubmission, accept_verdict},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
};

use crate::{
    Store, StoreError,
    chapters::{chapter_input, chapter_text, find_chapter},
    citations::verify_citations,
    command::{Begun, WriteTx},
    documents::{current_revision, version_document},
    entity::{
        model_request::{self, ModelRequestStatus},
        review, revision, run, tool_call,
    },
    execution::{check_model, halt},
    feedback::stored,
    reviews::apply_decision,
    workflow::{WorkspaceAck, load_version, touch},
};

impl Store {
    /// Delegates an open review of the active run to a review agent, with the user's draft
    /// feedback and the passages it cites as notes for it.
    pub async fn delegate_review(
        &self,
        command: CommandId,
        review_id: ReviewId,
        expected_revision: i64,
        notes: Option<ReviewFeedback>,
        citations: Vec<Citation>,
    ) -> Result<WorkspaceAck, StoreError> {
        let input = (review_id, expected_revision, &notes, &citations);
        let mut tx = match self
            .begin_command(command, "review.delegate", &input)
            .await?
        {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let review = review::Entity::find_by_id(review_id)
            .one(tx.db())
            .await?
            .ok_or(StoreError::NotFound(Resource::Review))?;
        let context = load_version(tx.db(), review.version_id).await?;
        context.check_revision(expected_revision)?;
        let parent = run::Entity::find_by_id(review.run_id)
            .one(tx.db())
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("review {review_id} has no run")))?;
        if review.status != ReviewStatus::Open
            || context.workflow.active_run_id != Some(parent.id)
            || parent.status != RunStatus::WaitingReview
        {
            return Err(StoreError::StateConflict(StateConflict::ReviewNotOpen));
        }
        check_model(tx.db(), &context.project).await?;
        let citations = verify_citations(tx.db(), context.version.id, &citations).await?;
        let now = tx.now();
        let child = run::ActiveModel {
            id: Set(RunId::generate()),
            project_id: Set(parent.project_id),
            version_id: Set(parent.version_id),
            chapter_id: Set(parent.chapter_id),
            review_id: Set(Some(review_id)),
            kind: Set(RunKind::Review),
            status: Set(RunStatus::Queued),
            halt: Set(None),
            round: Set(0),
            request_count: Set(0),
            request_allowance_end: Set(RunKind::Review.request_allowance()),
            consecutive_protocol_errors: Set(0),
            resume_count: Set(0),
            fold: Set(None),
            queued_at: Set(Some(now)),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(tx.db())
        .await?;
        let mut delegated = review.into_active_model();
        delegated.status = Set(ReviewStatus::Delegated);
        delegated.delegated_run_id = Set(Some(child.id));
        delegated.delegation_notes = Set(notes.map(|notes| notes.as_str().to_owned()));
        delegated.delegation_citations = Set(stored(citations));
        delegated.delegation_failure = Set(None);
        delegated.update(tx.db()).await?;
        let workflow = touch(tx.write(), context.version.id, context.project.id).await?;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }

    /// Takes a delegated review back: the review agent's run fails, a late verdict is never
    /// applied, and the user reviews it.
    pub async fn cancel_delegation(
        &self,
        command: CommandId,
        review_id: ReviewId,
    ) -> Result<WorkspaceAck, StoreError> {
        let mut tx = match self
            .begin_command(command, "review.cancel_delegation", &review_id)
            .await?
        {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let review = review::Entity::find_by_id(review_id)
            .one(tx.db())
            .await?
            .ok_or(StoreError::NotFound(Resource::Review))?;
        let context = load_version(tx.db(), review.version_id).await?;
        let child = match review.delegated_run_id {
            Some(id) if review.status == ReviewStatus::Delegated => {
                run::Entity::find_by_id(id).one(tx.db()).await?
            }
            _ => None,
        }
        .ok_or(StoreError::StateConflict(StateConflict::ReviewNotDelegated))?;
        let requests = model_request::Entity::find()
            .filter(model_request::Column::RunId.eq(child.id))
            .filter(model_request::Column::Status.eq(ModelRequestStatus::Dispatched))
            .all(tx.db())
            .await?;
        for request in requests {
            let mut cancelled = request.into_active_model();
            cancelled.status = Set(ModelRequestStatus::Cancelled);
            cancelled.completed_at = Set(Some(tx.now()));
            cancelled.update(tx.db()).await?;
        }
        // Halting the review run releases the review (see `release_delegation`); the user
        // cancelled it, so no failure is recorded.
        halt(tx.write(), child, RunStatus::Failed, RunHalt::StoppedByUser).await?;
        let mut released = review::Entity::find_by_id(review_id)
            .one(tx.db())
            .await?
            .ok_or(StoreError::NotFound(Resource::Review))?
            .into_active_model();
        released.delegation_failure = Set(None);
        released.update(tx.db()).await?;
        let workflow = touch(tx.write(), context.version.id, context.project.id).await?;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }
}

/// Returns a review to the user when the review run deciding it stops, fails or is
/// interrupted, and records why.
pub(crate) async fn release_delegation(
    tx: &WriteTx,
    run: &run::Model,
    reason: RunHalt,
) -> Result<(), StoreError> {
    let Some(review_id) = run.review_id else {
        return Ok(());
    };
    let review = review::Entity::find_by_id(review_id)
        .one(tx.db())
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("review run {} has no review", run.id)))?;
    if review.status == ReviewStatus::Delegated && review.delegated_run_id == Some(run.id) {
        let mut released = review.into_active_model();
        released.status = Set(ReviewStatus::Open);
        released.delegated_run_id = Set(None);
        released.delegation_failure = Set(Some(reason));
        released.update(tx.db()).await?;
    }
    Ok(())
}

/// Applies a review agent's verdict (D08: `passed` alone decides). A verdict that broke
/// a rule is a rejected round; one that arrives after the review was taken back is kept
/// in the run's history and not applied.
pub(crate) async fn apply_verdict(
    tx: &mut WriteTx,
    run: &run::Model,
    submission: VerdictSubmission,
) -> Result<Result<(), SubmissionProblem>, StoreError> {
    let verdict = match accept_verdict(submission) {
        Ok(verdict) => verdict,
        Err(problem) => return Ok(Err(problem)),
    };
    let review_id = run
        .review_id
        .ok_or_else(|| StoreError::Corrupt(format!("review run {} has no review", run.id)))?;
    let review = review::Entity::find_by_id(review_id)
        .one(tx.db())
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("review {review_id} is missing")))?;
    if review.status != ReviewStatus::Delegated || review.delegated_run_id != Some(run.id) {
        tracing::info!(run = %run.id, review = %review_id, "a stale verdict was not applied");
        return Ok(Ok(()));
    }
    let mut scored = review.into_active_model();
    scored.confidence_permille = Set(Some(verdict.confidence_permille));
    let review = scored.update(tx.db()).await?;
    let decision = Decision {
        approved: verdict.passed,
        feedback: (!verdict.passed && !verdict.issues.is_empty())
            .then(|| verdict.issues.join("\n")),
        citations: Vec::new(),
        source: DecisionSource::Delegate,
    };
    let context = load_version(tx.db(), review.version_id).await?;
    apply_decision(tx, context, review, decision).await?;
    Ok(Ok(()))
}

/// What the review run reviews.
pub(crate) async fn review_subject<C: ConnectionTrait>(
    db: &C,
    run: &run::Model,
) -> Result<StageInput, StoreError> {
    let review_id = run
        .review_id
        .ok_or_else(|| StoreError::Corrupt(format!("review run {} has no review", run.id)))?;
    let review = review::Entity::find_by_id(review_id)
        .one(db)
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("review {review_id} is missing")))?;
    let target = match review.target_revision_id {
        Some(id) => Some(
            revision::Entity::find_by_id(id)
                .one(db)
                .await?
                .ok_or_else(|| StoreError::Corrupt(format!("revision {id} is missing")))?,
        ),
        None => None,
    };
    let chapter = match review.chapter_id {
        Some(id) => Some(find_chapter(db, id).await?),
        None => None,
    };
    let missing = || StoreError::Corrupt(format!("review {review_id} lacks its subject"));
    let subject = match (review.kind, chapter) {
        (ReviewKind::ChapterOutline, Some(chapter)) => ReviewSubject::ChapterOutline {
            guidance: chapter_text(db, chapter.id, DocumentRole::ChapterGuidance).await?,
            outline: target.ok_or_else(missing)?.content,
            chapter: chapter_input(chapter),
        },
        (ReviewKind::ChapterFinal, Some(chapter)) => ReviewSubject::Chapter {
            body: target.ok_or_else(missing)?.content,
            summary: review.summary.clone().unwrap_or_default(),
            chapter: chapter_input(chapter),
        },
        (ReviewKind::ChapterEdit, Some(chapter)) => {
            let call = match review.target_call_id {
                Some(id) => tool_call::Entity::find_by_id(id).one(db).await?,
                None => None,
            }
            .ok_or_else(missing)?;
            let Some(CallRequest::Edit { edit }) = call.parsed else {
                return Err(missing());
            };
            ReviewSubject::Edit {
                body: chapter_text(db, chapter.id, DocumentRole::ChapterBody).await?,
                chapter: chapter_input(chapter),
                edit,
            }
        }
        (kind, None) => {
            let artifact = kind.artifact().ok_or_else(missing)?;
            let brief = version_document(db, review.version_id, DocumentRole::NovelBrief).await?;
            let target = target.ok_or_else(missing)?;
            ReviewSubject::Artifact {
                artifact,
                brief: current_revision(db, &brief)
                    .await?
                    .map(|revision| revision.content)
                    .unwrap_or_default(),
                content: target.content,
                chapter_plan: target.chapter_plan,
            }
        }
        (kind, Some(_)) => {
            return Err(StoreError::Corrupt(format!(
                "a {kind:?} review belongs to a chapter"
            )));
        }
    };
    Ok(StageInput::Review {
        kind: review.kind,
        subject,
        notes: match (review.delegation_notes, review.delegation_citations) {
            (None, None) => None,
            (text, citations) => Some(FeedbackItem {
                text: text.unwrap_or_default(),
                citations: citations.map(|citations| citations.0).unwrap_or_default(),
            }),
        },
    })
}
