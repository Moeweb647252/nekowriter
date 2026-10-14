//! The dashboard (`docs/product-spec.md` §15), computed from the records.

mod common;

use common::{answer, answer_with, append, call, dispatch_next, drafting, edit_round, open_store};
use nekowriter_core::{
    body::BodyEdit,
    chapter::{ChapterKind, ChapterLabel},
    dashboard::TimeBuckets,
    ids::{CommandId, ProjectId},
    run::{RunHalt, RunKind, RunStatus},
    time::Timestamp,
    usage::TokenUsage,
};
use nekowriter_store::{DashboardQuery, DashboardView, Store};

const DAY: i64 = 24 * 60 * 60 * 1000;

/// Two buckets: everything up to a day from now, then the day after.
fn query(project: Option<ProjectId>) -> DashboardQuery {
    let tomorrow = Timestamp::now().as_millis() + DAY;
    DashboardQuery {
        buckets: TimeBuckets::new(vec![
            Timestamp::from_millis(0),
            Timestamp::from_millis(tomorrow),
            Timestamp::from_millis(tomorrow + DAY),
        ])
        .unwrap(),
        project,
    }
}

async fn dashboard(store: &Store, project: Option<ProjectId>) -> DashboardView {
    store.dashboard(&query(project)).await.unwrap()
}

const REPORTED: TokenUsage = TokenUsage {
    input_tokens: Some(900),
    output_tokens: Some(300),
    cached_input_tokens: Some(800),
    reasoning_tokens: None,
};

#[tokio::test]
async fn usage_and_word_growth_come_from_the_requests_and_revisions() {
    let (_dir, store) = open_store().await;
    let (_, _, _) = drafting(&store).await;

    // The first body request reports its usage; every other request in the tests does not.
    let (_, request) = dispatch_next(&store).await;
    let mut reported = answer(&call("append_content"));
    reported.usage = REPORTED;
    reported.raw_usage = Some(r#"{"prompt_tokens":900}"#.to_owned());
    store
        .record_answer(request, reported, append("Rain fell on the roof."))
        .await
        .unwrap();
    // Five words, then three.
    let (_, request) = dispatch_next(&store).await;
    answer_with(
        &store,
        request,
        edit_round(BodyEdit::Replace {
            old_str: "on the roof".to_owned(),
            new_str: "there".to_owned(),
        }),
    )
    .await;

    let view = dashboard(&store, None).await;
    let writing = view.usage.writing;
    assert_eq!(writing.reported, 1);
    assert_eq!(writing.not_reported, writing.requests - 1);
    assert_eq!(
        (
            writing.input_tokens,
            writing.output_tokens,
            writing.cached_input_tokens
        ),
        (900, 300, 800)
    );
    assert_eq!(view.usage.buckets[0], writing);
    assert_eq!(view.usage.buckets[1].requests, 0);
    assert_eq!(view.usage.review.requests, 0);

    assert_eq!(view.words.net, 3);
    assert_eq!(view.words.buckets, [3, 0]);
    assert_eq!(view.words.total, 3);
    assert_eq!(view.words.projects[0].net, 3);

    let [job] = view.jobs.as_slice() else {
        panic!("one job: {:?}", view.jobs);
    };
    assert_eq!(job.kind, RunKind::ChapterBody);
    assert_eq!(job.status, RunStatus::Queued);
    assert!(matches!(
        job.chapter,
        Some(ChapterLabel {
            kind: ChapterKind::Main,
            number: 1,
            ..
        })
    ));
    assert_eq!(view.attention, Vec::new());
}

#[tokio::test]
async fn stopped_runs_need_attention_and_tests_count_apart() {
    let (_dir, store) = open_store().await;
    let (_, run, _) = drafting(&store).await;
    store.stop_run(CommandId::generate(), run).await.unwrap();
    store
        .record_connection_test(true, Some(REPORTED))
        .await
        .unwrap();
    store.record_connection_test(false, None).await.unwrap();

    let view = dashboard(&store, None).await;
    assert_eq!(view.jobs, Vec::new());
    let [job] = view.attention.as_slice() else {
        panic!("one run needs attention: {:?}", view.attention);
    };
    assert_eq!(
        (job.status, &job.halt),
        (RunStatus::Stopped, &Some(RunHalt::StoppedByUser))
    );
    let tests = view.usage.connection_tests;
    assert_eq!(
        (
            tests.requests,
            tests.reported,
            tests.no_answer,
            tests.input_tokens
        ),
        (2, 1, 1, 900)
    );

    // Another project's dashboard has none of it, except the connection tests.
    let other = dashboard(&store, Some(ProjectId::generate())).await;
    assert_eq!(other.attention, Vec::new());
    assert_eq!(other.usage.writing.requests, 0);
    assert_eq!(other.usage.connection_tests, tests);
}
