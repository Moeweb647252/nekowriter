//! Database access: connection, per-connection pragmas, sqlite-vec registration and the
//! process-wide write lock (spec/01 §5, §7).

pub mod entities;
pub mod json;

use std::path::Path;
use std::sync::Arc;

use sea_orm::raw_sql;
use sea_orm::sqlx::sqlite::SqliteJournalMode;
use sea_orm::{
    ConnectOptions, Database, DatabaseConnection, DbErr, SqliteTransactionMode, TransactionOptions,
    TransactionTrait,
};
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
        // `BEGIN IMMEDIATE` takes SQLite's write lock before the closure runs any statement.
        // With the process-wide mutex this is behaviourally identical for the application, but a
        // write that bypassed the lock would then *wait* for the reserved lock and be serialized
        // behind it, instead of failing at once with `SQLITE_BUSY`/`SQLITE_BUSY_SNAPSHOT`
        // (extended result code 517, which no `busy_timeout` can retry) when it read before
        // another connection committed (spec/01 §7, D-109).
        let tx = self
            .conn
            .begin_with_options(TransactionOptions {
                sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                ..Default::default()
            })
            .await?;
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

    /// Runs one raw SQL statement through [`Db::write`], i.e. under the process-wide write lock.
    ///
    /// Test-only, for the integration fixtures that mutate rows directly (a crash state, a
    /// status later milestones create). Routing them through the lock keeps the tests on the
    /// production discipline: a spawned runner's transaction and a fixture write can no longer
    /// overlap in one process (spec/01 §7, §11; PLAN M11 hardening).
    #[cfg(any(test, feature = "testing"))]
    pub async fn write_unprepared(&self, sql: &str) -> Result<(), DbErr> {
        let sql = sql.to_owned();
        self.write(move |tx| {
            Box::pin(async move {
                sea_orm::ConnectionTrait::execute_unprepared(tx, &sql).await?;
                Ok(())
            })
        })
        .await
    }

    /// Checks the sqlite-vec extension and returns its version string (spec/01 §5 step 4).
    pub async fn vec_version(&self) -> Result<String, DbErr> {
        use sea_orm::ConnectionTrait;
        let statement = raw_sql!(Sqlite, "select vec_version() as version");
        let row = self
            .conn
            .query_one_raw(statement)
            .await?
            .ok_or_else(|| DbErr::Custom("vec_version() returned no row".to_owned()))?;
        row.try_get::<String>("", "version")
    }

    /// Runs the initial migration against this database (test helper, spec/02 §2).
    #[cfg(test)]
    async fn migrate(&self) -> Result<(), DbErr> {
        nekowriter_migration::up(self.conn())
            .await
            .map_err(|err| DbErr::Custom(format!("migration failed: {err}")))
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

    /// `Db::write` begins with `BEGIN IMMEDIATE`: the transaction takes SQLite's write lock from
    /// its first statement instead of lazily on the first write, so a second connection cannot
    /// slip a write in between (the failure `SQLITE_BUSY_SNAPSHOT`, extended result code 517, that
    /// no busy timeout can retry; D-109).
    #[tokio::test]
    async fn write_transactions_take_the_write_lock_immediately() {
        let (_dir, db) = temp_db().await;
        db.conn()
            .execute_unprepared(
                "CREATE TABLE counter (id INTEGER PRIMARY KEY, value INTEGER NOT NULL)",
            )
            .await
            .expect("create table");
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let holder = {
            let db = db.clone();
            tokio::spawn(async move {
                db.write(move |tx| {
                    Box::pin(async move {
                        let _ = entered_tx.send(());
                        let _ = release_rx.await;
                        sea_orm::ConnectionTrait::execute_unprepared(
                            tx,
                            "INSERT INTO counter(id, value) VALUES (1, 1)",
                        )
                        .await?;
                        Ok(())
                    })
                })
                .await
            })
        };
        entered_rx.await.expect("the write transaction began");

        // A second connection with no busy wait must fail to take the write lock, even though the
        // open transaction has not written anything yet. With a deferred `BEGIN` the lock would
        // still be free here and this statement would succeed.
        let error = db
            .conn()
            .execute_unprepared("PRAGMA busy_timeout = 0; BEGIN IMMEDIATE")
            .await
            .expect_err("the open transaction already holds the write lock");
        assert!(
            format!("{error}").to_lowercase().contains("locked"),
            "unexpected error: {error}"
        );

        let _ = release_tx.send(());
        holder
            .await
            .expect("join the holder")
            .expect("the holder commits");
    }

    async fn migrated_db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = Db::connect(&dir.path().join("test.db"))
            .await
            .expect("connect");
        db.migrate().await.expect("migrate");
        (dir, db)
    }

    async fn count(db: &Db, sql: &str) -> i64 {
        let row = db
            .conn()
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                sql,
            ))
            .await
            .expect("query")
            .expect("row");
        row.try_get("", "c").expect("count")
    }

    /// Seeds one book with a main track, a chapter, a `search_docs` row (kind `passage`) and its
    /// `fts_book` row, plus a knowledge scope with an entry and its `fts_knowledge` row.
    async fn seed_indexed_book(db: &Db) -> (i32, i32, i32) {
        db.conn()
            .execute_unprepared(
                "INSERT INTO books(id, title, writing_language, status, settings, created_at, updated_at)
                 VALUES (1, 'Book', 'en', 'active', '{}', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
                 INSERT INTO tracks(id, book_id, kind, title, canon_mode, ord, created_at, updated_at)
                 VALUES (1, 1, 'main', 'Main', 'canon', 0, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
                 INSERT INTO chapters(id, book_id, track_id, ord, status, content, created_at, updated_at)
                 VALUES (1, 1, 1, 1, 'drafting', '{\"paragraphs\":[],\"next_pid\":1,\"tail_open\":false}',
                         '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
                 INSERT INTO search_docs(id, book_id, kind, ref_id, body, content_hash, updated_at)
                 VALUES (1, 1, 'passage', 1, 'hello world', 'h1', '2026-01-01T00:00:00.000Z');
                 INSERT INTO fts_book(rowid, title, body) VALUES (1, '', 'hello world');
                 INSERT INTO knowledge_scopes(id, name, created_at, updated_at)
                 VALUES (1, 'Scope', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
                 INSERT INTO knowledge_entries(id, scope_id, title, content, source, created_at, updated_at)
                 VALUES (1, 1, 'Entry', 'body', 'manual', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
                 INSERT INTO knowledge_chunks(id, entry_id, scope_id, ord, text, content_hash)
                 VALUES (1, 1, 1, 0, 'chunk text', 'h2');
                 INSERT INTO fts_knowledge(rowid, title, body) VALUES (1, '', 'chunk text');",
            )
            .await
            .expect("seed");
        (1, 1, 1)
    }

    /// spec/02 §5 invariant 11: deleting a book removes every index row of that book.
    #[tokio::test]
    async fn deleting_a_book_removes_its_index_rows() {
        let (_dir, db) = migrated_db().await;
        seed_indexed_book(&db).await;
        assert_eq!(count(&db, "SELECT count(*) AS c FROM fts_book").await, 1);

        db.conn()
            .execute_unprepared("DELETE FROM books WHERE id = 1")
            .await
            .expect("delete book");

        assert_eq!(count(&db, "SELECT count(*) AS c FROM search_docs").await, 0);
        assert_eq!(
            count(&db, "SELECT count(*) AS c FROM fts_book").await,
            0,
            "the book cascade must also remove the FTS row"
        );
    }

    /// spec/02 §5 invariant 11: deleting a knowledge scope removes every index row of that scope.
    #[tokio::test]
    async fn deleting_a_scope_removes_its_index_rows() {
        let (_dir, db) = migrated_db().await;
        seed_indexed_book(&db).await;
        assert_eq!(
            count(&db, "SELECT count(*) AS c FROM fts_knowledge").await,
            1
        );

        db.conn()
            .execute_unprepared("DELETE FROM knowledge_scopes WHERE id = 1")
            .await
            .expect("delete scope");

        assert_eq!(
            count(&db, "SELECT count(*) AS c FROM knowledge_chunks").await,
            0
        );
        assert_eq!(
            count(&db, "SELECT count(*) AS c FROM fts_knowledge").await,
            0,
            "the scope cascade must also remove the FTS row"
        );
    }

    /// spec/02 §5 invariant 11 (PLAN 0.3 "the delete triggers remove FTS rows"): a direct
    /// `search_docs` delete removes its `fts_book` row.
    #[tokio::test]
    async fn deleting_a_search_doc_removes_its_fts_row() {
        let (_dir, db) = migrated_db().await;
        seed_indexed_book(&db).await;

        db.conn()
            .execute_unprepared("DELETE FROM search_docs WHERE id = 1")
            .await
            .expect("delete doc");

        assert_eq!(count(&db, "SELECT count(*) AS c FROM fts_book").await, 0);
    }

    /// spec/02 §5 invariant 11: deleting a chapter removes its summary/passage docs and their FTS
    /// rows (those reference the chapter by `ref_id`, not by foreign key).
    #[tokio::test]
    async fn deleting_a_chapter_removes_its_summary_and_passage_rows() {
        let (_dir, db) = migrated_db().await;
        seed_indexed_book(&db).await;

        db.conn()
            .execute_unprepared("DELETE FROM chapters WHERE id = 1")
            .await
            .expect("delete chapter");

        assert_eq!(count(&db, "SELECT count(*) AS c FROM search_docs").await, 0);
        assert_eq!(count(&db, "SELECT count(*) AS c FROM fts_book").await, 0);
    }

    /// spec/02 §5 invariant 11 the other way round: no `fts_*` row may outlive its source, for
    /// every source table that cascades into the index.
    #[tokio::test]
    async fn no_fts_row_outlives_its_source() {
        let (_dir, db) = migrated_db().await;
        seed_indexed_book(&db).await;

        db.conn()
            .execute_unprepared("DELETE FROM knowledge_entries WHERE id = 1")
            .await
            .expect("delete entry");

        let orphan_book = count(
            &db,
            "SELECT count(*) AS c FROM fts_book WHERE rowid NOT IN (SELECT id FROM search_docs)",
        )
        .await;
        let orphan_knowledge = count(
            &db,
            "SELECT count(*) AS c FROM fts_knowledge
             WHERE rowid NOT IN (SELECT id FROM knowledge_chunks)",
        )
        .await;
        assert_eq!(orphan_book, 0, "orphaned fts_book rows");
        assert_eq!(orphan_knowledge, 0, "orphaned fts_knowledge rows");
    }
}
