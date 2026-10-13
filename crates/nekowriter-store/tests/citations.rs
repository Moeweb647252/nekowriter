//! Citations in feedback and review rejections (`docs/product-spec.md` §18.4).

mod common;

use common::{answer_with, append, dispatch_next, drafting, open_store, workspace};
use nekowriter_core::{
    citation::{
        ChapterLabel, Citation, CitationProblem, CitationSource, CitationTarget, VerifiedCitation,
    },
    document::DocumentRole,
    feedback::FeedbackText,
    ids::{CommandId, RevisionId, RunId, VersionId},
    review::{Rejection, ReviewDecision},
    run_message::{FeedbackItem, ToolOutcome, UnsealedMessage},
    stage::StageInput,
    workflow::ApprovalMode,
};
use nekowriter_store::{ProposalView, Store, StoreError, TranscriptEntry, WorkspaceView};

/// A body run whose chapter body reads "Rain fell on the roof.", and that body's revision.
async fn written(store: &Store) -> (VersionId, RunId, RevisionId) {
    let (version, run, _) = drafting(store).await;
    let (_, request) = dispatch_next(store).await;
    answer_with(store, request, append("Rain fell on the roof.")).await;
    let revision = workspace(store, version).await.chapters[0]
        .body
        .as_ref()
        .expect("the body is written")
        .id;
    (version, run, revision)
}

fn quoting(target: CitationTarget, start: u32, end: u32, quote: &str) -> Citation {
    Citation {
        target,
        start,
        end,
        quote: quote.to_owned(),
    }
}

async fn feedback(store: &Store, run: RunId, citations: Vec<Citation>) -> Result<(), StoreError> {
    store
        .submit_feedback(
            CommandId::generate(),
            run,
            FeedbackText::parse("Darker.").unwrap(),
            citations,
        )
        .await
        .map(|_| ())
}

/// The tool results the next request will carry, with their feedback.
async fn next_results(store: &Store) -> Vec<(ToolOutcome, Vec<FeedbackItem>)> {
    let run = store.next_queued_run().await.unwrap().unwrap();
    store
        .dispatch_input(run)
        .await
        .unwrap()
        .messages
        .into_iter()
        .filter_map(|entry| match entry {
            TranscriptEntry::Pending {
                message:
                    UnsealedMessage::ToolResult {
                        outcome, feedback, ..
                    },
                ..
            } => Some((outcome, feedback)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn feedback_quotes_the_body_and_the_writer_learns_where_from() {
    let (_dir, store) = open_store().await;
    let (version, run, revision_id) = written(&store).await;
    let fell = quoting(CitationTarget::Revision { revision_id }, 5, 9, "fell");

    feedback(&store, run, vec![fell.clone()]).await.unwrap();

    let view = workspace(&store, version).await;
    assert_eq!(view.feedback[0].citations, std::slice::from_ref(&fell));
    let chapter = &view.chapters[0];
    // Queued feedback travels with the result of the writer's next call.
    let (_, request) = dispatch_next(&store).await;
    answer_with(&store, request, append(" Then snow.")).await;
    assert_eq!(
        next_results(&store).await[0].1,
        [FeedbackItem {
            text: "Darker.".to_owned(),
            citations: vec![VerifiedCitation {
                citation: fell,
                source: CitationSource::Document {
                    role: DocumentRole::ChapterBody,
                    chapter: Some(ChapterLabel {
                        kind: chapter.kind,
                        number: chapter.number,
                        title: chapter.title.clone(),
                    }),
                },
            }],
        }]
    );
}

#[tokio::test]
async fn a_citation_must_quote_its_text_exactly() {
    let (_dir, store) = open_store().await;
    let (version, run, revision_id) = written(&store).await;
    let body = CitationTarget::Revision { revision_id };
    let refused = [
        (
            quoting(body.clone(), 5, 9, "fall"),
            CitationProblem::QuoteMismatch,
        ),
        (
            quoting(body.clone(), 18, 40, "roof."),
            CitationProblem::BadRange,
        ),
        (
            quoting(
                CitationTarget::Revision {
                    revision_id: RevisionId::generate(),
                },
                0,
                4,
                "Rain",
            ),
            CitationProblem::TargetGone,
        ),
    ];
    for (citation, problem) in refused {
        let error = feedback(&store, run, vec![citation]).await.unwrap_err();
        assert!(
            matches!(&error, StoreError::CitationInvalid(found) if *found == problem),
            "{error:?}"
        );
    }
    assert_eq!(workspace(&store, version).await.feedback, Vec::new());
}

/// A body run in `ApproveRequired` mode whose first edit, "Rain fell.", is proposed, and
/// a citation of "Rain" in the proposed text.
async fn proposed(store: &Store) -> (WorkspaceView, ProposalView, Citation) {
    let (version, _, _) = drafting(store).await;
    store
        .set_approval_mode(
            CommandId::generate(),
            version,
            ApprovalMode::ApproveRequired,
        )
        .await
        .unwrap();
    let (_, request) = dispatch_next(store).await;
    answer_with(store, request, append("Rain fell.")).await;
    let view = workspace(store, version).await;
    let proposal = view.proposal.clone().expect("an edit is proposed");
    let rain = quoting(
        CitationTarget::Proposal {
            call_id: proposal.call_id,
            candidate_hash: proposal.candidate_hash.clone(),
        },
        0,
        4,
        "Rain",
    );
    (view, proposal, rain)
}

#[tokio::test]
async fn a_declined_proposal_can_be_answered_with_citations_alone() {
    let (_dir, store) = open_store().await;
    let (view, proposal, rain) = proposed(&store).await;

    store
        .decide_review(
            CommandId::generate(),
            proposal.review.id,
            view.revision,
            ReviewDecision::Reject(Rejection::new(None, vec![rain.clone()]).unwrap()),
        )
        .await
        .unwrap();

    let results = next_results(&store).await;
    assert!(matches!(results[0].0, ToolOutcome::EditDeclined));
    assert_eq!(
        results[0].1,
        [FeedbackItem {
            text: String::new(),
            citations: vec![VerifiedCitation {
                citation: rain,
                source: CitationSource::Proposal,
            }],
        }]
    );
}

#[tokio::test]
async fn delegation_notes_carry_their_citations_to_the_review_agent() {
    let (_dir, store) = open_store().await;
    let (view, proposal, rain) = proposed(&store).await;

    store
        .delegate_review(
            CommandId::generate(),
            proposal.review.id,
            view.revision,
            None,
            vec![rain.clone()],
        )
        .await
        .unwrap();

    let (input, _) = dispatch_next(&store).await;
    let StageInput::Review { notes, .. } = input.stage else {
        panic!("a review run: {:?}", input.stage);
    };
    assert_eq!(
        notes,
        Some(FeedbackItem {
            text: String::new(),
            citations: vec![VerifiedCitation {
                citation: rain,
                source: CitationSource::Proposal,
            }],
        })
    );
}
