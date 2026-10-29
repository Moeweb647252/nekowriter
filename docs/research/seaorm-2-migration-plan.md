# Implementation Plan: Bring the Persistence Layer to the Best Shape of SeaORM 2.0.x (M14)

**Input:** the author's request: "migrate to the best shape of SeaORM 2.0.x".
**Date:** 2026-10-29. **Status:** plan, not implemented.
**Readers:** the executing agent (follow `AGENTS.md` §5 per task) and the author, who should confirm
§9 before task 14.2 starts.

Every code reference below was checked against the tree at `ea89eeb`. Line numbers drift, so
function names are the stable anchors. The claims in §1.8–§1.10 were verified by compiling and
running throwaway programs against the workspace's own `Cargo.lock` (sea-orm 2.0.4, sea-query
1.0.2) and by running `sea-orm-cli 2.0.4 generate entity --entity-format dense` on the full schema.

**What "migration" means here.** The dependencies are already SeaORM 2.0.4 (spec/01 §2), but the
code is written in the 1.x style:
- expanded entities with empty `Relation` enums;
- enum, boolean and JSON columns typed as `String`/`i32` and converted by hand;
- SQL assembled from strings;
- per-row lookup loops.

This milestone brings the **code** to the target shape of §2. It changes no dependency, no SQL
file, no API, no event, no prompt and no generated TypeScript binding.

---

## 0. Decisions at a glance

| Topic | Decision |
|---|---|
| **Schema ownership** | Unchanged: SQL-first (D-015). Entity-first `schema-sync` cannot express this schema's CHECK lists, FTS5/vec0 tables, partial unique indexes or triggers, and it never alters a CHECK. The entities describe the schema, and a drift test proves they match it (§3). |
| **Entity format** | All 36 entities use `#[sea_orm::model]` (the dense 2.0 format). The 36 empty `enum Relation {}` blocks are deleted. |
| **Relation graph** | Complete, in the shape SeaORM's own codegen produces:<br>• a `belongs_to` for every one of the 82 FKs;<br>• a reverse `has_many` on the parent for every FK whose (child, parent) pair is unambiguous;<br>• `has_many, via = "<junction>"` in both directions over the five join tables;<br>• `self_ref` for the three self references;<br>• `relation_enum` names where two FKs share a target. The reverse side of such pairs is omitted, as codegen omits it. |
| **Unique keys** | `#[sea_orm(unique)]` / `unique_key = ".."` on the 14 non-partial unique constraints. 2.0 then generates `find_by_*`, `filter_by_*` and `delete_by_*`, which replace the hand-written lookups. Partial unique indexes stay SQL-only. |
| **Enum columns** | The 39 CHECK-constrained `TEXT` columns become their Rust enums, through `DeriveActiveEnum` added to `string_enum!`. spec/02 §1 already requires this; the code does not do it today. |
| **Boolean columns** | The 8 `INTEGER` 0/1 flags become `bool`. The stored values do not change. |
| **JSON columns** | 30 of the 31 `json_valid` columns become `JsonText<T>`, one generic wrapper in `db/json.rs` (§2.4). It writes exactly the text `json::to_json` writes today. `app_settings.value` stays `String`, because its shape depends on `key`. |
| **Decoding** | Strict. A value that does not decode fails the query (→ `500 internal`) and names the column. This removes every silent fallback on column reads. **Author confirms (§9 Q1).** |
| **Writes** | SQL built from strings becomes typed `update_many().set(partial ActiveModel)` or `.col_expr(..)`, with typed filters: the session state machine, startup recovery, the indexer. |
| **Reads** | Per-row lookup loops become the Entity/Model Loader or one `is_in` query. List queries that never read `content` use a `DerivePartialModel`. Related-row existence filters use `has_related`. Filters and ordering use the typed `COLUMN` constant everywhere. |
| **Raw SQL** | Static raw statements in `src` use `raw_sql!` (named parameters). `Statement::from_sql_and_values` stays only where the table name is chosen at runtime from a closed set (`LinkTable`, `fresh_id`), and test fixtures keep their raw SQL. FTS5, vec0, `sqlite_master` and `INSERT OR IGNORE … SELECT` stay raw by nature. |
| **Not adopted** | Each rejected for a concrete reason (§2.10):<br>• nested ActiveModel saves;<br>• `ActiveModelBehavior` timestamp hooks;<br>• typed-ID newtypes;<br>• RBAC, Arrow, Seaography;<br>• regenerating the entities with `sea-orm-cli`. |
| **Guards first** | `tests/entity_schema.rs` lands before any entity changes, and each task extends it. A decode-every-row check runs on a **copy** of the author's database before shipping. |
| **Ordering** | Recommended: after 13.2 (whose kickoff snapshots then also guard this refactor) and before 13.3 (so the goal tables are born in the target shape). **Author confirms (§9 Q2).** |

---

## 1. Facts this plan rests on

1. **Crates and features** (`Cargo.toml`): `sea-orm 2.0.4` with `macros`, `with-json`,
   `with-chrono`, `sqlx-sqlite`, `runtime-tokio` and `sqlite-use-returning-for-3_35`. `macros`
   already exports `model`, `DeriveActiveEnum`, `DeriveValueType`, `DerivePartialModel` and
   `raw_sql!`. `Cargo.toml` and `Cargo.lock` do not change.
2. **Entities** (`crates/server/src/db/entities/`, 36 files): all use the expanded
   `DeriveEntityModel` form with `#[derive(DeriveRelation)] pub enum Relation {}`. None uses
   `#[sea_orm::model]`, and no query uses a relation. The only `DeriveActiveEnum` is
   `writing_skills::WritingSkillAuthor`.
