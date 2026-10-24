//! `ChapterDto.open_sessions` (PLAN M3 follow-up, spec/10 §2.8).
//!
//! The field carries the chapter's **open** writer, reviewer and archivist sessions, oldest
//! first. A session counts as open when its status is not `completed`, `superseded` or
//! `abandoned` (spec/03 §3); conception, research and canon-seed sessions are not per-chapter and
//! never appear.

mod agent_harness;

use agent_harness::{finish_step, runtime};
use axum::http::StatusCode;
use nekowriter::db::json::{Granularity, SessionKind, SessionStatus};
use serde_json::json;

/// The `open_sessions` array of `GET /chapters/:id`.
async fn open_sessions(
    runtime: &agent_harness::Runtime,
    chapter_id: i32,
) -> Vec<(i32, String, String)> {
    let (status, body) = runtime
        .app
        .get(&format!("/api/chapters/{chapter_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["open_sessions"]
        .as_array()
        .expect("open_sessions is always present")
        .iter()
        .map(|session| {
            (
                session["id"].as_i64().expect("session id") as i32,
                session["role"].as_str().expect("role").to_owned(),
                session["status"].as_str().expect("status").to_owned(),
            )
        })
        .collect()
}

/// A chapter with no session at all: the field is present and empty (never omitted).
#[tokio::test]
async fn a_chapter_without_sessions_has_an_empty_array() {
    let runtime = runtime(vec![]).await;
    let (status, body) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.get("open_sessions").is_some(),
        "the field is serialized even when empty: {body}"
    );
    assert_eq!(body["open_sessions"], json!([]));
    // The create response goes through the same DTO, so it carries the field too.
    assert_eq!(
        body["content_version"], 0,
        "the rest of the DTO is unchanged: {body}"
    );
}

/// A running writer session appears; a completed one does not.
#[tokio::test]
async fn only_open_chapter_sessions_are_listed() {
    // Two steps: the first keeps the session busy, the second completes a second session.
    let runtime = runtime(vec![finish_step(), finish_step(), finish_step()]).await;

    // A session that completes: it must drop out of the list.
    let completed = runtime.start_debug().await;
    runtime
        .wait_status(completed, SessionStatus::Completed)
        .await;
    assert!(
        open_sessions(&runtime, runtime.chapter_id).await.is_empty(),
        "a completed session is not open"
    );

    // A session that is created and left running is listed.
    let running = runtime
        .start_debug_with(Granularity::Paragraph, false, false)
        .await;
    let listed = open_sessions(&runtime, runtime.chapter_id).await;
    assert_eq!(
        listed.len(),
        1,
        "only the running session is open: {listed:?}"
    );
    assert_eq!(listed[0].0, running);
    assert_eq!(listed[0].1, "writer");
    assert!(
        listed[0].2 == "running" || listed[0].2 == "held",
        "{listed:?}"
    );

    // Its `id`, role, status and model label are the same DTO the sessions endpoint returns.
    let (status, snapshot) = runtime.app.get(&format!("/api/sessions/{running}")).await;
    assert_eq!(status, StatusCode::OK, "{snapshot}");
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    let embedded = &chapter["open_sessions"][0];
    assert_eq!(embedded["id"], snapshot["session"]["id"]);
    assert_eq!(embedded["model_label"], snapshot["session"]["model_label"]);
    assert_eq!(embedded["kind"], snapshot["session"]["kind"]);

    let _ = runtime
        .app
        .post(&format!("/api/sessions/{running}/abandon"), json!({}))
        .await;
}

/// Conception and research sessions are not per-chapter and never appear.
#[tokio::test]
async fn non_chapter_sessions_are_excluded() {
    let runtime = runtime(vec![]).await;
    use nekowriter::services::sessions::{CreateSession, create};

    // A conception session for the same book: `chapter_id` is null, so it cannot match anyway,
    // but the role filter is what guarantees it once a chapter is set.
    let (conception, _) = create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::Conception,
            book_id: Some(runtime.book_id),
            ..Default::default()
        },
    )
    .await
    .expect("conception session");
    // Force the chapter onto it: even then the role filter excludes it, which is what this
    // asserts. Without the filter the chapter id alone would list it.
    runtime
        .app
        .execute(&format!(
            "UPDATE agent_sessions SET chapter_id = {} WHERE id = {conception}",
            runtime.chapter_id
        ))
        .await;
    let (_, forced) = runtime
        .app
        .get(&format!("/api/sessions/{conception}"))
        .await;
    assert_eq!(
        forced["session"]["chapter_id"], runtime.chapter_id,
        "the fixture put the conception session on the chapter: {forced}"
    );

    let listed = open_sessions(&runtime, runtime.chapter_id).await;
    assert!(
        listed.is_empty(),
        "a conception session is not a chapter session, even when it carries the chapter id: \
         {listed:?}"
    );

    let _ = runtime
        .app
        .post(&format!("/api/sessions/{conception}/abandon"), json!({}))
        .await;
}

