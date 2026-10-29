//! Entity-vs-schema guards (PLAN M14.2, spec/02 §1–§2).
//!
//! Check 1 — table completeness: every ordinary table in `sqlite_master` is described by exactly
//! one entity module, and every entity has its table.
//!
//! Check 2 — columns: for each entity the column names, nullability, type affinity and primary
//! key equal `PRAGMA table_info`.
//!
//! The checks run against the real schema: the migrations are applied to a fresh temporary
//! database, then both sides are compared. They read only `EntityName`, `ColumnTrait::def` and
//! `PrimaryKeyTrait`, so they hold both for the pre-M14 expanded entities and for the dense
//! `#[sea_orm::model]` entities that replace them.
//!
//! `decode_all_rows` is the other half of the guard: it decodes every row of a database through
//! every entity, so a column whose Rust type does not match its stored value fails loudly. It is
//! `#[ignore]`d because it needs `NW_DECODE_DB`; it always works on a copy under the temp directory,
//! never on the file it was given.

use std::collections::{BTreeMap, BTreeSet};

use sea_orm::entity::Iterable;
use sea_orm::entity::prelude::*;
use sea_orm::{ConnectionTrait, DbBackend, Statement};

/// Every entity module, one entry per table. Keep sorted by module name.
macro_rules! for_all_entities {
    ($macro:ident) => {
        $macro!(
            agent_inbox,
            agent_messages,
            agent_sessions,
            agent_tool_calls,
            app_settings,
            appearances,
            book_scopes,
            book_writing_skills,
            books,
            change_log,
            chapter_feedback,
            chapter_versions,
            chapters,
            character_designs,
            entities,
            foreshadowing,
            foreshadowing_entities,
            index_meta,
            knowledge_chunks,
            knowledge_entries,
            knowledge_scopes,
            memory_note_entities,
            memory_notes,
            model_profiles,
            prompt_overrides,
            providers,
            relations,
            research_tasks,
            review_reports,
            search_docs,
            setting_docs,
            setting_proposals,
            timeline_event_entities,
            timeline_events,
            tracks,
            writing_skills
        );
    };
}

#[derive(Debug)]
struct EntityColumns {
    module: &'static str,
    table: &'static str,
    columns: Vec<ColumnInfo>,
}

#[derive(Debug)]
struct ColumnInfo {
    name: &'static str,
    nullable: bool,
    affinity: &'static str,
    primary_key: bool,
}

/// The SQLite type affinity of a SeaORM column type (https://sqlite.org/datatype3.html §3.1).
fn affinity(ty: &ColumnType) -> &'static str {
    match ty {
        ColumnType::Integer
        | ColumnType::BigInteger
        | ColumnType::SmallInteger
        | ColumnType::TinyInteger
        | ColumnType::TinyUnsigned
        | ColumnType::SmallUnsigned
        | ColumnType::Unsigned
        | ColumnType::BigUnsigned
        | ColumnType::Boolean => "INTEGER",
        ColumnType::String(_)
        | ColumnType::Text
        | ColumnType::Char(_)
        | ColumnType::Timestamp
        | ColumnType::TimestampWithTimeZone
        | ColumnType::DateTime
        | ColumnType::Date
        | ColumnType::Time
        | ColumnType::Enum { .. }
        | ColumnType::Uuid => "TEXT",
        ColumnType::Float | ColumnType::Double | ColumnType::Decimal(_) | ColumnType::Money(_) => {
            "REAL"
        }
        ColumnType::Blob | ColumnType::Binary(_) | ColumnType::VarBinary(_) => "BLOB",
        other => panic!("unhandled entity column type: {other:?}"),
    }
}

macro_rules! entity_columns {
    ($($module:ident),* $(,)?) => {
        fn all_entity_columns() -> Vec<EntityColumns> {
            vec![$(
                {
                    use nekowriter::db::entities::$module::{
                        Column, Entity, PrimaryKey,
                    };
                    let primary_key: BTreeSet<&'static str> = PrimaryKey::iter()
                        .map(|key| key.into_column().as_str())
                        .collect();
                    EntityColumns {
                        module: stringify!($module),
                        table: Entity::default().table_name(),
                        columns: Column::iter()
                            .map(|column| {
                                let def = column.def();
                                ColumnInfo {
                                    name: column.as_str(),
                                    nullable: def.is_null(),
                                    affinity: affinity(def.get_column_type()),
                                    primary_key: primary_key.contains(column.as_str()),
                                }
                            })
                            .collect(),
                    }
                }
            ),*]
        }
    };
}

for_all_entities!(entity_columns);

#[derive(Debug)]
struct PragmaColumn {
    name: String,
    not_null: bool,
    ty: String,
    pk: i32,
}

/// The tables that are not entities: SQLite internals, the migration ledger, the FTS5 virtual
/// tables and their shadow tables, and the vec0 tables the index manager creates at runtime.
fn is_internal_table(name: &str) -> bool {
    name.starts_with("sqlite_")
        || name == "seaql_migrations"
        || name.starts_with("fts_")
        || name.starts_with("vec_")
}

async fn pragma_table_info(conn: &DatabaseConnection, table: &str) -> Vec<PragmaColumn> {
    let statement =
        Statement::from_string(DbBackend::Sqlite, format!("PRAGMA table_info(\"{table}\")"));
    conn.query_all_raw(statement)
        .await
        .unwrap_or_else(|err| panic!("PRAGMA table_info({table}) failed: {err}"))
        .iter()
        .map(|row| PragmaColumn {
            name: row.try_get("", "name").expect("name"),
            not_null: row.try_get::<i32>("", "notnull").expect("notnull") != 0,
            ty: row.try_get("", "type").expect("type"),
            pk: row.try_get("", "pk").expect("pk"),
        })
        .collect()
}