3. **Enum columns are `String`.** The 38 enums of `db/json.rs::string_enum!` (plus
   `WritingSkillAuthor`) are converted by hand at every boundary:
   - about 125 `.as_str().to_owned()` / `.as_str().into()` calls in `src`;
   - 14 `services::enum_of` calls and about 66 `from_str_opt` calls outside `json.rs`;
   - silent fallbacks, e.g. `ChapterStatus::from_str_opt(&row.status).unwrap_or(ChapterStatus::Planned)`
     in `api/dto/chapters.rs`, `services/books.rs` and `services/maintenance.rs`;
   - string literals in filters, e.g. `agent_sessions::Column::Role.is_in(["planner", "writer"])`
     in `services/chapters.rs` and `services/books.rs`.

   About 115 places in `src` and `tests` compare an enum field with a string literal.
4. **JSON columns are `String`**, read in four ways:
   - `services::parse_json`, which errors (about 26 calls);
   - `json::from_json` (7);
   - `json::from_json_or_default`, which silently defaults (about 60, including `chapters.content`
     in `agent/runner.rs`, `agent/context/mod.rs` and `services/sessions.rs`);
   - raw `serde_json::from_str(..).unwrap_or_default()` (`services/providers.rs`,
     `services/sessions.rs`).

   A corrupt `provider_payload` is replayed as `ProviderPayload::Mock { content: Null }`
   (`agent/context/mod.rs`). Writes go through `json::to_json(..).map_err(..)?`.
5. **Booleans are `i32`**: `agent_sessions.auto_pause`, `pause_after_outline`, `pause_requested`;
   `agent_tool_calls.is_error`, `sealed`; `entities.is_protagonist`;
   `writing_skills.enabled_by_default`, `retired`.
6. **Schema shape** (all SQL files applied): 36 tables and 82 FKs.
   - **Cycles**: `chapters` ↔ `chapter_versions`; `agent_sessions` ↔ `agent_tool_calls`.
   - **Join tables** with composite keys: `book_scopes`, `book_writing_skills`,
     `foreshadowing_entities`, `memory_note_entities`, `timeline_event_entities`.
   - **Text keys**: `app_settings`, `index_meta`.
   - **Same-target or self FKs**:

   | Entity | Same-target or self FKs (the `relation_enum` names follow the field names) |
   |---|---|
   | `agent_sessions` | `parent_session_id` (self) |
   | `change_log` | `reverts_id`, `reverted_by_id` (both self) |
   | `chapters` | `official_version_id`, `synced_version_id` → `chapter_versions` |
   | `entities` | `first_chapter_id`, `last_chapter_id` → `chapters` |
   | `foreshadowing` | `planted_chapter_id`, `resolved_chapter_id` → `chapters` |
   | `relations` | `since_chapter_id`, `until_chapter_id` → `chapters`; `source_id`, `target_id` → `entities` |

   - **Non-partial unique constraints**, excluding primary keys:
     `agent_messages (session_id, seq)`, `agent_tool_calls (message_id, call_index)`,
     `appearances (chapter_id, entity_id)`, `chapter_versions (chapter_id, version_no)`,
     `chapters (track_id, ord)`, `character_designs (book_id, name)`,
     `knowledge_chunks (entry_id, ord)`, `knowledge_entries (scope_id, title)`,
     `knowledge_scopes (name)`, `model_profiles (provider_id, display_name)`,
     `providers (name)`, `search_docs (kind, ref_id, ref_part)`,
     `setting_docs (book_id, kind)`, `writing_skills (key)`.
   - **Partial unique indexes**, which stay SQL-only: `entities_protagonist`, the two entity-name
     indexes, the two `prompt_overrides` indexes, the two active-relation indexes, `tracks_main`.
7. **Stringly-built SQL in production code**:
   - `agent/session.rs::transition` and `guard_columns(.., set_sql: &str, ..)`, which
     `services/sessions.rs` feeds `"granularity = ?"`-style fragments;
   - the `canon_status = 'failed'` compare-and-swap and the `error = NULL` clear in
     `agent/session.rs`;
   - the recovery update in `agent/manager.rs`;
   - `services/indexer.rs`: `mark_doc_done`, `mark_chunk_done` and the failure loop interpolate
     ids and hashes into SQL with hand-written quote escaping (`hash.replace('\'', "''")`);
   - plain-table reads in `search/index.rs`.

   **Per-row lookup loops**:
   - `services/indexer.rs`: chunk → entry title;
   - `agent/context/mod.rs`: tool calls per ancestor session, ordered by id;
   - `services/books.rs` next-action rule 5: open session per chapter;
   - `services/canon/entity.rs`: appearances per entity or chapter;
   - entity-existence checks inside loops in `services/canon/{foreshadow,note,timeline}.rs`;
   - `search/maintenance.rs`: chunks by id.
8. **Verified on 2.0.4: columns and writes** (throwaway program 1):
   - `DeriveActiveEnum` and `EnumIter` work inside `string_enum!` with `$value:literal`,
     alongside `Copy`, `Hash` and the serde attributes.
   - `#[sea_orm::model]` moves relation fields out of `Model` into `ModelEx`, so existing
     `Model { .. }` and `ActiveModel { .. }` literals keep compiling.
   - The `JsonText<T>` of §2.4 works for structs, `Vec<T>`, foreign types and `Option<..>`. It
     stores `{"b":"x","a":2}` for a struct declared `b, a`: field order, byte-identical to
     `serde_json::to_string`. A shape mismatch fails with `Type("corrupt JSON column \"settings\": …")`.
     An empty string is rejected by `CHECK (json_valid(..))`.
   - `bool` fields are stored as SQLite `integer` 0/1.
   - `row.try_get::<ChapterStatus>("", "status")` works on raw rows.
   - `COLUMN.status.eq(ChapterStatus::InReview)`, `COLUMN.status.is_in([..])`,
     `COLUMN.sealed.eq(true)` and `order_by_asc(COLUMN.id)` work.
   - `update_many().set(partial_active_model).filter(..).exec(..)` writes only the `Set` fields
     and reports `rows_affected`.
   - `raw_sql!(Sqlite, ".. {id} .. ({..ids})")` works.
