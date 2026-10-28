use sea_orm_migration::prelude::*;

/// The `planner` role and `chapter_outline` session kind (spec/02 §1–§3, D-146).
///
/// The SQL runs **outside** the migrator's transaction: it needs `PRAGMA foreign_keys = OFF`
/// around a table rebuild, and SQLite ignores that pragma inside a transaction. See the file's own
/// comment for the ten foreign keys this protects.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0004_outline_sessions"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        crate::execute_sql_file(manager, include_str!("sql/0004_outline_sessions.sql")).await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Rebuilding the table back would have to refuse every `planner`/`chapter_outline` row it
        // meets or rewrite them; migrations are append-only after the first tag (spec/02 §1).
        Err(DbErr::Migration(
            "0004_outline_sessions has no down migration".to_owned(),
        ))
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(false)
    }
}
