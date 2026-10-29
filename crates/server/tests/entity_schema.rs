//! Entity-vs-schema guards (PLAN M14.2, spec/02 §1–§2).
//!
//! Check 1 — table completeness: every ordinary table in `sqlite_master` is described by exactly
//! one entity module, and every entity has its table.
//!
//! Check 2 — columns: for each entity the column names, nullability, type affinity and primary
//! key equal `PRAGMA table_info`.
//!
//! Check 3 — relations: every `belongs_to` is a foreign key, every unambiguous foreign key has
//! its reverse `has_many`, and every junction has a `via` relation on both ends.
//!
//! Check 4 — unique keys: the `unique` / `unique_key` attributes equal the non-partial unique
//! indexes.
//!
//! The checks run against the real schema: the migrations are applied to a fresh temporary
//! database, then both sides are compared. They read only `EntityName`, `ColumnTrait::def`,
//! `PrimaryKeyTrait` and `RelationTrait`, so checks 1–2 also hold for the pre-M14 expanded
//! entities.
//!
//! `decode_all_rows` is the other half of the guard: it decodes every row of a database through
//! every entity, so a column whose Rust type does not match its stored value fails loudly. It is
//! `#[ignore]`d because it needs `NW_DECODE_DB`; it always works on a copy under the temp directory,
//! never on the file it was given.

use std::collections::{BTreeMap, BTreeSet};

use nekowriter::db::entities::writing_skills::WritingSkillAuthor;
use nekowriter::db::json::{
    AgentRole, AnchorKind, AppearanceRole, BookStatus, CanonMode, CanonStatus, ChangeActor,
    ChangeOp, ChangeTargetType, ChapterStatus, EmbedState, EntityKind, EntityStatus, EntrySource,
    FeedbackMode, ForeshadowingStatus, Granularity, InboxKind, MessageKind, MessageRole, NoteKind,
    NoteStatus, Phase, PromptScope, ProposalStatus, ProposalTargetType, Protocol, RelationStatus,
    ResearchTaskStatus, ReviewVerdict, SearchDocKind, SessionKind, SessionStatus, SettingDocKind,
    ToolCallStatus, TrackKind, VersionKind,
};
use sea_orm::entity::Iterable;
use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{TableName, TableRef};
use sea_orm::{ConnectionTrait, DbBackend, Identity, RelationType, Statement};

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
    /// The complete column definition the macro generated, including `unique` / `unique_key`.
    definition: ColumnDef,
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
                                    definition: def,
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

// ── Check 3: the relation graph ───────────────────────────────────────────────

/// One relation definition, flattened to the SQL names that both sides of the check speak.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RelationShape {
    is_owner: bool,
    has_many: bool,
    from_table: String,
    from_column: String,
    to_table: String,
    to_column: String,
}

fn table_name_of(table: &TableRef) -> String {
    match table {
        TableRef::Table(TableName(_, name), _) => name.to_string(),
        other => panic!("unexpected table reference in a relation: {other:?}"),
    }
}

fn columns_of(identity: &Identity) -> Vec<String> {
    identity.iter().map(|column| column.to_string()).collect()
}

fn flatten_relation(def: RelationDef) -> RelationShape {
    let from = columns_of(&def.from_col);
    let to = columns_of(&def.to_col);
    assert_eq!(
        from.len(),
        1,
        "the schema only has single-column foreign keys"
    );
    assert_eq!(
        to.len(),
        1,
        "the schema only has single-column foreign keys"
    );
    RelationShape {
        is_owner: def.is_owner,
        has_many: def.rel_type == RelationType::HasMany,
        from_table: table_name_of(&def.from_tbl),
        from_column: from[0].clone(),
        to_table: table_name_of(&def.to_tbl),
        to_column: to[0].clone(),
    }
}

macro_rules! entity_relations {
    ($($module:ident),* $(,)?) => {
        fn all_relations() -> Vec<(&'static str, &'static str, Vec<RelationShape>)> {
            vec![$(
                {
                    use nekowriter::db::entities::$module::{Entity, Relation};
                    (
                        stringify!($module),
                        Entity::default().table_name(),
                        Relation::iter()
                            .map(|relation| flatten_relation(relation.def()))
                            .collect(),
                    )
                }
            ),*]
        }
    };
}

for_all_entities!(entity_relations);

#[derive(Debug)]
struct PragmaForeignKey {
    from: String,
    to_table: String,
    to_column: String,
}

async fn pragma_foreign_keys(conn: &DatabaseConnection, table: &str) -> Vec<PragmaForeignKey> {
    let statement = Statement::from_string(
        DbBackend::Sqlite,
        format!("PRAGMA foreign_key_list(\"{table}\")"),
    );
    conn.query_all_raw(statement)
        .await
        .unwrap_or_else(|err| panic!("PRAGMA foreign_key_list({table}) failed: {err}"))
        .iter()
        .map(|row| PragmaForeignKey {
            from: row.try_get("", "from").expect("from"),
            to_table: row.try_get("", "table").expect("table"),
            to_column: row.try_get("", "to").expect("to"),
        })
        .collect()
}

