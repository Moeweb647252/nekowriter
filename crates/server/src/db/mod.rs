//! Database access: connection, per-connection pragmas, sqlite-vec registration and the
//! process-wide write lock (spec/01 §5, §7).

pub mod entities;
pub mod json;

use std::path::Path;
use std::sync::Arc;

use sea_orm::sqlx::sqlite::SqliteJournalMode;
use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr, TransactionTrait};
use tokio::sync::Mutex;

/// Pool size (spec/01 §5 step 4).
const POOL_SIZE: u32 = 8;

/// Registers the statically linked sqlite-vec extension for every connection this process
/// opens. Must run before the first connection (spec/01 §5 step 3).
///
/// `sqlite3_auto_extension` is process-global and idempotent per extension; the caller wraps it
/// in [`std::sync::Once`].
fn register_sqlite_vec() {
    #[cfg(not(target_os = "windows"))]
    unsafe {
        // SAFETY: sqlite-vec's init function has the signature sqlite3_auto_extension expects
        // (a C function pointer taking no arguments and returning an int).
        type AutoExtensionFn = unsafe extern "C" fn(
            *mut libsqlite3_sys::sqlite3,
            *mut *mut std::ffi::c_char,
            *const libsqlite3_sys::sqlite3_api_routines,
        ) -> std::ffi::c_int;
        let init: AutoExtensionFn = std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ());
        libsqlite3_sys::sqlite3_auto_extension(Some(init));
    }
    #[cfg(target_os = "windows")]
    {
        tracing::warn!("sqlite-vec auto-extension is not registered on Windows builds");
    }
}

static REGISTER_VEC: std::sync::Once = std::sync::Once::new();

/// The database handle. All write transactions go through [`Db::write`].
#[derive(Clone)]
pub struct Db {
    conn: DatabaseConnection,
    write_lock: Arc<Mutex<()>>,
}

impl Db {
    /// Opens (creating when absent) the SQLite database at `path`.
    pub async fn connect(path: &Path) -> Result<Self, DbErr> {
        REGISTER_VEC.call_once(register_sqlite_vec);

        let url = format!("sqlite://{}?mode=rwc", path.display());
        let mut options = ConnectOptions::new(url);
        options
            .max_connections(POOL_SIZE)
            .min_connections(1)
            .sqlx_logging(false)
            // `busy_timeout` is a per-connection pragma; `foreign_keys`, `synchronous` and
            // `temp_store` are too (spec/01 §5 step 4).
            .map_sqlx_sqlite_opts(|opts| {
                opts.journal_mode(SqliteJournalMode::Wal)
                    .busy_timeout(std::time::Duration::from_millis(5_000))
                    .pragma("synchronous", "NORMAL")
                    .pragma("foreign_keys", "ON")
                    .pragma("temp_store", "MEMORY")
            });

        let conn = Database::connect(options).await?;
        Ok(Self {
            conn,
            write_lock: Arc::new(Mutex::new(())),
        })
    }

    /// Wraps an existing SeaORM connection (tests).
    pub fn from_connection(conn: DatabaseConnection) -> Self {
        Self {
            conn,
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    /// The underlying connection for read-only queries.
    pub fn conn(&self) -> &DatabaseConnection {
        &self.conn
    }

    /// Runs one write transaction while holding the process-wide write lock (spec/01 §7).
    ///
    /// SQLite allows one writer at a time, and a deferred transaction that reads before writing
    /// fails immediately with `SQLITE_BUSY` when another connection committed in between. The
    /// lock makes that impossible. The callback must not await an LLM call, a web request or an
    /// embedding: those belong to an agent tool's prepare phase (spec/05 §1.2).
    pub async fn write<F, T>(&self, f: F) -> Result<T, DbErr>
    where
        F: for<'c> FnOnce(
            &'c sea_orm::DatabaseTransaction,
        ) -> futures::future::BoxFuture<'c, Result<T, DbErr>>,
    {
        let _guard = self.write_lock.lock().await;
        let tx = self.conn.begin().await?;
        match f(&tx).await {
            Ok(value) => {
                tx.commit().await?;
                Ok(value)
            }
            Err(err) => {
                if let Err(rollback) = tx.rollback().await {
                    tracing::warn!(error = %rollback, "rollback failed after a failed transaction");
                }
                Err(err)
            }
        }
    }

    /// Checks the sqlite-vec extension and returns its version string (spec/01 §5 step 4).
    pub async fn vec_version(&self) -> Result<String, DbErr> {
        use sea_orm::{ConnectionTrait, Statement};
        let statement = Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "select vec_version() as version",
        );
        let row = self
            .conn
            .query_one_raw(statement)
            .await?
            .ok_or_else(|| DbErr::Custom("vec_version() returned no row".to_owned()))?;
        row.try_get::<String>("", "version")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::ConnectionTrait;

    async fn temp_db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = Db::connect(&dir.path().join("test.db"))
            .await
            .expect("connect");
        (dir, db)
    }

