//! spec/02 §5 invariant 5, the chapter-scoped half: at most one **open** writer session, one open
//! reviewer session and one open archivist session per chapter.
//!
//! Every caller checked this before creating its session (`chapters::write`,
//! `reviews::ask_reviewer`, `maintenance::maintain` / `resync`), but each of those checks is a read
//! in a *different* transaction from the insert, so two concurrent requests could both pass it and
//! leave a chapter with two open writer sessions — two runners writing the same chapter, one of
//! them unreachable from the agent panel. The guard now also runs inside session creation, under
//! the process-wide write lock, which is the only place the check and the insert are atomic
//! (D-121).
//!
//! These tests fail without the guard: the concurrent test creates two sessions, and the direct
//! test gets `Ok` on the second create instead of `409 chapter.session_exists`.

use axum::http::StatusCode;
use nekowriter::db::json::{Granularity, SessionKind, SessionStatus};
use nekowriter::services::sessions::{self, CreateSession};

use serde_json::json;

#[path = "agent_write_harness/mod.rs"]
mod agent_write_harness;

use agent_write_harness::{outline_step, runtime};

/// The open sessions of one chapter and role, read through the API.
async fn open_sessions(
    runtime: &agent_write_harness::Runtime,
    chapter_id: i32,
    kind: &str,
) -> Vec<serde_json::Value> {
    let (status, list) = runtime
        .app
        .get(&format!(
            "/api/books/{}/sessions?kind={kind}",
            runtime.book_id
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "list sessions: {list}");
    list.as_array()
        .expect("a session list")
        .iter()
        .filter(|session| session["chapter_id"] == json!(chapter_id))
        .filter(|session| {
            !matches!(
                session["status"].as_str(),
                Some("completed") | Some("superseded") | Some("abandoned")
            )
        })
        .cloned()
        .collect()
}

/// Two concurrent `POST /chapters/:id/write` calls start exactly one writer session; the other
/// answers the documented `409 chapter.session_exists` with its `session_id` (spec/10 §4).
#[tokio::test]
async fn concurrent_writes_start_one_writer_session() {
    let runtime = runtime(Vec::new()).await;
    // The predecessor must be official and canon-synced, or both calls stop at
    // `chapter.previous_not_ready` before the session is even considered.
    let first = runtime.chapter_id;
    runtime.app.put_text(first, 0, "第一段。").await;
    runtime.app.set_chapter_status(first, "official").await;
    runtime.app.mark_canon_synced(first).await;
    let (status, second) = runtime
        .app
        .post(
            &format!("/api/tracks/{}/chapters", runtime.track_id),
            json!({ "title": "Chapter Two" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let chapter_id = second["id"].as_i64().expect("chapter id") as i32;

    let path = format!("/api/chapters/{chapter_id}/write");
    let (a, b) = tokio::join!(
        runtime.app.post(&path, json!({})),
        runtime.app.post(&path, json!({}))
    );
    let mut created = 0;
    let mut refused = None;
    for (status, body) in [&a, &b] {
        match *status {
            StatusCode::CREATED => created += 1,
            StatusCode::CONFLICT => refused = Some(body.clone()),
            other => panic!("unexpected status {other}: {body}"),
        }
    }
    assert_eq!(
        created, 1,
        "exactly one write may start a session: {a:?} {b:?}"
    );
    let refused = refused.expect("the losing request answers 409");
    assert_eq!(
        refused["error"]["code"], "chapter.session_exists",
        "the documented code for a second writer session: {refused}"
    );
    assert!(
        refused["error"]["details"]["session_id"].is_i64(),
        "the 409 names the open session: {refused}"
    );
    assert_eq!(
        open_sessions(&runtime, chapter_id, "chapter_write")
            .await
            .len(),
        1,
        "invariant 5: one open writer session per chapter"
    );
}

/// Two concurrent "ask the reviewer" calls start exactly one reviewer session.
#[tokio::test]
async fn concurrent_reviews_start_one_reviewer_session() {
    let runtime = runtime(Vec::new()).await;
    let chapter_id = runtime.chapter_id;
    runtime.app.put_text(chapter_id, 0, "一段正文。").await;
    let (status, body) = runtime
        .app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let path = format!("/api/chapters/{chapter_id}/review");
    let (a, b) = tokio::join!(
        runtime.app.post(&path, json!({})),
        runtime.app.post(&path, json!({}))
    );
    // Ask-the-reviewer answers 200 with the session id (it returns an existing session when one
    // matches), so the assertion is on the number of open sessions, not on the status codes.
    for (status, body) in [&a, &b] {
        assert!(status.is_success(), "review: {status} {body}");
    }
    assert_eq!(
        open_sessions(&runtime, chapter_id, "chapter_review")
            .await
            .len(),
        1,
        "invariant 5: one open reviewer session per chapter: {a:?} {b:?}"
    );
}

/// The guard is in session creation itself, so a caller that forgets its own pre-check cannot
/// create a second open writer session either. This is the shape that fails without the fix.
#[tokio::test]
async fn session_creation_refuses_a_second_open_writer_session() {
    let runtime = runtime(Vec::new()).await;
    let chapter_id = runtime.chapter_id;
    let session_id = agent_write_harness::start_writer_on(
        &runtime,
        chapter_id,
        Granularity::Paragraph,
        false,
        false,
        false,
    )
    .await;

    let error = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(chapter_id),
            granularity: Some(Granularity::Free),
            ..CreateSession::default()
        },
    )
    .await
    .expect_err("a second open writer session for one chapter must be refused");
    assert_eq!(
        error.code,
        nekowriter::error::ErrorCode::ChapterSessionExists
    );
    assert_eq!(
        error
            .details
            .as_ref()
            .and_then(|details| details["session_id"].as_i64()),
        Some(session_id as i64),
        "the 409 names the open session: {:?}",
        error.details
    );
    assert_eq!(
        open_sessions(&runtime, chapter_id, "chapter_write")
            .await
            .len(),
        1,
        "invariant 5: one open writer session per chapter"
    );
}

/// Invariant 5 is one open session **per chapter-scoped role**, not one per chapter: the guard must
/// let a reviewer session start while a writer session is open on the same chapter. The workflow
/// preconditions are checked by their own callers (a writer session moves the chapter to
/// `drafting`, and asking the reviewer needs `in_review`), so this test goes through session
/// creation directly to pin the shape of the guard: it is keyed by role.
#[tokio::test]
async fn the_guard_is_per_role_not_per_chapter() {
    // The scripted outline holds because `pause_after_outline` is on (spec/03 §5.2), so the writer
    // session is open but not running.
    let runtime = runtime(vec![outline_step()]).await;
    let chapter_id = runtime.chapter_id;
    let writer = agent_write_harness::start_writer_on(
        &runtime,
        chapter_id,
        Granularity::Paragraph,
        false,
        true,
        false,
    )
    .await;
    runtime.wait_status(writer, SessionStatus::Held).await;

    // A reviewer session for the same chapter is a different role, so creation accepts it.
    let (reviewer, _warnings) = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterReview,
            book_id: Some(runtime.book_id),
            chapter_id: Some(chapter_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("a reviewer session may start beside an open writer session");
    assert_ne!(reviewer, writer);
    assert_eq!(
        open_sessions(&runtime, chapter_id, "chapter_write")
            .await
            .len(),
        1,
        "the writer session is still open"
    );
    assert_eq!(
        open_sessions(&runtime, chapter_id, "chapter_review")
            .await
            .len(),
        1,
        "the reviewer session is open too"
    );

    // A second writer session is still refused: the role is what the guard keys on.
    let error = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(chapter_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect_err("a second open writer session must be refused");
    assert_eq!(
        error.code,
        nekowriter::error::ErrorCode::ChapterSessionExists
    );
}