/// The five junction tables. Their two foreign keys become `has_many, via` on both ends instead
/// of a `has_many` to the junction itself (plan §2.1).
const JUNCTIONS: [&str; 5] = [
    "book_scopes",
    "book_writing_skills",
    "foreshadowing_entities",
    "memory_note_entities",
    "timeline_event_entities",
];

/// Check 3a: every `belongs_to` relation is a foreign key of the schema, and every foreign key is
/// a `belongs_to` relation.
#[tokio::test]
async fn every_belongs_to_matches_the_foreign_key_list() {
    let (_dir, conn) = migrated_db().await;
    for (module, table, relations) in all_relations() {
        let actual: BTreeSet<(String, String, String, String)> = relations
            .iter()
            .filter(|relation| !relation.has_many)
            .map(|relation| {
                (
                    relation.from_table.clone(),
                    relation.from_column.clone(),
                    relation.to_table.clone(),
                    relation.to_column.clone(),
                )
            })
            .collect();
        let expected: BTreeSet<(String, String, String, String)> =
            pragma_foreign_keys(&conn, table)
                .await
                .iter()
                .map(|foreign_key| {
                    (
                        table.to_string(),
                        foreign_key.from.clone(),
                        foreign_key.to_table.clone(),
                        foreign_key.to_column.clone(),
                    )
                })
                .collect();
        assert_eq!(
            actual, expected,
            "{module} ({table}) belongs_to relations disagree with PRAGMA foreign_key_list"
        );
    }
}