9. **Verified on 2.0.4: the full relation graph** (throwaway program 2). The dense entities that
   `sea-orm-cli` generated for this schema compile and run: 82 `belongs_to`, 60 reverse
   relations, M-N `via` in both directions, `self_ref`, both cycles. These work:
   - `books::Entity::load().with(chapters::Entity).with(knowledge_scopes::Entity).all(..)`
     (has_many plus M-N through `book_scopes`);
   - `models.load_many(chapters::Entity::find().order_by_asc(..), ..)`, an ordered Model Loader;
   - `find_related`;
   - a loader keyed on a same-target relation variant: `.with(entities::Relation::<Variant>)`.

   `#[sea_orm(unique)]` and `unique_key = "k"` generate `Entity::find_by_<col>(v)`,
   `Entity::find_by_<k>((a, b))`, `delete_by_*` and the loader's `filter_by_*`
   (`sea-orm-macros` `model_ex.rs::expand_find_by_unique_key`).
10. **What `sea-orm-cli` gets wrong on this schema**, and why the entities stay hand-written:
    - ids come out as `i64`, timestamps as `String`, and enum and JSON columns as `String`;
    - same-target relations get names like `ChapterVersions1`/`ChapterVersions2` and `SelfRef1`;
    - SQLite's partial unique indexes are read as plain unique keys, so `books → tracks` and
      `books → entities` become `HasOne`, which is wrong: a book has many of both;
    - the table-level `UNIQUE (..)` constraints are missed.
11. **2.0 gotchas the programs hit**:
    - sea-query 1.0 moved `Expr` arithmetic and comparisons to the `ExprTrait` trait, so
      `Expr::col(..).add(1)` needs `use sea_orm::sea_query::ExprTrait;`.
    - In `COLUMN`, an `ActiveEnum` or `JsonText` field gets the **untyped** `Column`, so
      `COLUMN.role.is_in(["planner"])` still compiles. The grep gate of §2.9 catches these.

---

## 2. Target shape

### 2.1 Entities

Every entity follows this shape (`chapters` shown after tasks 14.3–14.5):

```rust
//! `chapters` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

use crate::db::json::{CanonStatus, ChapterContent, ChapterStatus, JsonText};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "chapters")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: i32,
    #[sea_orm(unique_key = "track_ord")]
    pub track_id: i32,
    #[sea_orm(unique_key = "track_ord")]
    pub ord: i32,
    pub title: String,
    pub status: ChapterStatus,
    // … guidance, outline, reference_notes, summary unchanged …
    pub content: JsonText<ChapterContent>,
    pub content_version: i32,
    pub word_count: i32,
    pub official_version_id: Option<i32>,
    pub canon_status: CanonStatus,
    pub synced_version_id: Option<i32>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<super::books::Entity>,
    #[sea_orm(belongs_to, from = "track_id", to = "id")]
    pub track: BelongsTo<super::tracks::Entity>,
    #[sea_orm(belongs_to, relation_enum = "OfficialVersion", from = "official_version_id", to = "id")]
    pub official_version: BelongsTo<Option<super::chapter_versions::Entity>>,
    #[sea_orm(belongs_to, relation_enum = "SyncedVersion", from = "synced_version_id", to = "id")]
    pub synced_version: BelongsTo<Option<super::chapter_versions::Entity>>,
    #[sea_orm(has_many)]
    pub agent_sessions: HasMany<super::agent_sessions::Entity>,
    #[sea_orm(has_many)]
    pub appearances: HasMany<super::appearances::Entity>,
    // … one has_many per unambiguous child: change_log, chapter_feedback, memory_notes,
    //   review_reports, setting_proposals, timeline_events, tracks (anchor) …
}

impl ActiveModelBehavior for ActiveModel {}
```

Rules:
- **Forward field name**: the FK column without `_id`. `consumed_by_message_id` becomes
  `consumed_by_message`, and `parent_session_id` becomes `parent_session`.
- **Reverse field name**: the child table name, used as is (`agent_sessions`, `change_log`,
  `foreshadowing`).
- **M-N field**: the far table's name. When the junction's relation variant follows the far entity
  name (the normal `cake_filling`/`Filling` case), `#[sea_orm(has_many, via = "<junction>")]` is
  enough. These junctions name their FK fields after the local role (`book_scopes.book`/`scope`),
  so the far variant must be named and the near variant passed to `via`:
  `#[sea_orm(has_many, relation_enum = "Scope", via = "book_scopes::Book")]` on
  `pub knowledge_scopes: HasMany<..>` (and `relation_enum = "Book", via = "book_scopes::Scope"`
  in the other direction).
  - `entities` ↔ `timeline_events` / `memory_notes` / `foreshadowing`;
  - `books` ↔ `knowledge_scopes` (`book_scopes`) and `writing_skills` (`book_writing_skills`).

  A parent gets the `via` relation, not a `has_many` to the junction itself. The junction keeps
  its two `belongs_to`.
- **`relation_enum`**: where §1.6 requires it, plus one side of a `belongs_to` / `has_many` pair
  that would otherwise generate the same `RelatedEntity` variant (`Track`, `Session`,
  `ResearchTask`, `HeldToolCall`, `AnchorChapter`, `FocusEntity`), plus each junction end as above.
  It is the UpperCamelCase relation name and may differ from the field name.
