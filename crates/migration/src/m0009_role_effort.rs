use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, TransactionTrait};

/// Thinking effort per role (spec/02 §1–§2, spec/04 §5.1, D-162).
///
/// The SQL adds the two non-idempotent `model_profiles` columns, moves each profile's old effort
/// option into them, removes both old keys from `options`, and makes the old implicit Anthropic
/// `high` explicit in session snapshots. The module owns one transaction, so a failed guard rolls
/// the columns and every back-fill back together and leaves a retryable pre-0009 database.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0009_role_effort"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let tx = conn.begin().await?;
        match tx
            .execute_unprepared(include_str!("sql/0009_role_effort.sql"))
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
        // Dropping the levels would lose the author's configuration, and dropping the columns
        // would make the role-effort resolution fall back silently; migrations are append-only
        // after the first tag.
        Err(DbErr::Migration(
            "0009_role_effort has no down migration".to_owned(),
        ))
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(false)
    }
}