/// Check 3b: every unambiguous foreign key has its reverse `has_many` on the parent. Junction
/// foreign keys are covered by check 3c instead; a pair of foreign keys from one child to one
/// parent is ambiguous and has no reverse (the macro cannot tell which one it would mean).
#[tokio::test]
async fn every_unambiguous_foreign_key_has_a_reverse_has_many() {
    let (_dir, _conn) = migrated_db().await;
    let relations = all_relations();
    let by_table: BTreeMap<&str, &Vec<RelationShape>> = relations
        .iter()
        .map(|(_, table, defs)| (*table, defs))
        .collect();
    let mut foreign_key_count: BTreeMap<(String, String), usize> = BTreeMap::new();
    for (_, table, defs) in &relations {
        for def in defs.iter().filter(|def| !def.has_many) {
            *foreign_key_count
                .entry(((*table).to_string(), def.to_table.clone()))
                .or_default() += 1;
        }
    }

    let mut checked = 0;
    for (module, table, defs) in &relations {
        if JUNCTIONS.contains(table) {
            continue;
        }
        for def in defs.iter().filter(|def| !def.has_many) {
            let key = ((*table).to_string(), def.to_table.clone());
            if foreign_key_count[&key] > 1 {
                continue;
            }
            let parent = by_table[def.to_table.as_str()];
            let found = parent.iter().any(|candidate| {
                candidate.has_many
                    && candidate.from_table == def.to_table
                    && candidate.to_table == *table
                    && candidate.from_column == def.to_column
                    && candidate.to_column == def.from_column
            });
            assert!(
                found,
                "{module} ({table}): no reverse has_many for {} -> {} ({}.{}); parent defs: {parent:?}",
                table, def.to_table, def.from_column, def.to_column
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked, 60,
        "the schema has 60 unambiguous foreign keys with a reverse relation"
    );
}

fn assert_many_to_many<A, B>()
where
    A: EntityTrait + Related<B>,
    B: EntityTrait,
{
    let to = <A as Related<B>>::to();
    let via = <A as Related<B>>::via().unwrap_or_else(|| {
        panic!(
            "{} -> {} has no via relation",
            A::default().table_name(),
            B::default().table_name()
        )
    });
    assert_eq!(
        table_name_of(&via.from_tbl),
        A::default().table_name(),
        "the via relation does not start at the parent"
    );
    assert_eq!(
        table_name_of(&via.to_tbl),
        table_name_of(&to.from_tbl),
        "the via relation does not end at the junction"
    );
    assert_eq!(
        table_name_of(&to.to_tbl),
        B::default().table_name(),
        "the to relation does not end at the far entity"
    );
}

/// Check 3c: each junction table has a `via` relation on both ends.
#[tokio::test]
async fn every_junction_has_via_relations_on_both_ends() {
    use nekowriter::db::entities::{
        books, entities, foreshadowing, knowledge_scopes, memory_notes, timeline_events,
        writing_skills,
    };

    assert_many_to_many::<books::Entity, knowledge_scopes::Entity>();
    assert_many_to_many::<knowledge_scopes::Entity, books::Entity>();
    assert_many_to_many::<books::Entity, writing_skills::Entity>();
    assert_many_to_many::<writing_skills::Entity, books::Entity>();
    assert_many_to_many::<foreshadowing::Entity, entities::Entity>();
    assert_many_to_many::<entities::Entity, foreshadowing::Entity>();
    assert_many_to_many::<memory_notes::Entity, entities::Entity>();
    assert_many_to_many::<entities::Entity, memory_notes::Entity>();
    assert_many_to_many::<timeline_events::Entity, entities::Entity>();
    assert_many_to_many::<entities::Entity, timeline_events::Entity>();
}

// ── Check 4: unique keys ──────────────────────────────────────────────────────

/// The non-partial unique constraints that are not primary keys (plan §1.6), as
/// `module => [(key, columns)]`. A `""` key is a single-column `unique` attribute.
macro_rules! unique_groups {
    ($($module:ident => [$(($key:literal, [$($column:ident),*])),* $(,)?]),* $(,)?) => {
        fn all_unique_groups() -> Vec<(&'static str, &'static str, Vec<(&'static str, Vec<&'static str>)>)> {
            vec![$(
                (
                    stringify!($module),
                    nekowriter::db::entities::$module::Entity::default().table_name(),
                    vec![$(
                        (
                            $key,
                            vec![$(nekowriter::db::entities::$module::Column::$column.as_str()),*],
                        )
                    ),*],
                )
            ),*]
        }
    };
}

unique_groups! {
    agent_messages => [("session_seq", [SessionId, Seq])],
    agent_tool_calls => [("message_call", [MessageId, CallIndex])],
    appearances => [("chapter_entity", [ChapterId, EntityId])],
    chapter_versions => [("chapter_version", [ChapterId, VersionNo])],
    chapters => [("track_ord", [TrackId, Ord])],
    character_designs => [("book_name", [BookId, Name])],
    knowledge_chunks => [("entry_ord", [EntryId, Ord])],
    knowledge_entries => [("scope_title", [ScopeId, Title])],
    knowledge_scopes => [("", [Name])],
    model_profiles => [("provider_display_name", [ProviderId, DisplayName])],
    providers => [("", [Name])],
    search_docs => [("doc_ref", [Kind, RefId, RefPart])],
    setting_docs => [("book_kind", [BookId, Kind])],
    writing_skills => [("", [Key])],
}

fn expected_column_definition(column: &ColumnInfo, key: Option<&str>) -> ColumnDef {
    let mut definition = column.definition.get_column_type().clone().def();
    if column.nullable {
        definition = definition.nullable();
    }
    match key {
        None => definition,
        Some("") => definition.unique(),
        Some(key) => definition.unique_key(key),
    }
}

async fn pragma_unique_columns(conn: &DatabaseConnection, table: &str) -> BTreeSet<Vec<String>> {
    let indexes = conn
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            format!("PRAGMA index_list(\"{table}\")"),
        ))
        .await
        .unwrap_or_else(|err| panic!("PRAGMA index_list({table}) failed: {err}"));
    let mut unique = BTreeSet::new();
    for index in &indexes {
        let is_unique: i32 = index.try_get("", "unique").expect("unique");
        let is_partial: i32 = index.try_get("", "partial").expect("partial");
        let origin: String = index.try_get("", "origin").expect("origin");
        if is_unique == 0 || is_partial != 0 || origin == "pk" {
            continue;
        }
        let name: String = index.try_get("", "name").expect("name");
        let columns = conn
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("PRAGMA index_info(\"{name}\")"),
            ))
            .await
            .unwrap_or_else(|err| panic!("PRAGMA index_info({name}) failed: {err}"));
        let mut names: Vec<String> = columns
            .iter()
            .map(|row| row.try_get("", "name").expect("name"))
            .collect();
        names.sort();
        unique.insert(names);
    }
    unique
}

/// Check 4: the `unique` / `unique_key` attributes equal the non-partial unique indexes, in both
/// directions, and no other column carries a hidden uniqueness attribute.
#[tokio::test]
async fn every_unique_key_matches_the_index_list() {
    let (_dir, conn) = migrated_db().await;
    let groups = all_unique_groups();
    let groups_by_table: BTreeMap<&str, &Vec<(&str, Vec<&str>)>> = groups
        .iter()
        .map(|(_, table, groups)| (*table, groups))
        .collect();
    let empty = Vec::new();
    for entity in all_entity_columns() {
        let module = entity.module;
        let table = entity.table;
        let columns = &entity.columns;
        let entity_groups = groups_by_table.get(table).copied().unwrap_or(&empty);
        let mut expected: BTreeSet<Vec<String>> = BTreeSet::new();
        let mut grouped: BTreeSet<&str> = BTreeSet::new();
        for (key, group) in entity_groups {
            let mut names: Vec<String> = group.iter().map(|name| (*name).to_string()).collect();
            names.sort();
            expected.insert(names);
            grouped.extend(group.iter().copied());
            for column in columns.iter().filter(|column| group.contains(&column.name)) {
                assert_eq!(
                    column.definition,
                    expected_column_definition(column, Some(key)),
                    "{} ({}) column `{}` does not declare unique key `{key}`",
                    module,
                    table,
                    column.name
                );
            }
        }
        for column in columns {
            if !grouped.contains(column.name) {
                assert_eq!(
                    column.definition,
                    expected_column_definition(column, None),
                    "{} ({}) column `{}` carries an unexpected unique attribute",
                    module,
                    table,
                    column.name
                );
            }
        }
        let actual = pragma_unique_columns(&conn, table).await;
        assert_eq!(
            actual, expected,
            "{module} ({table}) unique keys disagree with PRAGMA index_list"
        );
    }
}

