//! Proposals (`docs/product-spec.md` §9.5, §9.6): in `ApproveRequired` mode an edit that
//! would change the body waits for the user's review instead of being committed.

use nekowriter_core::{
    body::{self, BodyEdit, EditProblem, EditResult},
    call::CallRequest,
    conflict::{Resource, StateConflict},
    document::DocumentRole,
    ids::{CommandId, ReviewId, RevisionId, ToolCallId, VersionId},
    review::{Decision, ReviewKind, ReviewStatus},
    run::RunStatus,
    run_message::ToolOutcome,
    workflow::{ApprovalMode, ChapterStep, WorkflowStage},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Store, StoreError,
    calls::{requeue, resolve},
    chapters::apply_edit,
    command::{Begun, WriteTx},
    documents::{chapter_document, current_revision},
    entity::{
        review, run,
        tool_call::{self, ToolCallStatus},
        workflow,
    },
    feedback::insert_bound_feedback,
    reviews::{decidable, record_decision},
    workflow::{VersionContext, WorkspaceAck, load_version, touch},
    workspace::ReviewView,
};

/// A proposed edit as the dock and the reader show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProposalView {
    pub review: ReviewView,
    pub call_id: ToolCallId,
    pub edit: BodyEdit,
    /// The body revision the edit applies to; `None` for an empty body.
    pub base: Option<RevisionId>,
}

