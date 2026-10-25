//! spec/08 §1.3: deleting a book stops its runners before the cascade.
//!
//! The delete used to cascade the book's rows while a session's runner was still waiting on its
//! provider call; the runner then wrote into (or failed on) rows that no longer existed. The
//! delete now abandons every open session — cancelling a running one and waiting for its runner —
//! before its own transaction, exactly like the track delete (D-110). The delayed script keeps
//! the runner inside a model call so the pre-fix delete would return while it is still alive.

use std::time::Duration;

use axum::http::StatusCode;
use nekowriter::db::json::{Granularity, SessionStatus};
use nekowriter_llm::mock::ScriptStep;

#[path = "agent_write_harness/mod.rs"]
mod agent_write_harness;

use agent_write_harness::{runtime, start_writer};

#[tokio::test]
async fn deleting_a_book_stops_its_runner_before_the_cascade() {
    let runtime = runtime(vec![ScriptStep::new().delay(Duration::from_secs(2))]).await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Running)
        .await;
    // Give the runner a moment to enter the delayed model call.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        runtime.app.state().manager().is_running(session_id),
        "the runner should be inside its model call"
    );

    let (status, body) = runtime
        .app
        .delete(&format!("/api/books/{}", runtime.book_id))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete book: {body}");
    assert!(
        !runtime.app.state().manager().is_running(session_id),
        "the book's runner must be stopped and awaited before the cascade (spec/08 §1.3)"
    );
    // The cascade removed the session row, so nothing can be continued afterwards.
    let (status, _) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
