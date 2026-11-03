use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, TransactionTrait};

/// Configurable tool-error circuit breaker (spec/03 §5.7, D-164).
///
/// The single `agent_sessions` column is not idempotent, so this module owns one transaction: a
/// failed run leaves a retryable pre-0010 database instead of a half-added column.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0010_tool_error_limit"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let tx = conn.begin().await?;
        match tx
            .execute_unprepared(include_str!("sql/0010_tool_error_limit.sql"))
            .await
        {
            Ok(_) => tx.commit().await,
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Dropping the column would change the behaviour of every stored session that chose a
        // non-default limit; migrations are append-only after the first tag.
        Err(DbErr::Migration(
            "0010_tool_error_limit has no down migration".to_owned(),
        ))
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(false)
    }
}
