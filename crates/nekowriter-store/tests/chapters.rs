//! Chapters through the store: guidance, the outline and its review, body edits, the
//! whole-chapter review, the next chapter, completion and extra chapters
//! (`docs/product-spec.md` §5.3, §7, §8.5, §11).

mod common;

use common::{
    answer, call, decide_review, dispatch_next, open_store, prepared, record, reject, workspace,
};
use nekowriter_core::{
    body::{BodyEdit, EditProblem},
    chapter::{ChapterGuidance, ChapterKind, ChapterStatus, ChapterTitle},
    conflict::StateConflict,
    document::DocumentRole,
    ids::{ChapterId, CommandId, VersionId},
    review::{ReviewDecision, ReviewKind, ReviewStatus},
    run::{RunKind, RunStatus},
    run_message::{ToolOutcome, UnsealedMessage},
    stage::{FoldStage, StageInput},
    submission::{ChapterSubmission, OutlineSubmission, SubmissionProblem},
    workflow::{ChapterStep, WorkflowStage},
};
use nekowriter_store::{RoundRecord, Store, StoreError, TranscriptEntry, WorkspaceView};

fn step(view: &WorkspaceView) -> (ChapterId, ChapterStep) {
    match view.version.stage {
        WorkflowStage::Chapter { chapter_id, step } => (chapter_id, step),
        ref stage => panic!("not in a chapter: {stage:?}"),
    }
}

async fn start(store: &Store, version: VersionId, guidance: &str) {
    let revision = workspace(store, version).await.revision;
    store
        .start_chapter(
            CommandId::generate(),
            version,
            revision,
            ChapterGuidance::parse(guidance).unwrap(),
        )
        .await
        .unwrap();
}

async fn submit_outline(store: &Store, chapter_id: ChapterId, content: &str) {
    record(
        store,
        RoundRecord::OutlineSubmission {
            call: call("submit_chapter_outline"),
            submission: OutlineSubmission {
                chapter_id,
                content: content.to_owned(),
            },
        },
    )
    .await;
}

async fn edit(store: &Store, edit: BodyEdit) {
    record(
        store,
        RoundRecord::BodyEdit {
            call: call("append_content"),
            edit,
        },
    )
    .await;
}

fn append(content: &str) -> BodyEdit {
    BodyEdit::Append {
        content: content.to_owned(),
    }
}

async fn submit_chapter(store: &Store, chapter_id: ChapterId) {
    record(
        store,
        RoundRecord::ChapterSubmission {
            call: call("submit_chapter"),
            submission: ChapterSubmission {
                chapter_id,
                summary: "The keeper wakes.".to_owned(),
            },
        },
    )
    .await;
}

/// The chapter's latest review.
async fn chapter_review(store: &Store, version: VersionId, decision: ReviewDecision) {
    let view = workspace(store, version).await;
    let (chapter_id, _) = step(&view);
    let review = view
        .chapters
        .iter()
        .find(|chapter| chapter.id == chapter_id)
        .and_then(|chapter| chapter.review.clone())
        .expect("the chapter has a review");
    decide_review(store, version, review.id, decision).await;
}

/// Writes the current chapter from its guidance to its approval.
async fn write_chapter(store: &Store, version: VersionId) -> ChapterId {
    start(store, version, "").await;
    let (chapter_id, _) = step(&workspace(store, version).await);
    submit_outline(store, chapter_id, "Scene one.").await;
    chapter_review(store, version, ReviewDecision::Approve).await;
    edit(store, append("Text.")).await;
    submit_chapter(store, chapter_id).await;
    chapter_review(store, version, ReviewDecision::Approve).await;
    chapter_id
}

/// The last pending message of the next queued run.
async fn last_pending(store: &Store) -> UnsealedMessage {
    let run = store.next_queued_run().await.unwrap().unwrap();
    let input = store.dispatch_input(run).await.unwrap();
    match input.messages.last() {
        Some(TranscriptEntry::Pending { message, .. }) => message.clone(),
        other => panic!("no pending message: {other:?}"),
    }
}

