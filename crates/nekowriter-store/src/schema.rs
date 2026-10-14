//! Schema creation and the schema identity check.
//!
//! The schema is defined only by the entities in `crate::entity`. A new database gets every
//! table at once; an existing database is opened only if it was created from exactly the
//! same definition. There are no migrations: a database from a different schema is refused
//! untouched (see `docs/product-spec.md` §16.1).
//!
//! The schema's identity is a fingerprint, the SHA-256 of every DDL statement plus
//! [`FORMAT_REVISION`]. Changing an entity changes the fingerprint automatically, so a
//! schema change can never be forgotten.

use nekowriter_core::time::Timestamp;
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, DatabaseConnection, DbBackend, DbErr, EntityTrait, Schema,
    Set, Statement, TransactionTrait,
};

use crate::{
    digest::sha256_hex,
    entity::{
        chapter, command_receipt, connection_test, document, embedding_settings, event, feedback,
        knowledge_entry, knowledge_space, model_request, project, project_prompt, prompt_template,
        review, revision, revision_dependency, run, run_message, schema_info, system_prompt,
        system_settings, tool_call, version, workflow,
    },
};

/// Bump this when the meaning of stored data changes without a DDL change, for example
/// the JSON shape of a text column. DDL changes need no bump.
const FORMAT_REVISION: u32 = 1;

/// Every statement that creates the schema of an empty database.
///
/// SQLite checks foreign keys only when rows change, so the order of tables does not matter.
fn create_statements() -> Vec<Statement> {
    let schema = Schema::new(DbBackend::Sqlite);
    let mut statements = Vec::new();
    add_entity(&mut statements, &schema, schema_info::Entity);
    add_entity(&mut statements, &schema, system_settings::Entity);
    add_entity(&mut statements, &schema, system_prompt::Entity);
    add_entity(&mut statements, &schema, embedding_settings::Entity);
    add_entity(&mut statements, &schema, project::Entity);
    add_entity(&mut statements, &schema, project_prompt::Entity);
    add_entity(&mut statements, &schema, prompt_template::Entity);
    add_entity(&mut statements, &schema, knowledge_space::Entity);
    add_entity(&mut statements, &schema, knowledge_entry::Entity);
    add_entity(&mut statements, &schema, version::Entity);
    add_entity(&mut statements, &schema, workflow::Entity);
    add_entity(&mut statements, &schema, document::Entity);
    add_entity(&mut statements, &schema, revision::Entity);
    add_entity(&mut statements, &schema, revision_dependency::Entity);
    add_entity(&mut statements, &schema, chapter::Entity);
    add_entity(&mut statements, &schema, run::Entity);
    add_entity(&mut statements, &schema, model_request::Entity);
    add_entity(&mut statements, &schema, connection_test::Entity);
    add_entity(&mut statements, &schema, run_message::Entity);
    add_entity(&mut statements, &schema, tool_call::Entity);
    add_entity(&mut statements, &schema, feedback::Entity);
    add_entity(&mut statements, &schema, review::Entity);
    add_entity(&mut statements, &schema, event::Entity);
    add_entity(&mut statements, &schema, command_receipt::Entity);
    statements
}

fn add_entity<E: EntityTrait>(statements: &mut Vec<Statement>, schema: &Schema, entity: E) {
    let backend = DbBackend::Sqlite;
    statements.push(backend.build(&schema.create_table_from_entity(entity)));
    for index in schema.create_index_from_entity(entity) {
        statements.push(backend.build(&index));
    }
}

/// The identity of the schema this build of the application reads and writes.
pub(crate) fn fingerprint() -> String {
    let header = format!("format revision {FORMAT_REVISION}\n");
    let statements: Vec<String> = create_statements()
        .into_iter()
        .map(|statement| format!("{};\n", statement.sql))
        .collect();
    sha256_hex(std::iter::once(header.as_bytes()).chain(statements.iter().map(String::as_bytes)))
}

/// What `prepare` found in the database file.
pub(crate) enum SchemaState {
    /// The database was empty and now holds the current schema.
    Created,
    /// The database already holds the current schema.
    Current,
    /// The database holds tables but no NekoWriter schema record.
    Foreign,
    /// The database was created from a different schema.
    Mismatch { created_by_version: String },
}

/// Creates the schema in an empty database, or checks that an existing database was
/// created from the current schema. Never modifies a database it refuses.
pub(crate) async fn prepare(
    db: &DatabaseConnection,
    app_version: &str,
) -> Result<SchemaState, DbErr> {
    let tables = table_names(db).await?;
    if tables.is_empty() {
        let txn = db.begin().await?;
        for statement in create_statements() {
            txn.execute_raw(statement).await?;
        }
        schema_info::ActiveModel {
            id: Set(schema_info::ROW_ID),
            fingerprint: Set(fingerprint()),
            created_by_version: Set(app_version.to_owned()),
            created_at: Set(Timestamp::now()),
            events_pruned_through: Set(0),
        }
        .insert(&txn)
        .await?;
        txn.commit().await?;
        return Ok(SchemaState::Created);
    }
    if !tables.iter().any(|table| table == "schema_info") {
        return Ok(SchemaState::Foreign);
    }
    let Some(info) = schema_info::Entity::find_by_id(schema_info::ROW_ID)
        .one(db)
        .await?
    else {
        return Ok(SchemaState::Foreign);
    };
    if info.fingerprint == fingerprint() {
        Ok(SchemaState::Current)
    } else {
        Ok(SchemaState::Mismatch {
            created_by_version: info.created_by_version,
        })
    }
}

async fn table_names<C: ConnectionTrait>(db: &C) -> Result<Vec<String>, DbErr> {
    let rows = db
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        ))
        .await?;
    rows.iter().map(|row| row.try_get_by_index(0)).collect()
}