- **`via_rel`**: a `has_many` whose child has two relations back to the parent (a `belongs_to`
  plus a `has_many`) needs `via_rel = "<child belongs_to variant>"`; the macro cannot infer which
  reverse the `has_many` means otherwise.
- **Self references** use `self_ref` with `relation_enum` and, where codegen gives a reverse side,
  `relation_reverse`. For example, on `agent_sessions`:
  `#[sea_orm(self_ref, relation_enum = "ParentSession", relation_reverse = "ChildSessions", from = "parent_session_id", to = "id")]`
  on `pub parent_session: BelongsTo<Option<Entity>>`, plus the `ChildSessions` `HasMany<Entity>`
  field. A `self_ref` `BelongsTo` does not generate a `Related` impl, so `find_related` is not
  available for it; joins use `Relation::ParentSession.def()` and the guard checks that
  `RelationDef` directly.
- **Same-target pairs** get no reverse field and no `Related` impl, so `find_related` does not
  compile there. Joins name `Relation::OfficialVersion.def()`, and loaders take
  `.with(chapters::Relation::OfficialVersion)`.
- **Nullability**: a nullable FK uses `BelongsTo<Option<..>>`, a `NOT NULL` FK uses
  `BelongsTo<..>`. Reverse relations are all `has_many`. No FK column is unique on its own, and
  codegen's `HasOne` (§1.10) is wrong here.
- **Unique keys**: `unique` for a single column (`providers.name`, `knowledge_scopes.name`,
  `writing_skills.key`). Composite keys get a `unique_key` named by meaning:
  `session_seq`, `message_call`, `chapter_entity`, `chapter_version`, `track_ord`, `book_name`,
  `entry_ord`, `scope_title`, `provider_display_name`, `doc_ref`, `book_kind`. These produce
  `chapters::Entity::find_by_track_ord((track_id, ord))` and similar finders.
- **No schema-only attributes**: `indexed`, `default_value`, `on_update`, `on_delete`. The SQL
  owns them, and schema generation is not used.
- **Keys**: composite and text keys keep `#[sea_orm(primary_key, auto_increment = false)]`.
- **Module docs**: unchanged. The `#[allow(clippy::module_inception)]` on `entities::entities`
  stays.

### 2.2 Enum columns

`string_enum!` in `db/json.rs` gains the SeaORM derive. `as_str`, `from_str_opt`, `Display` and
`FromStr` stay, for parsing that is not a column read (tool inputs, query parameters):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, EnumIter, DeriveActiveEnum)]
#[serde(rename_all = "snake_case")]
#[sea_orm(rs_type = "String", db_type = "Text")]
#[ts(export)]
pub enum $name {
    $(
        #[serde(rename = $value)]
        #[ts(rename = $value)]
        #[sea_orm(string_value = $value)]
        $variant,
    )+
}
```

**The 39 columns:**

| Table | Column → enum |
|---|---|
| `agent_inbox` | `kind` → `InboxKind` |
| `agent_messages` | `role` → `MessageRole`; `kind` → `MessageKind` |
| `agent_sessions` | `role` → `AgentRole`; `kind` → `SessionKind`; `status` → `SessionStatus`; `phase` → `Phase`; `granularity` → `Option<Granularity>` |
| `agent_tool_calls` | `status` → `ToolCallStatus` |
| `appearances` | `role` → `AppearanceRole` |
| `books` | `status` → `BookStatus` |
| `change_log` | `actor` → `ChangeActor`; `target_type` → `ChangeTargetType`; `op` → `ChangeOp` |
| `chapter_feedback` | `mode` → `FeedbackMode` |
| `chapter_versions` | `kind` → `VersionKind` |
| `chapters` | `status` → `ChapterStatus`; `canon_status` → `CanonStatus` |
| `entities` | `kind` → `EntityKind`; `status` → `EntityStatus` |
| `foreshadowing` | `status` → `ForeshadowingStatus` |
| `knowledge_chunks` | `embed_state` → `EmbedState` |
| `knowledge_entries` | `source` → `EntrySource` |
| `memory_notes` | `kind` → `NoteKind`; `status` → `NoteStatus` |
| `prompt_overrides` | `scope` → `PromptScope` |
| `providers` | `protocol` → `Protocol` |
| `relations` | `status` → `RelationStatus` |
| `research_tasks` | `status` → `ResearchTaskStatus` |
| `review_reports` | `verdict` → `ReviewVerdict` |
| `search_docs` | `kind` → `SearchDocKind`; `embed_state` → `EmbedState` |
| `setting_docs` | `kind` → `SettingDocKind` |
| `setting_proposals` | `target_type` → `ProposalTargetType`; `status` → `ProposalStatus` |
| `tracks` | `kind` → `TrackKind`; `canon_mode` → `CanonMode`; `anchor_kind` → `AnchorKind` |
| `writing_skills` | `author` → `WritingSkillAuthor` (already done) |

`agent_messages.stop_reason` has no CHECK and stays `Option<String>`.

**Call sites:**
- `Set(X::Y.as_str().to_owned())` becomes `Set(X::Y)`.
- `enum_of("t.c", &row.c)?` and `X::from_str_opt(&row.c).unwrap_or(..)` become `row.c`.
- Raw binds become `X::Y.into()`.
- Raw reads use `try_get::<X>`.
- `services::enum_of` is deleted.

### 2.3 Boolean columns

The eight fields of §1.5 become `bool`. `i32::from(flag)`, `!= 0` and `== 1` disappear at the
call sites. Raw SQL that compares with `0`/`1` keeps working.

### 2.4 JSON columns: `JsonText<T>`

`DeriveValueType` and `FromJsonQueryResult` do not fit:
- the orphan rule blocks the SeaORM traits for `Vec<StoredBlock>` and the foreign
  `nekowriter_llm::ToolDef`;
- `FromJsonQueryResult` goes through `serde_json::Value`, which re-orders keys.

So one generic wrapper goes in `db/json.rs`, exactly as program 1 ran it:

```rust
/// A typed JSON column (spec/02 §1, §4), stored as the exact `serde_json::to_string` text, which is
/// what `to_json` wrote before D-151: rows written before and after are byte-identical. Decoding
/// is strict; a value that does not deserialize fails the query and names the column.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JsonText<T>(pub T);

