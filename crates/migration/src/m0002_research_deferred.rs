use sea_orm_migration::prelude::*;

/// The `deferred` research status (spec/02 §1–§3, D-138).
///
/// The SQL runs **outside** the migrator's transaction: it needs `PRAGMA foreign_keys = OFF`
/// around a table rebuild, and SQLite ignores that pragma inside a transaction. See the file's own
/// comment for the full reasoning.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0002_research_deferred"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        crate::execute_sql_file(manager, include_str!("sql/0002_research_deferred.sql")).await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Rebuilding the table back would have to refuse every `deferred` row it meets or rewrite
        // them, and no released version can have written one (spec/02 §1: migrations are
        // append-only after the first tag).
        Err(DbErr::Migration(
            "0002_research_deferred has no down migration".to_owned(),
        ))
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(false)
    }
}