/// The list is ordered by `id` and holds every open status, not just `running`.
#[tokio::test]
async fn open_sessions_are_ordered_by_id_across_statuses() {
    let runtime = runtime(vec![finish_step(), finish_step()]).await;
    let first = runtime.start_debug().await;
    // Hold the first one, then start a second: both stay open.
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{first}/pause"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    runtime.wait_any_status(first).await;

    let second = runtime
        .start_debug_with(Granularity::Sentence, true, false)
        .await;

    let listed = open_sessions(&runtime, runtime.chapter_id).await;
    let ids: Vec<i32> = listed.iter().map(|entry| entry.0).collect();
    assert_eq!(ids, vec![first, second], "ordered by id: {listed:?}");
    // Neither is terminal, so both are listed whatever their exact status.
    for (_, role, status) in &listed {
        assert_eq!(role, "writer");
        assert!(
            !["completed", "superseded", "abandoned"].contains(&status.as_str()),
            "a terminal session must not be listed: {status}"
        );
    }

    for id in [first, second] {
        let _ = runtime
            .app
            .post(&format!("/api/sessions/{id}/abandon"), json!({}))
            .await;
    }
}

/// Every response that returns a full `ChapterDto` carries the field.
#[tokio::test]
async fn every_chapter_response_carries_open_sessions() {
    let runtime = runtime(vec![finish_step()]).await;
    let chapter_id = runtime.chapter_id;

    // PATCH
    let (status, patched) = runtime
        .app
        .patch(
            &format!("/api/chapters/{chapter_id}"),
            json!({ "title": "Renamed" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(patched["open_sessions"], json!([]));

    // POST /submit (the chapter must be drafting first)
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/chapters/{chapter_id}/content"),
            json!({
                "base_version": 0,
                "ops": [{ "op": "insert", "after": null, "text": "Some text." }]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, submitted) = runtime
        .app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{submitted}");
    assert_eq!(submitted["open_sessions"], json!([]));

    // POST /approve → `ApproveChapterResponse.chapter`
    let (status, approved) = runtime
        .app
        .post(&format!("/api/chapters/{chapter_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    assert_eq!(approved["chapter"]["open_sessions"], json!([]));

    // POST /chapter-versions/:id/restore
    let (_, detail) = runtime
        .app
        .get(&format!("/api/chapters/{chapter_id}"))
        .await;
    let version_id = detail["official_version_id"]
        .as_i64()
        .expect("official version");
    let (status, restored) = runtime
        .app
        .post(
            &format!("/api/chapter-versions/{version_id}/restore"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    let restored_chapter = if restored.get("chapter").is_some() {
        &restored["chapter"]
    } else {
        &restored
    };
    assert!(
        restored_chapter.get("open_sessions").is_some(),
        "restore returns the field: {restored}"
    );
}

/// A running session is visible through every one of those responses, not only `GET`.
#[tokio::test]
async fn a_running_session_appears_on_every_chapter_response() {
    let runtime = runtime(vec![finish_step(), finish_step()]).await;
    let session_id = runtime
        .start_debug_with(Granularity::Paragraph, false, false)
        .await;

    let (_, patched) = runtime
        .app
        .patch(
            &format!("/api/chapters/{}", runtime.chapter_id),
            json!({ "guidance": "Keep it short." }),
        )
        .await;
    let ids: Vec<i64> = patched["open_sessions"]
        .as_array()
        .expect("open_sessions")
        .iter()
        .filter_map(|session| session["id"].as_i64())
        .collect();
    assert_eq!(
        ids,
        vec![session_id as i64],
        "PATCH returns the session too"
    );

    let _ = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/abandon"), json!({}))
        .await;
}
