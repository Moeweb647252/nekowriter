//! Runs and the model (`docs/product-spec.md` §8.2, §8.7, §16.3): queued runs, dispatching
//! requests, recording answers and failures, continuing, and interrupting runs when the
//! process stops.
//!
//! The application orchestrates the calls in this order: [`Store::next_queued_run`],
//! [`Store::dispatch_input`], then (after the agent built the request)
//! [`Store::dispatch`], and finally [`Store::record_answer`] or
//! [`Store::record_request_failure`]. Each step checks the run's state again, so a run
//! that was stopped or changed in between is never advanced by a stale step.

use std::collections::BTreeSet;

use nekowriter_core::{
    call::CallRequest,
    conflict::{Resource, StateConflict},
    document::DocumentRole,
    ids::{ChapterId, CommandId, ModelRequestId, ProjectId, RunId, VersionId},
    model::{
        EffectiveModel, ModelProblem, ModelProfile, ModelService, ThinkingLevel, effective_model,
    },
    preparation::ArtifactKind,
    project::WritingLanguage,
    review::ReviewStatus,
    run::{ProviderFailure, REQUEST_ALLOWANCE, RunHalt, RunKind, RunStatus},
    run_message::{
        Correction, ModelToolCall, ProtocolProblem, RejectedArtifact, RunMessageBody, ToolOutcome,
        TranscriptMessage, UnsealedMessage,
    },
    stage::{FoldDocument, FoldInput, FoldStage, StageInput},
    usage::TokenUsage,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, Set,
};

use crate::{
    Store, StoreError,
    calls::{execute, held_call, hold, insert_call, requeue, resolve},
    chapters::{chapter_input, chapter_text, find_chapter},
    command::{Begun, WriteTx},
    delegation::{release_delegation, review_subject},
    documents::{current_revision, version_document},
    entity::{
        document,
        model_request::{self, ModelRequestStatus},
        project, review, revision, run, run_message, system_settings, tool_call,
    },
    feedback::{bound_feedback, feedback_marker, seal_feedback},
    preparation::required_artifacts,
    workflow::{VersionContext, WorkspaceAck, load_version, project_has_other_active_run, touch},
};

/// Everything the application needs to build a run's next request.
#[derive(Clone, Debug)]
pub struct DispatchInput {
    pub run_id: RunId,
    pub project_id: ProjectId,
    pub version_id: VersionId,
    pub writing_language: WritingLanguage,
    pub stage: StageInput,
    /// The folds of the version's completed stages, in story order (§11).
    pub folds: Vec<FoldInput>,
    /// The run's conversation, in order.
    pub messages: Vec<TranscriptEntry>,
    pub model: ModelSettings,
    /// The newest message the conversation had; [`Store::dispatch`] refuses to seal if
    /// another one arrived since.
    last_message_id: Option<i64>,
    /// The newest feedback the run had, for the same check: feedback can bind to a result
    /// without adding a message.
    feedback_marker: i32,
}

/// A message of the conversation, as the model sees it or still to be rendered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptEntry {
    Rendered(TranscriptMessage),
    Pending { id: i64, message: UnsealedMessage },
}

/// The stored settings a request's model is resolved from.
#[derive(Clone, Debug)]
pub struct ModelSettings {
    pub service: Option<ModelService>,
    pub default_model: Option<ModelProfile>,
    pub project_model: Option<ModelProfile>,
    pub thinking: ThinkingLevel,
}

/// The request that [`Store::dispatch`] seals.
#[derive(Clone, Debug)]
pub struct SealedRequest {
    /// The rendering of every pending message, by message ID.
    pub rendered: Vec<(i64, TranscriptMessage)>,
    /// The request body, as JSON.
    pub body: String,
    pub estimated_input_tokens: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// The request is recorded and the run is running; send it now.
    Dispatched { request_id: ModelRequestId },
    /// The run is no longer queued or its conversation changed; build the request again
    /// if it is still queued.
    Changed,
}

/// A complete answer from the model service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedAnswer {
    pub content: Option<String>,
    pub tool_calls: Vec<ModelToolCall>,
    pub finish_reason: Option<String>,
    pub usage: TokenUsage,
    pub raw_usage: Option<String>,
    pub provider_request_id: Option<String>,
}