#[tokio::test]
async fn an_outline_is_reviewed_and_revised_before_the_body_starts() {
    let (_dir, store) = open_store().await;
    let version = prepared(&store).await;
    let (first, ready) = step(&workspace(&store, version).await);
    assert_eq!(ready, ChapterStep::Ready);

    start(&store, version, "Keep it quiet.").await;
    let view = workspace(&store, version).await;
    assert_eq!(step(&view), (first, ChapterStep::OutlineGenerating));
    assert_eq!(view.run.as_ref().unwrap().kind, RunKind::ChapterOutline);
    assert_eq!(view.chapters[0].status, ChapterStatus::InProgress);

    let (input, _) = dispatch_next(&store).await;
    let StageInput::ChapterOutline { chapter, guidance } = &input.stage else {
        panic!("an outline run: {:?}", input.stage);
    };
    assert_eq!((chapter.number, chapter.title.as_str()), (1, "Arrival"));
    assert_eq!(guidance, "Keep it quiet.");
    assert_eq!(input.folds.len(), 1);
    assert_eq!(input.folds[0].stage, FoldStage::Preparation);
    assert_eq!(input.folds[0].documents.len(), 4);
    store
        .record_answer(
            store_request(&store, version).await,
            answer(&call("submit_chapter_outline")),
            RoundRecord::OutlineSubmission {
                call: call("submit_chapter_outline"),
                submission: OutlineSubmission {
                    chapter_id: first,
                    content: "The keeper finds the letter.".to_owned(),
                },
            },
        )
        .await
        .unwrap();
    let view = workspace(&store, version).await;
    assert_eq!(step(&view), (first, ChapterStep::OutlineReviewing));
    let review = view.chapters[0].review.clone().unwrap();
    assert_eq!(
        (review.kind, review.status),
        (ReviewKind::ChapterOutline, ReviewStatus::Open)
    );

    chapter_review(&store, version, reject("Start with the storm.")).await;
    assert_eq!(
        step(&workspace(&store, version).await),
        (first, ChapterStep::OutlineGenerating)
    );
    assert_eq!(
        last_pending(&store).await,
        UnsealedMessage::ReviewRejected {
            kind: ReviewKind::ChapterOutline,
            feedback: "Start with the storm.".to_owned(),
        }
    );

    submit_outline(&store, first, "A storm; the keeper finds the letter.").await;
    chapter_review(&store, version, ReviewDecision::Approve).await;
    let view = workspace(&store, version).await;
    assert_eq!(step(&view), (first, ChapterStep::Drafting));
    assert_eq!(view.run.as_ref().unwrap().kind, RunKind::ChapterBody);
    assert_eq!(view.chapters[0].outline.unwrap().number, 2);
}

/// The request in flight for the version's run.
async fn store_request(store: &Store, version: VersionId) -> nekowriter_core::ids::ModelRequestId {
    workspace(store, version)
        .await
        .run
        .unwrap()
        .request_in_flight
        .expect("a request is in flight")
}

#[tokio::test]
async fn the_body_is_edited_exactly_and_reviewed_as_a_whole() {
    let (_dir, store) = open_store().await;
    let version = prepared(&store).await;
    start(&store, version, "").await;
    let (chapter_id, _) = step(&workspace(&store, version).await);
    submit_outline(&store, chapter_id, "Scene one.").await;
    chapter_review(&store, version, ReviewDecision::Approve).await;

    let (input, _) = dispatch_next(&store).await;
    let StageInput::ChapterBody { body, .. } = &input.stage else {
        panic!("a body run: {:?}", input.stage);
    };
    assert_eq!(body, "");
    let stages: Vec<_> = input.folds.iter().map(|fold| fold.stage.clone()).collect();
    assert!(matches!(
        stages.as_slice(),
        [FoldStage::Preparation, FoldStage::ChapterOutline { chapter }] if chapter.id == chapter_id
    ));
    assert_eq!(
        input.folds[1].documents[0].role,
        DocumentRole::ChapterOutline
    );
    store
        .record_answer(
            store_request(&store, version).await,
            answer(&call("append_content")),
            RoundRecord::BodyEdit {
                call: call("append_content"),
                edit: append("The keeper woke. The lamp was dark."),
            },
        )
        .await
        .unwrap();
    let view = workspace(&store, version).await;
    assert_eq!(view.chapters[0].body.unwrap().word_count, Some(7));
    assert_eq!(view.run.as_ref().unwrap().status, RunStatus::Queued);

    edit(
        &store,
        BodyEdit::Replace {
            old_str: "dark".to_owned(),
            new_str: "lit".to_owned(),
        },
    )
    .await;
    edit(
        &store,
        BodyEdit::Replace {
            old_str: "The".to_owned(),
            new_str: "A".to_owned(),
        },
    )
    .await;
    let run = store.next_queued_run().await.unwrap().unwrap();
    let input = store.dispatch_input(run).await.unwrap();
    let StageInput::ChapterBody { body, .. } = &input.stage else {
        panic!("a body run");
    };
    assert_eq!(body, "The keeper woke. The lamp was lit.");
    // Each request seals the result before it, so only the last edit's result is pending.
    assert!(matches!(
        input.messages.last(),
        Some(TranscriptEntry::Pending {
            message: UnsealedMessage::ToolResult {
                outcome: ToolOutcome::EditRejected {
                    problem: EditProblem::AmbiguousMatch { matches: 2 }
                },
                ..
            },
            ..
        })
    ));

    submit_chapter(&store, chapter_id).await;
    let view = workspace(&store, version).await;
    assert_eq!(step(&view), (chapter_id, ChapterStep::Reviewing));
    let review = view.chapters[0].review.clone().unwrap();
    assert_eq!(review.kind, ReviewKind::ChapterFinal);
    assert_eq!(review.summary.as_deref(), Some("The keeper wakes."));
    assert_eq!(
        Some(review.target),
        view.chapters[0].body.map(|body| body.id)
    );

    chapter_review(&store, version, reject("More weather.")).await;
    assert_eq!(
        step(&workspace(&store, version).await),
        (chapter_id, ChapterStep::Drafting)
    );
    edit(&store, append(" Rain fell.")).await;
    submit_chapter(&store, chapter_id).await;
    chapter_review(&store, version, ReviewDecision::Approve).await;
    let view = workspace(&store, version).await;
    assert_eq!(step(&view), (chapter_id, ChapterStep::Done));
    assert_eq!(view.chapters[0].status, ChapterStatus::Approved);
    assert!(view.run.is_none());
}

