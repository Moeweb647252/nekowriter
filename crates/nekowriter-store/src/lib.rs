//! Persistence for NekoWriter: one SQLite database per data directory.
//!
//! [`Store`] is the only way to read and write it. Changes made on behalf of a user go
//! through idempotent commands (`command`), each of which commits its data, its receipt and
//! its events in one transaction. The schema comes from the entities in `entity` and is
//! checked on open (`schema`).

use std::{path::Path, time::Duration};

use nekowriter_core::time::Timestamp;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbErr, SqlxSqliteConnector,
    TransactionOptions, TransactionTrait,
    sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use thiserror::Error;

mod calls;
mod chapters;
mod citations;
mod command;
mod delegation;
mod digest;
mod documents;
mod entity;
mod error;
mod events;
mod execution;
mod export;
mod feedback;
mod preparation;
mod projects;
mod proposals;
mod questions;
mod reviews;
mod schema;
mod settings;
mod versions;
mod workflow;
mod workspace;

pub use documents::{RevisionSummary, RevisionView};
pub use error::StoreError;
pub use events::{EventBatch, EventRecord, RETENTION_MILLIS};
pub use execution::{
    DispatchInput, DispatchOutcome, ModelSettings, RecordedAnswer, RoundRecord, SealedRequest,
    TranscriptEntry,
};
pub use export::{ExportedChapter, ExportedVersion};
pub use feedback::{FeedbackView, HeldCallView};
pub use projects::{NewProject, ProjectListFilter, ProjectSettings, ProjectView, VersionSummary};
pub use proposals::ProposalView;
pub use questions::QuestionView;
pub use settings::{ApiKeyChange, SystemSettingsInput, SystemSettingsView};
pub use workflow::WorkspaceAck;
pub use workspace::{ArtifactView, ChapterView, ReviewView, RunView, WorkspaceView};

/// The file name of the database inside the data directory.
pub const DATABASE_FILE: &str = "nekowriter.sqlite3";

/// How long a write waits for another write to finish before failing.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Why the database could not be opened.
#[derive(Debug, Error)]
pub enum OpenError {
    #[error("the database could not be opened: {0}")]
    Database(#[from] DbErr),
    #[error("the database could not be opened: {0}")]
    Connection(#[from] sea_orm::sqlx::Error),
    #[error("the database file contains tables that were not created by NekoWriter")]
    Foreign,
    #[error(
        "the database was created by NekoWriter {created_by_version} with a different data format than this version uses"
    )]
    SchemaMismatch { created_by_version: String },
    #[error("old events could not be pruned: {0}")]
    Prune(#[source] StoreError),
}

/// A handle to the database. Cloning is cheap; all clones share one connection pool.
#[derive(Clone, Debug)]
pub struct Store {
    db: DatabaseConnection,
    events: events::Notifier,
}

impl Store {
    /// Opens the database at `path`, creating it and its schema when the file does not
    /// exist. `app_version` is recorded in a new database for later error messages.
    pub async fn open(path: &Path, app_version: &str) -> Result<Self, OpenError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await?;
        let db = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
        match schema::prepare(&db, app_version).await? {
            schema::SchemaState::Created | schema::SchemaState::Current => {}
            schema::SchemaState::Foreign => return Err(OpenError::Foreign),
            schema::SchemaState::Mismatch { created_by_version } => {
                return Err(OpenError::SchemaMismatch { created_by_version });
            }
        }
        let store = Self {
            events: events::Notifier::new(0),
            db,
        };
        store
            .prune_events(Timestamp::now())
            .await
            .map_err(OpenError::Prune)?;
        let cursor = store.event_cursor().await.map_err(OpenError::Prune)?;
        store.events.notify(cursor);
        Ok(store)
    }

    /// Writes the write-ahead log back into the database file and closes all connections.
    pub async fn close(self) -> Result<(), StoreError> {
        self.db
            .execute_unprepared("PRAGMA wal_checkpoint(TRUNCATE)")
            .await?;
        self.db.close().await?;
        Ok(())
    }

    /// Opens a write transaction. `BEGIN IMMEDIATE` takes SQLite's write lock up front, so a
    /// transaction that reads before it writes never fails to upgrade its lock; concurrent
    /// writers wait up to the busy timeout instead.
    pub(crate) async fn begin_write(&self) -> Result<DatabaseTransaction, DbErr> {
        self.db
            .begin_with_options(TransactionOptions {
                sqlite_transaction_mode: Some(sea_orm::SqliteTransactionMode::Immediate),
                ..TransactionOptions::default()
            })
            .await
    }
}
