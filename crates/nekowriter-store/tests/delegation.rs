//! Delegating reviews to a review agent (`docs/product-spec.md` §10).

mod common;

use common::{
    answer_with, append, call, dispatch_next, drafting, next_results, open_store, review_of,
    started, submit, workspace,
};
use nekowriter_core::{
    call::{CallRequest, StageSubmission},
    ids::{CommandId, ReviewId, VersionId},
    preparation::ArtifactKind,
    review::{DecisionSource, ReviewKind, ReviewStatus},
    run::{ProviderFailure, RunHalt, RunKind, RunStatus},
    run_message::{ToolOutcome, UnsealedMessage},
    stage::{ReviewSubject, StageInput},
    submission::{SubmissionProblem, VerdictSubmission},
    workflow::{ApprovalMode, PreparationStep, WorkflowStage},
};
use nekowriter_store::{RoundRecord, Store, TranscriptEntry};

fn verdict(passed: bool, confidence: f64, issues: &[&str]) -> RoundRecord {
    RoundRecord::Call {
        call: call("submit_review"),
        request: CallRequest::Submit {
            submission: StageSubmission::Review {
                verdict: VerdictSubmission {
                    passed,
                    confidence: serde_json::Number::from_f64(confidence).unwrap(),
                    issues: issues.iter().map(|&issue| issue.to_owned()).collect(),
                },
            },
        },
    }
}

async fn delegate(store: &Store, version: VersionId, review: ReviewId, notes: Option<&str>) {
    let revision = workspace(store, version).await.revision;
    store
        .delegate_review(
            CommandId::generate(),
            review,
            revision,
            notes.map(str::to_owned),
        )
        .await
        .unwrap();
}

/// A version whose first preparation round waits for review, with all four delegated.
async fn preparation_in_review(store: &Store) -> VersionId {
    let version = started(store).await;
    submit(store, &ArtifactKind::ALL).await;
    version
}

#[tokio::test]
async fn a_delegated_review_is_decided_by_the_agents_verdict() {
    let (_dir, store) = open_store().await;
    let version = preparation_in_review(&store).await;
    let (review, _) = review_of(&workspace(&store, version).await, ArtifactKind::Research);
    delegate(&store, version, review, Some("Check the sources.")).await;
    assert_eq!(
        review_of(&workspace(&store, version).await, ArtifactKind::Research).1,
        ReviewStatus::Delegated
    );

    let (input, request) = dispatch_next(&store).await;
    let StageInput::Review {
        kind,
        subject,
        notes,
    } = &input.stage
    else {
        panic!("a review run: {:?}", input.stage);
    };
    assert_eq!(*kind, ReviewKind::PreparationResearch);
    assert!(matches!(
        subject,
        ReviewSubject::Artifact {
            artifact: ArtifactKind::Research,
            ..
        }
    ));
    assert_eq!(notes.as_deref(), Some("Check the sources."));
    answer_with(&store, request, verdict(true, 0.9, &[])).await;

    let view = workspace(&store, version).await;
    let research = view
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == ArtifactKind::Research)
        .unwrap()
        .review
        .clone()
        .unwrap();
    assert_eq!(research.status, ReviewStatus::Approved);
    assert_eq!(research.decided_by, Some(DecisionSource::Delegate));
    assert_eq!(research.confidence_permille, Some(900));
    // The writer still waits for the other three reviews.
    assert_eq!(view.run.unwrap().status, RunStatus::WaitingReview);
}

#[tokio::test]
async fn a_failed_verdict_without_issues_still_tells_the_writer() {
    let (_dir, store) = open_store().await;
    let version = preparation_in_review(&store).await;
    let view = workspace(&store, version).await;
    for kind in ArtifactKind::ALL {
        let (review, _) = review_of(&view, kind);
        if kind == ArtifactKind::Outline {
            delegate(&store, version, review, None).await;
        } else {
            let revision = workspace(&store, version).await.revision;
            store
                .decide_review(
                    CommandId::generate(),
                    review,
                    revision,
                    nekowriter_core::review::ReviewDecision::Approve,
                )
                .await
                .unwrap();
        }
    }
    let (_, request) = dispatch_next(&store).await;
    answer_with(&store, request, verdict(false, 0.4, &[" "])).await;

    let view = workspace(&store, version).await;
    assert_eq!(
        view.version.stage,
        WorkflowStage::Preparation {
            step: PreparationStep::Revising
        }
    );
    let run = store.next_queued_run().await.unwrap().unwrap();
    let input = store.dispatch_input(run).await.unwrap();
    let Some(TranscriptEntry::Pending {
        message: UnsealedMessage::PreparationReviewOutcome { rejected, .. },
        ..
    }) = input.messages.last()
    else {
        panic!("the round's outcome is pending");
    };
    assert_eq!(rejected[0].artifact, ArtifactKind::Outline);
    assert_eq!(rejected[0].feedback, None);
}