impl<T> JsonText<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> std::ops::Deref for JsonText<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: Serialize> From<JsonText<T>> for sea_orm::Value {
    fn from(source: JsonText<T>) -> Self {
        let text = serde_json::to_string(&source.0).unwrap_or_else(|err| {
            // Unreachable for the json.rs types (derived Serialize, string-keyed maps). The empty
            // text fails the column's `CHECK (json_valid(..))`, so the write still aborts.
            tracing::error!(error = %err, "cannot serialize a JSON column");
            String::new()
        });
        sea_orm::Value::String(Some(text))
    }
}

impl<T: DeserializeOwned> sea_orm::TryGetable for JsonText<T> {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &sea_orm::QueryResult,
        idx: I,
    ) -> Result<Self, sea_orm::TryGetError> {
        let raw = String::try_get_by(res, idx)?;
        serde_json::from_str(&raw).map(JsonText).map_err(|err| {
            sea_orm::TryGetError::DbErr(sea_orm::DbErr::Type(format!(
                "corrupt JSON column {idx:?}: {err}"
            )))
        })
    }
}

impl<T: DeserializeOwned> sea_orm::sea_query::ValueType for JsonText<T> {
    fn try_from(v: sea_orm::Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        let raw = <String as sea_orm::sea_query::ValueType>::try_from(v)?;
        serde_json::from_str(&raw).map(JsonText).map_err(|_| sea_orm::sea_query::ValueTypeErr)
    }
    fn type_name() -> String {
        "JsonText".to_owned()
    }
    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::String
    }
    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::Text
    }
}

impl<T> sea_orm::sea_query::Nullable for JsonText<T> {
    fn null() -> sea_orm::Value {
        sea_orm::Value::String(None)
    }
}
```

**Column types** (the type is the one `json.rs` documents for the column, or the one its readers
already deserialize into):

| Column | Field type |
|---|---|
| `agent_messages.content` | `JsonText<Vec<StoredBlock>>` |
| `agent_messages.provider_payload`, `.request_meta`, `.usage` | `Option<JsonText<ProviderPayload \| RequestMeta \| Usage>>` |
| `agent_sessions.model_snapshot`, `.counters`, `.usage` | `JsonText<ModelSnapshot \| SessionCounters \| Usage>` |
| `agent_sessions.tools` | `JsonText<Vec<nekowriter_llm::ToolDef>>` |
| `agent_sessions.view_snapshot`, `.error` | `Option<JsonText<ViewSnapshot \| SessionError>>` |
| `agent_tool_calls.appendices` | `JsonText<Vec<Appendix>>` |
| `agent_tool_calls.effects` | `Option<JsonText<ToolEffects>>` |
| `agent_tool_calls.ui` | `Option<JsonText<serde_json::Value>>` (the tools produce free-form UI data) |
| `books.settings`, `.model_overrides`, `.conception` | `JsonText<BookSettings \| RoleModels \| ConceptionState>` |
| `change_log.before`, `.after` | `Option<JsonText<Snapshot>>` (tagged by `type`, so it is self-describing) |
| `chapters.content`, `chapter_versions.content` | `JsonText<ChapterContent>` |
| `review_reports.issues`, `chapter_feedback.issues` | `JsonText<Vec<ReviewIssue>>`; confirm `chapter_feedback`'s shape in `services/reviews.rs` |
| `entities.aliases`, `knowledge_entries.source_urls`, `.tags` | `JsonText<Vec<String>>` |
| `entities.attributes` | `JsonText<..>`, the map type its readers use (`services/canon/mod.rs`) |
| `providers.extra_headers` | `JsonText<BTreeMap<String, String>>` |
| `model_profiles.options` | `JsonText<ModelOptions>`, a new `json.rs` struct with the fields `ModelOptionsDto` reads today. The DTO stays where it is and converts with `From`, so entities never depend on `api/` (spec/01 §4 rule 4) and the bindings do not change. |
| `search_docs.meta` | `JsonText<serde_json::Value>` (`parse_pids` reads it loosely; typing its shape is not in scope) |
| `app_settings.value` | **stays `String`**, read with `parse_json` per key |

**Call sites:**
- `json::to_json(&x).map_err(..)?` in an `ActiveModel` becomes `Set(JsonText(x))`.
- Reads use `row.content.paragraphs` (through `Deref`) or `row.content.into_inner()`.
- An `Option<JsonText<T>>` that defaulted before becomes
  `row.effects.map(JsonText::into_inner).unwrap_or_default()`. Here `None` means SQL `NULL`,
  which is a legal state, not corruption.
- Once no column callers remain, `from_json_or_default` and `from_json` are deleted (with the
  `from_json_or_default` unit test). `to_json` and `parse_json` stay for `app_settings`.

### 2.5 Strict decoding

Under strict decoding a row that does not decode fails its query (`DbErr::Type`, so
`500 internal`). Today such a row silently reads as a default status, an empty chapter, or a Mock
payload.

`CHECK` constraints make an unknown enum value or invalid JSON syntax impossible, and
`#[serde(default)]` keeps old rows loading (spec/02 §4). What strictness removes is the hazard
where a shape mismatch makes `from_json_or_default` return an **empty** `ChapterContent`, and a
later edit then writes it back over the real text. Task 14.9 proves the author's database decodes
completely before this ships.