// ── Smoke: the relation graph is usable, not just declared ───────────────────

const SMOKE_SQL: &str = r#"
INSERT INTO books (id, title, writing_language, status, settings, created_at, updated_at)
VALUES (1, 'Smoke', 'en', 'active', '{}', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
INSERT INTO tracks (id, book_id, kind, title, canon_mode, anchor_kind, ord, created_at, updated_at)
VALUES (1, 1, 'main', 'Main', 'canon', 'none', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
INSERT INTO chapters (id, book_id, track_id, ord, title, status, guidance, outline,
    reference_notes, summary, content, content_version, word_count, canon_status, created_at, updated_at)
VALUES (1, 1, 1, 1, 'Chapter 1', 'drafting', '', '', '', '',
    '{"paragraphs":[],"next_pid":1,"tail_open":false}', 0, 0, 'none',
    '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
INSERT INTO knowledge_scopes (id, name, description, created_at, updated_at)
VALUES (1, 'Scope', '', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
INSERT INTO book_scopes (book_id, scope_id) VALUES (1, 1);
INSERT INTO entities (id, book_id, kind, name, aliases, summary, description, state, attributes,
    importance, status, is_protagonist, created_at, updated_at)
VALUES (1, 1, 'character', 'Hero', '[]', '', '', '', '{}', 1, 'active', 0,
    '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
INSERT INTO timeline_events (id, book_id, position, story_time, title, description, importance, created_at, updated_at)
VALUES (1, 1, 1.0, '', 'Event', '', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
INSERT INTO timeline_event_entities (event_id, entity_id) VALUES (1, 1);
INSERT INTO agent_sessions (id, book_id, role, kind, model_snapshot, system_prompt, tools, status,
    phase, counters, usage, created_at, updated_at)
VALUES
    (1, NULL, 'writer', 'chapter_write', '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'idle', 'none', '{}', '{}',
        '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
    (2, 1, 'writer', 'chapter_write', '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'idle', 'none', '{}', '{}',
        '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
    (3, 1, 'reviewer', 'chapter_review', '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'idle', 'none', '{}', '{}',
        '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
UPDATE agent_sessions SET parent_session_id = 2 WHERE id = 3;
"#;

async fn smoke_db() -> (tempfile::TempDir, nekowriter::db::Db) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = nekowriter::db::Db::connect(&dir.path().join("smoke.db"))
        .await
        .expect("connect");
    nekowriter_migration::up(db.conn()).await.expect("migrate");
    db.write(|tx| {
        Box::pin(async move {
            tx.execute_unprepared(SMOKE_SQL).await?;
            Ok(())
        })
    })
    .await
    .expect("seed");
    (dir, db)
}

/// `find_related` on a required foreign key (`chapters.book`) and a nullable one
/// (`agent_sessions.book`).
#[tokio::test]
async fn smoke_find_related_on_required_and_nullable_foreign_keys() {
    use nekowriter::db::entities::{agent_sessions, books, chapters};

    let (_dir, db) = smoke_db().await;
    let chapter = chapters::Entity::find_by_id(1)
        .one(db.conn())
        .await
        .expect("chapter")
        .expect("chapter row");
    let book = chapter
        .find_related(books::Entity)
        .one(db.conn())
        .await
        .expect("book")
        .expect("required relation");
    assert_eq!(book.title, "Smoke");

    let orphan = agent_sessions::Entity::find_by_id(1)
        .one(db.conn())
        .await
        .expect("session")
        .expect("session row");
    assert!(
        orphan
            .find_related(books::Entity)
            .one(db.conn())
            .await
            .expect("book")
            .is_none(),
        "a NULL foreign key has no related row"
    );

    let linked = agent_sessions::Entity::find_by_id(2)
        .one(db.conn())
        .await
        .expect("session")
        .expect("session row");
    let book = linked
        .find_related(books::Entity)
        .one(db.conn())
        .await
        .expect("book")
        .expect("nullable relation with a value");
    assert_eq!(book.id, 1);
}

/// The `parent_session` self reference. A `self_ref` `BelongsTo` does not generate a `Related`
/// impl in SeaORM 2.0.4, so `find_related` is not available; the relation is checked at the
/// `RelationDef` level and the stored parent link is read back.
#[tokio::test]
async fn smoke_parent_session_self_reference() {
    use nekowriter::db::entities::agent_sessions;

    let (_dir, db) = smoke_db().await;
    let child = agent_sessions::Entity::find_by_id(3)
        .one(db.conn())
        .await
        .expect("session")
        .expect("session row");
    assert_eq!(child.parent_session_id, Some(2));

    let def = agent_sessions::Relation::ParentSession.def();
    assert_eq!(table_name_of(&def.from_tbl), "agent_sessions");
    assert_eq!(table_name_of(&def.to_tbl), "agent_sessions");
    assert_eq!(columns_of(&def.from_col), ["parent_session_id"]);
    assert_eq!(columns_of(&def.to_col), ["id"]);
}

/// The Entity Loader over a `has_many` and an M-N relation.
#[tokio::test]
async fn smoke_entity_loader_with_tracks_and_knowledge_scopes() {
    use nekowriter::db::entities::{books, knowledge_scopes, tracks};

    let (_dir, db) = smoke_db().await;
    let loaded = books::Entity::load()
        .with(tracks::Entity)
        .with(knowledge_scopes::Entity)
        .all(db.conn())
        .await
        .expect("entity loader");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].tracks.len(), 1);
    assert_eq!(loaded[0].tracks[0].title, "Main");
    assert_eq!(loaded[0].knowledge_scopes.len(), 1);
    assert_eq!(loaded[0].knowledge_scopes[0].name, "Scope");
}

/// `entities` → `timeline_events` through the `timeline_event_entities` junction.
#[tokio::test]
async fn smoke_entities_to_timeline_events_via_junction() {
    use nekowriter::db::entities::{entities, timeline_events};

    let (_dir, db) = smoke_db().await;
    let entity = entities::Entity::find_by_id(1)
        .one(db.conn())
        .await
        .expect("entity")
        .expect("entity row");
    let events = entity
        .find_related(timeline_events::Entity)
        .all(db.conn())
        .await
        .expect("timeline events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].title, "Event");
}

/// The generated `find_by_track_ord` finder.
#[tokio::test]
async fn smoke_find_by_track_ord() {
    use nekowriter::db::entities::chapters;

    let (_dir, db) = smoke_db().await;
    let found = chapters::Entity::find_by_track_ord((1, 1))
        .one(db.conn())
        .await
        .expect("find_by_track_ord");
    assert_eq!(found.map(|chapter| chapter.id), Some(1));
    let missing = chapters::Entity::find_by_track_ord((1, 2))
        .one(db.conn())
        .await
        .expect("find_by_track_ord");
    assert!(missing.is_none());
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

// ── Check 5: enum and boolean columns ────────────────────────────────────────

/// Every CHECK-constrained `TEXT` column and its Rust enum, plus the matching `Column` variant.
macro_rules! enum_columns {
    ($(($module:ident, $field:ident, $ty:ty, $column:ident)),* $(,)?) => {
        fn enum_column_values() -> Vec<(&'static str, &'static str, Vec<String>)> {
            #[allow(unused_mut)]
            let mut rows = vec![$(
                (
                    nekowriter::db::entities::$module::Entity::default().table_name(),
                    nekowriter::db::entities::$module::Column::$column.as_str(),
                    <$ty as ActiveEnum>::values(),
                )
            ),*];
            rows.push((
                nekowriter::db::entities::agent_sessions::Entity::default().table_name(),
                nekowriter::db::entities::agent_sessions::Column::Granularity.as_str(),
                <Granularity as ActiveEnum>::values(),
            ));
            rows.push((
                nekowriter::db::entities::setting_proposals::Entity::default().table_name(),
                nekowriter::db::entities::setting_proposals::Column::TargetKind.as_str(),
                <SettingDocKind as ActiveEnum>::values(),
            ));
            rows
        }

        /// Compile-time proof that the entity field has the enum type.
        #[test]
        fn enum_fields_are_typed() {
            $(
                let _: fn(&nekowriter::db::entities::$module::Model) -> &$ty =
                    |model| &model.$field;
            )*
        }
    };
}

enum_columns! {
    (agent_inbox, kind, InboxKind, Kind),
    (agent_messages, role, MessageRole, Role),
    (agent_messages, kind, MessageKind, Kind),
    (agent_sessions, role, AgentRole, Role),
    (agent_sessions, kind, SessionKind, Kind),
    (agent_sessions, status, SessionStatus, Status),
    (agent_sessions, phase, Phase, Phase),
    (agent_tool_calls, status, ToolCallStatus, Status),
    (appearances, role, AppearanceRole, Role),
    (books, status, BookStatus, Status),
    (change_log, actor, ChangeActor, Actor),
    (change_log, target_type, ChangeTargetType, TargetType),
    (change_log, op, ChangeOp, Op),
    (chapter_feedback, mode, FeedbackMode, Mode),
    (chapter_versions, kind, VersionKind, Kind),
    (chapters, status, ChapterStatus, Status),
    (chapters, canon_status, CanonStatus, CanonStatus),
    (entities, kind, EntityKind, Kind),
    (entities, status, EntityStatus, Status),
    (foreshadowing, status, ForeshadowingStatus, Status),
    (knowledge_chunks, embed_state, EmbedState, EmbedState),
    (knowledge_entries, source, EntrySource, Source),
    (memory_notes, kind, NoteKind, Kind),
    (memory_notes, status, NoteStatus, Status),
    (prompt_overrides, scope, PromptScope, Scope),
    (providers, protocol, Protocol, Protocol),
    (relations, status, RelationStatus, Status),
    (research_tasks, status, ResearchTaskStatus, Status),
    (review_reports, verdict, ReviewVerdict, Verdict),
    (search_docs, kind, SearchDocKind, Kind),
    (search_docs, embed_state, EmbedState, EmbedState),
    (setting_docs, kind, SettingDocKind, Kind),
    (setting_proposals, target_type, ProposalTargetType, TargetType),
    (setting_proposals, status, ProposalStatus, Status),
    (tracks, kind, TrackKind, Kind),
    (tracks, canon_mode, CanonMode, CanonMode),
    (tracks, anchor_kind, AnchorKind, AnchorKind),
    (writing_skills, author, WritingSkillAuthor, Author),
}

/// The eight `INTEGER` 0/1 flags must be `bool` fields.
#[test]
fn boolean_fields_are_typed() {
    let _: fn(&nekowriter::db::entities::agent_sessions::Model) -> &bool =
        |model| &model.auto_pause;
    let _: fn(&nekowriter::db::entities::agent_sessions::Model) -> &bool =
        |model| &model.pause_after_outline;
    let _: fn(&nekowriter::db::entities::agent_sessions::Model) -> &bool =
        |model| &model.pause_requested;
    let _: fn(&nekowriter::db::entities::agent_tool_calls::Model) -> &bool =
        |model| &model.is_error;
    let _: fn(&nekowriter::db::entities::agent_tool_calls::Model) -> &bool = |model| &model.sealed;
    let _: fn(&nekowriter::db::entities::entities::Model) -> &bool = |model| &model.is_protagonist;
    let _: fn(&nekowriter::db::entities::writing_skills::Model) -> &bool =
        |model| &model.enabled_by_default;
    let _: fn(&nekowriter::db::entities::writing_skills::Model) -> &bool = |model| &model.retired;
}

/// The two nullable enum columns have the exact `Option<Enum>` type.
#[test]
fn nullable_enum_fields_are_typed() {
    let _: fn(&nekowriter::db::entities::agent_sessions::Model) -> &Option<Granularity> =
        |model| &model.granularity;
    let _: fn(&nekowriter::db::entities::setting_proposals::Model) -> &Option<SettingDocKind> =
        |model| &model.target_kind;
}

/// Parse the `CHECK (col IN ('a', 'b', ...))` lists from a table's `sqlite_master` SQL.
fn check_in_lists(sql: &str) -> Vec<(String, Vec<String>)> {
    let mut checks = Vec::new();
    let mut index = 0;
    while let Some(found) = sql[index..].find("CHECK") {
        let start = index + found + "CHECK".len();
        let rest = &sql[start..];
        let Some(open) = rest.find('(') else {
            index = start;
            continue;
        };
        let after = &rest[open + 1..];
        let Some(in_pos) = after.find(" IN (") else {
            index = start + open + 1;
            continue;
        };
        let column = after[..in_pos].split_whitespace().last().unwrap_or("");
        if !column
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        {
            index = start + open + 1;
            continue;
        }
        let values_start = start + open + 1 + in_pos + " IN (".len();
        let Some(close) = sql[values_start..].find(')') else {
            index = values_start;
            continue;
        };
        let values = &sql[values_start..values_start + close];
        let values = values
            .split(',')
            .filter_map(|value| {
                let value = value.trim();
                value
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
                    .map(str::to_owned)
            })
            .collect();
        checks.push((column.to_owned(), values));
        index = values_start + close + 1;
    }
    checks
}

/// Check 5: every enum column's `ActiveEnum::values()` equals its SQL CHECK list, in both
/// directions, and every `CHECK (col IN (...))` in the schema has an enum row.
#[tokio::test]
async fn every_enum_column_matches_its_check_constraint() {
    let (_dir, conn) = migrated_db().await;
    let enum_rows = enum_column_values();
    let expected: BTreeMap<(&str, &str), Vec<String>> = enum_rows
        .iter()
        .map(|(table, column, values)| ((*table, *column), values.clone()))
        .collect();
    assert_eq!(
        expected.len(),
        40,
        "the schema has 40 CHECK (col IN (...)) enum columns"
    );

    let mut seen = BTreeSet::new();
    for entity in all_entity_columns() {
        let module = entity.module;
        let table = entity.table;
        let sql: String = conn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?",
                [table.to_owned().into()],
            ))
            .await
            .unwrap_or_else(|err| panic!("sqlite_master query for {table} failed: {err}"))
            .unwrap_or_else(|| panic!("{table} has no sqlite_master row"))
            .try_get("", "sql")
            .expect("table sql");
        for (column, check_values) in check_in_lists(&sql) {
            let Some(values) = expected.get(&(table, column.as_str())) else {
                panic!("{module} ({table}) column `{column}` has a CHECK list but no enum row");
            };
            let actual: BTreeSet<&str> = values.iter().map(String::as_str).collect();
            let wanted: BTreeSet<&str> = check_values.iter().map(String::as_str).collect();
            assert_eq!(
                actual, wanted,
                "{module} ({table}) column `{column}`: ActiveEnum::values() disagrees with the SQL CHECK list"
            );
            seen.insert((table, column));
        }
    }
    for (table, column) in expected.keys() {
        assert!(
            seen.contains(&(*table, (*column).to_owned())),
            "{table}.{column} has an enum row but no CHECK (col IN (...)) in the schema"
        );
    }
}

/// `agent_sessions.granularity` is nullable and therefore not in the CHECK-only map.
#[test]
fn the_nullable_enum_has_values() {
    let values: BTreeSet<String> = <Granularity as ActiveEnum>::values().into_iter().collect();
    assert_eq!(
        values,
        BTreeSet::from([
            "sentence".to_owned(),
            "paragraph".to_owned(),
            "free".to_owned(),
            "chapter".to_owned()
        ])
    );
}

/// The plan §2.9 grep gate: a filter on an enum column must use the enum value, not a string
/// literal. Genuine `TEXT` columns (`name`, `note`, …) are allowed and are not in this list.
#[test]
fn enum_filter_literals_are_typed() {
    const ENUM_COLUMNS: [&str; 19] = [
        "Actor",
        "AnchorKind",
        "Author",
        "CanonMode",
        "CanonStatus",
        "EmbedState",
        "Granularity",
        "Kind",
        "Mode",
        "Op",
        "Phase",
        "Protocol",
        "Role",
        "Scope",
        "Source",
        "Status",
        "TargetKind",
        "TargetType",
        "Verdict",
    ];
    let source = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let pattern = r#"(Column::[A-Za-z]+|COLUMN\.[a-z_]+)\.(eq|ne|is_in|is_not_in)\(\s*\[?\s*""#;
    let output = std::process::Command::new("grep")
        .args(["-rnE", pattern, source])
        .output()
        .expect("run grep");
    assert!(
        matches!(output.status.code(), Some(0) | Some(1)),
        "grep failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some(column) = enum_column_name(line) else {
            continue;
        };
        assert!(
            !ENUM_COLUMNS.contains(&column.as_str()),
            "enum column `{column}` is filtered with a string literal: {line}"
        );
    }
}

/// The column of a gate line, for both the generated `Column::X` and literal `COLUMN.x` spellings.
fn enum_column_name(line: &str) -> Option<String> {
    let after = line
        .split_once("Column::")
        .map(|(_, rest)| rest)
        .or_else(|| line.split_once("COLUMN.").map(|(_, rest)| rest))?;
    let raw = after
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .next()?;
    if raw.is_empty() {
        return None;
    }
    let mut name = String::new();
    for part in raw.split('_') {
        let mut characters = part.chars();
        if let Some(first) = characters.next() {
            name.push(first.to_ascii_uppercase());
            name.extend(characters);
        }
    }
    Some(name)
}

#[test]
fn enum_gate_extraction_handles_column_spellings() {
    assert_eq!(
        enum_column_name(r#"    .filter(chapters::Column::TargetKind.eq("outline"))"#),
        Some("TargetKind".to_owned())
    );
    assert_eq!(
        enum_column_name(r#"    .filter(chapters::COLUMN.target_type.eq("setting_doc"))"#),
        Some("TargetType".to_owned())
    );
}

/// Plan §2.4: every `json_valid` column is a typed `JsonText<T>` field, so a decode failure is a
/// query error rather than a silent default. Only `app_settings.value` stays `String` because it
/// holds one JSON document per key, decoded with `parse_json` after the key is known.
#[test]
fn json_columns_are_typed() {
    use nekowriter::db::entities::{
        agent_messages, agent_sessions, agent_tool_calls, app_settings, books, change_log,
        chapter_feedback, chapter_versions, chapters, entities, knowledge_entries, model_profiles,
        providers, review_reports, search_docs, setting_proposals,
    };
    use nekowriter::db::json::{
        Appendix, BookSettings, ChapterContent, ConceptionState, JsonText, ModelOptions,
        ModelSnapshot, ProposalEdits, ProviderPayload, RequestMeta, ReviewIssue, RoleModels,
        SessionCounters, SessionError, Snapshot, StoredBlock, ToolEffects, Usage, ViewSnapshot,
    };

    /// Compile-time type assertion: the field has exactly this type.
    macro_rules! typed {
        ($entity:ty, $field:ident, $ty:ty) => {{
            let _: fn(&$entity) -> &$ty = |row| &row.$field;
        }};
    }

    typed!(agent_messages::Model, content, JsonText<Vec<StoredBlock>>);
    typed!(
        agent_messages::Model,
        provider_payload,
        Option<JsonText<ProviderPayload>>
    );
    typed!(
        agent_messages::Model,
        request_meta,
        Option<JsonText<RequestMeta>>
    );
    typed!(agent_messages::Model, usage, Option<JsonText<Usage>>);
    typed!(
        agent_sessions::Model,
        model_snapshot,
        JsonText<ModelSnapshot>
    );
    typed!(
        agent_sessions::Model,
        tools,
        JsonText<Vec<nekowriter_llm::ToolDef>>
    );
    typed!(
        agent_sessions::Model,
        view_snapshot,
        Option<JsonText<ViewSnapshot>>
    );
    typed!(agent_sessions::Model, counters, JsonText<SessionCounters>);
    typed!(agent_sessions::Model, usage, JsonText<Usage>);
    typed!(agent_sessions::Model, error, Option<JsonText<SessionError>>);
    typed!(agent_tool_calls::Model, appendices, JsonText<Vec<Appendix>>);
    typed!(
        agent_tool_calls::Model,
        ui,
        Option<JsonText<serde_json::Value>>
    );
    typed!(
        agent_tool_calls::Model,
        effects,
        Option<JsonText<ToolEffects>>
    );
    typed!(app_settings::Model, value, String);
    typed!(books::Model, settings, JsonText<BookSettings>);
    typed!(books::Model, model_overrides, JsonText<RoleModels>);
    typed!(books::Model, conception, JsonText<ConceptionState>);
    typed!(change_log::Model, before, Option<JsonText<Snapshot>>);
    typed!(change_log::Model, after, Option<JsonText<Snapshot>>);
    typed!(chapter_feedback::Model, issues, JsonText<Vec<ReviewIssue>>);
    typed!(chapter_versions::Model, content, JsonText<ChapterContent>);
    typed!(chapters::Model, content, JsonText<ChapterContent>);
    typed!(entities::Model, aliases, JsonText<Vec<String>>);
    typed!(
        entities::Model,
        attributes,
        JsonText<BTreeMap<String, String>>
    );
    typed!(knowledge_entries::Model, source_urls, JsonText<Vec<String>>);
    typed!(knowledge_entries::Model, tags, JsonText<Vec<String>>);
    typed!(model_profiles::Model, options, JsonText<ModelOptions>);
    typed!(
        providers::Model,
        extra_headers,
        JsonText<BTreeMap<String, String>>
    );
    typed!(review_reports::Model, issues, JsonText<Vec<ReviewIssue>>);
    typed!(search_docs::Model, meta, JsonText<serde_json::Value>);
    typed!(setting_proposals::Model, edits, JsonText<ProposalEdits>);
}

/// Check 6's other half: the compile-time guards in `json_columns_are_typed` cover exactly the
/// `json_valid` columns of the schema, so a new JSON column cannot be added without a guard.
#[tokio::test]
async fn every_json_column_has_a_typed_field_guard() {
    const JSON_COLUMNS: &[(&str, &str)] = &[
        ("agent_messages", "content"),
        ("agent_messages", "provider_payload"),
        ("agent_messages", "request_meta"),
        ("agent_messages", "usage"),
        ("agent_sessions", "model_snapshot"),
        ("agent_sessions", "tools"),
        ("agent_sessions", "view_snapshot"),
        ("agent_sessions", "counters"),
        ("agent_sessions", "usage"),
        ("agent_sessions", "error"),
        ("agent_tool_calls", "appendices"),
        ("agent_tool_calls", "ui"),
        ("agent_tool_calls", "effects"),
        ("books", "settings"),
        ("books", "model_overrides"),
        ("books", "conception"),
        ("change_log", "before"),
        ("change_log", "after"),
        ("chapter_feedback", "issues"),
        ("chapter_versions", "content"),
        ("chapters", "content"),
        ("entities", "aliases"),
        ("entities", "attributes"),
        ("knowledge_entries", "source_urls"),
        ("knowledge_entries", "tags"),
        ("model_profiles", "options"),
        ("providers", "extra_headers"),
        ("review_reports", "issues"),
        ("search_docs", "meta"),
        ("setting_proposals", "edits"),
    ];

    let (_dir, conn) = migrated_db().await;
    let tables = conn
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name, sql FROM sqlite_master WHERE type = 'table' \
             AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'seaql_%'",
        ))
        .await
        .expect("read the schema");
    let mut actual = BTreeSet::new();
    for table in &tables {
        let name: String = table.try_get("", "name").expect("table name");
        let sql: String = table.try_get("", "sql").expect("table SQL");
        let mut rest = sql.as_str();
        while let Some(position) = rest.find("json_valid(") {
            let tail = &rest[position + "json_valid(".len()..];
            let column = tail
                .split(|character: char| {
                    character == ')' || character == ',' || character.is_whitespace()
                })
                .next()
                .unwrap_or_default();
            if !column.is_empty() {
                actual.insert((name.clone(), column.to_owned()));
            }
            rest = tail;
        }
    }
    actual.remove(&("app_settings".to_owned(), "value".to_owned()));

    let declared: BTreeSet<(String, String)> = JSON_COLUMNS
        .iter()
        .map(|(table, column)| ((*table).to_owned(), (*column).to_owned()))
        .collect();
    assert_eq!(
        declared, actual,
        "every json_valid column must have a compile-time JsonText guard"
    );
}
