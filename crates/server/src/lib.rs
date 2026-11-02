//! `nekowriter` library: configuration, state, database, API and (from M3) the agent runtime.

pub mod agent;
pub mod api;
pub mod config;
pub mod db;
pub mod error;
pub mod events;
pub mod prompts;
pub mod search;
pub mod services;
pub mod state;
pub mod text;
pub mod web;

/// Test helpers behind the `testing` feature (spec/01 §11).
#[cfg(any(test, feature = "testing"))]
pub mod testing;

use std::path::Path;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;

/// Starts the server (spec/01 §5). Called from the binary.
pub async fn run(mut config: Config) -> anyhow::Result<()> {
    // The data directory is resolved before tracing so the log file can live inside it
    // (spec/01 §5 steps 1-2, §9).
    let data_dir = config.resolve_data_dir()?;
    let _log_guard = init_tracing(&data_dir);
    config.check_bind_allowed()?;
    // The embedding model cache lives under `<data_dir>/models` (spec/01 §5, spec/07 §4).
    config.data_dir = Some(data_dir.clone());

    tracing::info!(data_dir = %data_dir.display(), "starting NekoWriter");

    let db_path = data_dir.join("nekowriter.db");
    let db = crate::db::Db::connect(&db_path)
        .await
        .with_context(|| format!("cannot open the database at {}", db_path.display()))?;

    nekowriter_migration::up(db.conn())
        .await
        .context("database migration failed")?;

    let vec_version = db
        .vec_version()
        .await
        .context("sqlite-vec is not available")?;
    tracing::info!(vec_version, "sqlite-vec ready");

    // Load and validate the embedded prompt pack; a failure is a build defect (spec/01 §5).
    crate::prompts::PromptRegistry::embedded().context("the embedded prompt pack is invalid")?;

    let state = prepare_state(db, config.clone(), vec_version).await?;
    let app = api::build_app(state.clone());

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("cannot bind to {}", config.bind))?;
    tracing::info!(addr = %config.bind, "listening on http://{}/", config.bind);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(state))
        .await
        .context("server error")
}

/// Builds the application state and recovers the sessions the previous process left behind
/// (spec/01 §5 steps 7–8).
///
/// This is the production sequence between opening the database and serving: **state first,
/// then recovery, before any request is accepted**. It is a separate function so a test can drive
/// it end to end — a test that calls [`crate::agent::recover`] directly would pass even if `run()`
/// never called it, which is exactly the defect this factoring prevents.
pub async fn prepare_state(
    db: crate::db::Db,
    config: Config,
    vec_version: String,
) -> anyhow::Result<AppState> {
    let state = AppState::new(db, config, vec_version);
    // The search subsystem: install the configured embedding model and create the runtime vec
    // tables for its dimension before any request can search (spec/07 §3).
    crate::search::initialize(&state)
        .await
        .context("search index initialization failed")?;
    // spec/01 §5 step 8: every session left `running` becomes `interrupted` and every tool call
    // left `executing` becomes `pending`. A session the previous process left running has no
    // runner, and the control API refuses to continue anything but stopped/failed/interrupted, so
    // skipping this strands it beyond Abandon.
    // A model profile whose options or effort levels its provider's protocol cannot send is a
    // silent no-op on the wire (D-132, D-162): an option the adapter ignores or a level it cannot
    // place changes no request, and the turn still succeeds. The settings page reports both per
    // profile, and this line makes them visible to anyone reading the log without opening Settings.
    match crate::services::providers::stored_option_mismatches(state.db().conn()).await {
        Ok(mismatches) => {
            for (id, profile, provider, options, effort_levels) in mismatches {
                tracing::warn!(
                    profile_id = id,
                    profile = %profile,
                    provider = %provider,
                    options = ?options,
                    effort_levels = ?effort_levels,
                    "this model profile sets options or effort levels its provider's protocol cannot send; they are ignored when calling the model. Open Settings > Models and remove or replace them."
                );
            }
        }
        Err(error) => {
            tracing::warn!(error = %error, "cannot check model profiles for protocol mismatches")
        }
    }
    let report = crate::agent::recover(&state)
        .await
        .context("session recovery failed")?;
    if report.interrupted_sessions > 0 || report.reset_calls > 0 {
        tracing::info!(
            interrupted_sessions = report.interrupted_sessions,
            reset_calls = report.reset_calls,
            session_ids = ?report.session_ids(),
            "recovered sessions left running by the previous process"
        );
    }
    Ok(state)
}