### 2.6 Writes

- **`guard_columns` becomes `guard_update`.** The new signature is
  `guard_update(conn, session_id, expected: SessionStatus, changes: agent_sessions::ActiveModel)`,
  and the body runs:
  ```rust
  agent_sessions::Entity::update_many()
      .set(changes)
      .filter(agent_sessions::COLUMN.id.eq(session_id))
      .filter(agent_sessions::COLUMN.status.eq(expected))
      .exec(conn)
      .await?
  ```
  It keeps the `rows_affected == 0 → session.invalid_state` rule (spec/03 §14, D-034).
  - `NotSet` fields are not written.
  - `Set(None)` writes `NULL`.
  - `Set(false)` replaces `pause_requested = 0`.

  The `services/sessions.rs` patch builds an `agent_sessions::ActiveModel` instead of `sets` and
  `values`, and still refuses an empty patch with `validation`.
- **`transition`** builds the same partial `ActiveModel`: status, `updated_at`, `finished_at`
  (`Set(Some(now))` or `Set(None)`), and, when it leaves `running`, `pause_requested` plus the
  captured `view_snapshot`. It goes through `guard_update`. The event it returns is unchanged.
- **The `canon_status` compare-and-swap** (`syncing → failed`), the `error` clear and the
  `agent/manager.rs` recovery (`executing → pending`, `started_at = NULL`) each become
  `update_many()` with `col_expr` and typed filters.
- **`services/indexer.rs`** `embed_state` updates become `update_many()` with
  `col_expr(Column::EmbedState, Expr::value(EmbedState::Done))` plus id and `content_hash`
  filters. This deletes the hand-written quote escaping. `rebuild`'s two-statement
  `execute_unprepared` becomes two typed `update_many` calls in the same transaction.
- **Inserts and single-row updates** keep the current `ActiveModel` + `insert` / `update` /
  `exec_with_returning` style. Lookups by a unique key use the generated `find_by_*` (§2.1).
- **Static raw statements** in `src` (FTS5 reads and inserts, vec0 inserts and KNN, `sqlite_master`
  probes, `vec_version`) move to `raw_sql!` with named parameters. If `raw_sql!` cannot bind a
  value type (for example the embedding blob), that statement keeps `Statement` with a comment.
  Dynamic-table statements keep `Statement` with a comment naming the closed set. Every remaining
  raw statement has a one-line comment saying why it is raw.

### 2.7 Reads

- **Loaders replace per-row loops** (§1.7):
  - children of a list of parents use `parents.load_many(Child::find().order_by_asc(..), conn)`,
    with the ordered `Select` keeping today's order;
  - a parent per child uses `children.load_one(Parent, conn)`;
  - a tree read uses `Entity::load().with(..)`.

  A loop that only checks whether ids exist becomes one `COLUMN.id.is_in(ids)` query. The
  converted function's output must be identical, including order. Its existing tests are the
  proof, and a function with no test gets one first.
- **Partial models**: a query on `chapters` or `chapter_versions` that returns several rows and
  never reads `content` selects a `#[derive(DerivePartialModel)]` struct holding only the fields
  it uses. 2.0 needs no `from_query_result` attribute. This keeps list endpoints from decoding
  every chapter's JSON. Each partial model lives next to the service that uses it.
- **`has_related`**: a filter that loads related rows only to test whether one matches becomes
  `.has_related(Child, condition)`, a single `EXISTS` subquery.
- **Finders**: hand-written lookups on a unique key become the generated finders. For example,
  `.filter(Column::TrackId.eq(t)).filter(Column::Ord.eq(o))` becomes
  `find_by_track_ord((t, o))`.

### 2.8 Query style (`COLUMN`)

- `.filter(..)`, `.order_by_*(..)` and `Condition` items use `entity::COLUMN.field`.
- `col_expr`, `select_only().column(..)`, `group_by`, `Relation::X.def()` and spec/06's
  `visible_condition(vis, column)` keep `Column::`.
- Files that call `Expr` methods import `sea_orm::sea_query::ExprTrait`.

The sweep is mechanical and covers `src` and `tests`. It lands as its own commit so it reviews
as a rename.

### 2.9 Grep gate for enum filters

It must print nothing for the enum columns of §2.2, and every hit is converted to the enum:

```sh
grep -rnE '(Column::[A-Za-z]+|COLUMN\.[a-z_]+)\.(eq|ne|is_in|is_not_in)\(\s*\[?\s*"' crates/server/src
```

Hits on genuine string columns (`title`, `key`, `name`) are fine. Review each one.

### 2.10 Not adopted, and why

- **Nested ActiveModel saves**: canon rows take explicit ids from `services/canon::fresh_id`,
  which guards against SQLite reusing a deleted max rowid. Every write also records a
  `change_log` row and emits events in a fixed order. A nested save would let SQLite allocate ids
  and hide that order.
- **`ActiveModelBehavior` hooks for `updated_at`**: they do not run for `update_many` or raw SQL,
  so timestamps would have two code paths. Services keep setting them explicitly.
- **Typed-ID newtypes** (`DeriveValueType` wrappers as primary keys): they would reach every DTO
  and the TypeScript bindings, which is an API-surface change outside this milestone.
- **RBAC, Arrow, Seaography, ClickHouse**: not relevant to a local single-user app.
- **`sea-orm-cli generate entity` as the source**: see §1.10. Regenerate only as a cross-check
  (§8).
- **Entity-first `schema-sync`**: see §0 and D-015.

### 2.11 What does not change

