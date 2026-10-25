//! `SessionManager`: at most one runner per session, cancellation with a reason, startup
//! recovery and graceful shutdown (spec/01 §5 step 8, spec/03 §14).
//!
//! "One runner per session" is enforced structurally: the handle map is keyed by session id and
//! `spawn` is a no-op while an entry exists. The cancellation token carries a [`CancelReason`],
//! so the runner knows whether a stop means `stopped` (the author pressed Stop) or `interrupted`
//! (the server is shutting down) — spec/03 §5.1.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use tokio_util::sync::CancellationToken;

use crate::db::json::{SessionError, SessionStatus, ToolCallStatus};
use crate::error::AppError;
use crate::state::AppState;

use super::runner::{self, RunnerCtx};
use super::session::{self, TransitionCause};

/// Why a runner was cancelled (spec/03 §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    /// The author pressed Stop; the session becomes `stopped`.
    Stop,
    /// The author abandoned the session; the abandon operation writes `abandoned` itself.
    Abandon,
    /// The server is shutting down; the session becomes `interrupted` (spec/01 §5).
    Shutdown,
}

/// One running session.
struct RunnerHandle {
    /// The generation this handle belongs to; cleanup only removes its own generation.
    generation: u64,
    cancel: CancellationToken,
    reason: Arc<Mutex<Option<CancelReason>>>,
    task: tokio::task::JoinHandle<()>,
}

/// Owns the running sessions of one process.
#[derive(Default, Clone)]
pub struct SessionManager {
    runners: Arc<Mutex<HashMap<i32, RunnerHandle>>>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    /// Test-only: holds one exiting runner between `drive` returning and its handle being
    /// removed, so a test can deterministically land `continue` inside the dying-runner window.
    #[cfg(any(test, feature = "testing"))]
    exit_gate: Arc<Mutex<Option<Arc<TestExitGate>>>>,
}

/// Test-only synchronization for the window between a runner's final transition and the removal
/// of its handle (D-107). Production builds do not contain it.
#[cfg(any(test, feature = "testing"))]
#[derive(Default)]
pub struct TestExitGate {
    /// Notified once when the held runner reaches the gate, so the test knows the row's committed
    /// status and the runner's exit are still separated.
    pub entered: tokio::sync::Notify,
    /// Notified when the runner is released; a test may release it early instead of waiting for
    /// the hold to expire.
    pub release: tokio::sync::Notify,
    /// Notified after the held runner's handle was removed, so the session is genuinely idle.
    pub exited: tokio::sync::Notify,
}

/// How long a gated runner waits before releasing itself, so a forgotten gate cannot hang a test.
#[cfg(any(test, feature = "testing"))]
const TEST_EXIT_HOLD: Duration = Duration::from_secs(2);

