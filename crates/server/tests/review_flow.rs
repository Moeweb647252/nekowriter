//! Review flow end to end (PLAN M6.1, spec/08 §3, spec/03 §6, spec/05 §5).
//!
//! Every test drives real `chapter_review` sessions over the scripted provider, so the assertions
//! read the persisted report rows, session statuses and kickoff text instead of internal state.

mod review_harness;

use std::time::Duration;

use nekowriter::db::json::SessionStatus;
use nekowriter_llm::mock::ScriptStep;
use review_harness::{
    issue, kickoff_text, prepare_in_review, review_step, reviews, runtime, session_column,
    sessions_of_kind, start_reviewer,
};
use serde_json::json;

// ───────────────────────────── spec/08 §3: review → report ─────────────────────────────

/// A first review stores a report with the version it read and ends the session (spec/05 §5).
#[tokio::test]
async fn submit_review_stores_a_report_and_completes_the_session() {
    let runtime = runtime(vec![review_step(
        "revise",
        "the middle scene change is abrupt",
        json!([issue(json!(["p2"]), "旧剑")]),
        "please add a transition",
    )])
    .await;
    let version = prepare_in_review(&runtime).await;
    let session_id = start_reviewer(&runtime).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    let body = reviews(&runtime).await;
    let reports = body["reports"].as_array().expect("reports");
    assert_eq!(reports.len(), 1, "one report: {body}");
    assert_eq!(reports[0]["attempt"], 1);
    assert_eq!(reports[0]["verdict"], "revise");
    assert_eq!(reports[0]["session_id"], session_id);
    assert_eq!(
        reports[0]["content_version"], version,
        "the report records the version it reviewed (D-026)"
    );
    assert_eq!(reports[0]["user_comment"], "");
    assert_eq!(reports[0]["issues"][0]["pids"][0], "p2");
    assert!(
        body["feedback"].as_array().expect("feedback").is_empty(),
        "a report is not a send-back"
    );

    // The reviewer session keeps its base version; `pending_comment` stays empty for a first
    // review and is cleared after a re-review report consumes it.
    let base: Option<i32> = session_column(&runtime, session_id, "base_content_version").await;
    assert_eq!(base, Some(version));
    let pending: Option<String> = session_column(&runtime, session_id, "pending_comment").await;
    assert!(pending.is_none(), "no comment was pending");
}

/// The report drops pids that do not exist in the reviewed text, with a note in the result
/// (spec/05 §5).
#[tokio::test]
async fn submit_review_drops_unknown_pids_and_names_them_in_the_result() {
    let runtime = runtime(vec![review_step(
        "revise",
        "one dangling reference",
        json!([issue(json!(["p1", "p99"]), "旧剑")]),
        "fix it",
    )])
    .await;
    prepare_in_review(&runtime).await;
    let session_id = start_reviewer(&runtime).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    let body = reviews(&runtime).await;
    let issues = body["reports"][0]["issues"].as_array().expect("issues");
    assert_eq!(issues.len(), 1);
    assert_eq!(
        issues[0]["pids"],
        json!(["p1"]),
        "p99 does not exist and is dropped"
    );
    let calls = runtime.calls(session_id).await;
    let submit = calls
        .iter()
        .find(|call| call.name == "submit_review")
        .expect("a submit_review call");
    let text = submit.raw_result_text.clone().unwrap_or_default();
    assert!(
        text.contains("p99"),
        "the result names the dropped pid: {text:?}"
    );
    assert_eq!(submit.status, "done");
    // The completion itself is proven by the session reaching `completed` above: the stored
    // effects are cleared once they are applied (spec/03 §5.4).
}

// ───────────────────────────── spec/03 §6: re-review ─────────────────────────────

/// Same text: the re-review continues the completed session and report #2 records attempt 2 and
/// copies the comment into `user_comment` (spec/03 §6, spec/05 §5).
#[tokio::test]
async fn rereview_continues_the_completed_session_and_stores_the_comment() {
    let runtime = runtime(vec![
        review_step(
            "revise",
            "first pass",
            json!([issue(json!(["p1"]), "城门口")]),
            "first feedback",
        ),
        review_step("pass", "second pass", json!([]), "second feedback"),
    ])
    .await;
    prepare_in_review(&runtime).await;
    let session_id = start_reviewer(&runtime).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let first = reviews(&runtime).await;
    let report_id = first["reports"][0]["id"].as_i64().expect("report id") as i32;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/review-reports/{report_id}/rereview"),
            json!({ "comment": "the sword was destroyed in chapter 38" }),
        )
        .await;
    assert_eq!(status, 200, "rereview: {body}");
    assert_eq!(
        body["session_id"], session_id,
        "the same session continues when the text did not change"
    );
    runtime
        .wait_status_within(
            session_id,
            SessionStatus::Completed,
            Duration::from_secs(10),
        )
        .await;

    let second = reviews(&runtime).await;
    let reports = second["reports"].as_array().expect("reports");
    assert_eq!(reports.len(), 2, "two reports: {second}");
    // Newest first.
    assert_eq!(reports[0]["attempt"], 2);
    assert_eq!(reports[0]["verdict"], "pass");
    assert_eq!(
        reports[0]["user_comment"], "the sword was destroyed in chapter 38",
        "the comment reaches the next report"
    );
    assert_eq!(reports[1]["attempt"], 1);
    assert_eq!(
        reports[1]["user_comment"], "",
        "the earlier report is unchanged"
    );
    assert_eq!(
        sessions_of_kind(&runtime, "chapter_review").await.len(),
        1,
        "no new review session for an unchanged text"
    );
    let pending: Option<String> = session_column(&runtime, session_id, "pending_comment").await;
    assert!(
        pending.is_none(),
        "the comment is cleared after the report consumed it"
    );
}

