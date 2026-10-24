//! Send-back end to end (PLAN M6.2, spec/08 §1.1, §3, spec/03 §4).
//!
//! The tests drive a real reviewer session to a report, then send the chapter back in both modes
//! and assert the persisted state: the `chapter_feedback` row with the ticked issues, the
//! revision/redraft session's kickoff, the versions and the chapter status.

mod review_harness;

use std::time::Duration;

use nekowriter::db::json::SessionStatus;
use nekowriter_llm::mock::ScriptStep;
use review_harness::{
    base::Runtime, issue, kickoff_text, prepare_in_review, review_step, reviews, runtime,
    sessions_of_kind, start_reviewer,
};
use serde_json::json;

/// A report with two issues, driven to completion, returning its id.
async fn reviewed_report(runtime: &Runtime) -> i32 {
    let session_id = start_reviewer(runtime).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let body = reviews(runtime).await;
    body["reports"][0]["id"].as_i64().expect("report id") as i32
}

/// A step that enters the revising phase (spec/05 §4).
fn revision_plan_step() -> ScriptStep {
    ScriptStep::new().tool(
        "submit_revision_plan",
        json!({ "plan": "rewrite the second paragraph" }),
    )
}

/// A step that completes a revision (spec/05 §4).
fn finish_revision_step(summary: &str) -> ScriptStep {
    ScriptStep::new().tool("finish_revision", json!({ "summary": summary }))
}

// ───────────────────────────── revise ─────────────────────────────

/// `mode: revise` stores the feedback with the ticked issues, starts `chapter_revise` and moves
/// the chapter to `revising`; `finish_revision` saves a `revision` version and returns it to
/// review (spec/08 §1.1).
#[tokio::test]
async fn send_back_revise_starts_a_revision_with_the_ticked_issues() {
    let runtime = runtime(vec![
        review_step(
            "revise",
            "two problems",
            json!([issue(json!(["p1"]), "城门口"), issue(json!(["p2"]), "旧剑")]),
            "fix both",
        ),
        revision_plan_step(),
        finish_revision_step("rewrote the second paragraph"),
    ])
    .await;
    let version = prepare_in_review(&runtime).await;
    let report_id = reviewed_report(&runtime).await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/send-back", runtime.chapter_id),
            json!({
                "feedback": "only the second issue is mandatory",
                "mode": "revise",
                "review_report_id": report_id,
                "issue_indexes": [1],
            }),
        )
        .await;
    assert_eq!(status, 200, "send back: {body}");
    let revision_session = body["session_id"].as_i64().expect("session id") as i32;

    let (chapter_status, _, current_version) = runtime.chapter_status().await;
    assert_eq!(chapter_status, "revising", "spec/08 §1.1");
    assert_eq!(current_version, version, "the text is not changed yet");
    assert_eq!(
        sessions_of_kind(&runtime, "chapter_revise").await,
        vec![(revision_session, "running".to_owned())]
    );

    // The feedback row carries the ticked issue as data, not pasted text (spec/08 §3).
    let body = reviews(&runtime).await;
    let feedback = body["feedback"].as_array().expect("feedback");
    assert_eq!(feedback.len(), 1, "one feedback row: {body}");
    assert_eq!(feedback[0]["text"], "only the second issue is mandatory");
    assert_eq!(feedback[0]["mode"], "revise");
    assert_eq!(feedback[0]["review_report_id"], report_id);
    assert_eq!(feedback[0]["content_version"], version);
    assert_eq!(feedback[0]["session_id"], revision_session);
    let issues = feedback[0]["issues"].as_array().expect("issues");
    assert_eq!(issues.len(), 1, "only the ticked issue is copied");
    assert_eq!(issues[0]["pids"], json!(["p2"]));
    assert_eq!(issues[0]["quote"], "旧剑");

    // The revision kickoff repeats the feedback and the ticked issue (spec/08 §3).
    let kickoff = kickoff_text(&runtime, revision_session).await;
    assert!(
        kickoff.contains("only the second issue is mandatory"),
        "{kickoff:?}"
    );
    assert!(kickoff.contains("旧剑"), "{kickoff:?}");
    assert!(
        !kickoff.contains("p1「"),
        "the unticked issue must not reach the writer: {kickoff:?}"
    );

    runtime
        .wait_status_within(
            revision_session,
            SessionStatus::Completed,
            Duration::from_secs(10),
        )
        .await;
    let (chapter_status, _, _) = runtime.chapter_status().await;
    assert_eq!(
        chapter_status, "in_review",
        "a completed revision waits for review"
    );
    let versions = runtime.version_kinds().await;
    assert!(
        versions.contains(&"backup".to_owned()),
        "reflect: {versions:?}"
    );
    assert!(
        versions.contains(&"revision".to_owned()),
        "versions: {versions:?}"
    );

    // The revision version's note is the author-facing summary (spec/05 §4).
    let (status, versions) = runtime
        .app
        .get(&format!("/api/chapters/{}/versions", runtime.chapter_id))
        .await;
    assert_eq!(status, 200);
    let revision = versions
        .as_array()
        .expect("versions")
        .iter()
        .find(|version| version["kind"] == "revision")
        .expect("a revision version");
    assert_eq!(revision["note"], "rewrote the second paragraph");
    assert_eq!(
        revision["session_id"], revision_session,
        "the version records the session that produced it"
    );
}

// ───────────────────────────── redraft ─────────────────────────────

