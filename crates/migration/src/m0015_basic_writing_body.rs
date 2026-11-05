use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, TransactionTrait};

/// Ships the built-in `basic-writing` body (D-148 §4.2; 0005 deferred it to a migration).
///
/// The UPDATE is not idempotent in intent, so the module owns one transaction: a failed guard
/// leaves a retryable pre-0015 database.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0015_basic_writing_body"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let tx = conn.begin().await?;
        match tx
            .execute_unprepared(include_str!("sql/0015_basic_writing_body.sql"))
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
            "0015_basic_writing_body has no down migration".to_owned(),
        ))
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(false)
    }
}
