//! The NekoWriter application: the HTTP API, the event stream, the dispatcher that talks to
//! the model service, and the embedded web UI, served on the loopback interface.
//!
//! `main.rs` turns command-line arguments into a [`Config`], sets up logging and calls
//! [`start`]. Tests call [`start`] directly with a temporary data directory and port 0.

use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};

use nekowriter_agent::client::ChatClient;
use nekowriter_store::{DATABASE_FILE, OpenError, Store, StoreError};
use thiserror::Error;
use tokio::{net::TcpListener, task::JoinHandle};

mod data_dir;
mod dispatcher;
mod http;
mod lifecycle;
mod logging;
mod preview;

pub use data_dir::{DataDirError, DataDirLock, default_data_dir, prepare as prepare_data_dir};
pub use lifecycle::Shutdown;
pub use logging::{LoggingError, init_logging};

/// The application version, recorded in new databases and shown in diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The port the server listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 47821;

/// What [`start`] needs to know.
#[derive(Clone, Debug)]
pub struct Config {
    /// An absolute path to the data directory. It must exist.
    pub data_dir: PathBuf,
    /// The directory logs are written to, shown in diagnostics.
    pub log_dir: PathBuf,
    /// The loopback port to listen on; 0 picks a free port.
    pub port: u16,
}

/// Why the application could not start.
///
/// The messages are shown to the person starting the application, so each one says what
/// happened and what to do next.
#[derive(Debug, Error)]
pub enum StartupError {
    #[error(transparent)]
    DataDir(#[from] DataDirError),
    #[error(
        "The database in {data_dir} was not created by NekoWriter. Nothing was read or changed. Start NekoWriter with a different --data-dir."
    )]
    ForeignDatabase { data_dir: PathBuf },
    #[error(
        "The database in {data_dir} was created by NekoWriter {created_by_version}, whose data format this version ({VERSION}) cannot read. Nothing was read or changed. Open it with the version that created it, or start NekoWriter with a different --data-dir."
    )]
    IncompatibleDatabase {
        data_dir: PathBuf,
        created_by_version: String,
    },
    #[error("The database in {data_dir} could not be opened: {source}")]
    Database {
        data_dir: PathBuf,
        source: OpenError,
    },
    #[error(
        "The runs that were executing when NekoWriter last stopped could not be marked interrupted: {0}"
    )]
    Recovery(#[source] StoreError),
    #[error(
        "NekoWriter could not listen on {address}: {source}. Another program may be using the port; start NekoWriter with --port <number>."
    )]
    Listen {
        address: SocketAddr,
        source: io::Error,
    },
}

/// Why the application stopped with an error after it had started.
#[derive(Debug, Error)]
pub enum RunError {
    #[error("the HTTP server failed: {0}")]
    Serve(#[source] io::Error),
    #[error("the HTTP server task ended unexpectedly: {0}")]
    Task(#[source] tokio::task::JoinError),
    #[error("the dispatcher task ended unexpectedly: {0}")]
    Dispatcher(#[source] tokio::task::JoinError),
    #[error("the executing runs could not be marked interrupted: {0}")]
    Interrupt(#[source] StoreError),
    #[error("the database could not be closed cleanly: {0}")]
    Close(#[source] StoreError),
}

/// A started application.
pub struct RunningServer {
    address: SocketAddr,
    shutdown: Shutdown,
    store: Store,
    server: JoinHandle<io::Result<()>>,
    dispatcher: JoinHandle<()>,
    _lock: DataDirLock,
}

impl RunningServer {
    /// The address the server listens on.
    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The signal that stops the server.
    #[must_use]
    pub fn shutdown(&self) -> Shutdown {
        self.shutdown.clone()
    }

    /// Waits until the server and the dispatcher have stopped, marks the runs that were
    /// executing interrupted (§16.3), then closes the database and releases the data
    /// directory.
    pub async fn wait(self) -> Result<(), RunError> {
        let served = self.server.await;
        let dispatched = self.dispatcher.await;
        served.map_err(RunError::Task)?.map_err(RunError::Serve)?;
        dispatched.map_err(RunError::Dispatcher)?;
        interrupt_runs(&self.store)
            .await
            .map_err(RunError::Interrupt)?;
        self.store.close().await.map_err(RunError::Close)
    }
}

/// Locks the data directory, opens the database, marks the runs that were executing when
/// the application last stopped interrupted, and starts serving and dispatching.
pub async fn start(config: Config) -> Result<RunningServer, StartupError> {
    let lock = DataDirLock::acquire(&config.data_dir)?;
    let store = Store::open(&config.data_dir.join(DATABASE_FILE), VERSION)
        .await
        .map_err(|source| open_error(&config.data_dir, source))?;
    interrupt_runs(&store)
        .await
        .map_err(StartupError::Recovery)?;
    let requested = SocketAddr::from((Ipv4Addr::LOCALHOST, config.port));
    let listener = TcpListener::bind(requested)
        .await
        .map_err(|source| StartupError::Listen {
            address: requested,
            source,
        })?;
    let address = listener
        .local_addr()
        .map_err(|source| StartupError::Listen {
            address: requested,
            source,
        })?;
    let shutdown = Shutdown::new();
    let client = ChatClient::new();
    let previews = preview::Previews::new();
    let dispatcher = dispatcher::Dispatcher {
        store: store.clone(),
        client: client.clone(),
        previews: previews.clone(),
        shutdown: shutdown.clone(),
    }
    .spawn();
    let state = http::AppState {
        store: store.clone(),
        shutdown: shutdown.clone(),
        client,
        previews,
        diagnostics: Arc::new(http::Diagnostics {
            data_dir: config.data_dir,
            log_dir: config.log_dir,
        }),
        port: address.port(),
    };
    let stopped = shutdown.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, http::router(state))
            .with_graceful_shutdown(async move { stopped.triggered().await })
            .await
    });
    Ok(RunningServer {
        address,
        shutdown,
        store,
        server,
        dispatcher,
        _lock: lock,
    })
}

/// Startup recovery and shutdown share this step (§16.3).
async fn interrupt_runs(store: &Store) -> Result<(), StoreError> {
    let interrupted = store.interrupt_runs().await?;
    if interrupted > 0 {
        tracing::info!(
            interrupted,
            "runs that were executing are interrupted until the user continues them"
        );
    }
    Ok(())
}

fn open_error(data_dir: &std::path::Path, source: OpenError) -> StartupError {
    let data_dir = data_dir.to_owned();
    match source {
        OpenError::Foreign => StartupError::ForeignDatabase { data_dir },
        OpenError::SchemaMismatch { created_by_version } => StartupError::IncompatibleDatabase {
            data_dir,
            created_by_version,
        },
        source => StartupError::Database { data_dir, source },
    }
}
