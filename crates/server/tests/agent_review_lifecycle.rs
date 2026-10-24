//! Regression tests from the M3 milestone review: the process lifecycle (startup recovery and
//! graceful shutdown).
//!
//! These are NEW files (m3-review) so they never collide with another milestone's tests.

mod agent_harness;

use std::time::{Duration, Instant};

use nekowriter::db::json::{SessionStatus, ToolCallStatus};

/// The RFC 3339 timestamp the fixture rows carry (`DateTime<Utc>` round-trips through the entity).
const TS: &str = "2026-01-01T00:00:00Z";

/// Seeds one session left `running` by a previous process, plus one call caught `executing`.
///
/// Raw SQL keeps the fixture self-contained: the point of the test is the startup sequence, not
/// the session-creation flow (which `agent_harness` covers elsewhere).
async fn seed_orphaned_session(db: &nekowriter::db::Db) -> i32 {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

    let session = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO agent_sessions \
             (role, kind, model_snapshot, system_prompt, tools, status, phase, \
              counters, usage, created_at, updated_at) \
             VALUES ('writer', 'chapter_write', '{}', 'frozen prompt', '[]', 'running', \
                     'drafting', '{}', '{}', ?, ?) RETURNING id",
            vec![TS.into(), TS.into()],
        ))
        .await
        .expect("insert session")
        .expect("returned row");
    let session_id: i32 = session.try_get("", "id").expect("session id");

    let message = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO agent_messages (session_id, seq, role, kind, content, created_at) \
             VALUES (?, 1, 'assistant', 'assistant', '[]', ?) RETURNING id",
            vec![session_id.into(), TS.into()],
        ))
        .await
        .expect("insert message")
        .expect("returned row");
    let message_id: i32 = message.try_get("", "id").expect("message id");

    // A call the crash caught mid-`execute`: it committed no effect (D-033).
    db.conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO agent_tool_calls \
             (session_id, message_id, call_index, provider_call_id, name, input, status, \
              appendices, sealed, is_error, created_at, started_at) \
             VALUES (?, ?, 0, 'call_0', 'echo', '{}', 'executing', '[]', 0, 0, ?, ?) RETURNING id",
            vec![session_id.into(), message_id.into(), TS.into(), TS.into()],
        ))
        .await
        .expect("insert call");

    session_id
}

/// Reads a session's status through a fresh connection (the server holds its own pool).
async fn status_from_disk(path: &std::path::Path, session_id: i32) -> Option<String> {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

    let db = nekowriter::db::Db::connect(path).await.ok()?;
    let row = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT status FROM agent_sessions WHERE id = ?",
            vec![session_id.into()],
        ))
        .await
        .ok()??;
    row.try_get("", "status").ok()
}

/// spec/01 §5 step 8: "**Recover sessions**: every session in status `running` → `interrupted`.
/// Every tool call in status `executing` → `pending`."
///
/// This drives **`lib::run` itself** — the production entry point — against a data directory that
/// already holds an orphaned session. That is deliberate: for most of M3 the recovery helper
/// existed and worked, but nothing in `run()` called it, so a real restart left every session the
/// previous process had been running stuck in `running` with no runner. A test that calls
/// `agent::recover()` (or a helper `run()` happens to share) passes even when the wiring is
/// missing, which is precisely how the defect survived review; only asserting on the effect of
/// `run()` itself can catch a missing call.
#[tokio::test]
async fn run_recovers_orphaned_sessions_before_serving() {
    let dir = tempfile::tempdir().expect("temp dir");
    let config = nekowriter::config::Config {
        // Port 0: the test never speaks HTTP, it only needs `run` to get as far as serving.
        bind: "127.0.0.1:0".parse().expect("bind"),
        data_dir: Some(dir.path().to_path_buf()),
        web_dir: None,
        enable_demo_provider: true,
        allow_remote: false,
    };

    // The previous process's database: migrate it and seed the orphan, then close the handle the
    // way a process exit would.
    let db_path = dir.path().join("nekowriter.db");
    let session_id = {
        let db = nekowriter::db::Db::connect(&db_path)
            .await
            .expect("connect the fixture database");
        nekowriter_migration::up(db.conn()).await.expect("migrate");
        seed_orphaned_session(&db).await
    };
    assert_eq!(
        status_from_disk(&db_path, session_id).await.as_deref(),
        Some("running"),
        "the fixture starts where a crash left it"
    );

    // Start the real server. `run` never returns until shutdown, so it is spawned and aborted
    // once the assertion has been made.
    let handle = tokio::spawn(async move { nekowriter::run(config).await });

    // The session must become `interrupted` as part of startup.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if handle.is_finished() {
            let outcome = handle.await.expect("the server task is not cancelled");
            panic!("run() returned early: {outcome:?}");
        }
        let last = status_from_disk(&db_path, session_id).await;
        if last.as_deref() == Some(SessionStatus::Interrupted.as_str()) {
            break;
        }
        if Instant::now() > deadline {
            panic!(
                "lib::run did not recover the orphaned session: status is {last:?}, expected \
                 `interrupted`. spec/01 §5 step 8 requires recovery before the server serves."
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        status_from_disk(&db_path, session_id).await.as_deref(),
        Some("interrupted")
    );

    // The call caught in `executing` is re-runnable (spec/01 §5 step 8, D-033).
    let call_status = {
        use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
        let db = nekowriter::db::Db::connect(&db_path)
            .await
            .expect("reconnect");
        let row = db
            .conn()
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT status, started_at FROM agent_tool_calls",
                vec![],
            ))
            .await
            .expect("query call")
            .expect("a call row");
        (
            row.try_get::<String>("", "status").expect("status"),
            row.try_get::<Option<String>>("", "started_at")
                .expect("started_at"),
        )
    };
    assert_eq!(
        call_status.0,
        ToolCallStatus::Pending.as_str(),
        "a call left executing had no committed effect and must be reset to pending"
    );
    assert!(
        call_status.1.is_none(),
        "the reset clears started_at, so the retry looks like a first run"
    );

    handle.abort();
}

