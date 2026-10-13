//! Proposals in `ApproveRequired` mode and switching modes (`docs/product-spec.md` §9.5,
//! §9.6).

mod common;

use common::{
    answer_with, append, body, dispatch_next, drafting, edit_round as edit, next_results,
    open_store, workspace,
};
use nekowriter_core::{
    body::{BodyEdit, EditProblem},
    conflict::StateConflict,
    feedback::FeedbackText,
    ids::{CommandId, RunId, VersionId},
    review::{ReviewDecision, ReviewFeedback},
    run::RunStatus,
    run_message::ToolOutcome,
    workflow::ApprovalMode,
};
use nekowriter_store::{Store, StoreError};

async fn set_mode(store: &Store, version: VersionId, mode: ApprovalMode) {
    store
        .set_approval_mode(CommandId::generate(), version, mode)
        .await
        .unwrap();
}

async fn decide_proposal(store: &Store, version: VersionId, decision: ReviewDecision) {
    let view = workspace(store, version).await;
    let review = view.proposal.expect("an edit is proposed").review_id;
    store
        .decide_review(CommandId::generate(), review, view.revision, decision)
        .await
        .unwrap();
}

/// A body run in `ApproveRequired` mode whose first edit is proposed.
async fn proposed(store: &Store) -> (VersionId, RunId) {
    let (version, run, _) = drafting(store).await;
    set_mode(store, version, ApprovalMode::ApproveRequired).await;
    let (_, request) = dispatch_next(store).await;
    answer_with(store, request, append("Rain fell.")).await;
    (version, run)
}

#[tokio::test]
async fn an_edit_waits_as_a_proposal_until_it_is_accepted() {
    let (_dir, store) = open_store().await;
    let (version, _) = proposed(&store).await;

    let view = workspace(&store, version).await;
    assert_eq!(view.approval_mode, ApprovalMode::ApproveRequired);
    assert_eq!(view.run.as_ref().unwrap().status, RunStatus::WaitingReview);
    let proposal = view.proposal.unwrap();
    assert_eq!(
        proposal.edit,
        BodyEdit::Append {
            content: "Rain fell.".to_owned()
        }
    );
    assert_eq!(proposal.base, None);
    assert_eq!(body(&store, version).await, None);

    decide_proposal(&store, version, ReviewDecision::Approve).await;

    let view = workspace(&store, version).await;
    assert_eq!(view.run.unwrap().status, RunStatus::Queued);
    assert_eq!(view.proposal, None);
    assert_eq!(body(&store, version).await, Some(2));
    assert!(matches!(
        next_results(&store).await[0].0,
        ToolOutcome::EditApplied { .. }
    ));
}

#[tokio::test]
async fn a_declined_proposal_leaves_the_body_and_returns_the_feedback() {
    let (_dir, store) = open_store().await;
    let (version, run) = proposed(&store).await;
    // While the proposal waits, the review's feedback is the way to talk to the writer.
    let refused = store
        .submit_feedback(
            CommandId::generate(),
            run,
            FeedbackText::parse("Not yet.").unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        refused,
        StoreError::StateConflict(StateConflict::RunNotWriting)
    ));

    decide_proposal(
        &store,
        version,
        ReviewDecision::Reject {
            feedback: ReviewFeedback::parse("Use snow instead.").unwrap(),
        },
    )
    .await;

    assert_eq!(body(&store, version).await, None);
    assert_eq!(
        workspace(&store, version).await.run.unwrap().status,
        RunStatus::Queued
    );
    assert_eq!(
        next_results(&store).await,
        [(
            ToolOutcome::EditDeclined,
            vec!["Use snow instead.".to_owned()]
        )]
    );
}

#[tokio::test]
async fn edits_that_fail_or_change_nothing_need_no_approval() {
    let (_dir, store) = open_store().await;
    let (version, _, _) = drafting(&store).await;
    set_mode(&store, version, ApprovalMode::ApproveRequired).await;
    let (_, request) = dispatch_next(&store).await;

    answer_with(
        &store,
        request,
        edit(BodyEdit::Replace {
            old_str: "missing".to_owned(),
            new_str: "x".to_owned(),
        }),
    )
    .await;

    let view = workspace(&store, version).await;
    assert_eq!(view.proposal, None);
    assert_eq!(view.run.unwrap().status, RunStatus::Queued);
    assert_eq!(
        next_results(&store).await[0].0,
        ToolOutcome::EditRejected {
            problem: EditProblem::NoMatch
        }
    );
}

#[tokio::test]
async fn switching_back_to_auto_keeps_a_pending_proposal_and_applies_later_edits() {
    let (_dir, store) = open_store().await;
    let (version, _) = proposed(&store).await;

    set_mode(&store, version, ApprovalMode::AutoApprove).await;
    assert!(workspace(&store, version).await.proposal.is_some());
    decide_proposal(&store, version, ReviewDecision::Approve).await;

    let (_, request) = dispatch_next(&store).await;
    answer_with(&store, request, append(" Snow too.")).await;
    let view = workspace(&store, version).await;
    assert_eq!(view.proposal, None);
    assert_eq!(body(&store, version).await, Some(4));
}

#[tokio::test]
async fn an_edit_held_by_a_pause_goes_to_review_once_the_mode_requires_it() {
    let (_dir, store) = open_store().await;
    let (version, run, _) = drafting(&store).await;
    let (_, request) = dispatch_next(&store).await;
    store
        .request_pause(CommandId::generate(), run)
        .await
        .unwrap();
    answer_with(&store, request, append("Rain fell.")).await;

    set_mode(&store, version, ApprovalMode::ApproveRequired).await;
    store.resume_run(CommandId::generate(), run).await.unwrap();

    let view = workspace(&store, version).await;
    assert_eq!(view.run.unwrap().status, RunStatus::WaitingReview);
    assert!(view.proposal.is_some());
    assert_eq!(body(&store, version).await, None);
}