/// What the answer means for the run, as the agent classified it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoundRecord {
    /// A valid call of one of the stage's tools.
    Call {
        call: ModelToolCall,
        request: CallRequest,
    },
    ProtocolError {
        calls: Vec<(ModelToolCall, ProtocolProblem)>,
        correction: Correction,
    },
}

impl Store {
    /// The run that has waited longest for the model, if any.
    pub async fn next_queued_run(&self) -> Result<Option<RunId>, StoreError> {
        Ok(run::Entity::find()
            .filter(run::Column::Status.eq(RunStatus::Queued))
            .order_by_asc(run::Column::QueuedAt)
            .one(&self.db)
            .await?
            .map(|run| run.id))
    }

    pub async fn dispatch_input(&self, run_id: RunId) -> Result<DispatchInput, StoreError> {
        let run = find_run(&self.db, run_id).await?;
        let project = project::Entity::find_by_id(run.project_id)
            .one(&self.db)
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("run {run_id} has no project")))?;
        let stage = stage_input(&self.db, &run).await?;
        let folds = folds(&self.db, run.version_id).await?;
        let rows = run_message::Entity::find()
            .filter(run_message::Column::RunId.eq(run_id))
            .order_by_asc(run_message::Column::Id)
            .all(&self.db)
            .await?;
        let last_message_id = rows.last().map(|row| row.id);
        let feedback_marker = feedback_marker(&self.db, run_id).await?;
        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            messages.push(match row.rendered {
                Some(rendered) => TranscriptEntry::Rendered(rendered),
                None => TranscriptEntry::Pending {
                    id: row.id,
                    message: self.resolve(&run, row.body).await?,
                },
            });
        }
        let model = model_settings(&self.db, &project).await?;
        Ok(DispatchInput {
            run_id,
            project_id: run.project_id,
            version_id: run.version_id,
            writing_language: project.writing_language,
            stage,
            folds,
            messages,
            model,
            last_message_id,
            feedback_marker,
        })
    }

    /// Gathers the data a pending message refers to.
    async fn resolve(
        &self,
        run: &run::Model,
        body: RunMessageBody,
    ) -> Result<UnsealedMessage, StoreError> {
        match body {
            RunMessageBody::Assistant { .. } => Err(StoreError::Corrupt(
                "an assistant message was stored without its rendering".to_owned(),
            )),
            RunMessageBody::ToolResult { tool_call_id } => {
                let call = tool_call::Entity::find_by_id(tool_call_id)
                    .one(&self.db)
                    .await?
                    .ok_or_else(|| {
                        StoreError::Corrupt(format!("tool call {tool_call_id} is missing"))
                    })?;
                let outcome = call.outcome.ok_or_else(|| {
                    StoreError::Corrupt(format!("tool call {tool_call_id} has no outcome"))
                })?;
                Ok(UnsealedMessage::ToolResult {
                    provider_call_id: call.provider_id,
                    outcome,
                    feedback: bound_feedback(&self.db, tool_call_id).await?,
                })
            }
            RunMessageBody::PreparationReviewOutcome { round } => {
                let reviews = review::Entity::find()
                    .filter(review::Column::RunId.eq(run.id))
                    .filter(review::Column::Round.eq(round))
                    .all(&self.db)
                    .await?;
                let mut rejected = Vec::new();
                let mut stale = BTreeSet::new();
                for kind in ArtifactKind::ALL {
                    for review in reviews
                        .iter()
                        .filter(|review| review.kind.artifact() == Some(kind))
                    {
                        match review.status {
                            ReviewStatus::Rejected => rejected.push(RejectedArtifact {
                                artifact: kind,
                                feedback: review.feedback.clone(),
                            }),
                            ReviewStatus::Stale => {
                                stale.insert(kind);
                            }
                            ReviewStatus::Open
                            | ReviewStatus::Delegated
                            | ReviewStatus::Approved => {}
                        }
                    }
                }
                Ok(UnsealedMessage::PreparationReviewOutcome {
                    rejected,
                    stale: stale.into_iter().collect(),
                })
            }
            RunMessageBody::ReviewRejected { review_id } => {
                let review = review::Entity::find_by_id(review_id)
                    .one(&self.db)
                    .await?
                    .ok_or_else(|| StoreError::Corrupt(format!("review {review_id} is missing")))?;
                Ok(UnsealedMessage::ReviewRejected {
                    kind: review.kind,
                    feedback: review.feedback,
                })
            }
            RunMessageBody::Correction { correction } => {
                Ok(UnsealedMessage::Correction { correction })
            }
        }
    }

    /// Seals the request: renders the pending messages into the conversation, counts the
    /// request against the run's allowance and marks the run running.
    pub async fn dispatch(
        &self,
        input: &DispatchInput,
        request: SealedRequest,
    ) -> Result<DispatchOutcome, StoreError> {
        let mut tx = self.begin_internal().await?;
        let run = find_run(tx.db(), input.run_id).await?;
        let last_message_id = run_message::Entity::find()
            .filter(run_message::Column::RunId.eq(run.id))
            .order_by_desc(run_message::Column::Id)
            .one(tx.db())
            .await?
            .map(|row| row.id);
        if run.status != RunStatus::Queued
            || last_message_id != input.last_message_id
            || feedback_marker(tx.db(), run.id).await? != input.feedback_marker
        {
            return Ok(DispatchOutcome::Changed);
        }
        if run.request_count >= run.request_allowance_end {
            halt(&mut tx, run, RunStatus::Stopped, RunHalt::RequestLimit).await?;
            tx.commit().await?;
            return Ok(DispatchOutcome::Changed);
        }
        let request_id = ModelRequestId::generate();
        let now = tx.now();
        model_request::ActiveModel {
            id: Set(request_id),
            run_id: Set(run.id),
            attempt: Set(run.request_count + 1),
            status: Set(ModelRequestStatus::Dispatched),
            body: Set(request.body),
            estimated_input_tokens: Set(
                i32::try_from(request.estimated_input_tokens).unwrap_or(i32::MAX)
            ),
            finish_reason: Set(None),
            usage: Set(None),
            raw_usage: Set(None),
            failure: Set(None),
            provider_request_id: Set(None),
            dispatched_at: Set(now),
            completed_at: Set(None),
        }
        .insert(tx.db())
        .await?;
        let unsealed = run_message::Entity::find()
            .filter(run_message::Column::RunId.eq(run.id))
            .filter(run_message::Column::SealedBy.is_null())
            .all(tx.db())
            .await?;
        for row in unsealed {
            let rendered = match &row.rendered {
                Some(_) => None,
                None => Some(
                    request
                        .rendered
                        .iter()
                        .find(|(id, _)| *id == row.id)
                        .map(|(_, message)| message.clone())
                        .ok_or_else(|| {
                            StoreError::Corrupt(format!("message {} was not rendered", row.id))
                        })?,
                ),
            };
            let mut sealed = row.into_active_model();
            if let Some(rendered) = rendered {
                sealed.rendered = Set(Some(rendered));
            }
            sealed.sealed_by = Set(Some(request_id));
            sealed.update(tx.db()).await?;
        }
        seal_feedback(&tx, run.id, request_id).await?;
        let (version_id, project_id) = (run.version_id, run.project_id);
        let count = run.request_count + 1;
        let mut running = run.into_active_model();
        running.status = Set(RunStatus::Running);
        running.request_count = Set(count);
        running.queued_at = Set(None);
        running.updated_at = Set(now);
        running.update(tx.db()).await?;
        touch(&mut tx, version_id, project_id).await?;
        tx.commit().await?;
        Ok(DispatchOutcome::Dispatched { request_id })
    }

    /// Fails a queued or running run that cannot go on, for example because the model
    /// settings do not allow a request or the context does not fit. A request still in
    /// flight gets an unknown result. A run in any other state is left alone, because it
    /// changed since the caller looked at it.
    pub async fn fail_run(&self, run_id: RunId, reason: RunHalt) -> Result<(), StoreError> {
        let mut tx = self.begin_internal().await?;
        let run = find_run(tx.db(), run_id).await?;
        if matches!(run.status, RunStatus::Queued | RunStatus::Running) {
            let requests = model_request::Entity::find()
                .filter(model_request::Column::RunId.eq(run_id))
                .filter(model_request::Column::Status.eq(ModelRequestStatus::Dispatched))
                .all(tx.db())
                .await?;
            for request in requests {
                mark_unknown(&tx, request).await?;
            }
            halt(&mut tx, run, RunStatus::Failed, reason).await?;
        }
        tx.commit().await
    }

    /// Records a complete answer and what it means for the run.
    pub async fn record_answer(
        &self,
        request_id: ModelRequestId,
        answer: RecordedAnswer,
        round: RoundRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin_internal().await?;
        let request = find_request(tx.db(), request_id).await?;
        let run = find_run(tx.db(), request.run_id).await?;
        let current =
            request.status == ModelRequestStatus::Dispatched && run.status == RunStatus::Running;
        let in_flight = request.status == ModelRequestStatus::Dispatched;
        let mut completed = request.into_active_model();
        if in_flight {
            completed.status = Set(ModelRequestStatus::Succeeded);
        }
        completed.finish_reason = Set(answer.finish_reason);
        completed.usage = Set(Some(answer.usage));
        completed.raw_usage = Set(answer.raw_usage);
        completed.provider_request_id = Set(answer.provider_request_id);
        completed.completed_at = Set(Some(tx.now()));
        completed.update(tx.db()).await?;
        if !current {
            // The run was stopped or interrupted while the request was in flight; the
            // answer is kept for the usage record only, and a cancelled or unknown request
            // keeps that status.
            return tx.commit().await;
        }
        append_message(
            &mut tx,
            run.id,
            RunMessageBody::Assistant {
                content: answer.content,
                tool_calls: answer.tool_calls,
            },
        )
        .await?;
        match round {
            RoundRecord::Call { call, request } => {
                let workflow = load_version(tx.db(), run.version_id).await?.workflow;
                let call = insert_call(&tx, &run, request_id, &call, Some(request)).await?;
                if workflow.pause_requested {
                    hold(&mut tx, run, call, workflow).await?;
                } else {
                    execute(&mut tx, run, call).await?;
                }
            }
            RoundRecord::ProtocolError { calls, correction } => {
                for (call, problem) in calls {
                    let call = insert_call(&tx, &run, request_id, &call, None).await?;
                    resolve(&mut tx, &call, ToolOutcome::ProtocolError { problem }).await?;
                }
                append_message(&mut tx, run.id, RunMessageBody::Correction { correction }).await?;
                requeue(&mut tx, run, true).await?;
            }
        }
        tx.commit().await
    }

    /// Records that a request failed; the run fails with the reason.
    pub async fn record_request_failure(
        &self,
        request_id: ModelRequestId,
        failure: ProviderFailure,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin_internal().await?;
        let request = find_request(tx.db(), request_id).await?;
        let run = find_run(tx.db(), request.run_id).await?;
        if request.status != ModelRequestStatus::Dispatched {
            // Stopped or interrupted first: the request keeps that status.
            return tx.commit().await;
        }
        let current = run.status == RunStatus::Running;
        let mut failed = request.into_active_model();
        failed.status = Set(ModelRequestStatus::Failed);
        failed.failure = Set(Some(failure.clone()));
        failed.completed_at = Set(Some(tx.now()));
        failed.update(tx.db()).await?;
        if current {
            halt(
                &mut tx,
                run,
                RunStatus::Failed,
                RunHalt::Provider { failure },
            )
            .await?;
        }
        tx.commit().await
    }

    /// Continues a stopped, failed or interrupted run with a fresh request allowance
    /// (§8.7).
    pub async fn continue_run(
        &self,
        command: CommandId,
        run_id: RunId,
        expected_revision: i64,
    ) -> Result<WorkspaceAck, StoreError> {
        let input = (run_id, expected_revision);
        let mut tx = match self.begin_command(command, "run.continue", &input).await? {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let run = run::Entity::find_by_id(run_id)
            .one(tx.db())
            .await?
            .ok_or(StoreError::NotFound(Resource::Run))?;
        let context: VersionContext = load_version(tx.db(), run.version_id).await?;
        context.check_revision(expected_revision)?;
        if run.kind == RunKind::Review {
            return Err(StoreError::StateConflict(
                StateConflict::ReviewRunNotContinuable,
            ));
        }
        if !run.status.can_continue() {
            return Err(StoreError::StateConflict(StateConflict::RunNotContinuable));
        }
        if project_has_other_active_run(tx.db(), run.project_id, Some(run.id)).await? {
            return Err(StoreError::StateConflict(StateConflict::ProjectBusy));
        }
        check_model(tx.db(), &context.project).await?;
        let (count, resumes) = (run.request_count, run.resume_count);
        // A run stopped at a held call returns to waiting for the user there.
        let held = held_call(tx.db(), run.id).await?.is_some();
        let mut continued = run.into_active_model();
        continued.status = Set(if held {
            RunStatus::Paused
        } else {
            RunStatus::Queued
        });
        continued.halt = Set(None);
        continued.request_allowance_end = Set(count + REQUEST_ALLOWANCE);
        continued.consecutive_protocol_errors = Set(0);
        continued.resume_count = Set(resumes + 1);
        continued.queued_at = Set((!held).then(|| tx.now()));
        continued.updated_at = Set(tx.now());
        continued.update(tx.db()).await?;
        let workflow = touch(tx.write(), context.version.id, context.project.id).await?;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }

    /// Stops a run that is waiting for the model or talking to it (§16.3). A request in
    /// flight is cancelled; its answer, if it still arrives, only counts as usage.
    /// Committed text and feedback stay.
    pub async fn stop_run(
        &self,
        command: CommandId,
        run_id: RunId,
    ) -> Result<WorkspaceAck, StoreError> {
        let mut tx = match self.begin_command(command, "run.stop", &run_id).await? {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let run = run::Entity::find_by_id(run_id)
            .one(tx.db())
            .await?
            .ok_or(StoreError::NotFound(Resource::Run))?;
        let context = load_version(tx.db(), run.version_id).await?;
        if !run.status.can_stop() {
            return Err(StoreError::StateConflict(StateConflict::RunNotStoppable));
        }
        let requests = model_request::Entity::find()
            .filter(model_request::Column::RunId.eq(run_id))
            .filter(model_request::Column::Status.eq(ModelRequestStatus::Dispatched))
            .all(tx.db())
            .await?;
        for request in requests {
            let mut cancelled = request.into_active_model();
            cancelled.status = Set(ModelRequestStatus::Cancelled);
            cancelled.completed_at = Set(Some(tx.now()));
            cancelled.update(tx.db()).await?;
        }
        halt(tx.write(), run, RunStatus::Stopped, RunHalt::StoppedByUser).await?;
        let workflow = load_version(tx.db(), context.version.id).await?.workflow;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }

    /// The run's current status; the dispatcher watches it to abandon a request whose run
    /// stopped.
    pub async fn run_status(&self, run_id: RunId) -> Result<RunStatus, StoreError> {
        Ok(find_run(&self.db, run_id).await?.status)
    }

    /// Marks every executing run interrupted and every request in flight unknown, so
    /// nothing is sent until the user continues (§16.3). Used at startup and shutdown;
    /// returns the number of runs interrupted.
    pub async fn interrupt_runs(&self) -> Result<usize, StoreError> {
        let mut tx = self.begin_internal().await?;
        let requests = model_request::Entity::find()
            .filter(model_request::Column::Status.eq(ModelRequestStatus::Dispatched))
            .all(tx.db())
            .await?;
        for request in requests {
            mark_unknown(&tx, request).await?;
        }
        let runs = run::Entity::find()
            .filter(run::Column::Status.is_in([RunStatus::Queued, RunStatus::Running]))
            .all(tx.db())
            .await?;
        let interrupted = runs.len();
        for run in runs {
            halt(
                &mut tx,
                run,
                RunStatus::Interrupted,
                RunHalt::ProcessStopped,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(interrupted)
    }
}

impl ModelSettings {
    /// The model requests use, or why there is none (§13.3).
    pub fn resolve(self) -> Result<EffectiveModel, ModelProblem> {
        effective_model(
            self.service,
            self.default_model,
            self.project_model,
            self.thinking,
        )
    }
}

/// Fails unless the project's model settings allow requests. Commands that start or
/// continue model work run this check, and the dispatcher runs it again before sending.
pub(crate) async fn check_model<C: ConnectionTrait>(
    db: &C,
    project: &project::Model,
) -> Result<(), StoreError> {
    model_settings(db, project)
        .await?
        .resolve()
        .map(drop)
        .map_err(StoreError::ModelNotReady)
}

async fn model_settings<C: ConnectionTrait>(
    db: &C,
    project: &project::Model,
) -> Result<ModelSettings, StoreError> {
    let settings = system_settings::Entity::find_by_id(system_settings::ROW_ID)
        .one(db)
        .await?;
    Ok(ModelSettings {
        service: settings.as_ref().and_then(|row| {
            row.api_base_url.clone().map(|base_url| ModelService {
                base_url,
                api_key: row.api_key.clone(),
            })
        }),
        default_model: settings.and_then(|row| row.default_model),
        project_model: project.model.clone(),
        thinking: project.thinking_level,
    })
}

/// Creates a queued run for the version's current stage.
pub(crate) async fn create_run(
    tx: &mut WriteTx,
    context: &VersionContext,
    kind: RunKind,
    chapter_id: Option<ChapterId>,
) -> Result<run::Model, StoreError> {
    let now = tx.now();
    Ok(run::ActiveModel {
        id: Set(RunId::generate()),
        project_id: Set(context.project.id),
        version_id: Set(context.version.id),
        chapter_id: Set(chapter_id),
        review_id: Set(None),
        kind: Set(kind),
        status: Set(RunStatus::Queued),
        halt: Set(None),
        round: Set(0),
        request_count: Set(0),
        request_allowance_end: Set(kind.request_allowance()),
        consecutive_protocol_errors: Set(0),
        resume_count: Set(0),
        fold: Set(None),
        queued_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(tx.db())
    .await?)
}

/// Appends a message to a run's conversation. Assistant messages are stored rendered; the
/// others are rendered when a request seals them.
pub(crate) async fn append_message(
    tx: &mut WriteTx,
    run_id: RunId,
    body: RunMessageBody,
) -> Result<(), StoreError> {
    let rendered = match &body {
        RunMessageBody::Assistant {
            content,
            tool_calls,
        } => Some(TranscriptMessage::Assistant {
            content: content.clone(),
            tool_calls: tool_calls.clone(),
        }),
        _ => None,
    };
    run_message::ActiveModel {
        run_id: Set(run_id),
        body: Set(body),
        rendered: Set(rendered),
        sealed_by: Set(None),
        created_at: Set(tx.now()),
        ..Default::default()
    }
    .insert(tx.db())
    .await?;
    Ok(())
}

async fn mark_unknown(tx: &WriteTx, request: model_request::Model) -> Result<(), StoreError> {
    let mut unknown = request.into_active_model();
    unknown.status = Set(ModelRequestStatus::Unknown);
    unknown.completed_at = Set(Some(tx.now()));
    unknown.update(tx.db()).await?;
    Ok(())
}

/// Queues a run whose submission was rejected in review, so it revises. Returns its ID.
pub(crate) async fn queue_for_revision(
    tx: &mut WriteTx,
    run: run::Model,
) -> Result<RunId, StoreError> {
    let id = run.id;
    let mut queued = run.into_active_model();
    queued.status = Set(RunStatus::Queued);
    queued.queued_at = Set(Some(tx.now()));
    queued.updated_at = Set(tx.now());
    queued.update(tx.db()).await?;
    Ok(id)
}

/// What the run's stage is about.
async fn stage_input<C: ConnectionTrait>(
    db: &C,
    run: &run::Model,
) -> Result<StageInput, StoreError> {
    if run.kind == RunKind::Review {
        return review_subject(db, run).await;
    }
    let chapter = match run.chapter_id {
        Some(id) => Some(find_chapter(db, id).await?),
        None => None,
    };
    match (run.kind, chapter) {
        (RunKind::Preparation, None) => {
            let brief = version_document(db, run.version_id, DocumentRole::NovelBrief).await?;
            let brief = current_revision(db, &brief)
                .await?
                .ok_or(StoreError::StateConflict(StateConflict::BriefMissing))?;
            Ok(StageInput::Preparation {
                brief: brief.content,
                required: required_artifacts(db, run).await?,
            })
        }
        (RunKind::ChapterOutline, Some(chapter)) => Ok(StageInput::ChapterOutline {
            guidance: chapter_text(db, chapter.id, DocumentRole::ChapterGuidance).await?,
            chapter: chapter_input(chapter),
        }),
        (RunKind::ChapterBody, Some(chapter)) => Ok(StageInput::ChapterBody {
            guidance: chapter_text(db, chapter.id, DocumentRole::ChapterGuidance).await?,
            body: chapter_text(db, chapter.id, DocumentRole::ChapterBody).await?,
            chapter: chapter_input(chapter),
        }),
        (kind, _) => Err(StoreError::Corrupt(format!(
            "run {} of kind {kind:?} has the wrong chapter",
            run.id
        ))),
    }
}

/// The folds of the version's completed stages, in story order: the stages run one after
/// another, so their runs' creation order is the story order.
async fn folds<C: ConnectionTrait>(
    db: &C,
    version_id: VersionId,
) -> Result<Vec<FoldInput>, StoreError> {
    let runs = run::Entity::find()
        .filter(run::Column::VersionId.eq(version_id))
        .filter(run::Column::Status.eq(RunStatus::Succeeded))
        .filter(run::Column::Kind.ne(RunKind::Review))
        .order_by_asc(run::Column::CreatedAt)
        .all(db)
        .await?;
    let mut folds = Vec::with_capacity(runs.len());
    for run in runs {
        let fold = run
            .fold
            .ok_or_else(|| StoreError::Corrupt(format!("succeeded run {} has no fold", run.id)))?;
        let chapter = match run.chapter_id {
            Some(id) => Some(chapter_input(find_chapter(db, id).await?)),
            None => None,
        };
        let stage = match (run.kind, chapter) {
            (RunKind::Preparation, None) => FoldStage::Preparation,
            (RunKind::ChapterOutline, Some(chapter)) => FoldStage::ChapterOutline { chapter },
            (RunKind::ChapterBody, Some(chapter)) => FoldStage::ChapterBody { chapter },
            (kind, _) => {
                return Err(StoreError::Corrupt(format!(
                    "run {} of kind {kind:?} has the wrong chapter",
                    run.id
                )));
            }
        };
        let mut documents = Vec::with_capacity(fold.0.len());
        for id in fold.0 {
            let revision = revision::Entity::find_by_id(id)
                .one(db)
                .await?
                .ok_or_else(|| StoreError::Corrupt(format!("folded revision {id} is missing")))?;
            let role = document::Entity::find_by_id(revision.document_id)
                .one(db)
                .await?
                .ok_or_else(|| StoreError::Corrupt(format!("revision {id} has no document")))?
                .role;
            documents.push(FoldDocument {
                role,
                content: revision.content,
                chapter_plan: revision.chapter_plan,
            });
        }
        folds.push(FoldInput { stage, documents });
    }
    Ok(folds)
}

/// Moves the run to `status` (stopped, failed or interrupted) with the reason.
pub(crate) async fn halt(
    tx: &mut WriteTx,
    run: run::Model,
    status: RunStatus,
    reason: RunHalt,
) -> Result<(), StoreError> {
    let (version_id, project_id) = (run.version_id, run.project_id);
    let mut halted = run.into_active_model();
    halted.status = Set(status);
    halted.halt = Set(Some(reason.clone()));
    halted.queued_at = Set(None);
    halted.updated_at = Set(tx.now());
    let halted = halted.update(tx.db()).await?;
    if halted.kind == RunKind::Review {
        release_delegation(tx, &halted, reason).await?;
    }
    touch(tx, version_id, project_id).await?;
    Ok(())
}

pub(crate) async fn find_run<C: ConnectionTrait>(
    db: &C,
    id: RunId,
) -> Result<run::Model, StoreError> {
    run::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(StoreError::NotFound(Resource::Run))
}

async fn find_request<C: ConnectionTrait>(
    db: &C,
    id: ModelRequestId,
) -> Result<model_request::Model, StoreError> {
    model_request::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("model request {id} is missing")))
}