/// spec/01 §5 graceful shutdown: cancel every runner with the `shutdown` reason, so the sessions
/// they own become `interrupted` — "exactly as startup recovery would" — and not `stopped`.
///
/// The fixture parks the runner **inside the model stream** (the scripted provider sleeps before
/// emitting anything, and records the request before it sleeps), so the cancellation is observed
/// by `run_with_retry`'s select rather than at the loop top. That is the harder half of the rule:
/// a shutdown during a stream must still give `interrupted`, so fixing only the loop-top branch
/// is not enough (spec/03 §5.1 names all three await points).
#[tokio::test]
async fn shutdown_interrupts_running_sessions_rather_than_stopping_them() {
    let runtime = agent_harness::runtime(vec![
        nekowriter_llm::mock::ScriptStep::new()
            .text("a reply that is still streaming when the shutdown arrives")
            .delay(Duration::from_secs(30)),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    assert_eq!(
        runtime.session_row(session_id).await.status,
        "running",
        "the fixture session is running"
    );

    // Wait until the provider has received the request: the runner is now inside `run`, i.e. in
    // the streaming await point, not at the loop top.
    let deadline = Instant::now() + Duration::from_secs(10);
    while runtime.scripted.requests().is_empty() {
        assert!(
            Instant::now() < deadline,
            "the runner never started its model request"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // Give the runner a moment to reach the select inside `run_with_retry` (the provider is
    // sleeping on the step's `delay` by now).
    tokio::time::sleep(Duration::from_millis(50)).await;

    // The production shutdown path (what `run` awaits after the signal).
    let cancelled = nekowriter::shutdown_runners(runtime.app.state()).await;
    assert_eq!(cancelled, 1, "one runner was cancelled");
    runtime.wait_idle(session_id).await;

    assert_eq!(
        runtime.session_row(session_id).await.status,
        SessionStatus::Interrupted.as_str(),
        "a shutdown gives `interrupted`, not `stopped` (spec/01 §5, spec/03 §5.1)"
    );
    // And the author can pick it up again, which is the point of `interrupted`.
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/continue"),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "an interrupted session is continuable: {body}"
    );
}

/// spec/03 §5.5: "`refusal` → finish all its calls as error (not executed);
/// `s.error ← {code: "provider.refusal", message}; transition(s, failed)".
#[tokio::test]
async fn a_refusal_finishes_its_calls_as_errors() {
    // A response that both refuses and carries a tool call: every call of it must end as an error
    // and none may run.
    let step = nekowriter_llm::mock::ScriptStep::new()
        .tool("echo", serde_json::json!({ "text": "must not run" }))
        .stop(nekowriter_llm::StopReason::Refusal {
            category: Some("cyber".to_owned()),
            explanation: Some("declined".to_owned()),
        });
    let runtime = agent_harness::runtime(vec![step]).await;
    let session_id = runtime.start_debug().await;

    let row = runtime.wait_status(session_id, SessionStatus::Failed).await;
    let error = row.error().expect("the session records its error");
    assert_eq!(error.code, "provider.refusal");

    let calls = runtime.calls(session_id).await;
    assert_eq!(
        calls.len(),
        1,
        "the refusing response's call exists: {calls:?}"
    );
    assert_eq!(
        calls[0].status_enum(),
        ToolCallStatus::Error,
        "a call of a refusing response is finished as an error, not left pending: {calls:?}"
    );
    assert!(calls[0].is_error, "the call is an error result: {calls:?}");
    assert!(
        calls[0].sealed,
        "the batch is sealed, so a Continue can rebuild a valid request: {calls:?}"
    );
    // "Not executed" means the handler never ran: `echo` returns its `text` argument verbatim, so
    // that argument appearing in the result would prove execution. The result may carry the
    // generic `result.error` wording (spec/09 §2 has no refusal-specific code), which is fine —
    // what must not be there is the tool's own output.
    let result = calls[0].raw_result_text.as_deref().unwrap_or_default();
    assert!(
        !result.contains("must not run"),
        "the refusing response's call must not be executed: {result}"
    );
    assert!(
        !result.is_empty(),
        "the call still carries a model-readable error result: {calls:?}"
    );
}
