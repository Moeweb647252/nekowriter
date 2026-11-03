use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, TransactionTrait};

/// Knowledge-check research topics (spec/02 §1–§2, §5; spec/10 §2.5; D-168).
///
/// The single `research_tasks.check_id` column is not idempotent, so this module owns one
/// transaction: a failed guard or ALTER leaves a retryable pre-0011 database.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0011_check_topics"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let tx = conn.begin().await?;
        match tx
            .execute_unprepared(include_str!("sql/0011_check_topics.sql"))
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
        // Dropping the column would sever the check-topic list and is not reversible without
        // guessing which check owned each topic; migrations are append-only after the first tag.
        Err(DbErr::Migration(
            "0011_check_topics has no down migration".to_owned(),
        ))
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(false)
    }
}
