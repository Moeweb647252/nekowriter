use sea_orm_migration::prelude::*;

/// The `writing_skills` and `book_writing_skills` tables (design §4, D-148).
///
/// The SQL only creates two new tables and inserts one row; it never touches an existing table, so
/// it runs inside the migrator's transaction. The seed row is a literal: built-in bodies ship
/// empty and the reconciler is deliberately deferred (design §4.2).
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0005_writing_skills"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        crate::execute_sql_file(manager, include_str!("sql/0005_writing_skills.sql")).await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Dropping the tables would delete every user-written skill; migrations are append-only
        // after the first tag (spec/02 §1).
        Err(DbErr::Migration(
            "0005_writing_skills has no down migration".to_owned(),
        ))
    }
}
