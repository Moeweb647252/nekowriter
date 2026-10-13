//! Preparation through the store: brief, start, rounds of submissions and reviews, and the
//! fold that ends the stage (`docs/product-spec.md` §6, §8, §11).

mod common;

use common::{
    answer, briefed, decide, dispatch_next, preparation_submission, reject, review_of, stage,
    started, submit, workspace,
};

use std::collections::BTreeSet;

use nekowriter_core::{
    call::StageSubmission,
    chapter::ChapterStatus,
    conflict::StateConflict,
    ids::{CommandId, ModelRequestId, VersionId},
    model::ModelProblem,
    preparation::{ArtifactKind, NovelBrief},
    review::{ReviewDecision, ReviewStatus},
    run::{RunHalt, RunStatus},
    run_message::{
        ArgumentsProblem, Correction, ModelToolCall, ProtocolProblem, ToolOutcome, UnsealedMessage,
    },
    stage::StageInput,
    workflow::{ChapterStep, PreparationStep, WorkflowStage},
};
use nekowriter_store::{DispatchInput, RoundRecord, Store, StoreError, TranscriptEntry};

#[tokio::test]
async fn preparation_needs_a_ready_model_to_start() {
    let (_dir, store) = common::open_store().await;
    let (version, revision) = briefed(&store).await;

    let error = store
        .start_preparation(CommandId::generate(), version, revision)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        StoreError::ModelNotReady(ModelProblem::ServiceNotConfigured)
    ));
    assert_eq!(
        stage(&workspace(&store, version).await),
        &WorkflowStage::NEW_VERSION
    );
}

#[tokio::test]
async fn a_first_round_asks_for_all_four_artifacts_from_the_brief() {
    let (_dir, store) = common::open_store().await;
    started(&store).await;

    let run = store.next_queued_run().await.unwrap().unwrap();
    let input = store.dispatch_input(run).await.unwrap();

    assert_eq!(
        input.stage,
        StageInput::Preparation {
            brief: "A ghost story in a lighthouse.".to_owned(),
            required: ArtifactKind::ALL.into_iter().collect(),
        }
    );
    assert_eq!(input.messages, Vec::new());
}

const DOWNSTREAM: [ArtifactKind; 3] = [
    ArtifactKind::Background,
    ArtifactKind::Characters,
    ArtifactKind::Outline,
];

fn call() -> ModelToolCall {
    common::call("submit_preparation")
}

fn pending_messages(input: &DispatchInput) -> Vec<UnsealedMessage> {
    input
        .messages
        .iter()
        .filter_map(|entry| match entry {
            TranscriptEntry::Pending { message, .. } => Some(message.clone()),
            TranscriptEntry::Rendered(_) => None,
        })
        .collect()
}

async fn required_next(store: &Store) -> BTreeSet<ArtifactKind> {
    let run = store.next_queued_run().await.unwrap().unwrap();
    let StageInput::Preparation { required, .. } = store.dispatch_input(run).await.unwrap().stage
    else {
        panic!("a preparation run is queued");
    };
    required
}

/// Round 1: everything is submitted; research is rejected, the rest approved.
async fn round_one_rejects_research(store: &Store, version: VersionId) {
    submit(store, &ArtifactKind::ALL).await;
    let view = workspace(store, version).await;
    assert_eq!(
        stage(&view),
        &WorkflowStage::Preparation {
            step: PreparationStep::Reviewing
        }
    );
    assert_eq!(view.run.as_ref().unwrap().status, RunStatus::WaitingReview);
    for kind in ArtifactKind::ALL {
        assert_eq!(review_of(&view, kind).1, ReviewStatus::Open, "{kind:?}");
    }
    for kind in DOWNSTREAM {
        decide(store, version, kind, ReviewDecision::Approve).await;
    }
    decide(
        store,
        version,
        ArtifactKind::Research,
        reject("Cite the tide tables."),
    )
    .await;
    assert_eq!(
        stage(&workspace(store, version).await),
        &WorkflowStage::Preparation {
            step: PreparationStep::Revising
        }
    );

    // The revision asks for research only. The model first receives the accepted
    // submission's result, then the review outcome.
    let run = store.next_queued_run().await.unwrap().unwrap();
    let input = store.dispatch_input(run).await.unwrap();
    assert_eq!(
        required_next(store).await,
        BTreeSet::from([ArtifactKind::Research])
    );
    let pending = pending_messages(&input);
    assert!(matches!(
        pending[0],
        UnsealedMessage::ToolResult {
            outcome: ToolOutcome::SubmissionAccepted,
            ..
        }
    ));
    let UnsealedMessage::PreparationReviewOutcome { rejected, stale } = &pending[1] else {
        panic!("the review outcome follows the tool result");
    };
    assert_eq!(rejected[0].artifact, ArtifactKind::Research);
    assert_eq!(rejected[0].feedback, "Cite the tide tables.");
    assert_eq!(stale, &Vec::<ArtifactKind>::new());
}

