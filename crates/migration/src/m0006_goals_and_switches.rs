use sea_orm_migration::prelude::*;

/// The writing-goals tables and the per-book canon system switches (spec/02 §1–§5, D-150).
///
/// The SQL runs **outside** the migrator's transaction: it rebuilds `change_log` and
/// `setting_proposals` with new CHECK lists, and SQLite ignores `PRAGMA foreign_keys = OFF` inside a
/// transaction. Neither table is referenced by another table, so the rebuild is simpler than
/// 0004's; the file itself lists the row-count guards and the final `foreign_key_check`.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0006_goals_and_switches"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        crate::execute_sql_file(manager, include_str!("sql/0006_goals_and_switches.sql")).await
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