- `crates/migration/` and every SQL file;
- `Db::connect`, `Db::write` and the write lock (spec/01 §7);
- `services::write` and its error encoding (D-064);
- DTOs, API responses and SSE events;
- `web/src/api/gen/*`, which `scripts/check-bindings.sh` must leave unchanged;
- prompts;
- test fixtures' raw SQL;
- `DateTimeUtc` storage;
- the 13.2 kickoff baseline snapshots, if 13.2 has run.

---

## 3. Guards

**`crates/server/tests/entity_schema.rs`.** It migrates a temp database and checks it against one
macro-generated list of the 36 entity modules.

1. **Table completeness**: every table in `sqlite_master` has exactly one entity, and every entity
   has a table. The list excludes `sqlite_%`, `seaql_migrations`, FTS5 shadow tables and the vec0
   tables.
2. **Columns**: for each entity, the column names, nullability (`Option<..>` vs `notnull`), type
   affinity (`TEXT` vs `INTEGER` vs `REAL`) and primary key match `PRAGMA table_info`.
3. **Relations** (from 14.3):
   - The set of `belongs_to` definitions, `(from table, from column, to table, to column)` read
     from `Relation::iter().map(|r| r.def())`, equals `PRAGMA foreign_key_list` for every table.
   - Every unambiguous FK has its reverse `has_many` on the parent.
   - Each junction has `via` relations on both ends.
4. **Unique keys** (from 14.3): for each entity, its `unique` / `unique_key` column sets equal the
   non-partial unique indexes of `PRAGMA index_list` (`unique = 1`, `partial = 0`), excluding
   primary keys.
5. **Enums** (from 14.4): a table of `(table, column, enum)` rows.
   - Each row asserts at compile time that the field has the enum type:
     `let _: fn(&chapters::Model) -> &ChapterStatus = |m| &m.status;`
   - `ActiveEnum::values()` equals the CHECK list parsed from `sqlite_master.sql`, in both
     directions.
   - Every `CHECK (col IN (..))` column in the schema has a row, so a new enum column without a
     mapping fails.
   - The eight boolean fields get the same compile-time assertion with `bool`.
6. **JSON** (from 14.5): every `json_valid(col)` column except `app_settings.value` has a row
   asserting its field is `JsonText<..>` or `Option<JsonText<..>>`.

**`decode_all_rows`.** An `#[ignore]` test in the same file. It reads `NW_DECODE_DB`, copies that
database into `target/tmp`, runs the migrator (a no-op on a current database) and calls
`Entity::find().all()` for every entity, printing the row counts. 14.2 adds it. 14.9 runs it on a
copy of the author's database.

---

## 4. Spec and document changes (task 14.1)

| Document | Change |
|---|---|
| spec/01 §3.4 and the §3 tree comment on `entities/` | Entities are hand-written in the `#[sea_orm::model]` format with the full relation graph and unique keys, checked against the SQL by `tests/entity_schema.rs`. |
| spec/02 §1 | **Entities** (new bullet): the format, relation and key naming, no schema-only attributes. **Booleans**: Rust `bool`. **Enums**: entity fields use the enum type. **JSON columns**: entity fields are `JsonText<T>`, which stores `serde_json::to_string` text; decoding is strict; `app_settings.value` is the exception. |
| NOTES.md, "Shared Rust foundation" | The `entities/` and `json.rs` bullets describe the following: `JsonText`, the enum derive, `COLUMN`, finders, loaders, partial models, `guard_update`, `raw_sql!`, and the `ExprTrait` import. |
| DECISIONS.md | Append **D-151 SeaORM 2 target shape**: context (§1), decision (§0, §2.10), consequences (strict decoding, no binding or schema change, the 14.9 report). |
| PLAN.md | Append the §5 table as M14, plus one line under the M13 heading: "13.3 starts after M14" (if §9 Q2 is accepted). |

---

## 5. Tasks (M14)

