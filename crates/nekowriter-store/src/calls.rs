//! Tool calls through their life (`docs/product-spec.md` §9.1): a call arrives, is held
//! while the user pauses the run or is carried out at once, and its outcome becomes a
//! result that the next request seals, together with the feedback bound to it.

use nekowriter_core::{
    call::{CallRequest, StageSubmission},
    ids::{ModelRequestId, ToolCallId},
    question::{Question, check_questions},
    run::{MAX_CONSECUTIVE_PROTOCOL_ERRORS, RunHalt, RunStatus},
    run_message::{Correction, ModelToolCall, RunMessageBody, ToolOutcome},
    submission::SubmissionProblem,
    workflow::{PreparationStep, WorkflowStage},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
};

use crate::{
    StoreError,
    chapters::{apply_chapter_submission, apply_edit, apply_outline_submission},
    command::WriteTx,
    delegation::apply_verdict,
    entity::{
        run,
        tool_call::{self, ToolCallStatus},
        workflow,
    },
    execution::{append_message, halt},
    feedback::{bind_queued, has_queued},
    preparation::apply_submission,
    proposals::{needs_review, propose},
    workflow::{advance, load_version, touch},
};

/// Records a call the model made, as the next in the run's sequence. `parsed` is absent
/// when the call broke the protocol.
pub(crate) async fn insert_call(
    tx: &WriteTx,
    run: &run::Model,
    request_id: ModelRequestId,
    call: &ModelToolCall,
    parsed: Option<CallRequest>,
) -> Result<tool_call::Model, StoreError> {
    let sequence = tool_call::Entity::find()
        .filter(tool_call::Column::RunId.eq(run.id))
        .order_by_desc(tool_call::Column::Sequence)
        .one(tx.db())
        .await?
        .map_or(1, |last| last.sequence + 1);
    Ok(tool_call::ActiveModel {
        id: Set(ToolCallId::generate()),
        run_id: Set(run.id),
        sequence: Set(sequence),
        request_id: Set(request_id),
        provider_id: Set(call.provider_id.clone()),
        name: Set(call.name.clone()),
        arguments: Set(call.arguments.clone()),
        parsed: Set(parsed),
        base_revision_id: Set(None),
        status: Set(ToolCallStatus::Received),
        outcome: Set(None),
        created_at: Set(tx.now()),
    }
    .insert(tx.db())
    .await?)
}

/// Holds the call before it is carried out: the requested pause takes effect and the run
/// waits for the user (§9.3, §9.4).
pub(crate) async fn hold(
    tx: &mut WriteTx,
    run: run::Model,
    call: tool_call::Model,
    workflow: workflow::Model,
) -> Result<(), StoreError> {
    let mut held = call.into_active_model();
    held.status = Set(ToolCallStatus::Held);
    held.update(tx.db()).await?;
    let mut paused = run.into_active_model();
    paused.status = Set(RunStatus::Paused);
    paused.queued_at = Set(None);
    paused.updated_at = Set(tx.now());
    let paused = paused.update(tx.db()).await?;
    let mut workflow = workflow.into_active_model();
    workflow.pause_requested = Set(false);
    workflow.update(tx.db()).await?;
    touch(tx, paused.version_id, paused.project_id).await?;
    Ok(())
}

/// The run's held call, if it is paused at one.
pub(crate) async fn held_call<C: sea_orm::ConnectionTrait>(
    db: &C,
    run_id: nekowriter_core::ids::RunId,
) -> Result<Option<tool_call::Model>, StoreError> {
    Ok(tool_call::Entity::find()
        .filter(tool_call::Column::RunId.eq(run_id))
        .filter(tool_call::Column::Status.eq(ToolCallStatus::Held))
        .one(db)
        .await?)
}

/// Carries out a valid call through the current gate. A submission that arrives while the
/// user's feedback is still waiting for a result is a rejected round, so the feedback is
/// delivered with its result and the writer acts on it first (§9.4).
pub(crate) async fn execute(
    tx: &mut WriteTx,
    run: run::Model,
    call: tool_call::Model,
) -> Result<(), StoreError> {
    let request = call
        .parsed
        .clone()
        .ok_or_else(|| StoreError::Corrupt(format!("tool call {} was never parsed", call.id)))?;
    let submission = match request {
        CallRequest::Ask { questions } => return ask(tx, run, &call, &questions).await,
        CallRequest::QueryKnowledge { .. } => {
            let mut executing = call.into_active_model();
            executing.status = Set(ToolCallStatus::Executing);
            executing.update(tx.db()).await?;
            return requeue(tx, run, false).await;
        }
        CallRequest::Edit { edit } => {
            if let Some(base) = needs_review(tx.db(), &run, &edit).await? {
                return propose(tx, run, call, base).await;
            }
            let outcome = apply_edit(tx, &run, &edit).await?;
            resolve(tx, &call, outcome).await?;
            return requeue(tx, run, false).await;
        }
        CallRequest::Submit { submission } => submission,
    };
    if has_queued(tx.db(), run.id).await? {
        return settle_submission(tx, run, &call, Err(SubmissionProblem::FeedbackPending)).await;
    }
    let accepted = match submission {
        StageSubmission::Preparation { submission } => {
            apply_submission(tx, &run, submission).await?.map(|round| {
                let step = PreparationStep::Reviewing;
                (round, WorkflowStage::Preparation { step })
            })
        }
        StageSubmission::ChapterOutline { submission } => {
            apply_outline_submission(tx, &run, submission).await?
        }
        StageSubmission::Chapter { submission } => {
            apply_chapter_submission(tx, &run, submission).await?
        }
        StageSubmission::Review { verdict } => {
            return match apply_verdict(tx, &run, verdict).await? {
                Ok(()) => {
                    resolve(tx, &call, ToolOutcome::SubmissionAccepted).await?;
                    finish(tx, run).await
                }
                Err(problem) => settle_submission(tx, run, &call, Err(problem)).await,
            };
        }
    };
    settle_submission(tx, run, &call, accepted).await
}

