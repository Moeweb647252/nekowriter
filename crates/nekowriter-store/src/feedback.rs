//! Feedback, pause and resume (`docs/product-spec.md` §9.2–§9.4).
//!
//! A run's queued and bound feedback form its one unsealed batch. The batch travels with
//! one tool result: a result that is not sent yet if the batch is bound to one (or the user
//! catches a requested pause before it is held), otherwise the result of the next call. The
//! request that seals the result includes the whole batch, in order.

use nekowriter_core::{
    body::BodyEdit,
    call::CallRequest,
    citation::{Citation, Citations, VerifiedCitation},
    conflict::{Resource, StateConflict},
    feedback::{FeedbackDelivery, FeedbackStatus, FeedbackText},
    ids::{CommandId, FeedbackId, ModelRequestId, RunId, ToolCallId},
    run::{RunKind, RunStatus},
    run_message::{FeedbackItem, RunMessageBody},
    time::Timestamp,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Store, StoreError,
    calls::{execute, held_call},
    citations::verify_citations,
    command::{Begun, WriteTx},
    entity::{
        feedback,
        model_request::{self, ModelRequestStatus},
        run, run_message,
        tool_call::{self, ToolCallStatus},
    },
    workflow::{VersionContext, WorkspaceAck, load_version, touch},
};

/// Feedback as the dock shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FeedbackView {
    pub id: FeedbackId,
    pub text: String,
    pub citations: Vec<Citation>,
    pub status: FeedbackStatus,
    /// For included feedback: what became of the request that carried it.
    pub delivery: Option<FeedbackDelivery>,
    pub created_at: Timestamp,
}

/// The call a paused run holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HeldCallView {
    pub id: ToolCallId,
    pub tool: String,
    /// The edit, if the held call is one; otherwise it is the stage's submission.
    pub edit: Option<BodyEdit>,
}

