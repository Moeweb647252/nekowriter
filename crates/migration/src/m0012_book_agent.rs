use sea_orm_migration::prelude::*;

/// The book-agent role, session kind and write mode (spec/02 §1–§3, D-169).
///
/// SQLite ignores `PRAGMA foreign_keys = OFF` inside a transaction, so the module acquires one
/// pool connection, switches the pragma on it, and runs the `BEGIN`/`COMMIT` rebuild on that same
/// connection. A failed guard rolls the old `agent_sessions` table back and leaves the migration
/// retryable.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0012_book_agent"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let pool = match manager.get_connection() {
            SchemaManagerConnection::Connection(conn) => conn.get_sqlite_connection_pool(),
            _ => {
                return Err(DbErr::Migration(
                    "0012_book_agent needs a live SQLite connection".to_owned(),
                ));
            }
        };
        let mut conn = pool
            .acquire()
            .await
            .map_err(|error| DbErr::Migration(error.to_string()))?;
        match sea_orm::sqlx::raw_sql(include_str!("sql/0012_book_agent.sql"))
            .execute(&mut *conn)
            .await
        {
            Ok(_) => Ok(()),
            Err(error) => {
                // The SQL owns `BEGIN`/`COMMIT`; a failed guard leaves the transaction open.
                // Roll it back and restore the connection's foreign-key contract before
                // returning, so the database can be retried through a fresh connection.
                let _ = sea_orm::sqlx::raw_sql("ROLLBACK; PRAGMA foreign_keys = ON;")
                    .execute(&mut *conn)
                    .await;
                Err(DbErr::Migration(error.to_string()))
            }
        }
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Rebuilding back would have to refuse or rewrite every `assistant`/`book_agent` row;
        // migrations are append-only after the first tag (spec/02 §1).
        Err(DbErr::Migration(
            "0012_book_agent has no down migration".to_owned(),
        ))
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(false)
    }
}
