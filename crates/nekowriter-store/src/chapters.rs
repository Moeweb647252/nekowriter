//! Chapters (`docs/product-spec.md` §5.3, §7): starting a chapter from its guidance, the
//! outline and body runs' submissions and edits, the outline and whole-chapter reviews,
//! moving to the next chapter, completion and extra chapters.

use nekowriter_core::{
    body::{self, BodyEdit, EditResult},
    chapter::{ChapterGuidance, ChapterInput, ChapterKind, ChapterStatus, ChapterTitle},
    conflict::StateConflict,
    document::DocumentRole,
    ids::{ChapterId, CommandId, DocumentId, PlanItemId, ReviewId, RevisionId, VersionId},
    review::{ReviewDecision, ReviewKind, ReviewStatus},
    run::{Fold, RunKind, RunStatus},
    run_message::{RunMessageBody, ToolOutcome},
    submission::{
        ChapterSubmission, OutlineSubmission, SubmissionProblem, accept_chapter, accept_outline,
    },
    text::word_count,
    time::Timestamp,
    workflow::{ChapterStep, WorkflowStage},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, Set,
};

use crate::{
    Store, StoreError,
    command::{Begun, WriteTx},
    documents::{NewRevision, add_revision, chapter_document, current_revision},
    entity::{chapter, document, review, run, workflow},
    execution::{append_message, check_model, create_run, queue_for_revision},
    workflow::{VersionContext, WorkspaceAck, advance, load_version, project_has_other_active_run},
};

/// The fields of a new chapter.
pub(crate) struct NewChapter {
    pub(crate) kind: ChapterKind,
    pub(crate) number: i32,
    pub(crate) plan_item_id: Option<PlanItemId>,
    pub(crate) title: String,
    pub(crate) goal: String,
}

/// Inserts a chapter with its guidance, outline and body documents.
pub(crate) async fn create_chapter_rows<C: ConnectionTrait>(
    db: &C,
    version_id: VersionId,
    new: NewChapter,
    now: Timestamp,
) -> Result<ChapterId, StoreError> {
    let id = ChapterId::generate();
    chapter::ActiveModel {
        id: Set(id),
        version_id: Set(version_id),
        kind: Set(new.kind),
        number: Set(new.number),
        plan_item_id: Set(new.plan_item_id),
        title: Set(new.title),
        goal: Set(new.goal),
        status: Set(ChapterStatus::NotStarted),
        created_at: Set(now),
    }
    .insert(db)
    .await?;
    for role in DocumentRole::CHAPTER_ROLES {
        document::ActiveModel {
            id: Set(DocumentId::generate()),
            version_id: Set(version_id),
            chapter_id: Set(Some(id)),
            role: Set(role),
            current_revision_id: Set(None),
        }
        .insert(db)
        .await?;
    }
    Ok(id)
}

pub(crate) async fn find_chapter<C: ConnectionTrait>(
    db: &C,
    id: ChapterId,
) -> Result<chapter::Model, StoreError> {
    chapter::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("chapter {id} is missing")))
}

/// What the chapter's runs are told about it.
pub(crate) fn chapter_input(chapter: chapter::Model) -> ChapterInput {
    ChapterInput {
        id: chapter.id,
        kind: chapter.kind,
        number: chapter.number,
        title: chapter.title,
        goal: chapter.goal,
    }
}

/// The current text of a chapter's document; empty before its first revision.
pub(crate) async fn chapter_text<C: ConnectionTrait>(
    db: &C,
    chapter_id: ChapterId,
    role: DocumentRole,
) -> Result<String, StoreError> {
    let document = chapter_document(db, chapter_id, role).await?;
    Ok(current_revision(db, &document)
        .await?
        .map(|revision| revision.content)
        .unwrap_or_default())
}

