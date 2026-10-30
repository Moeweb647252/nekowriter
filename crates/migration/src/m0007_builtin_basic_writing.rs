use sea_orm_migration::prelude::*;

/// The first shipped built-in writing-skill body (D-156, design §5).
///
/// One row-keyed `UPDATE`; it runs inside the migrator's transaction because it only changes a
/// column value on a row that `0005_writing_skills` already inserted.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0007_builtin_basic_writing"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        crate::execute_sql_file(manager, include_str!("sql/0007_builtin_basic_writing.sql")).await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Built-in skill bodies are content the author may have selected; migrations are
        // append-only after the first tag (spec/02 §1).
        Err(DbErr::Migration(
            "0007_builtin_basic_writing has no down migration".to_owned(),
        ))
    }
}
