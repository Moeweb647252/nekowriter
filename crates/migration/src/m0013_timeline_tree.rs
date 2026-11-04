use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, TransactionTrait};

/// The timeline tree and foreshadowing anchors (spec/02 §1–§5; D-170).
///
/// The SQL adds the non-idempotent `timeline_events` and `foreshadowing` columns, creates one
/// contentless root per book and reparents every existing timeline row to it. The module owns one
/// transaction, so a failed guard rolls the columns, roots and reparenting back to a retryable
/// pre-0013 database.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0013_timeline_tree"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let tx = conn.begin().await?;
        match tx
            .execute_unprepared(include_str!("sql/0013_timeline_tree.sql"))
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
        // Dropping the tree columns would strand every planned entry and every anchor; migrations
        // are append-only after the first tag.
        Err(DbErr::Migration(
            "0013_timeline_tree has no down migration".to_owned(),
        ))
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(false)
    }
}