/// The chapter at `step` of the workflow, or [`StateConflict::WrongStep`].
fn chapter_at(context: &VersionContext, expected: ChapterStep) -> Result<ChapterId, StoreError> {
    match context.workflow.stage {
        WorkflowStage::Chapter { chapter_id, step } if step == expected => Ok(chapter_id),
        _ => Err(StoreError::StateConflict(StateConflict::WrongStep)),
    }
}

fn the_run_chapter(run: &run::Model) -> Result<ChapterId, StoreError> {
    run.chapter_id
        .ok_or_else(|| StoreError::Corrupt(format!("chapter run {} has no chapter", run.id)))
}

impl Store {
    /// Saves the chapter's guidance and starts its outline (§7).
    pub async fn start_chapter(
        &self,
        command: CommandId,
        version_id: VersionId,
        expected_revision: i64,
        guidance: ChapterGuidance,
    ) -> Result<WorkspaceAck, StoreError> {
        let input = (version_id, expected_revision, &guidance);
        let mut tx = match self.begin_command(command, "chapter.start", &input).await? {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let context = load_version(tx.db(), version_id).await?;
        context.check_revision(expected_revision)?;
        let chapter_id = chapter_at(&context, ChapterStep::Ready)?;
        if project_has_other_active_run(tx.db(), context.project.id, None).await? {
            return Err(StoreError::StateConflict(StateConflict::ProjectBusy));
        }
        check_model(tx.db(), &context.project).await?;
        let now = tx.now();
        let document = chapter_document(tx.db(), chapter_id, DocumentRole::ChapterGuidance).await?;
        add_revision(
            tx.db(),
            document,
            NewRevision::text(guidance.as_str().to_owned(), None),
            now,
        )
        .await?;
        let mut started = find_chapter(tx.db(), chapter_id).await?.into_active_model();
        started.status = Set(ChapterStatus::InProgress);
        started.update(tx.db()).await?;
        let run = create_run(
            tx.write(),
            &context,
            RunKind::ChapterOutline,
            Some(chapter_id),
        )
        .await?;
        let workflow = advance(
            tx.write(),
            context.workflow,
            context.project.id,
            WorkflowStage::Chapter {
                chapter_id,
                step: ChapterStep::OutlineGenerating,
            },
            Some(run.id),
        )
        .await?;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }

    /// Moves from an approved main chapter to the next one's guidance, without a model call.
    pub async fn next_chapter(
        &self,
        command: CommandId,
        version_id: VersionId,
        expected_revision: i64,
    ) -> Result<WorkspaceAck, StoreError> {
        let input = (version_id, expected_revision);
        let mut tx = match self.begin_command(command, "chapter.next", &input).await? {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let context = load_version(tx.db(), version_id).await?;
        context.check_revision(expected_revision)?;
        let current = find_chapter(tx.db(), chapter_at(&context, ChapterStep::Done)?).await?;
        let next = match current.kind {
            ChapterKind::Main => next_main_chapter(tx.db(), &current).await?,
            ChapterKind::Extra => None,
        }
        .ok_or(StoreError::StateConflict(StateConflict::WrongStep))?;
        let workflow = advance(
            tx.write(),
            context.workflow,
            context.project.id,
            WorkflowStage::Chapter {
                chapter_id: next.id,
                step: ChapterStep::Ready,
            },
            None,
        )
        .await?;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }

    /// Confirms the novel complete after its last main chapter, or finishes an extra
    /// chapter (§5.3). The first completion time is kept.
    pub async fn complete_version(
        &self,
        command: CommandId,
        version_id: VersionId,
        expected_revision: i64,
    ) -> Result<WorkspaceAck, StoreError> {
        let input = (version_id, expected_revision);
        let mut tx = match self
            .begin_command(command, "version.complete", &input)
            .await?
        {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let context = load_version(tx.db(), version_id).await?;
        context.check_revision(expected_revision)?;
        let current = find_chapter(tx.db(), chapter_at(&context, ChapterStep::Done)?).await?;
        if current.kind == ChapterKind::Main
            && unapproved_main_chapters(tx.db(), version_id).await? > 0
        {
            return Err(StoreError::StateConflict(StateConflict::WrongStep));
        }
        let first_completion = context.workflow.completed_at.is_none();
        let workflow = advance(
            tx.write(),
            context.workflow,
            context.project.id,
            WorkflowStage::Completed,
            None,
        )
        .await?;
        let workflow = if first_completion {
            let mut completed = workflow.into_active_model();
            completed.completed_at = Set(Some(tx.now()));
            completed.update(tx.db()).await?
        } else {
            workflow
        };
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }

    /// Adds an extra chapter to a completed novel and opens its guidance (§7, D13).
    pub async fn add_extra_chapter(
        &self,
        command: CommandId,
        version_id: VersionId,
        expected_revision: i64,
        title: ChapterTitle,
    ) -> Result<WorkspaceAck, StoreError> {
        let input = (version_id, expected_revision, &title);
        let mut tx = match self
            .begin_command(command, "chapter.add_extra", &input)
            .await?
        {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let context = load_version(tx.db(), version_id).await?;
        context.check_revision(expected_revision)?;
        if context.workflow.stage != WorkflowStage::Completed {
            return Err(StoreError::StateConflict(StateConflict::WrongStep));
        }
        let extras = chapter::Entity::find()
            .filter(chapter::Column::VersionId.eq(version_id))
            .filter(chapter::Column::Kind.eq(ChapterKind::Extra))
            .count(tx.db())
            .await?;
        let now = tx.now();
        let chapter_id = create_chapter_rows(
            tx.db(),
            version_id,
            NewChapter {
                kind: ChapterKind::Extra,
                number: i32::try_from(extras + 1).unwrap_or(i32::MAX),
                plan_item_id: None,
                title: title.as_str().to_owned(),
                goal: String::new(),
            },
            now,
        )
        .await?;
        let workflow = advance(
            tx.write(),
            context.workflow,
            context.project.id,
            WorkflowStage::Chapter {
                chapter_id,
                step: ChapterStep::Ready,
            },
            None,
        )
        .await?;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }
}

async fn next_main_chapter<C: ConnectionTrait>(
    db: &C,
    current: &chapter::Model,
) -> Result<Option<chapter::Model>, StoreError> {
    Ok(chapter::Entity::find()
        .filter(chapter::Column::VersionId.eq(current.version_id))
        .filter(chapter::Column::Kind.eq(ChapterKind::Main))
        .filter(chapter::Column::Number.eq(current.number + 1))
        .one(db)
        .await?)
}

async fn unapproved_main_chapters<C: ConnectionTrait>(
    db: &C,
    version_id: VersionId,
) -> Result<u64, StoreError> {
    Ok(chapter::Entity::find()
        .filter(chapter::Column::VersionId.eq(version_id))
        .filter(chapter::Column::Kind.eq(ChapterKind::Main))
        .filter(chapter::Column::Status.ne(ChapterStatus::Approved))
        .count(db)
        .await?)
}

/// Applies a chapter outline submission. On success the outline revision and its open
/// review are written; the caller moves the workflow to the returned stage.
pub(crate) async fn apply_outline_submission(
    tx: &mut WriteTx,
    run: &run::Model,
    submission: OutlineSubmission,
) -> Result<Result<(i32, WorkflowStage), SubmissionProblem>, StoreError> {
    let chapter_id = the_run_chapter(run)?;
    if submission.chapter_id != chapter_id {
        return Ok(Err(SubmissionProblem::WrongChapter));
    }
    let content = match accept_outline(&submission.content) {
        Ok(content) => content,
        Err(problem) => return Ok(Err(problem)),
    };
    let now = tx.now();
    let document = chapter_document(tx.db(), chapter_id, DocumentRole::ChapterOutline).await?;
    let revision = add_revision(
        tx.db(),
        document,
        NewRevision::text(content, Some(run.id)),
        now,
    )
    .await?;
    let round = run.round + 1;
    insert_chapter_review(
        tx,
        run,
        round,
        ReviewKind::ChapterOutline,
        revision.id,
        None,
    )
    .await?;
    Ok(Ok((
        round,
        WorkflowStage::Chapter {
            chapter_id,
            step: ChapterStep::OutlineReviewing,
        },
    )))
}

/// Applies a chapter submission: the current body goes to its whole-chapter review.
pub(crate) async fn apply_chapter_submission(
    tx: &mut WriteTx,
    run: &run::Model,
    submission: ChapterSubmission,
) -> Result<Result<(i32, WorkflowStage), SubmissionProblem>, StoreError> {
    let chapter_id = the_run_chapter(run)?;
    if submission.chapter_id != chapter_id {
        return Ok(Err(SubmissionProblem::WrongChapter));
    }
    let document = chapter_document(tx.db(), chapter_id, DocumentRole::ChapterBody).await?;
    let body = current_revision(tx.db(), &document).await?;
    let text = body
        .as_ref()
        .map_or("", |revision| revision.content.as_str());
    let summary = match accept_chapter(&submission.summary, text) {
        Ok(summary) => summary,
        Err(problem) => return Ok(Err(problem)),
    };
    let target = body
        .map(|revision| revision.id)
        .ok_or_else(|| StoreError::Corrupt("a non-empty body without a revision".to_owned()))?;
    let round = run.round + 1;
    insert_chapter_review(
        tx,
        run,
        round,
        ReviewKind::ChapterFinal,
        target,
        Some(summary),
    )
    .await?;
    Ok(Ok((
        round,
        WorkflowStage::Chapter {
            chapter_id,
            step: ChapterStep::Reviewing,
        },
    )))
}

/// Applies a body edit to the run's chapter (§8.5) and says what happened.
pub(crate) async fn apply_edit(
    tx: &mut WriteTx,
    run: &run::Model,
    edit: &BodyEdit,
) -> Result<ToolOutcome, StoreError> {
    let chapter_id = the_run_chapter(run)?;
    let document = chapter_document(tx.db(), chapter_id, DocumentRole::ChapterBody).await?;
    let current = current_revision(tx.db(), &document).await?;
    let text = current
        .as_ref()
        .map_or("", |revision| revision.content.as_str());
    Ok(match body::apply(text, edit) {
        Ok(EditResult::Changed(changed)) => {
            let count = word_count(&changed);
            let now = tx.now();
            let revision = add_revision(
                tx.db(),
                document,
                NewRevision {
                    content: changed,
                    chapter_plan: None,
                    word_count: Some(count),
                    created_by_run: Some(run.id),
                },
                now,
            )
            .await?;
            ToolOutcome::EditApplied {
                base_revision: current.map(|revision| revision.id),
                revision: revision.id,
                word_count: count,
            }
        }
        Ok(EditResult::NoChange) => ToolOutcome::EditNoChange {
            word_count: word_count(text),
        },
        Err(problem) => ToolOutcome::EditRejected { problem },
    })
}

async fn insert_chapter_review(
    tx: &WriteTx,
    run: &run::Model,
    round: i32,
    kind: ReviewKind,
    target: RevisionId,
    summary: Option<String>,
) -> Result<(), StoreError> {
    review::ActiveModel {
        id: Set(ReviewId::generate()),
        version_id: Set(run.version_id),
        chapter_id: Set(run.chapter_id),
        run_id: Set(run.id),
        round: Set(round),
        kind: Set(kind),
        target_revision_id: Set(Some(target)),
        target_call_id: Set(None),
        status: Set(ReviewStatus::Open),
        summary: Set(summary),
        feedback: Set(None),
        decided_at: Set(None),
        created_at: Set(tx.now()),
    }
    .insert(tx.db())
    .await?;
    Ok(())
}

/// Decides a chapter outline or whole-chapter review. Approval folds the stage: an
/// approved outline starts the body, an approved chapter is done. Rejection sends the
/// feedback to the same run, which revises.
pub(crate) async fn decide_chapter_review(
    tx: &mut WriteTx,
    context: VersionContext,
    review: review::Model,
    decision: ReviewDecision,
) -> Result<workflow::Model, StoreError> {
    let (review_step, revise_step, role) = match review.kind {
        ReviewKind::ChapterOutline => (
            ChapterStep::OutlineReviewing,
            ChapterStep::OutlineGenerating,
            DocumentRole::ChapterOutline,
        ),
        ReviewKind::ChapterFinal => (
            ChapterStep::Reviewing,
            ChapterStep::Drafting,
            DocumentRole::ChapterBody,
        ),
        kind => {
            return Err(StoreError::Corrupt(format!(
                "a {kind:?} review is not a chapter review"
            )));
        }
    };
    let not_open = || StoreError::StateConflict(StateConflict::ReviewNotOpen);
    let chapter_id = chapter_at(&context, review_step).map_err(|_| not_open())?;
    if review.status != ReviewStatus::Open
        || review.chapter_id != Some(chapter_id)
        || context.workflow.active_run_id != Some(review.run_id)
    {
        return Err(not_open());
    }
    let run = run::Entity::find_by_id(review.run_id)
        .one(tx.db())
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("review {} has no run", review.id)))?;
    if run.status != RunStatus::WaitingReview || run.round != review.round {
        return Err(not_open());
    }
    let document = chapter_document(tx.db(), chapter_id, role).await?;
    let Some(target) = review.target_revision_id else {
        return Err(StoreError::Corrupt(format!(
            "chapter review {} has no target revision",
            review.id
        )));
    };
    if document.current_revision_id != Some(target) {
        return Err(StoreError::Corrupt(format!(
            "review {} targets a revision that is no longer current",
            review.id
        )));
    }

    let review_id = review.id;
    let mut decided = review.into_active_model();
    decided.decided_at = Set(Some(tx.now()));
    let project_id = context.project.id;
    match decision {
        ReviewDecision::Reject { feedback } => {
            decided.status = Set(ReviewStatus::Rejected);
            decided.feedback = Set(Some(feedback.as_str().to_owned()));
            decided.update(tx.db()).await?;
            append_message(tx, run.id, RunMessageBody::ReviewRejected { review_id }).await?;
            let run_id = queue_for_revision(tx, run).await?;
            let stage = WorkflowStage::Chapter {
                chapter_id,
                step: revise_step,
            };
            advance(tx, context.workflow, project_id, stage, Some(run_id)).await
        }
        ReviewDecision::Approve => {
            decided.status = Set(ReviewStatus::Approved);
            decided.update(tx.db()).await?;
            let mut done = run.into_active_model();
            done.status = Set(RunStatus::Succeeded);
            done.fold = Set(Some(Fold(vec![target])));
            done.queued_at = Set(None);
            done.updated_at = Set(tx.now());
            done.update(tx.db()).await?;
            if role == DocumentRole::ChapterOutline {
                let body = create_run(tx, &context, RunKind::ChapterBody, Some(chapter_id)).await?;
                let stage = WorkflowStage::Chapter {
                    chapter_id,
                    step: ChapterStep::Drafting,
                };
                advance(tx, context.workflow, project_id, stage, Some(body.id)).await
            } else {
                let mut approved = find_chapter(tx.db(), chapter_id).await?.into_active_model();
                approved.status = Set(ChapterStatus::Approved);
                approved.update(tx.db()).await?;
                let stage = WorkflowStage::Chapter {
                    chapter_id,
                    step: ChapterStep::Done,
                };
                advance(tx, context.workflow, project_id, stage, None).await
            }
        }
    }
}