/// `mode: redraft` archives the current text as a `backup`, clears it, starts `chapter_write`
/// with the redraft kickoff and moves the chapter to `drafting` (spec/08 §1.1, §3).
#[tokio::test]
async fn send_back_redraft_archives_the_text_and_starts_a_write_session() {
    let runtime = runtime(vec![
        review_step(
            "revise",
            "start over",
            json!([issue(json!(["p2"]), "旧剑")]),
            "rewrite from the outline",
        ),
        ScriptStep::new().tool(
            "submit_outline",
            json!({ "outline": "场景一：重写的城门口", "reference_notes": "- 雨" }),
        ),
        ScriptStep::new().tool("insert_text", json!({ "text": "重写后的第一段。" })),
        ScriptStep::new().tool("finish_draft", json!({})),
        ScriptStep::new().tool("finish_draft", json!({})),
    ])
    .await;
    let version = prepare_in_review(&runtime).await;
    // The redraft kickoff shows the archived outline as the previous one (spec/08 §3).
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/chapters/{}", runtime.chapter_id),
            json!({ "outline": "场景一：城门口" }),
        )
        .await;
    assert_eq!(status, 200, "set outline: {body}");
    let old_content = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await
        .1["content"]
        .clone();
    // The redraft copies the book's writer switches; the script does not want an outline hold.
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/books/{}", runtime.book_id),
            json!({
                "settings": {
                    "granularity": "paragraph",
                    "auto_pause": false,
                    "pause_after_outline": false,
                    "target_words_min": 3000,
                    "target_words_max": 4000,
                    "recent_summary_count": 5,
                    "midterm_window": 10,
                    "auto_maintenance": true,
                },
            }),
        )
        .await;
    assert_eq!(status, 200, "book settings: {body}");
    let report_id = reviewed_report(&runtime).await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/send-back", runtime.chapter_id),
            json!({
                "feedback": "rewrite from the outline",
                "mode": "redraft",
                "review_report_id": report_id,
                "issue_indexes": [0],
            }),
        )
        .await;
    assert_eq!(status, 200, "send back: {body}");
    let redraft_session = body["session_id"].as_i64().expect("session id") as i32;

    let (chapter_status, _, current_version) = runtime.chapter_status().await;
    assert_eq!(chapter_status, "drafting", "spec/08 §1.1");
    assert_eq!(
        current_version,
        version + 1,
        "clearing the text is a content change"
    );
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(
        chapter["content"]["paragraphs"],
        json!([]),
        "the redraft starts from an empty draft"
    );
    let versions = runtime.version_kinds().await;
    assert_eq!(
        versions.iter().filter(|kind| *kind == "backup").count(),
        1,
        "the previous text is archived: {versions:?}"
    );

    // The redraft kickoff carries the feedback, its issue quote (pids of the archived text are
    // not reused) and the previous outline (spec/08 §3, D-060).
    let kickoff = kickoff_text(&runtime, redraft_session).await;
    assert!(kickoff.contains("rewrite from the outline"), "{kickoff:?}");
    assert!(
        kickoff.contains("旧剑"),
        "the issue quote reaches the writer: {kickoff:?}"
    );
    assert!(
        kickoff.contains("场景一：城门口"),
        "the archived outline is shown as the previous one: {kickoff:?}"
    );

    runtime
        .wait_status_within(
            redraft_session,
            SessionStatus::Completed,
            Duration::from_secs(10),
        )
        .await;
    let (chapter_status, _, _) = runtime.chapter_status().await;
    assert_eq!(
        chapter_status, "in_review",
        "the rewritten draft waits for review"
    );
    let versions = runtime.version_kinds().await;
    assert!(
        versions.iter().any(|kind| kind == "draft"),
        "the redrafted draft is versioned: {versions:?}"
    );
    assert_ne!(
        old_content["paragraphs"],
        json!([]),
        "the archived content really had text"
    );
}

// ───────────────────────────── preconditions ─────────────────────────────

/// Send-back requires `in_review` and non-empty feedback; a ticked issue index must exist
/// (spec/08 §1.1, spec/10 §2.2).
#[tokio::test]
async fn send_back_validates_the_chapter_status_feedback_and_issue_indexes() {
    let runtime = runtime(vec![review_step(
        "revise",
        "one problem",
        json!([issue(json!(["p1"]), "城门口")]),
        "fix it",
    )])
    .await;
    // Not in review yet.
    runtime
        .app
        .put_text(runtime.chapter_id, 0, "第一段。")
        .await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/send-back", runtime.chapter_id),
            json!({ "feedback": "x", "mode": "revise" }),
        )
        .await;
    assert_eq!(status, 409, "not in review: {body}");
    assert_eq!(body["error"]["code"], "chapter.invalid_state");

    prepare_in_review(&runtime).await;
    let report_id = reviewed_report(&runtime).await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/send-back", runtime.chapter_id),
            json!({ "feedback": "   ", "mode": "revise" }),
        )
        .await;
    assert_eq!(status, 400, "empty feedback: {body}");
    assert_eq!(body["error"]["code"], "validation.failed");

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/send-back", runtime.chapter_id),
            json!({
                "feedback": "fix it",
                "mode": "revise",
                "review_report_id": report_id,
                "issue_indexes": [7],
            }),
        )
        .await;
    assert_eq!(status, 400, "no such issue: {body}");
    assert_eq!(body["error"]["code"], "validation.failed");

    // Nothing was stored by the rejected requests.
    let body = reviews(&runtime).await;
    assert!(body["feedback"].as_array().expect("feedback").is_empty());
    assert_eq!(runtime.chapter_status().await.0, "in_review");
}