impl Store {
    /// Switches between applying edits at once and reviewing each (§9.6). A proposal that
    /// is already pending stays pending; edits after the switch follow the new mode.
    pub async fn set_approval_mode(
        &self,
        command: CommandId,
        version_id: VersionId,
        mode: ApprovalMode,
    ) -> Result<WorkspaceAck, StoreError> {
        let input = (version_id, mode);
        let mut tx = match self
            .begin_command(command, "version.approval_mode", &input)
            .await?
        {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let context = load_version(tx.db(), version_id).await?;
        let mut workflow = context.workflow.into_active_model();
        workflow.approval_mode = Set(mode);
        workflow.update(tx.db()).await?;
        let workflow = touch(tx.write(), version_id, context.project.id).await?;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }
}

/// Whether the edit has to wait for review: the version reviews each edit and the edit
/// would change the body. Returns the body revision it applies to.
pub(crate) async fn needs_review<C: ConnectionTrait>(
    db: &C,
    run: &run::Model,
    edit: &BodyEdit,
) -> Result<Option<Option<RevisionId>>, StoreError> {
    let workflow = load_version(db, run.version_id).await?.workflow;
    if workflow.approval_mode != ApprovalMode::ApproveRequired {
        return Ok(None);
    }
    let chapter_id = run
        .chapter_id
        .ok_or_else(|| StoreError::Corrupt(format!("body run {} has no chapter", run.id)))?;
    let document = chapter_document(db, chapter_id, DocumentRole::ChapterBody).await?;
    let current = current_revision(db, &document).await?;
    let text = current
        .as_ref()
        .map_or("", |revision| revision.content.as_str());
    Ok(match body::apply(text, edit) {
        Ok(EditResult::Changed(_)) => Some(current.map(|revision| revision.id)),
        // An edit that fails or changes nothing needs no approval.
        Ok(EditResult::NoChange) | Err(_) => None,
    })
}

/// Holds the call as a proposal: its edit review opens and the run waits for it.
pub(crate) async fn propose(
    tx: &mut WriteTx,
    run: run::Model,
    call: tool_call::Model,
    base: Option<RevisionId>,
) -> Result<(), StoreError> {
    let call_id = call.id;
    let mut proposed = call.into_active_model();
    proposed.status = Set(ToolCallStatus::AwaitingReview);
    proposed.base_revision_id = Set(base);
    proposed.update(tx.db()).await?;
    review::ActiveModel {
        id: Set(ReviewId::generate()),
        version_id: Set(run.version_id),
        chapter_id: Set(run.chapter_id),
        run_id: Set(run.id),
        round: Set(run.round),
        kind: Set(ReviewKind::ChapterEdit),
        target_revision_id: Set(None),
        target_call_id: Set(Some(call_id)),
        status: Set(ReviewStatus::Open),
        summary: Set(None),
        feedback: Set(None),
        decided_at: Set(None),
        created_at: Set(tx.now()),
        ..Default::default()
    }
    .insert(tx.db())
    .await?;
    let (version_id, project_id) = (run.version_id, run.project_id);
    let mut waiting = run.into_active_model();
    waiting.status = Set(RunStatus::WaitingReview);
    waiting.queued_at = Set(None);
    waiting.updated_at = Set(tx.now());
    waiting.update(tx.db()).await?;
    touch(tx, version_id, project_id).await?;
    Ok(())
}

/// Decides an edit review. Accepting commits the edit if the body is still the one it was
/// proposed against; declining leaves the body unchanged and sends the feedback with the
/// result. Either way the writer goes on (§9.5).
pub(crate) async fn decide_edit_review(
    tx: &mut WriteTx,
    context: VersionContext,
    review: review::Model,
    decision: Decision,
) -> Result<workflow::Model, StoreError> {
    let not_open = || StoreError::StateConflict(StateConflict::ReviewNotOpen);
    let drafting = matches!(
        context.workflow.stage,
        WorkflowStage::Chapter {
            step: ChapterStep::Drafting,
            ..
        }
    );
    if !decidable(&review, decision.source)
        || !drafting
        || context.workflow.active_run_id != Some(review.run_id)
    {
        return Err(not_open());
    }
    let run = run::Entity::find_by_id(review.run_id)
        .one(tx.db())
        .await?
        .ok_or(StoreError::NotFound(Resource::Run))?;
    let call = match review.target_call_id {
        Some(id) => tool_call::Entity::find_by_id(id).one(tx.db()).await?,
        None => None,
    }
    .filter(|call| call.status == ToolCallStatus::AwaitingReview)
    .ok_or_else(not_open)?;
    if run.status != RunStatus::WaitingReview {
        return Err(not_open());
    }
    let Some(CallRequest::Edit { edit }) = call.parsed.clone() else {
        return Err(StoreError::Corrupt(format!(
            "proposal {} is not an edit",
            call.id
        )));
    };

    let approved = decision.approved;
    let feedback = decision.feedback.clone();
    record_decision(tx, review, decision).await?;
    let outcome = if approved {
        accept(tx, &run, &call, &edit).await?
    } else {
        if let Some(feedback) = feedback {
            insert_bound_feedback(tx, run.id, call.id, &feedback).await?;
        }
        ToolOutcome::EditDeclined
    };
    resolve(tx, &call, outcome).await?;
    let (version_id, project_id) = (run.version_id, run.project_id);
    requeue(tx, run, false).await?;
    touch(tx, version_id, project_id).await
}

/// Commits an accepted proposal, unless the body changed since it was proposed.
async fn accept(
    tx: &mut WriteTx,
    run: &run::Model,
    call: &tool_call::Model,
    edit: &BodyEdit,
) -> Result<ToolOutcome, StoreError> {
    let chapter_id = run
        .chapter_id
        .ok_or_else(|| StoreError::Corrupt(format!("body run {} has no chapter", run.id)))?;
    let document = chapter_document(tx.db(), chapter_id, DocumentRole::ChapterBody).await?;
    if document.current_revision_id != call.base_revision_id {
        return Ok(ToolOutcome::EditRejected {
            problem: EditProblem::StaleBase,
        });
    }
    apply_edit(tx, run, edit).await
}

/// The active run's pending proposal.
pub(crate) async fn proposal_view<C: ConnectionTrait>(
    db: &C,
    run_id: nekowriter_core::ids::RunId,
) -> Result<Option<ProposalView>, StoreError> {
    let Some(call) = tool_call::Entity::find()
        .filter(tool_call::Column::RunId.eq(run_id))
        .filter(tool_call::Column::Status.eq(ToolCallStatus::AwaitingReview))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    let review = review::Entity::find()
        .filter(review::Column::TargetCallId.eq(call.id))
        .one(db)
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("proposal {} has no review", call.id)))?;
    let Some(CallRequest::Edit { edit }) = call.parsed else {
        return Err(StoreError::Corrupt(format!(
            "proposal {} is not an edit",
            call.id
        )));
    };
    Ok(Some(ProposalView {
        review: ReviewView::from(&review),
        call_id: call.id,
        edit,
        base: call.base_revision_id,
    }))
}
