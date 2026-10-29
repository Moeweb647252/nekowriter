use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, TransactionTrait};

/// The writing-goals tables and the per-book canon system switches (spec/02 §1–§5, D-150).
///
/// The migration runs in a Rust-managed transaction rather than the migrator's. It rebuilds
/// `change_log` and `setting_proposals` with new CHECK lists; neither table is referenced by
/// another, so with `foreign_keys` left on the `DROP TABLE`s have nothing to cascade into. The
/// guards run inside the same transaction, so a broken copy rolls the rebuilt tables, the goal
/// tables and the non-idempotent `ALTER TABLE ... ADD COLUMN` back together; a repaired database
/// can then retry the migration cleanly.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0006_goals_and_switches"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let tx = conn.begin().await?;
        match tx
            .execute_unprepared(include_str!("sql/0006_goals_and_switches.sql"))
            .await
        {
            Ok(_) => tx.commit().await,
            Err(error) => {
                // The transaction owns one pooled connection, so the rollback also removes the
                // temp guard tables and leaves the pre-0006 database untouched.
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Dropping the tables would delete every goal and every frozen system set; migrations are
        // append-only after the first tag (spec/02 §1).
        Err(DbErr::Migration(
            "0006_goals_and_switches has no down migration".to_owned(),
        ))
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(false)
    }
}
