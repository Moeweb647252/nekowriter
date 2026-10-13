//! The `nekowriter` command: starts the local service and opens the web UI.

use std::{path::PathBuf, process::ExitCode};

use clap::Parser;
use nekowriter_app::{
    Config, DEFAULT_PORT, DataDirError, LoggingError, RunError, StartupError, default_data_dir,
    init_logging, prepare_data_dir,
};
use thiserror::Error;

/// NekoWriter, a local AI novel-writing workbench.
#[derive(Debug, Parser)]
#[command(name = "nekowriter", version)]
struct Cli {
    /// The directory for the database and logs [default: NekoWriter in the local
    /// application-data directory]
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// The port to listen on, on 127.0.0.1
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,
    /// Do not open the web UI in the default browser
    #[arg(long)]
    no_browser: bool,
}

#[derive(Debug, Error)]
enum MainError {
    #[error(transparent)]
    DataDir(#[from] DataDirError),
    #[error(transparent)]
    Logging(#[from] LoggingError),
    #[error(transparent)]
    Startup(#[from] StartupError),
    #[error("NekoWriter stopped with an error: {0}")]
    Run(#[from] RunError),
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Once logging is set up, the log reaches both the terminal and the log file.
            if tracing::dispatcher::has_been_set() {
                tracing::error!("{error}");
            } else {
                eprintln!("{error}");
            }
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), MainError> {
    let data_dir = match cli.data_dir {
        Some(path) => prepare_data_dir(&path)?,
        None => prepare_data_dir(&default_data_dir()?)?,
    };
    let log_dir = data_dir.join("logs");
    let _log_guard = init_logging(&log_dir)?;
    let server = nekowriter_app::start(Config {
        data_dir: data_dir.clone(),
        log_dir,
        port: cli.port,
    })
    .await?;
    server.shutdown().trigger_on_signals();
    let url = format!("http://{}/", server.address());
    tracing::info!(%url, data_dir = %data_dir.display(), "NekoWriter is running; press Ctrl+C to stop it");
    if !cli.no_browser {
        open_browser(url);
    }
    server.wait().await?;
    tracing::info!("NekoWriter stopped");
    Ok(())
}

fn open_browser(url: String) {
    tokio::task::spawn_blocking(move || {
        if let Err(error) = webbrowser::open(&url) {
            tracing::warn!(%error, %url, "the browser could not be opened; open the URL yourself");
        }
    });
}
