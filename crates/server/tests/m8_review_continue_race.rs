//! M8 review, finding F9: `Continue` must never leave `running` with no runner.
//!
//! `continue_session` used to wait only for a **cancelled** runner. A failed session's runner
//! commits `failed` and then keeps running for a moment (publishing events, removing its handle);
//! a Continue that lands in that window was swallowed by `SessionManager::spawn`, whose no-op
//! guard treats any live handle as an active runner. The row stayed `running` forever, which is
//! exactly the reported `agent_write_holds` flake ("session did not finish; it is running").
//!
//! The test uses the manager's test-only exit gate (D-100), so it lands in that window
//! deterministically instead of relying on scheduler luck. It fails without the fix.

use std::time::Duration;

use nekowriter::db::json::{Granularity, SessionStatus};

#[path = "agent_write_harness/mod.rs"]
mod agent_write_harness;

use agent_write_harness::{refusal_step, runtime, start_writer};

#[tokio::test]
async fn continue_cannot_leave_a_running_session_without_a_runner() {
    // A provider refusal fails the session; the gate then holds the failed runner after its
    // terminal write, before its handle disappears.
    let runtime = runtime(vec![refusal_step("the model refused the turn")]).await;
    let manager = runtime.app.state().manager();
    let gate = manager.test_hold_next_runner_exit();

    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    // The failure is committed, and the runner is held at the gate (it has not exited yet).
    runtime.wait_status(session_id, SessionStatus::Failed).await;
    tokio::time::timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .expect("the failed runner reaches the exit gate");

    // Continue while the previous runner is still alive: it must not be mistaken for an active
    // runner. The gate releases itself after TEST_EXIT_HOLD.
    let handle = tokio::spawn({
        let state = runtime.app.state().clone();
        async move { nekowriter::services::sessions::continue_session(&state, session_id).await }
    });
    tokio::time::timeout(Duration::from_secs(10), gate.exited.notified())
        .await
        .expect("the dying runner leaves the gate");
    let result = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("continue finishes")
        .expect("the continue task did not panic");
    assert!(result.is_ok(), "continue failed: {:?}", result.err());

    // The invariant: after Continue, either a runner is registered or the session is no longer
    // `running`. A `running` row with no runner would never move again.
    let row = runtime.session_row(session_id).await;
    assert!(
        manager.is_running(session_id) || row.status != SessionStatus::Running.as_str(),
        "session {session_id} is {} with no runner: Continue lost its spawn",
        row.status
    );
}
