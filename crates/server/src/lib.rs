//! `nekowriter` library: configuration, state, database, API and (from M3) the agent runtime.

pub mod api;
pub mod config;
pub mod db;
pub mod error;
pub mod events;
pub mod services;
pub mod state;
pub mod text;

/// Test helpers behind the `testing` feature (spec/01 §11).
#[cfg(any(test, feature = "testing"))]
pub mod testing;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;

/// Starts the server (spec/01 §5). Called from the binary.
pub async fn run(config: Config) -> anyhow::Result<()> {
    init_tracing();
    let data_dir = config.resolve_data_dir()?;
    config.check_bind_allowed()?;

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

    let state = AppState::new(db, config.clone(), vec_version);
    let app = api::build_app(state);

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("cannot bind to {}", config.bind))?;
    tracing::info!(addr = %config.bind, "listening on http://{}/", config.bind);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")
}

/// Resolves after Ctrl+C or SIGTERM.
async fn shutdown_signal() {
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
}

/// Initializes `tracing` with `RUST_LOG` (default `info,nekowriter=debug`). Pretty output in
/// debug builds, JSON in release (spec/01 §9).
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,nekowriter=debug"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if cfg!(debug_assertions) {
        builder.init();
    } else {
        builder.json().init();
    }
}