/// Shows the questions and waits for the answers, or tells the agent why they cannot be
/// shown.
async fn ask(
    tx: &mut WriteTx,
    run: run::Model,
    call: &tool_call::Model,
    questions: &[Question],
) -> Result<(), StoreError> {
    if let Err(problem) = check_questions(questions) {
        resolve(tx, call, ToolOutcome::QuestionRejected { problem }).await?;
        return requeue(tx, run, false).await;
    }
    let mut waiting_call = call.clone().into_active_model();
    waiting_call.status = Set(ToolCallStatus::AwaitingAnswer);
    waiting_call.update(tx.db()).await?;
    let (version_id, project_id) = (run.version_id, run.project_id);
    let mut waiting = run.into_active_model();
    waiting.status = Set(RunStatus::WaitingAnswer);
    waiting.queued_at = Set(None);
    waiting.updated_at = Set(tx.now());
    waiting.update(tx.db()).await?;
    touch(tx, version_id, project_id).await?;
    Ok(())
}

/// Ends a review run whose verdict was taken.
async fn finish(tx: &mut WriteTx, run: run::Model) -> Result<(), StoreError> {
    let (version_id, project_id) = (run.version_id, run.project_id);
    let mut done = run.into_active_model();
    done.status = Set(RunStatus::Succeeded);
    done.queued_at = Set(None);
    done.updated_at = Set(tx.now());
    done.update(tx.db()).await?;
    touch(tx, version_id, project_id).await?;
    Ok(())
}

/// Records the call's outcome and queues its result for the model. Queued feedback that
/// waits for a later result binds to this one.
pub(crate) async fn resolve(
    tx: &mut WriteTx,
    call: &tool_call::Model,
    outcome: ToolOutcome,
) -> Result<(), StoreError> {
    let mut resolved = call.clone().into_active_model();
    resolved.status = Set(ToolCallStatus::Resolved);
    resolved.outcome = Set(Some(outcome));
    resolved.update(tx.db()).await?;
    append_message(
        tx,
        call.run_id,
        RunMessageBody::ToolResult {
            tool_call_id: call.id,
        },
    )
    .await?;
    bind_queued(tx, call).await
}

/// Ends a submission round: an accepted submission waits for review at `stage` with the
/// new round number; a rejected one is corrected and the run goes on.
async fn settle_submission(
    tx: &mut WriteTx,
    run: run::Model,
    call: &tool_call::Model,
    accepted: Result<(i32, WorkflowStage), SubmissionProblem>,
) -> Result<(), StoreError> {
    match accepted {
        Ok((round, stage)) => {
            resolve(tx, call, ToolOutcome::SubmissionAccepted).await?;
            let (project_id, version_id, run_id) = (run.project_id, run.version_id, run.id);
            let mut waiting = run.into_active_model();
            waiting.status = Set(RunStatus::WaitingReview);
            waiting.round = Set(round);
            waiting.consecutive_protocol_errors = Set(0);
            waiting.queued_at = Set(None);
            waiting.updated_at = Set(tx.now());
            waiting.update(tx.db()).await?;
            let context = load_version(tx.db(), version_id).await?;
            advance(tx, context.workflow, project_id, stage, Some(run_id)).await?;
        }
        Err(problem) => {
            resolve(tx, call, ToolOutcome::SubmissionRejected { problem }).await?;
            let correction = Correction::SubmissionRejected;
            append_message(tx, run.id, RunMessageBody::Correction { correction }).await?;
            requeue(tx, run, false).await?;
        }
    }
    Ok(())
}

/// Queues the run for its next round. A protocol error counts toward the consecutive
/// limit; any other round, a rejected submission included, ends the series (§8.4).
pub(crate) async fn requeue(
    tx: &mut WriteTx,
    run: run::Model,
    protocol_error: bool,
) -> Result<(), StoreError> {
    let errors = if protocol_error {
        run.consecutive_protocol_errors + 1
    } else {
        0
    };
    if errors >= MAX_CONSECUTIVE_PROTOCOL_ERRORS {
        return halt(tx, run, RunStatus::Stopped, RunHalt::ProtocolErrorLimit).await;
    }
    if run.request_count >= run.request_allowance_end {
        return halt(tx, run, RunStatus::Stopped, RunHalt::RequestLimit).await;
    }
    let (version_id, project_id) = (run.version_id, run.project_id);
    let mut queued = run.into_active_model();
    queued.status = Set(RunStatus::Queued);
    queued.consecutive_protocol_errors = Set(errors);
    queued.queued_at = Set(Some(tx.now()));
    queued.updated_at = Set(tx.now());
    queued.update(tx.db()).await?;
    touch(tx, version_id, project_id).await?;
    Ok(())
}