#[tokio::test]
async fn submitting_a_chapter_without_body_text_is_a_rejected_round() {
    let (_dir, store) = open_store().await;
    let version = prepared(&store).await;
    start(&store, version, "").await;
    let (chapter_id, _) = step(&workspace(&store, version).await);
    submit_outline(&store, chapter_id, "Scene one.").await;
    chapter_review(&store, version, ReviewDecision::Approve).await;

    submit_chapter(&store, chapter_id).await;

    let view = workspace(&store, version).await;
    assert_eq!(step(&view), (chapter_id, ChapterStep::Drafting));
    let run = view.run.unwrap();
    assert_eq!(
        (run.status, run.consecutive_protocol_errors),
        (RunStatus::Queued, 0)
    );
    let run = store.next_queued_run().await.unwrap().unwrap();
    let input = store.dispatch_input(run).await.unwrap();
    assert!(input.messages.iter().any(|entry| matches!(
        entry,
        TranscriptEntry::Pending {
            message: UnsealedMessage::ToolResult {
                outcome: ToolOutcome::SubmissionRejected {
                    problem: SubmissionProblem::BodyEmpty
                },
                ..
            },
            ..
        }
    )));
}

/// Where each fold of the next queued run's input comes from: (chapter number, stage).
async fn fold_order(store: &Store) -> Vec<(i32, &'static str)> {
    let run = store.next_queued_run().await.unwrap().unwrap();
    let input = store.dispatch_input(run).await.unwrap();
    input
        .folds
        .iter()
        .map(|fold| match &fold.stage {
            FoldStage::Preparation => (0, "preparation"),
            FoldStage::ChapterOutline { chapter } => (chapter.number, "outline"),
            FoldStage::ChapterBody { chapter } => (chapter.number, "body"),
        })
        .collect()
}

#[tokio::test]
async fn the_last_chapter_completes_the_novel_and_extra_chapters_follow() {
    let (_dir, store) = open_store().await;
    let version = prepared(&store).await;
    write_chapter(&store, version).await;

    // A plan chapter remains, so the novel cannot be completed yet.
    assert!(matches!(
        complete(&store, version).await,
        Err(StoreError::StateConflict(StateConflict::WrongStep))
    ));
    next(&store, version).await.unwrap();
    start(&store, version, "").await;
    let (second, _) = step(&workspace(&store, version).await);
    submit_outline(&store, second, "Scene two.").await;
    chapter_review(&store, version, ReviewDecision::Approve).await;
    // Every completed stage is carried once, in story order.
    assert_eq!(
        fold_order(&store).await,
        [
            (0, "preparation"),
            (1, "outline"),
            (1, "body"),
            (2, "outline")
        ]
    );
    edit(&store, append("Text.")).await;
    submit_chapter(&store, second).await;
    chapter_review(&store, version, ReviewDecision::Approve).await;

    assert!(matches!(
        next(&store, version).await,
        Err(StoreError::StateConflict(StateConflict::WrongStep))
    ));
    complete(&store, version).await.unwrap();
    let view = workspace(&store, version).await;
    assert_eq!(view.version.stage, WorkflowStage::Completed);
    let completed_at = view.completed_at.expect("the completion time is recorded");

    let revision = view.revision;
    store
        .add_extra_chapter(
            CommandId::generate(),
            version,
            revision,
            ChapterTitle::parse("Coda").unwrap(),
        )
        .await
        .unwrap();
    let view = workspace(&store, version).await;
    let extra = view.chapters.last().unwrap();
    assert_eq!(
        (extra.kind, extra.number, extra.title.as_str()),
        (ChapterKind::Extra, 1, "Coda")
    );
    assert_eq!(step(&view), (extra.id, ChapterStep::Ready));
    write_chapter(&store, version).await;
    complete(&store, version).await.unwrap();
    let view = workspace(&store, version).await;
    assert_eq!(view.version.stage, WorkflowStage::Completed);
    assert_eq!(view.completed_at, Some(completed_at));
}

async fn next(store: &Store, version: VersionId) -> Result<(), StoreError> {
    let revision = workspace(store, version).await.revision;
    store
        .next_chapter(CommandId::generate(), version, revision)
        .await
        .map(drop)
}

async fn complete(store: &Store, version: VersionId) -> Result<(), StoreError> {
    let revision = workspace(store, version).await.revision;
    store
        .complete_version(CommandId::generate(), version, revision)
        .await
        .map(drop)
}