async fn migrated_db() -> (tempfile::TempDir, DatabaseConnection) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = nekowriter::db::Db::connect(&dir.path().join("schema.db"))
        .await
        .expect("connect");
    nekowriter_migration::up(db.conn()).await.expect("migrate");
    (dir, db.conn().clone())
}

/// Check 1: every table has exactly one entity and every entity has its table.
#[tokio::test]
async fn every_table_has_exactly_one_entity_and_every_entity_has_a_table() {
    let (_dir, conn) = migrated_db().await;
    let rows = conn
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
        ))
        .await
        .expect("sqlite_master");
    let actual: BTreeSet<String> = rows
        .iter()
        .map(|row| row.try_get::<String>("", "name").expect("name"))
        .filter(|name| !is_internal_table(name))
        .collect();

    let entities = all_entity_columns();
    let expected: BTreeSet<&'static str> = entities.iter().map(|entity| entity.table).collect();
    assert_eq!(
        entities.len(),
        expected.len(),
        "two entity modules declare the same table"
    );
    let actual: BTreeSet<&str> = actual.iter().map(String::as_str).collect();
    assert_eq!(
        actual, expected,
        "sqlite_master and the entity modules disagree about the tables"
    );
}

/// Check 2: column names, nullability, type affinity and primary key equal `PRAGMA table_info`.
#[tokio::test]
async fn every_entity_column_matches_pragma_table_info() {
    let (_dir, conn) = migrated_db().await;
    for entity in all_entity_columns() {
        let pragma = pragma_table_info(&conn, entity.table).await;
        let actual: BTreeMap<&str, &PragmaColumn> = pragma
            .iter()
            .map(|column| (column.name.as_str(), column))
            .collect();
        let expected: BTreeSet<&str> = entity.columns.iter().map(|column| column.name).collect();
        assert_eq!(
            expected,
            actual.keys().copied().collect(),
            "{} ({}) column names disagree with sqlite_master",
            entity.module,
            entity.table
        );

        let actual_primary_key: BTreeSet<&str> = pragma
            .iter()
            .filter(|column| column.pk > 0)
            .map(|column| column.name.as_str())
            .collect();
        let expected_primary_key: BTreeSet<&str> = entity
            .columns
            .iter()
            .filter(|column| column.primary_key)
            .map(|column| column.name)
            .collect();
        assert_eq!(
            expected_primary_key, actual_primary_key,
            "{} ({}) primary key disagrees with sqlite_master",
            entity.module, entity.table
        );

        for column in &entity.columns {
            let info = actual[column.name];
            assert_eq!(
                column.affinity,
                info.ty.to_uppercase(),
                "{} ({}) column `{}` has affinity {} but the SQL says {}",
                entity.module,
                entity.table,
                column.name,
                column.affinity,
                info.ty
            );
            if column.primary_key {
                // `INTEGER PRIMARY KEY` is a rowid alias and `PRAGMA table_info` reports
                // `notnull = 0` for it even though the Rust field is not an `Option`.
                assert!(
                    !column.nullable,
                    "{} ({}) primary key column `{}` must not be an Option",
                    entity.module, entity.table, column.name
                );
            } else {
                assert_eq!(
                    column.nullable, !info.not_null,
                    "{} ({}) column `{}` nullability disagrees with sqlite_master",
                    entity.module, entity.table, column.name
                );
            }
        }
    }
}

macro_rules! decode_entities {
    ($($module:ident),* $(,)?) => {
        async fn decode_all(conn: &DatabaseConnection) {
            $(
                {
                    use nekowriter::db::entities::$module::Entity;
                    let rows = Entity::find()
                        .all(conn)
                        .await
                        .unwrap_or_else(|err| {
                            panic!(
                                "entity `{}` failed to decode: {err}",
                                stringify!($module)
                            )
                        });
                    println!("{:>7}  {}", rows.len(), Entity::default().table_name());
                }
            )*
        }
    };
}

for_all_entities!(decode_entities);

/// Decodes every row of a database through every entity. Run manually:
///
/// ```sh
/// NW_DECODE_DB=/path/to/copy.db cargo test -p nekowriter --test entity_schema \
///     -- --ignored decode_all_rows --nocapture
/// ```
///
/// The given file is copied under the temp directory first, then the migrator runs on the copy
/// (a no-op on a current database); the source file is never opened for writing.
#[tokio::test]
#[ignore = "needs NW_DECODE_DB; run explicitly with --ignored"]
async fn decode_all_rows() {
    let source =
        std::env::var_os("NW_DECODE_DB").expect("NW_DECODE_DB must name a database file to decode");
    let source = std::path::PathBuf::from(source);
    assert!(
        source.is_file(),
        "NW_DECODE_DB is not a file: {}",
        source.display()
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let copy = dir.path().join("decode.db");
    std::fs::copy(&source, &copy).expect("copy database");
    for suffix in ["-wal", "-shm"] {
        let sidecar = std::path::PathBuf::from(format!("{}{suffix}", source.display()));
        if sidecar.is_file() {
            std::fs::copy(&sidecar, dir.path().join(format!("decode.db{suffix}")))
                .expect("copy database sidecar");
        }
    }

    let db = nekowriter::db::Db::connect(&copy).await.expect("connect");
    nekowriter_migration::up(db.conn())
        .await
        .expect("migrate the copy");
    println!("decoding {}", source.display());
    decode_all(db.conn()).await;
}