/// Round 2: new research makes everything written from it stale.
async fn round_two_makes_dependents_stale(store: &Store, version: VersionId) {
    submit(store, &[ArtifactKind::Research]).await;
    let view = workspace(store, version).await;
    assert_eq!(
        review_of(&view, ArtifactKind::Research).1,
        ReviewStatus::Open
    );
    for kind in DOWNSTREAM {
        assert_eq!(review_of(&view, kind).1, ReviewStatus::Stale, "{kind:?}");
    }
    decide(
        store,
        version,
        ArtifactKind::Research,
        ReviewDecision::Approve,
    )
    .await;
    assert_eq!(required_next(store).await, BTreeSet::from(DOWNSTREAM));
}

/// Round 3: the stale artifacts are resubmitted and approved, and the stage folds.
async fn round_three_folds(store: &Store, version: VersionId) {
    submit(store, &DOWNSTREAM).await;
    for kind in DOWNSTREAM {
        decide(store, version, kind, ReviewDecision::Approve).await;
    }
    let view = workspace(store, version).await;
    assert_eq!(view.chapters.len(), 2);
    assert_eq!(view.chapters[0].title, "Arrival");
    assert_eq!(view.chapters[0].status, ChapterStatus::NotStarted);
    assert_eq!(
        stage(&view),
        &WorkflowStage::Chapter {
            chapter_id: view.chapters[0].id,
            step: ChapterStep::Ready
        }
    );
    assert!(view.run.is_none());
    assert_eq!(store.next_queued_run().await.unwrap(), None);
}

#[tokio::test]
async fn preparation_runs_through_rejection_and_staleness_to_the_fold() {
    let (_dir, store) = common::open_store().await;
    let version = started(&store).await;

    round_one_rejects_research(&store, version).await;
    round_two_makes_dependents_stale(&store, version).await;
    round_three_folds(&store, version).await;
}

#[tokio::test]
async fn a_rejected_submission_is_corrected_without_counting_as_a_protocol_error() {
    let (_dir, store) = common::open_store().await;
    let version = started(&store).await;

    submit(&store, &[ArtifactKind::Background]).await;

    let view = workspace(&store, version).await;
    let run = view.run.unwrap();
    assert_eq!(run.status, RunStatus::Queued);
    assert_eq!(run.consecutive_protocol_errors, 0);
    let input = store.dispatch_input(run.id).await.unwrap();
    let pending: Vec<_> = input
        .messages
        .iter()
        .filter_map(|entry| match entry {
            TranscriptEntry::Pending { message, .. } => Some(message.clone()),
            TranscriptEntry::Rendered(_) => None,
        })
        .collect();
    assert!(matches!(
        pending[0],
        UnsealedMessage::ToolResult {
            outcome: ToolOutcome::SubmissionRejected { .. },
            ..
        }
    ));
    assert_eq!(
        pending[1],
        UnsealedMessage::Correction {
            correction: Correction::SubmissionRejected
        }
    );
}

#[tokio::test]
async fn three_protocol_errors_in_a_row_stop_the_run_until_it_is_continued() {
    let (_dir, store) = common::open_store().await;
    let version = started(&store).await;
    let broken = RoundRecord::ProtocolError {
        calls: vec![(
            call(),
            ProtocolProblem::InvalidArguments {
                problem: ArgumentsProblem::NotJson,
            },
        )],
        correction: Correction::ProtocolError,
    };

    for _ in 0..3 {
        let (_, request) = dispatch_next(&store).await;
        store
            .record_answer(request, answer(&call()), broken.clone())
            .await
            .unwrap();
    }

    let view = workspace(&store, version).await;
    let run = view.run.clone().unwrap();
    assert_eq!(run.status, RunStatus::Stopped);
    assert_eq!(run.halt, Some(RunHalt::ProtocolErrorLimit));
    assert_eq!(store.next_queued_run().await.unwrap(), None);

    store
        .continue_run(CommandId::generate(), run.id, view.revision)
        .await
        .unwrap();
    let view = workspace(&store, version).await;
    let run = view.run.unwrap();
    assert_eq!(run.status, RunStatus::Queued);
    assert_eq!(run.consecutive_protocol_errors, 0);
    assert_eq!(run.request_allowance_end, run.request_count + 100);
}

