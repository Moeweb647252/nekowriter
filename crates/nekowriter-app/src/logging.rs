//! Logging to standard output and to daily log files in the data directory.

use std::{fs, io, path::Path};

use thiserror::Error;
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{InitError, RollingFileAppender, Rotation},
};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// How many daily log files are kept.
const KEPT_LOG_FILES: usize = 7;

/// The log filter unless `RUST_LOG` sets another one.
const DEFAULT_FILTER: &str = "info,sqlx=warn,sea_orm=warn";

#[derive(Debug, Error)]
pub enum LoggingError {
    #[error("The log directory {path} could not be created: {source}")]
    Directory { path: String, source: io::Error },
    #[error("The log file could not be opened: {0}")]
    File(#[from] InitError),
    #[error("Logging could not be set up: {0}")]
    Subscriber(#[from] tracing_subscriber::util::TryInitError),
}

/// Sends log records to standard output and to `log_dir/nekowriter.<date>.log`. Records
/// are written to the file by a background thread until the returned guard drops.
pub fn init_logging(log_dir: &Path) -> Result<WorkerGuard, LoggingError> {
    fs::create_dir_all(log_dir).map_err(|source| LoggingError::Directory {
        path: log_dir.display().to_string(),
        source,
    })?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("nekowriter")
        .filename_suffix("log")
        .max_log_files(KEPT_LOG_FILES)
        .build(log_dir)?;
    let (file_writer, guard) = tracing_appender::non_blocking(appender);
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .with(fmt::layer().with_ansi(false).with_writer(file_writer))
        .try_init()?;
    Ok(guard)
}
