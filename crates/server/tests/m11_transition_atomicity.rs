//! PLAN 11 hardening (task-24): a status change and its domain consequence commit together.
//!
//! `session::transition` writes the session status **and** (for an archivist session) the chapter's
//! `canon_status`. On a plain pool connection those two UPDATEs were separate auto-commits, so a
//! reader could observe the session `failed` while the chapter still said `syncing` — the transient
//! state the M9 review caught as a flake, and a violation of spec/01 §7 ("every write transaction
//! goes through `Db::write`"). The runner now performs every status change inside one write
//! transaction (D-113).
//!
//! The test-only pause between the two writes widens the window deterministically: inside the
//! transaction it changes nothing for readers, while after reverting the fix it exposes the
//! inconsistent pair immediately.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use nekowriter::agent::session::TEST_TRANSITION_PAUSE_MS;
use nekowriter::db::json::{SessionKind, SessionStatus};
use nekowriter::services::sessions::{self, CreateSession};

#[path = "agent_write_harness/mod.rs"]
mod agent_write_harness;

use agent_write_harness::{refusal_step, runtime};

#[tokio::test]
async fn a_failed_session_and_its_chapter_consequence_commit_together() {
    let runtime = runtime(vec![refusal_step("the model refused the turn")]).await;
    let state = runtime.app.state().clone();
    // The chapter a maintenance session runs on: official and currently synced, so the session's
    // creation side effect moves it to `syncing` and its failure must move it to `failed`.
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET status = 'official', canon_status = 'synced' WHERE id = {}",
            runtime.chapter_id
        ))
        .await;
    TEST_TRANSITION_PAUSE_MS.store(150, Ordering::Relaxed);

    let (session_id, _warnings) = sessions::create(
        &state,
        CreateSession {
            kind: SessionKind::ChapterMaintenance,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            research_task_id: None,
            granularity: None,
            auto_pause: None,
            pause_after_outline: None,
            redraft: false,
            guidance: None,
            kickoff_extra: None,
            pending_comment: None,
            legacy_planning_fixture: false,
            legacy_system_prompt: None,
            legacy_kickoff_text: None,
            chapter_guard: false,
            force_previous: false,
        },
    )
    .await
    .expect("create the maintenance session");

    // A reader outside the write lock: as long as the session is not terminal the chapter is
    // `syncing`, and the moment the session is `failed` the chapter must be `failed` as well.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let row = runtime.session_row(session_id).await;
        let canon = {
            use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
            let row = runtime
                .app
                .db()
                .conn()
                .query_one_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "SELECT canon_status FROM chapters WHERE id = ?",
                    vec![runtime.chapter_id.into()],
                ))
                .await
                .expect("query the chapter")
                .expect("the chapter row");
            row.try_get::<String>("", "canon_status")
                .expect("canon_status")
        };
        if row.status == SessionStatus::Failed.as_str() {
            assert_eq!(
                canon, "failed",
                "a failed session was visible before its chapter consequence committed (spec/01 §7)"
            );
            break;
        }
        assert_eq!(
            canon, "syncing",
            "the chapter left `syncing` before the session became terminal"
        );
        assert!(
            Instant::now() < deadline,
            "the session never failed; it is {}",
            row.status
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    TEST_TRANSITION_PAUSE_MS.store(0, Ordering::Relaxed);
}
