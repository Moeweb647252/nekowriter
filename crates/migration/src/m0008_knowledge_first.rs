use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, TransactionTrait};

/// Knowledge-first visibility and research-task ownership (spec/02 §1–§5, D-158).
///
/// The SQL adds two non-idempotent `research_tasks` columns and replaces `book_scopes` with the
/// exclusion table. The module owns one transaction so a failed guard rolls the column additions
/// and the table swap back together; a repaired database can retry from the same state.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0008_knowledge_first"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let tx = conn.begin().await?;
        match tx
            .execute_unprepared(include_str!("sql/0008_knowledge_first.sql"))
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
        // Dropping the exclusions would make previously hidden scopes visible, and dropping the
        // research columns would lose ownership; migrations are append-only after the first tag.
        Err(DbErr::Migration(
            "0008_knowledge_first has no down migration".to_owned(),
        ))
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(false)
    }
}