    #[tokio::test]
    async fn vec_version_available() {
        let (_dir, db) = temp_db().await;
        let version = db.vec_version().await.expect("vec_version");
        assert!(!version.is_empty());
    }

    #[tokio::test]
    async fn vec0_table_insert_and_knn() {
        let (_dir, db) = temp_db().await;
        db.conn()
            .execute_unprepared(
                "CREATE VIRTUAL TABLE vec_test USING vec0(embedding float[4] distance_metric=cosine)",
            )
            .await
            .expect("create vec0");
        db.conn()
            .execute_unprepared(
                "INSERT INTO vec_test(rowid, embedding) VALUES
                   (1, '[1.0, 0.0, 0.0, 0.0]'), (2, '[0.0, 1.0, 0.0, 0.0]')",
            )
            .await
            .expect("insert");
        let row = db
            .conn()
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT rowid FROM vec_test WHERE embedding MATCH '[0.9, 0.1, 0.0, 0.0]' AND k = 1",
            ))
            .await
            .expect("knn")
            .expect("one row");
        let rowid: i64 = row.try_get("", "rowid").expect("rowid");
        assert_eq!(rowid, 1);
    }

    #[tokio::test]
    async fn fts5_insert_and_match() {
        let (_dir, db) = temp_db().await;
        db.conn()
            .execute_unprepared(
                "CREATE VIRTUAL TABLE fts_test USING fts5(title, body, tokenize = 'unicode61 remove_diacritics 2')",
            )
            .await
            .expect("create fts5");
        db.conn()
            .execute_unprepared(
                "INSERT INTO fts_test(rowid, title, body) VALUES (1, 'a', 'hello world')",
            )
            .await
            .expect("insert");
        let row = db
            .conn()
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT rowid FROM fts_test WHERE fts_test MATCH 'hello'",
            ))
            .await
            .expect("match")
            .expect("one row");
        let rowid: i64 = row.try_get("", "rowid").expect("rowid");
        assert_eq!(rowid, 1);
    }

    #[tokio::test]
    async fn pragmas_are_set_per_connection() {
        let (_dir, db) = temp_db().await;
        // Use several connections from the pool and check the pragmas on each.
        let mut tasks = Vec::new();
        for _ in 0..4 {
            let db = db.clone();
            tasks.push(tokio::spawn(async move {
                let row = db
                    .conn()
                    .query_one_raw(sea_orm::Statement::from_string(
                        sea_orm::DatabaseBackend::Sqlite,
                        "SELECT (SELECT * FROM pragma_journal_mode) AS journal,
                                (SELECT * FROM pragma_foreign_keys) AS fk,
                                (SELECT * FROM pragma_synchronous) AS sync",
                    ))
                    .await
                    .expect("query pragmas")
                    .expect("row");
                let journal: String = row.try_get("", "journal").expect("journal");
                let fk: i64 = row.try_get("", "fk").expect("fk");
                (journal, fk)
            }));
        }
        for task in tasks {
            let (journal, fk) = task.await.expect("join");
            assert_eq!(journal.to_lowercase(), "wal");
            assert_eq!(fk, 1);
        }
    }

    #[tokio::test]
    async fn fifty_concurrent_read_then_write_transactions_succeed() {
        let (_dir, db) = temp_db().await;
        db.conn()
            .execute_unprepared(
                "CREATE TABLE counter (id INTEGER PRIMARY KEY, value INTEGER NOT NULL)",
            )
            .await
            .expect("create table");
        db.conn()
            .execute_unprepared("INSERT INTO counter(id, value) VALUES (1, 0)")
            .await
            .expect("seed");

        let mut tasks = Vec::new();
        for _ in 0..50 {
            let db = db.clone();
            tasks.push(tokio::spawn(async move {
                db.write(|tx| {
                    Box::pin(async move {
                        use sea_orm::ConnectionTrait;
                        let row = tx
                            .query_one_raw(sea_orm::Statement::from_string(
                                sea_orm::DatabaseBackend::Sqlite,
                                "SELECT value FROM counter WHERE id = 1",
                            ))
                            .await?
                            .expect("row");
                        let value: i64 = row.try_get("", "value")?;
                        tx.execute_unprepared(&format!(
                            "UPDATE counter SET value = {} WHERE id = 1",
                            value + 1
                        ))
                        .await?;
                        Ok(())
                    })
                })
                .await
            }));
        }
        for task in tasks {
            task.await.expect("join").expect("write transaction");
        }
        let row = db
            .conn()
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT value FROM counter WHERE id = 1",
            ))
            .await
            .expect("query")
            .expect("row");
        let value: i64 = row.try_get("", "value").expect("value");
        assert_eq!(value, 50, "no lost updates without SQLITE_BUSY");
    }
}