/// A changed text: the re-review starts a NEW session whose kickoff carries the previous report
/// and the comment (spec/03 §6, D-046).
#[tokio::test]
async fn rereview_after_an_edit_starts_a_new_session_with_the_prior_report() {
    let runtime = runtime(vec![
        review_step(
            "revise",
            "first pass summary",
            json!([issue(json!(["p1"]), "城门口")]),
            "first feedback for the writer",
        ),
        review_step("pass", "second pass", json!([]), "second feedback"),
    ])
    .await;
    let first_version = prepare_in_review(&runtime).await;
    let session_id = start_reviewer(&runtime).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let first = reviews(&runtime).await;
    let report = &first["reports"][0];
    let report_id = report["id"].as_i64().expect("report id") as i32;

    // The author edits the text: the report is now for an earlier version (spec/08 §1.2).
    runtime
        .app
        .put_text(
            runtime.chapter_id,
            first_version,
            "第一段，改过的开头。\n\n第二段，他握紧了手中的旧剑。\n\n第三段，钟声敲了七下。",
        )
        .await;
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    let new_version = chapter["content_version"].as_i64().expect("version") as i32;
    assert_ne!(new_version, first_version);

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/review-reports/{report_id}/rereview"),
            json!({ "comment": "the letter's seal was already broken" }),
        )
        .await;
    assert_eq!(status, 200, "rereview: {body}");
    let second_session = body["session_id"].as_i64().expect("session id") as i32;
    assert_ne!(
        second_session, session_id,
        "a changed text starts a new reviewer session"
    );

    // The new kickoff carries the previous report and the comment (`reviewer.kickoff` `prior`).
    let kickoff = kickoff_text(&runtime, second_session).await;
    assert!(
        kickoff.contains("first pass summary"),
        "the prior summary is in the kickoff: {kickoff:?}"
    );
    assert!(
        kickoff.contains("first feedback for the writer"),
        "the prior feedback is in the kickoff: {kickoff:?}"
    );
    assert!(
        kickoff.contains("the letter's seal was already broken"),
        "the author's comment is in the kickoff: {kickoff:?}"
    );

    runtime
        .wait_status_within(
            second_session,
            SessionStatus::Completed,
            Duration::from_secs(10),
        )
        .await;
    let second = reviews(&runtime).await;
    let reports = second["reports"].as_array().expect("reports");
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0]["attempt"], 2);
    assert_eq!(reports[0]["session_id"], second_session);
    assert_eq!(
        reports[0]["content_version"], new_version,
        "the new report records the new version"
    );
    assert_eq!(
        reports[0]["user_comment"],
        "the letter's seal was already broken"
    );
    assert_eq!(
        sessions_of_kind(&runtime, "chapter_review").await.len(),
        2,
        "the old session stays completed and a new one runs"
    );
}

/// A re-review on a report whose session is still working returns `409 session.invalid_state`
/// (spec/03 §6: it is still working).
#[tokio::test]
async fn rereview_while_the_session_is_running_is_invalid_state() {
    let runtime = runtime(vec![
        review_step("revise", "first", json!([]), "first"),
        ScriptStep::new()
            .tool(
                "submit_review",
                json!({
                    "verdict": "pass",
                    "summary": "second",
                    "issues": [],
                    "feedback": "second",
                }),
            )
            .delay(Duration::from_millis(600)),
    ])
    .await;
    prepare_in_review(&runtime).await;
    let session_id = start_reviewer(&runtime).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let body = reviews(&runtime).await;
    let report_id = body["reports"][0]["id"].as_i64().expect("report id") as i32;

    let (status, first) = runtime
        .app
        .post(
            &format!("/api/review-reports/{report_id}/rereview"),
            json!({ "comment": "please look again" }),
        )
        .await;
    assert_eq!(status, 200, "the first re-review starts: {first}");
    // The session is running now; a second comment is rejected by the status table.
    let (status, second) = runtime
        .app
        .post(
            &format!("/api/review-reports/{report_id}/rereview"),
            json!({ "comment": "and again" }),
        )
        .await;
    assert_eq!(
        status,
        axum::http::StatusCode::CONFLICT,
        "a working reviewer cannot be re-reviewed: {second}"
    );
    assert_eq!(second["error"]["code"], "session.invalid_state");
    assert_eq!(second["error"]["details"]["status"], "running");
    runtime
        .wait_status_within(
            session_id,
            SessionStatus::Completed,
            Duration::from_secs(10),
        )
        .await;
}
