//! Pause, feedback and their timing (`docs/product-spec.md` §9.2–§9.4): every row of the
//! `AutoApprove` table, through the store.

mod common;

use common::{
    answer_with, append, body, dispatch_next, drafting, next_results, open_store,
    submit_chapter as submit, workspace,
};
use nekowriter_core::{
    body::BodyEdit,
    conflict::StateConflict,
    feedback::{FeedbackDelivery, FeedbackStatus, FeedbackText},
    ids::{CommandId, RunId},
    run::{ProviderFailure, RunStatus},
    run_message::ToolOutcome,
    submission::SubmissionProblem,
    workflow::{ChapterStep, WorkflowStage},
};
use nekowriter_store::{Store, StoreError};

async fn feedback(store: &Store, run: RunId, text: &str) {
    store
        .submit_feedback(
            CommandId::generate(),
            run,
            FeedbackText::parse(text).unwrap(),
        )
        .await
        .unwrap();
}

async fn pause(store: &Store, run: RunId) {
    store
        .request_pause(CommandId::generate(), run)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_pause_holds_the_next_call_until_the_user_continues() {
    let (_dir, store) = open_store().await;
    let (version, run, _) = drafting(&store).await;
    let (_, request) = dispatch_next(&store).await;

    pause(&store, run).await;
    answer_with(&store, request, append("Rain fell.")).await;

    let view = workspace(&store, version).await;
    assert_eq!(view.run.as_ref().unwrap().status, RunStatus::Paused);
    assert!(!view.pause_requested);
    let held = view.held.unwrap();
    assert_eq!(held.tool, "append_content");
    assert!(matches!(held.edit, Some(BodyEdit::Append { .. })));
    assert_eq!(body(&store, version).await, None);

    store.resume_run(CommandId::generate(), run).await.unwrap();
    let view = workspace(&store, version).await;
    assert_eq!(view.run.unwrap().status, RunStatus::Queued);
    assert_eq!(view.held, None);
    assert_eq!(body(&store, version).await, Some(2));
}

#[tokio::test]
async fn feedback_while_a_call_is_held_resumes_it_and_travels_with_its_result() {
    let (_dir, store) = open_store().await;
    let (version, run, _) = drafting(&store).await;
    let (_, request) = dispatch_next(&store).await;
    pause(&store, run).await;
    answer_with(&store, request, append("Rain fell.")).await;

    feedback(&store, run, "Make it a storm.").await;

    assert_eq!(body(&store, version).await, Some(2));
    let results = next_results(&store).await;
    assert_eq!(results.len(), 1);
    assert!(matches!(results[0].0, ToolOutcome::EditApplied { .. }));
    assert_eq!(results[0].1, ["Make it a storm."]);
}

#[tokio::test]
async fn first_feedback_waits_for_the_next_new_result_and_never_binds_an_older_one() {
    let (_dir, store) = open_store().await;
    let (_, run, _) = drafting(&store).await;
    let (_, first) = dispatch_next(&store).await;
    answer_with(&store, first, append("One.")).await;

    // The first result is finished but not sent; feedback waits for a later one.
    feedback(&store, run, "Slower.").await;
    let results = next_results(&store).await;
    assert_eq!(results[0].1, Vec::<String>::new());

    let (_, second) = dispatch_next(&store).await;
    feedback(&store, run, "And quieter.").await;
    answer_with(&store, second, append(" Two.")).await;
    let results = next_results(&store).await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].1, ["Slower.", "And quieter."]);
}

#[tokio::test]
async fn feedback_joins_a_batch_bound_to_a_result_that_is_not_sent_yet() {
    let (_dir, store) = open_store().await;
    let (_, run, _) = drafting(&store).await;
    let (_, request) = dispatch_next(&store).await;
    feedback(&store, run, "A.").await;
    answer_with(&store, request, append("One.")).await;

    feedback(&store, run, "B.").await;

    let results = next_results(&store).await;
    assert_eq!(results[0].1, ["A.", "B."]);
}

#[tokio::test]
async fn feedback_before_a_requested_pause_is_held_takes_the_latest_result_and_clears_it() {
    let (_dir, store) = open_store().await;
    let (version, run, _) = drafting(&store).await;
    let (_, request) = dispatch_next(&store).await;
    answer_with(&store, request, append("One.")).await;
    pause(&store, run).await;
    assert!(workspace(&store, version).await.pause_requested);

    feedback(&store, run, "Wait, warmer.").await;

    let view = workspace(&store, version).await;
    assert!(!view.pause_requested);
    assert_eq!(view.run.unwrap().status, RunStatus::Queued);
    let results = next_results(&store).await;
    assert_eq!(results[0].1, ["Wait, warmer."]);
}

