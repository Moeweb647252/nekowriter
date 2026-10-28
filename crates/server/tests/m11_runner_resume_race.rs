//! PLAN M11 hardening (task-24): `resume` must not lose its spawn while the previous runner is
//! still exiting.
//!
//! A held session's runner commits `held` and then keeps running for a moment (publishing events,
//! removing its own handle). `resume`'s `wait_cancelled` returns at once in that window because
//! the runner was not cancelled, and `SessionManager::spawn` used to treat any live handle as an
//! active runner and drop the spawn: the row stayed `running` with nothing to drive it. The M8
//! review fixed the same window for `Continue` per caller; the general fix belongs in `spawn`,
//! which now chains a runner behind a live one (D-109).
//!
//! The manager's test-only exit gate (D-107) lands the resume inside the window deterministically,
//! so the test fails without the fix instead of relying on scheduler luck.

use std::time::{Duration, Instant};

use nekowriter::api::dto::sessions::ResumeInput;
use nekowriter::db::json::{Granularity, SessionStatus};

#[path = "agent_write_harness/mod.rs"]
mod agent_write_harness;

use agent_write_harness::{runtime, start_writer};
use nekowriter_llm::mock::ScriptStep;
use serde_json::json;

#[tokio::test]
async fn resume_cannot_leave_a_running_session_without_a_runner() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("insert_text", json!({ "text": "第一段。" })),
    ])
    .await;
    let manager = runtime.app.state().manager();
    // Arm the gate before the session starts: the next runner to exit (the one that commits
    // `held`) is held between that commit and the removal of its handle.
    let gate = manager.test_hold_next_runner_exit();

    let session_id = start_writer(&runtime, Granularity::Paragraph, true, false).await;
    runtime.wait_status(session_id, SessionStatus::Held).await;
    tokio::time::timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .expect("the held runner reaches the exit gate");

    // Resume (approve the outline) while the previous runner is still alive: the spawn must not
    // be mistaken for a duplicate runner and dropped.
    let resume_result = tokio::spawn({
        let state = runtime.app.state().clone();
        async move {
            nekowriter::services::sessions::resume(
                &state,
                session_id,
                ResumeInput { feedback: None },
            )
            .await
        }
    });
    let resume_result = tokio::time::timeout(Duration::from_secs(10), resume_result)
        .await
        .expect("resume finishes")
        .expect("the resume task did not panic");
    assert!(
        resume_result.is_ok(),
        "resume failed: {:?}",
        resume_result.err()
    );

    // Let the dying runner leave the gate. While it is still held, `is_running` is true for it,
    // so the invariant can only be checked after it is gone (that is exactly the window in which
    // the spawn used to be lost).
    tokio::time::timeout(Duration::from_secs(10), gate.exited.notified())
        .await
        .expect("the dying runner leaves the gate");

    // The invariant: once the dying runner has left, either a runner is registered or the session
    // is no longer `running`. A `running` row with no runner would never move again.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let row = runtime.session_row(session_id).await;
        if manager.is_running(session_id) || row.status != SessionStatus::Running.as_str() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "resume lost its spawn: session {session_id} is {} with no runner",
            row.status
        );
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}