impl SessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test-only: arms a deterministic gate that holds the **next** runner to exit in the window
    /// between its final write and the removal of its handle (D-107).
    #[cfg(any(test, feature = "testing"))]
    pub fn test_hold_next_runner_exit(&self) -> Arc<TestExitGate> {
        let gate = Arc::new(TestExitGate::default());
        *lock(&self.exit_gate) = Some(gate.clone());
        gate
    }

    /// Spawns a runner unless a live one is already active for this session (spec/03 §14).
    ///
    /// A handle whose token was cancelled is stale: its runner is on its way out and must not
    /// swallow the spawn that Continue/resume just requested, so it is replaced here. The stale
    /// task's own cleanup only removes the entry when it still owns it (see below), so it cannot
    /// drop the new handle.
    pub fn spawn(&self, state: AppState, session_id: i32) {
        let mut runners = lock(&self.runners);
        if let Some(existing) = runners.get(&session_id)
            && !existing.task.is_finished()
            && !existing.cancel.is_cancelled()
        {
            tracing::debug!(session_id, "a runner is already active; spawn is a no-op");
            return;
        }
        let cancel = CancellationToken::new();
        let reason = Arc::new(Mutex::new(None));
        let cancel_for_task = cancel.clone();
        let reason_for_task = reason.clone();
        let runners_for_task = self.runners.clone();
        let generation = self
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        #[cfg(any(test, feature = "testing"))]
        let exit_gate_for_task = self.exit_gate.clone();
        let task = tokio::spawn(async move {
            // The shared cell, not its current value: `cancel` writes the reason after this task
            // started, so a value captured here would always be `None` and every cancellation
            // (shutdown and abandon included) would look like a Stop (spec/03 §5.1, spec/01 §5).
            let ctx = RunnerCtx {
                state,
                session_id,
                cancel: cancel_for_task,
                cancel_reason: reason_for_task,
            };
            if let Err(error) = runner::drive(ctx).await {
                tracing::error!(session_id, error = %error, "session runner failed");
            }
            // Test-only window (D-107): hold this dying runner after its final transition and
            // before its handle disappears, so a test can prove that Continue cannot lose the
            // spawn. A production build has neither the gate nor the wait.
            #[cfg(any(test, feature = "testing"))]
            let gate = {
                let held = lock(&exit_gate_for_task).take();
                if let Some(gate) = &held {
                    gate.entered.notify_one();
                    let _ = tokio::time::timeout(TEST_EXIT_HOLD, gate.release.notified()).await;
                }
                held
            };
            // Remove the entry only when it is still this runner's own handle: a cancelled
            // runner that is exiting must not delete the handle of the runner that replaced it.
            let mut runners = lock(&runners_for_task);
            if runners
                .get(&session_id)
                .is_some_and(|handle| handle.generation == generation)
            {
                runners.remove(&session_id);
            }
            drop(runners);
            #[cfg(any(test, feature = "testing"))]
            if let Some(gate) = gate {
                gate.exited.notify_one();
            }
        });
        runners.insert(
            session_id,
            RunnerHandle {
                generation,
                cancel,
                reason,
                task,
            },
        );
    }

    /// Requests cancellation of a session's runner with a reason and returns at once
    /// (spec/03 §6: Stop "cancels the runner's token and returns at once").
    pub fn cancel(&self, session_id: i32, reason: CancelReason) -> bool {
        let runners = lock(&self.runners);
        let Some(handle) = runners.get(&session_id) else {
            return false;
        };
        *lock(&handle.reason) = Some(reason);
        handle.cancel.cancel();
        true
    }

    /// True when a runner is active for the session.
    pub fn is_running(&self, session_id: i32) -> bool {
        let runners = lock(&self.runners);
        runners
            .get(&session_id)
            .is_some_and(|handle| !handle.task.is_finished())
    }

    /// Waits until the session's runner exits (used by Abandon, which must stop it first).
    pub async fn wait_for(&self, session_id: i32, timeout: Duration) -> bool {
        let task = {
            let runners = lock(&self.runners);
            runners
                .get(&session_id)
                .map(|handle| handle.task.abort_handle())
        };
        let Some(_) = task else {
            return true;
        };
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            if !self.is_running(session_id) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        !self.is_running(session_id)
    }

    /// Waits for a **cancelled** runner to finish its last write, so an operation that changes the
    /// status right after cancelling cannot race it.
    ///
    /// It returns at once when the session has no runner or when the live runner was not
    /// cancelled: a healthy runner keeps working and must not be waited for. Returns true when no
    /// cancelled runner remains.
    pub async fn wait_cancelled(&self, session_id: i32, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let (cancelled, finished) = {
                let runners = lock(&self.runners);
                match runners.get(&session_id) {
                    Some(handle) => (handle.cancel.is_cancelled(), handle.task.is_finished()),
                    None => return true,
                }
            };
            if !cancelled || finished {
                return true;
            }
            if tokio::time::Instant::now() > deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Cancels every runner with the `shutdown` reason and waits up to `timeout` for them
    /// (spec/01 §5 graceful shutdown).
    pub async fn shutdown(&self, timeout: Duration) -> usize {
        let ids: Vec<i32> = {
            let runners = lock(&self.runners);
            runners.keys().copied().collect()
        };
        for id in &ids {
            self.cancel(*id, CancelReason::Shutdown);
        }
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            let active = {
                let runners = lock(&self.runners);
                runners.values().any(|handle| !handle.task.is_finished())
            };
            if !active {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        ids.len()
    }

    /// The number of active runners (tests).
    pub fn active_count(&self) -> usize {
        let runners = lock(&self.runners);
        runners
            .values()
            .filter(|handle| !handle.task.is_finished())
            .count()
    }
}

/// Recovers a database after a crash or restart (spec/01 §5 step 8, spec/03 §5.4):
///
/// 1. every session in `running` becomes `interrupted` (the author can Continue it), and
/// 2. every tool call in `executing` becomes `pending`, because a call found in `executing` had
///    no committed effects (D-033) and its prepare phase only reads.
///
/// It runs in one transaction, so a crash during recovery cannot leave a half-recovered database.
pub async fn recover(state: &AppState) -> Result<RecoveryReport, AppError> {
    let report = crate::services::write(state.db(), |tx| {
        Box::pin(async move { recover_inner(tx).await })
    })
    .await?;
    for events in report.interrupted.1.iter() {
        state.events().publish_all(events.clone());
    }
    Ok(report)
}

/// The in-transaction part of [`recover`].
async fn recover_inner(tx: &sea_orm::DatabaseTransaction) -> Result<RecoveryReport, AppError> {
    let interrupted = interrupted_running_sessions(tx).await?;
    let reset_calls = reset_executing_calls(tx).await?;
    Ok(RecoveryReport {
        interrupted_sessions: interrupted.0.len(),
        reset_calls,
        interrupted,
    })
}

/// What [`recover`] did (logged at startup, asserted by tests).
#[derive(Debug, Clone)]
pub struct RecoveryReport {
    pub interrupted_sessions: usize,
    pub reset_calls: u64,
    /// The ids of the sessions that were interrupted and their events.
    pub interrupted: (Vec<i32>, Vec<Vec<crate::events::ServerEvent>>),
}

impl RecoveryReport {
    pub fn session_ids(&self) -> &[i32] {
        &self.interrupted.0
    }
}

/// All sessions in `running`, transitioned to `interrupted`; returns their ids and events.
async fn interrupted_running_sessions(
    tx: &sea_orm::DatabaseTransaction,
) -> Result<(Vec<i32>, Vec<Vec<crate::events::ServerEvent>>), AppError> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let rows = crate::db::entities::agent_sessions::Entity::find()
        .filter(
            crate::db::entities::agent_sessions::Column::Status.eq(SessionStatus::Running.as_str()),
        )
        .all(tx)
        .await?;
    let mut ids = Vec::new();
    let mut events = Vec::new();
    for row in rows {
        ids.push(row.id);
        let mut published = session::transition(
            tx,
            &row,
            SessionStatus::Interrupted,
            TransitionCause::Shutdown,
        )
        .await?;
        // Recovery stores the view snapshot for interrupted sessions too (spec/03 §7): the
        // transition already does it for writer and conception sessions, because it captures the
        // snapshot whenever it leaves `running`.
        events.push(std::mem::take(&mut published));
    }
    Ok((ids, events))
}

/// Every `executing` tool call becomes `pending` again (spec/01 §5 step 8).
async fn reset_executing_calls(tx: &sea_orm::DatabaseTransaction) -> Result<u64, AppError> {
    let result = tx
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "UPDATE agent_tool_calls SET status = ?, started_at = NULL WHERE status = ?",
            vec![
                ToolCallStatus::Pending.as_str().to_owned().into(),
                ToolCallStatus::Executing.as_str().to_owned().into(),
            ],
        ))
        .await?;
    Ok(result.rows_affected())
}

/// Clears a stale session error before a Continue (spec/03 §6).
pub async fn clear_session_error(
    conn: &impl sea_orm::ConnectionTrait,
    session_id: i32,
    error: &SessionError,
) -> Result<(), AppError> {
    session::store_error(conn, session_id, error).await
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_manager_has_no_runners() {
        let manager = SessionManager::new();
        assert_eq!(manager.active_count(), 0);
        assert!(!manager.is_running(1));
        assert!(!manager.cancel(1, CancelReason::Stop));
    }
}