#[tokio::test]
async fn messages_are_rendered_once_when_a_request_seals_them() {
    let (_dir, store) = common::open_store().await;
    started(&store).await;
    submit(&store, &[ArtifactKind::Research]).await;

    let (first, _) = dispatch_next(&store).await;
    let pending = first
        .messages
        .iter()
        .filter(|entry| matches!(entry, TranscriptEntry::Pending { .. }))
        .count();
    assert_eq!(pending, 2);

    // The next request sees the same messages already rendered.
    let run = first.run_id;
    store
        .record_answer(
            store_request(&store, run).await,
            answer(&call()),
            common::submit_round(
                "submit_preparation",
                StageSubmission::Preparation {
                    submission: preparation_submission(&[ArtifactKind::Research]),
                },
            ),
        )
        .await
        .unwrap();
    let next = store.dispatch_input(run).await.unwrap();
    assert!(matches!(next.messages[1], TranscriptEntry::Rendered(_)));
    assert!(matches!(next.messages[2], TranscriptEntry::Rendered(_)));
}

/// The request currently in flight for `run`.
async fn store_request(store: &Store, run: nekowriter_core::ids::RunId) -> ModelRequestId {
    let version = store.dispatch_input(run).await.unwrap().version_id;
    workspace(store, version)
        .await
        .run
        .unwrap()
        .request_in_flight
        .expect("a request is in flight")
}

#[tokio::test]
async fn interrupted_runs_wait_for_the_user_to_continue() {
    let (_dir, store) = common::open_store().await;
    let version = started(&store).await;
    let (_, request) = dispatch_next(&store).await;

    assert_eq!(store.interrupt_runs().await.unwrap(), 1);

    let view = workspace(&store, version).await;
    let run = view.run.unwrap();
    assert_eq!(run.status, RunStatus::Interrupted);
    assert_eq!(run.halt, Some(RunHalt::ProcessStopped));
    assert_eq!(store.next_queued_run().await.unwrap(), None);
    // An answer that arrives after the interruption does not move the run.
    store
        .record_answer(
            request,
            answer(&call()),
            common::submit_round(
                "submit_preparation",
                StageSubmission::Preparation {
                    submission: preparation_submission(&ArtifactKind::ALL),
                },
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        workspace(&store, version).await.run.unwrap().status,
        RunStatus::Interrupted
    );
}

#[tokio::test]
async fn a_project_writes_one_version_at_a_time_and_the_brief_locks_at_the_start() {
    let (_dir, store) = common::open_store().await;
    let first = started(&store).await;
    let project = workspace(&store, first).await.project;
    let project = store
        .create_version(
            CommandId::generate(),
            project.id,
            project.revision,
            nekowriter_core::project::NovelTitle::parse("Second draft").unwrap(),
        )
        .await
        .unwrap();
    let second = project.selected_version.id;
    let view = workspace(&store, second).await;
    let ack = store
        .save_brief(
            CommandId::generate(),
            second,
            view.revision,
            NovelBrief::parse("Another take.").unwrap(),
        )
        .await
        .unwrap();

    let busy = store
        .start_preparation(CommandId::generate(), second, ack.revision)
        .await
        .unwrap_err();
    assert!(matches!(
        busy,
        StoreError::StateConflict(StateConflict::ProjectBusy)
    ));

    let locked = store
        .save_brief(
            CommandId::generate(),
            first,
            workspace(&store, first).await.revision,
            NovelBrief::parse("Changed my mind.").unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        locked,
        StoreError::StateConflict(StateConflict::BriefLocked)
    ));
}

#[tokio::test]
async fn a_stopped_run_ignores_its_late_answer_and_continues_when_asked() {
    let (_dir, store) = common::open_store().await;
    let version = started(&store).await;
    let (input, request) = dispatch_next(&store).await;

    store
        .stop_run(CommandId::generate(), input.run_id)
        .await
        .unwrap();
    let run = workspace(&store, version).await.run.unwrap();
    assert_eq!(
        (run.status, run.halt),
        (RunStatus::Stopped, Some(RunHalt::StoppedByUser))
    );
    let again = store
        .stop_run(CommandId::generate(), input.run_id)
        .await
        .unwrap_err();
    assert!(matches!(
        again,
        StoreError::StateConflict(StateConflict::RunNotStoppable)
    ));

    // An answer that still arrives counts as usage only.
    store
        .record_answer(
            request,
            answer(&call()),
            common::submit_round(
                "submit_preparation",
                StageSubmission::Preparation {
                    submission: preparation_submission(&ArtifactKind::ALL),
                },
            ),
        )
        .await
        .unwrap();
    let view = workspace(&store, version).await;
    assert_eq!(view.run.as_ref().unwrap().status, RunStatus::Stopped);
    assert_eq!(
        stage(&view),
        &WorkflowStage::Preparation {
            step: PreparationStep::Generating
        }
    );

    store
        .continue_run(CommandId::generate(), input.run_id, view.revision)
        .await
        .unwrap();
    assert_eq!(
        workspace(&store, version).await.run.unwrap().status,
        RunStatus::Queued
    );
}
