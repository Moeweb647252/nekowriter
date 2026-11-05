use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, TransactionTrait};

/// Adds the step-pause flag to `agent_sessions` (spec/02 §1–§2; D-173).
///
/// The ALTER is not idempotent, so the module owns one transaction: a failed guard or ALTER leaves
/// a retryable pre-0014 database.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0014_step_pause"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let tx = conn.begin().await?;
        match tx
            .execute_unprepared(include_str!("sql/0014_step_pause.sql"))
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
        Err(DbErr::Migration(
            "0014_step_pause has no down migration".to_owned(),
        ))
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(false)
    }
}
