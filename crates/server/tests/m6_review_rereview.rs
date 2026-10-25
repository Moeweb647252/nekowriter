//! M6 milestone review regression for re-review session state (spec/03 §3, §6, spec/10 §2.8).
//!
//! A report's session is `completed` before a re-review continues it. `transition` wrote
//! `finished_at` for terminal statuses but never cleared it, so the re-opened session ran with the
//! old completion timestamp — a session that `SessionDto` reported as both `running` and finished.

mod review_harness;

use std::time::Duration;

use nekowriter::db::json::SessionStatus;
use nekowriter_llm::mock::ScriptStep;
use review_harness::{
    prepare_in_review, review_step, reviews, runtime, session_column, start_reviewer,
};
use serde_json::json;

/// A re-review of the same text re-opens the completed session: while it runs `finished_at` is
/// empty again, and its second completion stamps it.
#[tokio::test]
async fn rereview_clears_and_restamps_finished_at() {
    let runtime = runtime(vec![
        review_step("revise", "first", json!([]), "first"),
        ScriptStep::new()
            .tool(
                "submit_review",
                json!({ "verdict": "pass", "summary": "second", "issues": [], "feedback": "second" }),
            )
            .delay(Duration::from_millis(1_500)),
    ])
    .await;
    prepare_in_review(&runtime).await;
    let session = start_reviewer(&runtime).await;
    runtime.wait_status(session, SessionStatus::Completed).await;
    let finished: Option<String> = session_column(&runtime, session, "finished_at").await;
    assert!(finished.is_some(), "a completed session is finished");

    let body = reviews(&runtime).await;
    let report_id = body["reports"][0]["id"].as_i64().expect("report id") as i32;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/review-reports/{report_id}/rereview"),
            json!({ "comment": "look again" }),
        )
        .await;
    assert_eq!(status, 200, "rereview: {body}");

    let current: Option<String> = session_column(&runtime, session, "status").await;
    assert_eq!(current.as_deref(), Some("running"));
    let finished_while_running: Option<String> =
        session_column(&runtime, session, "finished_at").await;
    assert!(
        finished_while_running.is_none(),
        "a running session is not finished: {finished_while_running:?}"
    );

    runtime
        .wait_status_within(session, SessionStatus::Completed, Duration::from_secs(10))
        .await;
    let finished_again: Option<String> = session_column(&runtime, session, "finished_at").await;
    assert!(
        finished_again.is_some(),
        "the second completion stamps the time again"
    );
}