impl Store {
    /// Gives a writing run feedback (§9.2, §9.3). Feedback while a pause is requested
    /// clears the request; feedback while a call is held clears the pause and carries the
    /// call out, so the feedback travels with its result.
    pub async fn submit_feedback(
        &self,
        command: CommandId,
        run_id: RunId,
        text: FeedbackText,
        citations: Vec<Citation>,
    ) -> Result<WorkspaceAck, StoreError> {
        let input = (run_id, &text, &citations);
        let mut tx = match self
            .begin_command(command, "feedback.submit", &input)
            .await?
        {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let (run, context) = writing_run(tx.db(), run_id).await?;
        let citations = verify_citations(tx.db(), context.version.id, &citations).await?;
        let batch = feedback::Entity::find()
            .filter(feedback::Column::RunId.eq(run_id))
            .filter(feedback::Column::Status.is_in([FeedbackStatus::Queued, FeedbackStatus::Bound]))
            .order_by_asc(feedback::Column::Sequence)
            .all(tx.db())
            .await?;
        let bound_to = batch.iter().find_map(|item| item.bound_tool_call_id);
        let (status, bound_to, anchor) = match bound_to {
            // The batch already travels with a result that is not sent yet.
            Some(call) => (FeedbackStatus::Bound, Some(call), None),
            None => match latest_unsealed_result(tx.db(), run_id).await? {
                // Caught before a requested pause was held: the batch takes the latest
                // result, and the pause is no longer needed.
                Some(call) if context.workflow.pause_requested => {
                    for item in batch {
                        bind(tx.db(), item, call).await?;
                    }
                    (FeedbackStatus::Bound, Some(call), None)
                }
                // Otherwise the batch waits for the result of a later call.
                _ => {
                    let anchor = match batch.first().and_then(|first| first.after_tool_sequence) {
                        Some(anchor) => anchor,
                        None => highest_resolved_sequence(tx.db(), run_id).await?,
                    };
                    (FeedbackStatus::Queued, None, Some(anchor))
                }
            },
        };
        let sequence = next_sequence(tx.db(), run_id).await?;
        feedback::ActiveModel {
            id: Set(FeedbackId::generate()),
            run_id: Set(run_id),
            sequence: Set(sequence),
            text: Set(text.as_str().to_owned()),
            citations: Set(stored(citations)),
            status: Set(status),
            after_tool_sequence: Set(anchor),
            bound_tool_call_id: Set(bound_to),
            included_in: Set(None),
            created_at: Set(tx.now()),
        }
        .insert(tx.db())
        .await?;

        set_pause_requested(tx.db(), &context, false).await?;
        if run.status == RunStatus::Paused {
            resume(tx.write(), run).await?;
        }
        let workflow = touch(tx.write(), context.version.id, context.project.id).await?;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }

    /// Asks the writing run to pause at its next tool call (§9.3). A run that is already
    /// paused stays paused.
    pub async fn request_pause(
        &self,
        command: CommandId,
        run_id: RunId,
    ) -> Result<WorkspaceAck, StoreError> {
        self.set_pause(command, "run.pause", run_id, true).await
    }

    /// Withdraws a pause request that has not been held yet.
    pub async fn cancel_pause(
        &self,
        command: CommandId,
        run_id: RunId,
    ) -> Result<WorkspaceAck, StoreError> {
        self.set_pause(command, "run.cancel_pause", run_id, false)
            .await
    }

    async fn set_pause(
        &self,
        command: CommandId,
        kind: &'static str,
        run_id: RunId,
        requested: bool,
    ) -> Result<WorkspaceAck, StoreError> {
        let mut tx = match self.begin_command(command, kind, &run_id).await? {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let (run, context) = writing_run(tx.db(), run_id).await?;
        if run.status != RunStatus::Paused {
            set_pause_requested(tx.db(), &context, requested).await?;
        }
        let workflow = touch(tx.write(), context.version.id, context.project.id).await?;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }

    /// Continues a paused run: its held call is carried out through the current gate and
    /// queued feedback travels with the result (§9.3).
    pub async fn resume_run(
        &self,
        command: CommandId,
        run_id: RunId,
    ) -> Result<WorkspaceAck, StoreError> {
        let mut tx = match self.begin_command(command, "run.resume", &run_id).await? {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let (run, context) = writing_run(tx.db(), run_id).await?;
        if run.status != RunStatus::Paused {
            return Err(StoreError::StateConflict(StateConflict::RunNotPaused));
        }
        resume(tx.write(), run).await?;
        let workflow = touch(tx.write(), context.version.id, context.project.id).await?;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }
}

/// The run, if it is a chapter body run that is writing.
async fn writing_run<C: ConnectionTrait>(
    db: &C,
    run_id: RunId,
) -> Result<(run::Model, VersionContext), StoreError> {
    let run = run::Entity::find_by_id(run_id)
        .one(db)
        .await?
        .ok_or(StoreError::NotFound(Resource::Run))?;
    let context = load_version(db, run.version_id).await?;
    if run.kind != RunKind::ChapterBody || !run.status.is_writing() {
        return Err(StoreError::StateConflict(StateConflict::RunNotWriting));
    }
    Ok((run, context))
}

async fn set_pause_requested<C: ConnectionTrait>(
    db: &C,
    context: &VersionContext,
    requested: bool,
) -> Result<(), StoreError> {
    if context.workflow.pause_requested != requested {
        let mut workflow = context.workflow.clone().into_active_model();
        workflow.pause_requested = Set(requested);
        workflow.update(db).await?;
    }
    Ok(())
}

/// Carries out the paused run's held call.
async fn resume(tx: &mut WriteTx, run: run::Model) -> Result<(), StoreError> {
    let call = held_call(tx.db(), run.id)
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("paused run {} holds no call", run.id)))?;
    execute(tx, run, call).await
}

async fn bind<C: ConnectionTrait>(
    db: &C,
    item: feedback::Model,
    call: ToolCallId,
) -> Result<(), StoreError> {
    let mut bound = item.into_active_model();
    bound.status = Set(FeedbackStatus::Bound);
    bound.bound_tool_call_id = Set(Some(call));
    bound.update(db).await?;
    Ok(())
}

/// Feedback from a declined proposal: it travels with that call's result.
pub(crate) async fn insert_bound_feedback(
    tx: &WriteTx,
    run_id: RunId,
    call: ToolCallId,
    text: &str,
    citations: Vec<VerifiedCitation>,
) -> Result<(), StoreError> {
    let sequence = next_sequence(tx.db(), run_id).await?;
    feedback::ActiveModel {
        id: Set(FeedbackId::generate()),
        run_id: Set(run_id),
        sequence: Set(sequence),
        text: Set(text.to_owned()),
        citations: Set(stored(citations)),
        status: Set(FeedbackStatus::Bound),
        after_tool_sequence: Set(None),
        bound_tool_call_id: Set(Some(call)),
        included_in: Set(None),
        created_at: Set(tx.now()),
    }
    .insert(tx.db())
    .await?;
    Ok(())
}

async fn next_sequence<C: ConnectionTrait>(db: &C, run_id: RunId) -> Result<i32, StoreError> {
    Ok(feedback::Entity::find()
        .filter(feedback::Column::RunId.eq(run_id))
        .order_by_desc(feedback::Column::Sequence)
        .one(db)
        .await?
        .map_or(1, |last| last.sequence + 1))
}

/// Binds the run's queued batch to the result of `call`, if the batch waits for a result
/// of a call after its anchor.
pub(crate) async fn bind_queued(tx: &WriteTx, call: &tool_call::Model) -> Result<(), StoreError> {
    let queued = feedback::Entity::find()
        .filter(feedback::Column::RunId.eq(call.run_id))
        .filter(feedback::Column::Status.eq(FeedbackStatus::Queued))
        .all(tx.db())
        .await?;
    for item in queued {
        if item
            .after_tool_sequence
            .is_none_or(|anchor| call.sequence > anchor)
        {
            bind(tx.db(), item, call.id).await?;
        }
    }
    Ok(())
}

/// Whether the run has feedback that waits for a result.
pub(crate) async fn has_queued<C: ConnectionTrait>(
    db: &C,
    run_id: RunId,
) -> Result<bool, StoreError> {
    Ok(feedback::Entity::find()
        .filter(feedback::Column::RunId.eq(run_id))
        .filter(feedback::Column::Status.eq(FeedbackStatus::Queued))
        .one(db)
        .await?
        .is_some())
}

/// The feedback bound to the call's result, in order.
pub(crate) async fn bound_feedback<C: ConnectionTrait>(
    db: &C,
    call: ToolCallId,
) -> Result<Vec<FeedbackItem>, StoreError> {
    Ok(feedback::Entity::find()
        .filter(feedback::Column::BoundToolCallId.eq(call))
        .filter(feedback::Column::Status.eq(FeedbackStatus::Bound))
        .order_by_asc(feedback::Column::Sequence)
        .all(db)
        .await?
        .into_iter()
        .map(|item| FeedbackItem {
            text: item.text,
            citations: item
                .citations
                .map(|citations| citations.0)
                .unwrap_or_default(),
        })
        .collect())
}

/// Citations as a stored column: none is no column value.
pub(crate) fn stored(citations: Vec<VerifiedCitation>) -> Option<Citations> {
    (!citations.is_empty()).then_some(Citations(citations))
}

/// The run's newest feedback, so a request built before it arrived is built again.
pub(crate) async fn feedback_marker<C: ConnectionTrait>(
    db: &C,
    run_id: RunId,
) -> Result<i32, StoreError> {
    Ok(feedback::Entity::find()
        .filter(feedback::Column::RunId.eq(run_id))
        .order_by_desc(feedback::Column::Sequence)
        .one(db)
        .await?
        .map_or(0, |last| last.sequence))
}

/// Marks the run's bound feedback included in the request that seals its results.
pub(crate) async fn seal_feedback(
    tx: &WriteTx,
    run_id: RunId,
    request: ModelRequestId,
) -> Result<(), StoreError> {
    let bound = feedback::Entity::find()
        .filter(feedback::Column::RunId.eq(run_id))
        .filter(feedback::Column::Status.eq(FeedbackStatus::Bound))
        .all(tx.db())
        .await?;
    for item in bound {
        let mut included = item.into_active_model();
        included.status = Set(FeedbackStatus::Included);
        included.included_in = Set(Some(request));
        included.update(tx.db()).await?;
    }
    Ok(())
}

/// The call whose result the next request will carry last, if any result is not sent yet.
async fn latest_unsealed_result<C: ConnectionTrait>(
    db: &C,
    run_id: RunId,
) -> Result<Option<ToolCallId>, StoreError> {
    let unsealed = run_message::Entity::find()
        .filter(run_message::Column::RunId.eq(run_id))
        .filter(run_message::Column::SealedBy.is_null())
        .order_by_desc(run_message::Column::Id)
        .all(db)
        .await?;
    Ok(unsealed.into_iter().find_map(|message| match message.body {
        RunMessageBody::ToolResult { tool_call_id } => Some(tool_call_id),
        _ => None,
    }))
}

async fn highest_resolved_sequence<C: ConnectionTrait>(
    db: &C,
    run_id: RunId,
) -> Result<i32, StoreError> {
    Ok(tool_call::Entity::find()
        .filter(tool_call::Column::RunId.eq(run_id))
        .filter(tool_call::Column::Status.eq(ToolCallStatus::Resolved))
        .order_by_desc(tool_call::Column::Sequence)
        .one(db)
        .await?
        .map_or(0, |call| call.sequence))
}

/// The feedback the dock shows for the run: the unsealed batch, then the feedback the
/// latest request carried while that request has not answered (§9.2).
pub(crate) async fn feedback_views<C: ConnectionTrait>(
    db: &C,
    run_id: RunId,
) -> Result<Vec<FeedbackView>, StoreError> {
    let latest = model_request::Entity::find()
        .filter(model_request::Column::RunId.eq(run_id))
        .order_by_desc(model_request::Column::Attempt)
        .one(db)
        .await?;
    let delivery = latest.as_ref().and_then(|request| match request.status {
        ModelRequestStatus::Dispatched => Some(FeedbackDelivery::Sent),
        ModelRequestStatus::Failed => Some(FeedbackDelivery::Failed),
        ModelRequestStatus::Unknown => Some(FeedbackDelivery::Unknown),
        ModelRequestStatus::Cancelled => Some(FeedbackDelivery::Cancelled),
        ModelRequestStatus::Succeeded => None,
    });
    let items = feedback::Entity::find()
        .filter(feedback::Column::RunId.eq(run_id))
        .order_by_asc(feedback::Column::Sequence)
        .all(db)
        .await?;
    Ok(items
        .into_iter()
        .filter_map(|item| {
            let delivery = match item.status {
                FeedbackStatus::Queued | FeedbackStatus::Bound => None,
                FeedbackStatus::Included => {
                    let carried_by_latest = latest
                        .as_ref()
                        .is_some_and(|request| item.included_in == Some(request.id));
                    Some(delivery.filter(|_| carried_by_latest)?)
                }
            };
            Some(FeedbackView {
                id: item.id,
                text: item.text,
                citations: item
                    .citations
                    .map(|citations| {
                        citations
                            .0
                            .into_iter()
                            .map(|cited| cited.citation)
                            .collect()
                    })
                    .unwrap_or_default(),
                status: item.status,
                delivery,
                created_at: item.created_at,
            })
        })
        .collect())
}

/// The call the run holds, as the dock shows it.
pub(crate) async fn held_view<C: ConnectionTrait>(
    db: &C,
    run_id: RunId,
) -> Result<Option<HeldCallView>, StoreError> {
    Ok(held_call(db, run_id).await?.map(|call| HeldCallView {
        id: call.id,
        tool: call.name,
        edit: match call.parsed {
            Some(CallRequest::Edit { edit }) => Some(edit),
            _ => None,
        },
    }))
}