#[tokio::test]
async fn a_cancelled_delegation_ignores_the_late_verdict() {
    let (_dir, store) = open_store().await;
    let version = preparation_in_review(&store).await;
    let (review, _) = review_of(&workspace(&store, version).await, ArtifactKind::Background);
    delegate(&store, version, review, None).await;
    let (input, request) = dispatch_next(&store).await;

    store
        .cancel_delegation(CommandId::generate(), review)
        .await
        .unwrap();
    answer_with(&store, request, verdict(true, 1.0, &[])).await;

    let view = workspace(&store, version).await;
    let background = review_of(&view, ArtifactKind::Background);
    assert_eq!(background.1, ReviewStatus::Open);
    assert_eq!(
        store.run_status(input.run_id).await.unwrap(),
        RunStatus::Failed
    );
}

#[tokio::test]
async fn a_failed_delegation_returns_the_review_with_the_reason() {
    let (_dir, store) = open_store().await;
    let version = preparation_in_review(&store).await;
    let (review, _) = review_of(&workspace(&store, version).await, ArtifactKind::Characters);
    delegate(&store, version, review, None).await;
    let (_, request) = dispatch_next(&store).await;

    store
        .record_request_failure(request, ProviderFailure::HttpStatus { status: 500 })
        .await
        .unwrap();

    let view = workspace(&store, version).await;
    let characters = view
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == ArtifactKind::Characters)
        .unwrap()
        .review
        .clone()
        .unwrap();
    assert_eq!(characters.status, ReviewStatus::Open);
    assert_eq!(
        characters.delegation_failure,
        Some(RunHalt::Provider {
            failure: ProviderFailure::HttpStatus { status: 500 }
        })
    );
}

#[tokio::test]
async fn a_verdict_with_a_bad_confidence_is_a_rejected_round() {
    let (_dir, store) = open_store().await;
    let version = preparation_in_review(&store).await;
    let (review, _) = review_of(&workspace(&store, version).await, ArtifactKind::Research);
    delegate(&store, version, review, None).await;
    let (_, request) = dispatch_next(&store).await;

    answer_with(&store, request, verdict(true, 7.0, &[])).await;

    assert_eq!(
        review_of(&workspace(&store, version).await, ArtifactKind::Research).1,
        ReviewStatus::Delegated
    );
    assert_eq!(
        next_results(&store).await[0].0,
        ToolOutcome::SubmissionRejected {
            problem: SubmissionProblem::ConfidenceOutOfRange
        }
    );
    let (_, request) = dispatch_next(&store).await;
    answer_with(&store, request, verdict(true, 0.5, &[])).await;
    assert_eq!(
        review_of(&workspace(&store, version).await, ArtifactKind::Research).1,
        ReviewStatus::Approved
    );
}

#[tokio::test]
async fn a_delegated_edit_review_applies_an_approved_edit() {
    let (_dir, store) = open_store().await;
    let (version, _, _) = drafting(&store).await;
    store
        .set_approval_mode(
            CommandId::generate(),
            version,
            ApprovalMode::ApproveRequired,
        )
        .await
        .unwrap();
    let (_, request) = dispatch_next(&store).await;
    answer_with(&store, request, append("Rain fell.")).await;
    let review = workspace(&store, version).await.proposal.unwrap().review.id;

    delegate(&store, version, review, None).await;
    let (input, request) = dispatch_next(&store).await;
    assert!(matches!(
        input.stage,
        StageInput::Review {
            kind: ReviewKind::ChapterEdit,
            subject: ReviewSubject::Edit { .. },
            ..
        }
    ));
    answer_with(&store, request, verdict(true, 0.8, &[])).await;

    let view = workspace(&store, version).await;
    assert_eq!(view.proposal, None);
    assert_eq!(view.run.as_ref().unwrap().kind, RunKind::ChapterBody);
    assert_eq!(view.run.unwrap().status, RunStatus::Queued);
    assert!(matches!(
        next_results(&store).await[0].0,
        ToolOutcome::EditApplied { .. }
    ));
}