Every task leaves the following unchanged: SQL files, prompts, `web/src/api/gen`, API responses,
events, and (if present) the 13.2 snapshots. Each task ends with `scripts/check.sh` green and one
commit.

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 14.1 | **Docs first**: §4, D-151, the M14 table. | plan §4 | `check.sh` green (docs only) |
| 14.2 | **Guards on unchanged code**: `tests/entity_schema.rs` checks 1–2 and `decode_all_rows`. | 02 §1–§2 | The new test passes before any entity changes. Deliberately breaking one entity column in a scratch edit makes it fail. |
| 14.3 | **Entity format, relation graph, unique keys**: all 36 entities become `#[sea_orm::model]`, the empty `Relation` enums go, and the relation graph and unique keys of §2.1 come in. Enum, JSON and bool fields stay `String`/`i32` in this task. | 02 §2 | Checks 3–4 pass. Smoke tests: `find_related` on a required FK, a nullable FK and the `parent_session` self reference; `books::Entity::load().with(tracks::Entity).with(knowledge_scopes::Entity)`; `entities` → `timeline_events` via the junction; `chapters::Entity::find_by_track_ord`. The suite passes with no other test edits. |
| 14.4 | **Enum and boolean columns**: the `string_enum!` derive, the 39 enum fields and the 8 bool fields; delete `enum_of` and the column uses of `from_str_opt`/`as_str`; the §2.9 grep gate; compile-driven test edits (string literal → enum). | 02 §1, §3 | Check 5 passes. Existing tests pass, changed only from string to enum comparisons. No `web/src/api/gen` diff. The grep gate lists only genuine string columns. |
| 14.5 | **JSON columns**: `JsonText<T>` with its unit tests, the 30 fields, `json::ModelOptions`, and removal of every silent column fallback (§1.4). | 02 §1, §4 | `JsonText` tests: for every type in the existing round-trip test, the stored text equals `serde_json::to_string`; `Option` ↔ `NULL`; a shape mismatch names the column; the empty string fails the CHECK. Check 6 passes. `strict_decode.rs`: a chapter whose `content` is valid JSON of the wrong shape (set through `Db::write_unprepared`) makes `GET /api/chapters/:id` answer `500 internal`, and a write session refuses to start instead of seeing an empty chapter. |
| 14.6 | **Writes** (§2.6): `guard_update`, `transition`, the session patch, the `canon_status` compare-and-swap, the `error` clear, manager recovery, indexer `embed_state`, `raw_sql!` for static raw statements, and a reason comment on every remaining raw statement. | 03 §3, §14; 07 §4 | The race and transition suites (`review_races`, `outline_concurrency`, `m11_transition_atomicity`, the agent harness suites) and the M7 index suites pass unchanged. A unit test: `guard_update` with a stale expected status returns `session.invalid_state` and changes no column. A unit test per non-trivial converted statement compares `.build(DbBackend::Sqlite).to_string()` with the SQL it replaces, ignoring quoting. `grep -n 'format!' crates/server/src/services/indexer.rs` shows no SQL. |
| 14.7 | **Reads** (§2.7): loaders for the §1.7 loops, `is_in` batching, partial models for the `chapters`/`chapter_versions` list queries, `has_related`, generated finders. | 07, 08 §7 | Each converted function keeps its output, including order, as proven by its existing tests or a new one written **before** the change. `overview_actions.rs` and the agent-context snapshots are unchanged. |
| 14.8 | **`COLUMN` sweep** (§2.8): mechanical, `src` and `tests`. | — | No behavior change. `grep` shows `Column::` only where §2.8 allows it. |
| 14.9 | **Pre-ship**: run `decode_all_rows` on a **copy** of the author's database; run smoke items 1–4 and 8 with the Demo provider; compare `cargo build -p nekowriter` times before M14 and after it. | PLAN §M11.5 | Every row decodes (any failure is reported with its table, id and column, and is not "fixed" in code). The checklist passes. The build times and row counts are in the final message and in D-151's consequences. |

---

## 6. Risks

| Risk | Mitigation |
|---|---|
| Strict decoding turns a silently defaulted row into a 500. | It is the intended behavior (§2.5). 14.9 proves the real database is clean, and an error names the column. |
| The compiler does not see string literals on enum columns (§1.11). | The grep gate in 14.4. Review of every enum filter touched. |
| A loader or batched query changes an order that a kickoff depends on. | Ordered `Select`s, tests written before the change, and the 13.2 kickoff snapshots (14.7). |
| A typed update writes a column the string version did not. | Partial `ActiveModel`s set only what the old fragment set, and the SQL-equality unit tests in 14.6. |
| Merge conflicts with M13, which touches the same services. | Run M14 between 13.2 and 13.3 (§9 Q2). Never interleave the two in one branch. |
| Compile-time growth: `ModelEx`, `ActiveModelEx`, `EntityLoader` and the finders are generated per entity. | Measured in 14.9. Acceptable unless the author says otherwise. |
| Scope creep ("while I'm here"). | §2.10 and §2.11 are binding. Anything else is a new D-entry question for the author. |

---

## 7. Notes for the executing agent

- `#[sea_orm::model]` must sit **above** `#[derive(.., DeriveEntityModel)]`. Keep
  `impl ActiveModelBehavior for ActiveModel {}`.
- `ModelEx` never derives `Eq`. `Model` keeps today's derives (`Clone, Debug, PartialEq`).
- Imports:
  - `JsonText` and the enums come from `crate::db::json`;
  - `EnumIter` and `DeriveActiveEnum` come from `sea_orm`;
  - `ExprTrait` comes from `sea_orm::sea_query`;
  - `QueryFilter`, `QueryOrder`, `QuerySelect` and `LoaderTrait` are imported as needed.
- `Update::one(..)` / `Delete::one(..)` need `.validate()?` before `.build(..)` in 2.0. The
  generated `delete_by_*` returns `ValidatedDeleteOne`.
- When the compiler disagrees with a snippet here, the compiler wins. Fix the snippet in this plan
  and record it in the decision log (D-152 records the M14.3 corrections).
- SeaORM 2.x docs: <https://www.sea-ql.org/SeaORM/docs/>. The crate source is in
  `~/.cargo/registry/src/*/sea-orm-2.0.4/` and `sea-orm-macros-2.0.4/`.

---

## 8. Cross-checking with SeaORM's codegen

The codegen output is a reference for the relation graph's shape, never something to copy
(§1.10). A build is already installed at `target/tmp/seaorm-cli/bin/sea-orm-cli`, and its output
for `ea89eeb` is in `target/tmp/seaorm-gen/out/`. To regenerate it after a schema change:

```sh
cargo install sea-orm-cli@2.0.4 --locked --no-default-features \
  --features codegen,sqlx-sqlite,runtime-tokio --root target/tmp/seaorm-cli
# Build target/tmp/seaorm-gen/schema.db by applying crates/migration/src/sql/*.sql in order
# with python3's sqlite3 module, then drop the fts_* tables (sea-schema cannot read them).
target/tmp/seaorm-cli/bin/sea-orm-cli generate entity --entity-format dense \
  -u sqlite://target/tmp/seaorm-gen/schema.db -o target/tmp/seaorm-gen/out
```

---

## 9. For the author to confirm before 14.2

1. **Q1 Strict decoding** (§2.5). Recommended: yes. The alternative keeps per-column lenient
   fallbacks, which preserves today's failure behavior and its silent-empty-chapter hazard.
2. **Q2 Ordering.** Recommended: M14 runs after 13.2 and before 13.3. The alternative is after
   M13, which means the goal entities are written in the old style and then migrated.