/// Resolves after Ctrl+C or SIGTERM, then cancels every runner with the `shutdown` reason and
/// waits up to [`SHUTDOWN_RUNNER_TIMEOUT`] for them to exit (spec/01 §5 graceful shutdown).
///
/// The runners transition their sessions to `interrupted` themselves — the same status startup
/// recovery produces — and `axum::serve` returns only after this future completes, so the pool
/// closes after the runners' last write rather than during it.
async fn shutdown_signal(state: AppState) {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %err, "cannot listen for Ctrl+C");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(err) => {
                tracing::error!(error = %err, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
    shutdown_runners(&state).await;
}

/// How long graceful shutdown waits for the runners to finish their last write (spec/01 §5).
pub const SHUTDOWN_RUNNER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Cancels every runner with the `shutdown` reason and waits for them (spec/01 §5).
///
/// Runners transition their sessions to `interrupted`, so a restart sees the same state recovery
/// would have produced from a crash. Returns how many runners were cancelled.
pub async fn shutdown_runners(state: &AppState) -> usize {
    let cancelled = state.manager().shutdown(SHUTDOWN_RUNNER_TIMEOUT).await;
    if cancelled > 0 {
        tracing::info!(
            runners = cancelled,
            "cancelled running sessions for shutdown"
        );
    }
    cancelled
}

/// Initializes `tracing` with `RUST_LOG` (default `info,nekowriter=debug`). Pretty output on
/// stderr in debug builds, JSON in release, and in both cases rolling daily JSON files under
/// `<data_dir>/logs/` kept for seven days (spec/01 §9, §6).
///
/// Returns the appender guard: it must stay alive for as long as logging should continue, so the
/// caller binds it to a variable for the process lifetime.
pub fn init_tracing(data_dir: &Path) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,nekowriter=debug"));

    let appender = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("nekowriter")
        .filename_suffix("log")
        .max_log_files(7)
        .build(data_dir.join("logs"))
        // The data directory (with `logs/`) is created before tracing starts, so a failure here
        // is a real filesystem problem rather than a missing directory.
        .expect("create the rolling log file appender under <data_dir>/logs");
    let (file_writer, guard) = tracing_appender::non_blocking(appender);

    // The file sink always writes JSON, with color codes off so the files stay machine-readable.
    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_ansi(false)
        .with_writer(file_writer);

    // Pretty on stderr in debug builds, JSON in release (spec/01 §9). The two branches are
    // compile-time alternatives, so each owns its `file_layer`.
    #[cfg(debug_assertions)]
    {
        let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
        tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .with(file_layer)
            .init();
    }
    #[cfg(not(debug_assertions))]
    {
        let stderr_layer = tracing_subscriber::fmt::layer()
            .json()
            .with_writer(std::io::stderr);
        tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .with(file_layer)
            .init();
    }

    guard
}

#[cfg(test)]
mod tests {
    use super::*;

    /// spec/01 §9: logs go to stderr **and** to `<data_dir>/logs/`. The global subscriber can only
    /// be installed once per process, so this test installs it, logs, and inspects the file —
    /// it must not run alongside another test that calls `init_tracing`.
    #[test]
    fn init_tracing_writes_the_log_file_under_the_data_dir() {
        let dir = tempfile::tempdir().expect("temp dir");
        let data_dir = dir.path();
        std::fs::create_dir_all(data_dir.join("logs")).expect("logs dir");

        let guard = init_tracing(data_dir);
        tracing::info!(marker = "nekowriter-log-test", "log file probe");
        // Dropping the guard flushes the non-blocking writer.
        drop(guard);

        let mut entries: Vec<_> = std::fs::read_dir(data_dir.join("logs"))
            .expect("read logs dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .collect();
        entries.sort();
        assert_eq!(
            entries.len(),
            1,
            "expected exactly one log file: {entries:?}"
        );

        let content = std::fs::read_to_string(&entries[0]).expect("read log file");
        assert!(
            content.contains("log file probe"),
            "the log file did not receive the event: {content:?}"
        );
        assert!(
            content.contains("\"level\""),
            "the log file should be JSON lines: {content:?}"
        );
    }

    /// The default `RUST_LOG` filter is `info,nekowriter=debug` (spec/01 §5 step 1).
    #[test]
    fn default_filter_mentions_the_crate_at_debug() {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,nekowriter=debug"));
        assert!(format!("{filter}").contains("nekowriter=debug"));
    }
}
