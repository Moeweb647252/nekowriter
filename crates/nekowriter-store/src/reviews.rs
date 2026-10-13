//! Review decisions (`docs/product-spec.md` §10). A decision is one command whatever the
//! review is about; what it does next depends on the review's kind.

use nekowriter_core::{
    conflict::Resource,
    ids::{CommandId, ReviewId},
    review::{ReviewDecision, ReviewKind},
};
use sea_orm::EntityTrait;
use serde::Serialize;

use crate::{
    Store, StoreError,
    chapters::decide_chapter_review,
    command::Begun,
    entity::review,
    preparation::decide_preparation_review,
    proposals::decide_edit_review,
    workflow::{WorkspaceAck, load_version},
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
        let workflow = match review.kind {
            ReviewKind::PreparationBackground
            | ReviewKind::PreparationResearch
            | ReviewKind::PreparationCharacters
            | ReviewKind::PreparationOutline => {
                decide_preparation_review(tx.write(), context, review, decision).await?
            }
            ReviewKind::ChapterOutline | ReviewKind::ChapterFinal => {
                decide_chapter_review(tx.write(), context, review, decision).await?
            }
            ReviewKind::ChapterEdit => {
                decide_edit_review(tx.write(), context, review, decision).await?
            }
        };
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
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
