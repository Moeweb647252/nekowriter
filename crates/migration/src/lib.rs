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

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m0001_init::Migration),
            Box::new(m0002_research_deferred::Migration),
            Box::new(m0003_relation_pair_unique::Migration),
            Box::new(m0004_outline_sessions::Migration),
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
