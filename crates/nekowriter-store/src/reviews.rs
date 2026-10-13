//! Review decisions (`docs/product-spec.md` §10). A decision is one command whatever the
//! review is about; what it does next depends on the review's kind.

use nekowriter_core::{
    conflict::Resource,
    ids::{CommandId, ReviewId},
    review::{Decision, DecisionSource, ReviewDecision, ReviewKind, ReviewStatus},
};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
use serde::Serialize;

use crate::{
    Store, StoreError,
    chapters::decide_chapter_review,
    command::{Begun, WriteTx},
    entity::{review, workflow},
    preparation::decide_preparation_review,
    proposals::decide_edit_review,
    workflow::{VersionContext, WorkspaceAck, load_version},
};

impl Store {
    /// Approves or rejects a review. A rejection carries the feedback the writer revises
    /// with.
    pub async fn decide_review(
        &self,
        command: CommandId,
        review_id: ReviewId,
        expected_revision: i64,
        decision: ReviewDecision,
    ) -> Result<WorkspaceAck, StoreError> {
        let input = (review_id, expected_revision, DecisionInput::from(&decision));
        let mut tx = match self.begin_command(command, "review.decide", &input).await? {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let review = review::Entity::find_by_id(review_id)
            .one(tx.db())
            .await?
            .ok_or(StoreError::NotFound(Resource::Review))?;
        let context = load_version(tx.db(), review.version_id).await?;
        context.check_revision(expected_revision)?;
        let workflow = apply_decision(tx.write(), context, review, decision.into()).await?;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }
}

/// Whether `source` may decide the review now: the user decides an open review, a review
/// agent the review delegated to it.
pub(crate) fn decidable(review: &review::Model, source: DecisionSource) -> bool {
    match source {
        DecisionSource::User => review.status == ReviewStatus::Open,
        DecisionSource::Delegate => review.status == ReviewStatus::Delegated,
    }
}

/// Marks the review decided. A delegate's rejection may come without feedback.
pub(crate) async fn record_decision(
    tx: &WriteTx,
    review: review::Model,
    decision: Decision,
) -> Result<review::Model, StoreError> {
    let mut decided = review.into_active_model();
    decided.status = Set(if decision.approved {
        ReviewStatus::Approved
    } else {
        ReviewStatus::Rejected
    });
    decided.feedback = Set(decision.feedback.filter(|_| !decision.approved));
    decided.decided_at = Set(Some(tx.now()));
    decided.decided_by = Set(Some(decision.source));
    decided.delegated_run_id = Set(None);
    decided.delegation_failure = Set(None);
    Ok(decided.update(tx.db()).await?)
}

/// Applies a decision by its review's kind.
pub(crate) async fn apply_decision(
    tx: &mut WriteTx,
    context: VersionContext,
    review: review::Model,
    decision: Decision,
) -> Result<workflow::Model, StoreError> {
    match review.kind {
        ReviewKind::PreparationBackground
        | ReviewKind::PreparationResearch
        | ReviewKind::PreparationCharacters
        | ReviewKind::PreparationOutline => {
            decide_preparation_review(tx, context, review, decision).await
        }
        ReviewKind::ChapterOutline | ReviewKind::ChapterFinal => {
            decide_chapter_review(tx, context, review, decision).await
        }
        ReviewKind::ChapterEdit => decide_edit_review(tx, context, review, decision).await,
    }
}

/// The decision as hashed for idempotency.
#[derive(Serialize)]
enum DecisionInput<'a> {
    Approve,
    Reject(&'a str),
}

impl<'a> From<&'a ReviewDecision> for DecisionInput<'a> {
    fn from(decision: &'a ReviewDecision) -> Self {
        match decision {
            ReviewDecision::Approve => Self::Approve,
            ReviewDecision::Reject { feedback } => Self::Reject(feedback.as_str()),
        }
    }
}