#[tokio::test]
async fn a_cancelled_pause_lets_the_next_call_through() {
    let (_dir, store) = open_store().await;
    let (version, run, _) = drafting(&store).await;
    let (_, request) = dispatch_next(&store).await;
    pause(&store, run).await;
    store
        .cancel_pause(CommandId::generate(), run)
        .await
        .unwrap();

    answer_with(&store, request, append("One.")).await;

    assert_eq!(
        workspace(&store, version).await.run.unwrap().status,
        RunStatus::Queued
    );
}

#[tokio::test]
async fn a_submission_with_undelivered_feedback_is_rejected_and_carries_it() {
    let (_dir, store) = open_store().await;
    let (version, run, chapter) = drafting(&store).await;
    let (_, first) = dispatch_next(&store).await;
    answer_with(&store, first, append("One.")).await;
    let (_, second) = dispatch_next(&store).await;
    feedback(&store, run, "Add the storm first.").await;

    answer_with(&store, second, submit(chapter)).await;

    let view = workspace(&store, version).await;
    assert!(matches!(
        view.version.stage,
        WorkflowStage::Chapter {
            step: ChapterStep::Drafting,
            ..
        }
    ));
    let results = next_results(&store).await;
    assert_eq!(
        results[0],
        (
            ToolOutcome::SubmissionRejected {
                problem: SubmissionProblem::FeedbackPending
            },
            vec!["Add the storm first.".to_owned()]
        )
    );
}

#[tokio::test]
async fn a_submission_during_a_requested_pause_is_held_and_rechecked_on_continue() {
    let (_dir, store) = open_store().await;
    let (version, run, chapter) = drafting(&store).await;
    let (_, first) = dispatch_next(&store).await;
    answer_with(&store, first, append("One.")).await;
    let (_, second) = dispatch_next(&store).await;
    pause(&store, run).await;

    answer_with(&store, second, submit(chapter)).await;
    let view = workspace(&store, version).await;
    assert_eq!(view.run.as_ref().unwrap().status, RunStatus::Paused);
    let held = view.held.unwrap();
    assert_eq!((held.tool.as_str(), held.edit), ("submit_chapter", None));

    store.resume_run(CommandId::generate(), run).await.unwrap();
    assert!(matches!(
        workspace(&store, version).await.version.stage,
        WorkflowStage::Chapter {
            step: ChapterStep::Reviewing,
            ..
        }
    ));
}

#[tokio::test]
async fn the_dock_sees_included_feedback_until_its_request_answers() {
    let (_dir, store) = open_store().await;
    let (version, run, _) = drafting(&store).await;
    let (_, first) = dispatch_next(&store).await;
    feedback(&store, run, "Colder.").await;
    let view = workspace(&store, version).await;
    assert_eq!(view.feedback[0].status, FeedbackStatus::Queued);

    answer_with(&store, first, append("One.")).await;
    assert_eq!(
        workspace(&store, version).await.feedback[0].status,
        FeedbackStatus::Bound
    );
    let (_, second) = dispatch_next(&store).await;
    let item = workspace(&store, version).await.feedback[0].clone();
    assert_eq!(
        (item.status, item.delivery),
        (FeedbackStatus::Included, Some(FeedbackDelivery::Sent))
    );

    store
        .record_request_failure(second, ProviderFailure::Connect)
        .await
        .unwrap();
    assert_eq!(
        workspace(&store, version).await.feedback[0].delivery,
        Some(FeedbackDelivery::Failed)
    );
    let revision = workspace(&store, version).await.revision;
    store
        .continue_run(CommandId::generate(), run, revision)
        .await
        .unwrap();
    let (_, third) = dispatch_next(&store).await;
    answer_with(&store, third, append(" Two.")).await;
    assert_eq!(workspace(&store, version).await.feedback, Vec::new());
}

#[tokio::test]
async fn a_paused_run_that_is_stopped_returns_to_its_held_call() {
    let (_dir, store) = open_store().await;
    let (version, run, _) = drafting(&store).await;
    let (_, request) = dispatch_next(&store).await;
    pause(&store, run).await;
    answer_with(&store, request, append("One.")).await;

    store.stop_run(CommandId::generate(), run).await.unwrap();
    let revision = workspace(&store, version).await.revision;
    store
        .continue_run(CommandId::generate(), run, revision)
        .await
        .unwrap();

    let view = workspace(&store, version).await;
    assert_eq!(view.run.unwrap().status, RunStatus::Paused);
    assert!(view.held.is_some());
    assert_eq!(store.next_queued_run().await.unwrap(), None);
}

#[tokio::test]
async fn only_a_body_that_is_being_written_takes_feedback_or_a_pause() {
    let (_dir, store) = open_store().await;
    let version = common::started(&store).await;
    let run = workspace(&store, version).await.run.unwrap().id;

    let error = store
        .submit_feedback(
            CommandId::generate(),
            run,
            FeedbackText::parse("Faster.").unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::StateConflict(StateConflict::RunNotWriting)
    ));
    let error = store
        .request_pause(CommandId::generate(), run)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::StateConflict(StateConflict::RunNotWriting)
    ));
}
