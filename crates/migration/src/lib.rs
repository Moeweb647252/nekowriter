//! NekoWriter database migrations.
//!
//! Migrations execute embedded SQL files (`src/sql/NNNN_name.sql`). The SQL is the source of
//! truth for the schema (see `docs/spec/02-data-model.md`); the SeaORM entities in
//! `crates/server/src/db/entities/` are hand-written to match it.

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::ConnectionTrait;

mod m0001_init;
mod m0002_research_deferred;
mod m0003_relation_pair_unique;
mod m0004_outline_sessions;
mod m0005_writing_skills;
mod m0006_goals_and_switches;
mod m0007_builtin_basic_writing;
mod m0008_knowledge_first;
mod m0009_role_effort;
mod m0010_tool_error_limit;
mod m0011_check_topics;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m0001_init::Migration),
            Box::new(m0002_research_deferred::Migration),
            Box::new(m0003_relation_pair_unique::Migration),
            Box::new(m0004_outline_sessions::Migration),
            Box::new(m0005_writing_skills::Migration),
            Box::new(m0006_goals_and_switches::Migration),
            Box::new(m0007_builtin_basic_writing::Migration),
            Box::new(m0008_knowledge_first::Migration),
            Box::new(m0009_role_effort::Migration),
            Box::new(m0010_tool_error_limit::Migration),
            Box::new(m0011_check_topics::Migration),
        ]
    }
}

/// Runs one embedded SQL file with `execute_unprepared`, which SQLite accepts for multiple
/// statements in one call.
pub(crate) async fn execute_sql_file(manager: &SchemaManager<'_>, sql: &str) -> Result<(), DbErr> {
    let conn = manager.get_connection();
    conn.execute_unprepared(sql).await.map(|_| ())
}

/// Applies every pending migration to `conn`.
///
/// Callers (the server and its tests) use this instead of importing `MigratorTrait`, so the
/// `sea-orm-migration` dependency stays inside this crate.
pub async fn up(conn: &sea_orm::DatabaseConnection) -> Result<(), DbErr> {
    use sea_orm_migration::MigratorTrait;
    Migrator::up(conn, None).await
}

/// Applies the first `count` migrations. Tests use this to build a pre-0010 fixture without
/// dropping columns from the finished schema (which SQLite does not guarantee under a pooled
/// connection).
pub async fn up_to(conn: &sea_orm::DatabaseConnection, count: u32) -> Result<(), DbErr> {
    use sea_orm_migration::MigratorTrait;
    Migrator::up(conn, Some(count)).await
}
