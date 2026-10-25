//! M6 milestone review regressions for send-back report validation (spec/08 §1.1, spec/10 §2.2).
//!
//! `review_report_id?` names the report the dialog was built from. It used to be looked up only
//! when the author had ticked at least one issue: a nonexistent id reached the `chapter_feedback`
//! foreign key as an internal 500, and another chapter's report was stored silently.

mod review_harness;

use std::time::Duration;

use nekowriter::db::json::SessionStatus;
use review_harness::{Runtime, prepare_in_review, review_step, reviews, runtime, start_reviewer};
use serde_json::json;

/// A second chapter with its own completed review report; returns the report id.
async fn report_of_another_chapter(runtime: &Runtime) -> i32 {
    let (status, chapter) = runtime
        .app
        .post(
            &format!("/api/tracks/{}/chapters", runtime.track_id),
            json!({ "title": "Another chapter" }),
        )
        .await;
    assert_eq!(status, 201, "create chapter: {chapter}");
    let other = chapter["id"].as_i64().expect("chapter id") as i32;
    runtime.app.put_text(other, 0, "第一段。\n\n第二段。").await;
    runtime.app.set_chapter_status(other, "in_review").await;
    let (status, started) = runtime
        .app
        .post(&format!("/api/chapters/{other}/review"), json!({}))
        .await;
    assert_eq!(status, 200, "ask the reviewer: {started}");
    let reviewer = started["session_id"].as_i64().expect("session id") as i32;
    runtime
        .wait_status_within(reviewer, SessionStatus::Completed, Duration::from_secs(10))
        .await;
    let (status, body) = runtime
        .app
        .get(&format!("/api/chapters/{other}/reviews"))
        .await;
    assert_eq!(status, 200, "reviews: {body}");
    body["reports"][0]["id"].as_i64().expect("report id") as i32
}

/// A send-back that names a report which does not exist is a 404, stores nothing and does not
/// reach the foreign key as an internal error.
#[tokio::test]
async fn send_back_with_an_unknown_report_is_not_found() {
    let runtime = runtime(vec![review_step("revise", "s", json!([]), "f")]).await;
    let version = prepare_in_review(&runtime).await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/send-back", runtime.chapter_id),
            json!({ "feedback": "fix it", "mode": "revise", "review_report_id": 999_999 }),
        )
        .await;
    assert_eq!(
        status, 404,
        "an unknown report is not_found, not an internal error: {body}"
    );
    assert_eq!(body["error"]["code"], "not_found");

    // Nothing was stored: no feedback row, no session, the chapter still waits for review.
    let body = reviews(&runtime).await;
    assert!(
        body["feedback"].as_array().expect("feedback").is_empty(),
        "no feedback row: {body}"
    );
    let (chapter_status, _, current_version) = runtime.chapter_status().await;
    assert_eq!(chapter_status, "in_review");
    assert_eq!(current_version, version);
}

/// A send-back whose report belongs to another chapter is a validation error: the feedback row
/// must not point at a report of a different text.
#[tokio::test]
async fn send_back_with_another_chapters_report_is_a_validation_error() {
    let runtime = runtime(vec![
        review_step("revise", "s", json!([]), "f"),
        review_step("pass", "s2", json!([]), "f2"),
    ])
    .await;
    // The first scripted review belongs to the harness chapter.
    prepare_in_review(&runtime).await;
    let own_reviewer = start_reviewer(&runtime).await;
    runtime
        .wait_status_within(
            own_reviewer,
            SessionStatus::Completed,
            Duration::from_secs(10),
        )
        .await;
    let foreign = report_of_another_chapter(&runtime).await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/send-back", runtime.chapter_id),
            json!({ "feedback": "fix it", "mode": "revise", "review_report_id": foreign }),
        )
        .await;
    assert_eq!(
        status, 400,
        "a foreign report is a validation error: {body}"
    );
    assert_eq!(body["error"]["code"], "validation.failed");
    let body = reviews(&runtime).await;
    assert!(
        body["feedback"].as_array().expect("feedback").is_empty(),
        "no feedback row: {body}"
    );
    assert_eq!(runtime.chapter_status().await.0, "in_review");
}

/// A send-back that ticks issues must still copy exactly those issues (the validation tightening
/// must not change the happy path).
#[tokio::test]
async fn send_back_still_copies_the_ticked_issue() {
    let runtime = runtime(vec![review_step(
        "revise",
        "s",
        json!([{
            "severity": "minor",
            "category": "prose",
            "pids": ["p1"],
            "quote": "第一段",
            "problem": "p",
            "suggestion": "s"
        }]),
        "f",
    )])
    .await;
    prepare_in_review(&runtime).await;
    let reviewer = start_reviewer(&runtime).await;
    runtime
        .wait_status_within(reviewer, SessionStatus::Completed, Duration::from_secs(10))
        .await;
    let body = reviews(&runtime).await;
    let report = body["reports"][0]["id"].as_i64().expect("report id") as i32;
    let (status, started) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/send-back", runtime.chapter_id),
            json!({
                "feedback": "fix the first paragraph",
                "mode": "revise",
                "review_report_id": report,
                "issue_indexes": [0],
            }),
        )
        .await;
    assert_eq!(status, 200, "send back: {started}");
    let body = reviews(&runtime).await;
    let feedback = &body["feedback"][0];
    assert_eq!(feedback["review_report_id"], report);
    assert_eq!(feedback["issues"][0]["pids"], json!(["p1"]));
    // Stop the revision runner so the test leaves no session behind.
    if let Some(session) = started["session_id"].as_i64() {
        let (status, _) = runtime
            .app
            .post(&format!("/api/sessions/{session}/abandon"), json!({}))
            .await;
        assert_eq!(status, 200);
    }
}
