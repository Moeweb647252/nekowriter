# 02 — Data Model

## 1. Conventions

- **Engine**: one SQLite database file, WAL mode, `foreign_keys=ON`.
- **Primary keys**: `id INTEGER PRIMARY KEY` (rowid alias). Rust type `i32`. TS type `number`.
  Agents see ids as `#12`.
- **Timestamps**: `TEXT` RFC 3339 UTC with milliseconds (`chrono::DateTime<Utc>`, SeaORM
  `DateTimeUtc`), set by the application, never by SQL defaults.
- **Booleans**: `INTEGER` 0/1.
- **JSON columns**: `TEXT` with `CHECK (json_valid(col))`, mapped to typed Rust structs through
  SeaORM `Json` plus serde (see §4). Never store untyped JSON blobs.
- **Enums**: `TEXT` with `CHECK (... IN (...))`. Rust enums use
  `#[derive(DeriveActiveEnum, EnumIter)] #[sea_orm(rs_type = "String", db_type = "Text")]` and
  `#[serde(rename_all = "snake_case")]`.
- **Canon visibility**: canon tables have `track_id`. `NULL` means shared canon (main story
  plus canon-mode side stories). Non-NULL means private to that isolated side track (spec/06 §2).
- **Ordering**: chapters use `ord INTEGER`, contiguous `1..n` per track, and a chapter's number
  (as in 第12章) is its `ord`. Reorder and delete renumber with a two-phase update (first
  `ord = -ord - 1`, then the final values) inside one transaction. Timeline events use
  `position REAL` with midpoint insertion; renormalize to multiples of 1024 when a gap falls below
  `1e-6`. Character designs use `rank INTEGER` (1 = most important), renumbered 1..n on change.
- **Migrations**: `crates/migration/src/sql/0001_init.sql` holds the full initial schema below.
  Until the first tagged release, 0001 may be edited and dev databases recreated. After that,
  migrations are append-only.
  An edit to 0001 reaches **only** fresh databases: the migrator records applied files in
  `seaql_migrations` and runs the pending ones only, and a database created after the first run
  already records `m0001_init`. A schema change that must reach an existing database therefore
  needs both — the edit for fresh databases, plus a later file that upgrades the existing ones.
  `0002_research_deferred.sql` is the first of those: SQLite cannot alter a CHECK constraint in
  place, so it rebuilds `research_tasks` (create, copy, drop, rename) with `PRAGMA foreign_keys`
  off, because `Db::connect` turns the pragma on and a `DROP TABLE` would then apply
  `agent_sessions`' `ON DELETE CASCADE` and `knowledge_entries`' `ON DELETE SET NULL`. Its
  module declares `use_transaction() = Some(false)`: the pragma is a no-op inside a transaction,
  so the migrator must not wrap the file, which opens its own.
  `0004_outline_sessions.sql` follows the same pattern for `agent_sessions`: its CHECK constraints
  list the roles and kinds, so it is rebuilt (create, copy, drop, rename, recreate its three
  indexes) with `PRAGMA foreign_keys = OFF`. Ten foreign keys reference the table (cascade:
  `agent_messages`, `agent_tool_calls`, `agent_inbox`; set-null: `chapter_versions`,
  `setting_proposals`, `change_log`, `research_tasks`, `review_reports`, `chapter_feedback`, and
  the table's own `parent_session_id`), so the migration snapshots the row counts of all ten
  before the rebuild and fails loudly if any changed, then runs `PRAGMA foreign_key_check`. It
  also moves `prompt_overrides` rows of `writer.kickoff.chapter` to `planner.kickoff` where the
  same scope has no planner override yet.

## 2. Schema (0001_init.sql, upgraded by 0002_research_deferred.sql, 0003 and 0004)

```sql
-- ───────────────────────────── App configuration ─────────────────────────────
CREATE TABLE app_settings (
  key         TEXT PRIMARY KEY,
  value       TEXT NOT NULL CHECK (json_valid(value)),
  updated_at  TEXT NOT NULL
);

CREATE TABLE providers (
  id             INTEGER PRIMARY KEY,
  name           TEXT NOT NULL UNIQUE,
  protocol       TEXT NOT NULL CHECK (protocol IN ('openai','anthropic','mock')),
  base_url       TEXT NOT NULL,
  api_key        TEXT,                                  -- literal key, 'env:NAME', or NULL
  extra_headers  TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(extra_headers)),
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL
);

CREATE TABLE model_profiles (
  id                 INTEGER PRIMARY KEY,
  provider_id        INTEGER NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  display_name       TEXT NOT NULL,
  model              TEXT NOT NULL,                     -- API model id
  context_window     INTEGER NOT NULL CHECK (context_window > 0),
  max_output_tokens  INTEGER NOT NULL CHECK (max_output_tokens > 0),
  options            TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(options)),   -- ModelOptions
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL,
  UNIQUE (provider_id, display_name)
);

-- ───────────────────────────── Books, tracks, chapters ─────────────────────────────
CREATE TABLE books (
  id                INTEGER PRIMARY KEY,
  title             TEXT NOT NULL,
  idea              TEXT NOT NULL DEFAULT '',
  genre             TEXT NOT NULL DEFAULT '',
  writing_language  TEXT NOT NULL,                      -- BCP-47, e.g. 'zh-CN', 'en'
  status            TEXT NOT NULL CHECK (status IN ('conceiving','active','archived')),
  settings          TEXT NOT NULL CHECK (json_valid(settings)),               -- BookSettings
  model_overrides   TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(model_overrides)), -- RoleModels
  conception        TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(conception)),      -- ConceptionState
  created_at        TEXT NOT NULL,
  updated_at        TEXT NOT NULL
);

CREATE TABLE tracks (
  id                 INTEGER PRIMARY KEY,
  book_id            INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  kind               TEXT NOT NULL CHECK (kind IN ('main','side')),
  title              TEXT NOT NULL,
  synopsis           TEXT NOT NULL DEFAULT '',
  canon_mode         TEXT NOT NULL CHECK (canon_mode IN ('canon','isolated')),
  anchor_kind        TEXT NOT NULL DEFAULT 'none'
                     CHECK (anchor_kind IN ('none','before_start','after_chapter','after_end')),
  anchor_chapter_id  INTEGER REFERENCES chapters(id) ON DELETE CASCADE,  -- services refuse to delete an
                                                        -- anchor chapter (spec/08 §1.1); CASCADE only acts
                                                        -- in a book delete, where SET NULL would break the
                                                        -- CHECK below and abort the delete
  focus_entity_id    INTEGER REFERENCES entities(id) ON DELETE SET NULL,   -- side story's protagonist (optional)
  ord                INTEGER NOT NULL,                  -- main track 0; side tracks 1, 2, … in creation order
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL,
  CHECK (kind = 'side' OR (canon_mode = 'canon' AND anchor_kind = 'none')),
  CHECK (anchor_kind <> 'after_chapter' OR anchor_chapter_id IS NOT NULL)
);
CREATE UNIQUE INDEX tracks_one_main ON tracks(book_id) WHERE kind = 'main';

CREATE TABLE chapters (
  id                   INTEGER PRIMARY KEY,
  book_id              INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  track_id             INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  ord                  INTEGER NOT NULL,
  title                TEXT NOT NULL DEFAULT '',
  status               TEXT NOT NULL
                       CHECK (status IN ('planned','drafting','in_review','revising','official')),
  guidance             TEXT NOT NULL DEFAULT '',        -- author's instructions for this chapter
  outline              TEXT NOT NULL DEFAULT '',        -- detailed chapter outline (Markdown)
  reference_notes      TEXT NOT NULL DEFAULT '',        -- facts the writer committed to follow
  summary              TEXT NOT NULL DEFAULT '',        -- written by maintenance
  content              TEXT NOT NULL CHECK (json_valid(content)),            -- ChapterContent
  content_version      INTEGER NOT NULL DEFAULT 0,      -- +1 on every content change
  word_count           INTEGER NOT NULL DEFAULT 0,
  official_version_id  INTEGER REFERENCES chapter_versions(id) ON DELETE SET NULL,
  canon_status         TEXT NOT NULL DEFAULT 'none'
                       CHECK (canon_status IN ('none','pending','syncing','synced','stale','failed')),
  synced_version_id    INTEGER REFERENCES chapter_versions(id) ON DELETE SET NULL,
  created_at           TEXT NOT NULL,
  updated_at           TEXT NOT NULL,
  UNIQUE (track_id, ord)
);
CREATE INDEX chapters_book ON chapters(book_id);

CREATE TABLE chapter_versions (
  id          INTEGER PRIMARY KEY,
  chapter_id  INTEGER NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
  version_no  INTEGER NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN ('draft','revision','official','manual','backup')),
  title       TEXT NOT NULL DEFAULT '',
  content     TEXT NOT NULL CHECK (json_valid(content)),                      -- ChapterContent
  word_count  INTEGER NOT NULL,
  note        TEXT NOT NULL DEFAULT '',
  session_id  INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
  created_at  TEXT NOT NULL,
  UNIQUE (chapter_id, version_no)
);

-- ───────────────────────────── Setting materials ─────────────────────────────
CREATE TABLE setting_docs (
  id                 INTEGER PRIMARY KEY,
  book_id            INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  kind               TEXT NOT NULL CHECK (kind IN ('outline','world','protagonist')),
  content            TEXT NOT NULL DEFAULT '',          -- Markdown
  revision           INTEGER NOT NULL DEFAULT 0,        -- +1 on every change
  approved_revision  INTEGER,                           -- conception review bookkeeping
  updated_at         TEXT NOT NULL,
  UNIQUE (book_id, kind)
);

CREATE TABLE character_designs (
  id          INTEGER PRIMARY KEY,
  book_id     INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  rank        INTEGER NOT NULL,                         -- 1 = most important
  name        TEXT NOT NULL,
  role_label  TEXT NOT NULL DEFAULT '',                 -- e.g. 主角 / 女主 / 反派 / 导师
  summary     TEXT NOT NULL DEFAULT '',
  profile     TEXT NOT NULL DEFAULT '',                 -- Markdown
  entity_id   INTEGER REFERENCES entities(id) ON DELETE SET NULL,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  UNIQUE (book_id, name)
);

CREATE TABLE setting_proposals (
  id           INTEGER PRIMARY KEY,
  book_id      INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  target_type  TEXT NOT NULL CHECK (target_type IN ('setting_doc','character_design','new_character')),
  target_kind  TEXT CHECK (target_kind IS NULL OR target_kind IN ('outline','world','protagonist')),
  target_id    INTEGER,                                 -- character_designs.id
  edits        TEXT NOT NULL CHECK (json_valid(edits)), -- ProposalEdits
  rationale    TEXT NOT NULL DEFAULT '',
  status       TEXT NOT NULL CHECK (status IN ('pending','accepted','rejected','conflict')),
  chapter_id   INTEGER REFERENCES chapters(id) ON DELETE SET NULL,
  session_id   INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
  created_at   TEXT NOT NULL,
  decided_at   TEXT
);
CREATE INDEX setting_proposals_book ON setting_proposals(book_id, status);

-- ───────────────────────────── Canon ─────────────────────────────
CREATE TABLE entities (
  id                INTEGER PRIMARY KEY,
  book_id           INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  track_id          INTEGER REFERENCES tracks(id) ON DELETE CASCADE,   -- NULL = shared canon
  kind              TEXT NOT NULL CHECK (kind IN ('character','organization','location','item','other')),
  name              TEXT NOT NULL,                      -- the entity's title
  aliases           TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(aliases)),     -- string[]
  summary           TEXT NOT NULL DEFAULT '',           -- one line
  description       TEXT NOT NULL DEFAULT '',           -- Markdown
  state             TEXT NOT NULL DEFAULT '',           -- current/immediate state
  attributes        TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(attributes)),  -- {string: string}
  importance        INTEGER NOT NULL DEFAULT 2 CHECK (importance BETWEEN 1 AND 5),
  status            TEXT NOT NULL DEFAULT 'active'
                    CHECK (status IN ('active','inactive','dead','destroyed','missing','unknown')),
  is_protagonist    INTEGER NOT NULL DEFAULT 0,
  first_chapter_id  INTEGER REFERENCES chapters(id) ON DELETE SET NULL,
  last_chapter_id   INTEGER REFERENCES chapters(id) ON DELETE SET NULL,
  created_at        TEXT NOT NULL,
  updated_at        TEXT NOT NULL
);
CREATE UNIQUE INDEX entities_shared_name ON entities(book_id, name) WHERE track_id IS NULL;
CREATE UNIQUE INDEX entities_track_name  ON entities(book_id, track_id, name) WHERE track_id IS NOT NULL;
CREATE UNIQUE INDEX entities_protagonist ON entities(book_id) WHERE is_protagonist = 1;
CREATE INDEX entities_book_kind ON entities(book_id, kind);

CREATE TABLE relations (
  id                INTEGER PRIMARY KEY,
  book_id           INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  track_id          INTEGER REFERENCES tracks(id) ON DELETE CASCADE,
  source_id         INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
  target_id         INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
  kind              TEXT NOT NULL,                      -- what SOURCE is to TARGET ("师兄")
  inverse_kind      TEXT NOT NULL DEFAULT '',           -- what TARGET is to SOURCE ("师弟"); '' = n/a
  description       TEXT NOT NULL DEFAULT '',
  status            TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','ended')),
  since_chapter_id  INTEGER REFERENCES chapters(id) ON DELETE SET NULL,
  until_chapter_id  INTEGER REFERENCES chapters(id) ON DELETE SET NULL,
  created_at        TEXT NOT NULL,
  updated_at        TEXT NOT NULL,
  CHECK (source_id <> target_id)
);
CREATE INDEX relations_source ON relations(source_id);
CREATE INDEX relations_target ON relations(target_id);
-- one active relation per ordered (source, target) pair in shared canon, and separately per
-- isolated track; `kind` is data about the bond, not part of its identity (spec/06 §1.2, D-143)
CREATE UNIQUE INDEX relations_active_shared ON relations(source_id, target_id)
  WHERE status = 'active' AND track_id IS NULL;
CREATE UNIQUE INDEX relations_active_track ON relations(track_id, source_id, target_id)
  WHERE status = 'active' AND track_id IS NOT NULL;

CREATE TABLE appearances (
  id          INTEGER PRIMARY KEY,
  chapter_id  INTEGER NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
  entity_id   INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
  role        TEXT NOT NULL CHECK (role IN ('pov','major','minor','mentioned')),
  note        TEXT NOT NULL DEFAULT '',
  UNIQUE (chapter_id, entity_id)
);
CREATE INDEX appearances_entity ON appearances(entity_id);

CREATE TABLE timeline_events (
  id           INTEGER PRIMARY KEY,
  book_id      INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  track_id     INTEGER REFERENCES tracks(id) ON DELETE CASCADE,
  position     REAL NOT NULL,                           -- chronological story order, ascending
  story_time   TEXT NOT NULL DEFAULT '',                -- free label: "天元历三千年春", "Day 3, night"
  title        TEXT NOT NULL,
  description  TEXT NOT NULL DEFAULT '',
  chapter_id   INTEGER REFERENCES chapters(id) ON DELETE SET NULL,   -- source chapter
  importance   INTEGER NOT NULL DEFAULT 2 CHECK (importance BETWEEN 1 AND 5),
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL
);
CREATE INDEX timeline_book_position ON timeline_events(book_id, position);
CREATE TABLE timeline_event_entities (
  event_id   INTEGER NOT NULL REFERENCES timeline_events(id) ON DELETE CASCADE,
  entity_id  INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
  PRIMARY KEY (event_id, entity_id)
);
CREATE INDEX timeline_event_entities_entity ON timeline_event_entities(entity_id);

CREATE TABLE memory_notes (
  id          INTEGER PRIMARY KEY,
  book_id     INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  track_id    INTEGER REFERENCES tracks(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL CHECK (kind IN ('fact','world_rule','other')),
  content     TEXT NOT NULL,
  chapter_id  INTEGER REFERENCES chapters(id) ON DELETE SET NULL,       -- where it was established
  importance  INTEGER NOT NULL DEFAULT 2 CHECK (importance BETWEEN 1 AND 5),
  status      TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','retired')),
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);
CREATE INDEX memory_notes_book ON memory_notes(book_id, kind, status);
CREATE TABLE memory_note_entities (
  note_id    INTEGER NOT NULL REFERENCES memory_notes(id) ON DELETE CASCADE,
  entity_id  INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
  PRIMARY KEY (note_id, entity_id)
);

CREATE TABLE foreshadowing (
  id                   INTEGER PRIMARY KEY,
  book_id              INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  track_id             INTEGER REFERENCES tracks(id) ON DELETE CASCADE,
  title                TEXT NOT NULL,
  description          TEXT NOT NULL DEFAULT '',
  planned_payoff       TEXT NOT NULL DEFAULT '',
  status               TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','resolved','abandoned')),
  planted_chapter_id   INTEGER REFERENCES chapters(id) ON DELETE SET NULL,
  resolved_chapter_id  INTEGER REFERENCES chapters(id) ON DELETE SET NULL,
  resolution           TEXT NOT NULL DEFAULT '',
  importance           INTEGER NOT NULL DEFAULT 2 CHECK (importance BETWEEN 1 AND 5),
  created_at           TEXT NOT NULL,
  updated_at           TEXT NOT NULL
);
CREATE INDEX foreshadowing_book ON foreshadowing(book_id, status);
CREATE TABLE foreshadowing_entities (
  foreshadowing_id  INTEGER NOT NULL REFERENCES foreshadowing(id) ON DELETE CASCADE,
  entity_id         INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
  PRIMARY KEY (foreshadowing_id, entity_id)
);

CREATE TABLE change_log (
  id              INTEGER PRIMARY KEY,
  book_id         INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  track_id        INTEGER REFERENCES tracks(id) ON DELETE CASCADE,   -- an isolated track's history goes with it
  chapter_id      INTEGER REFERENCES chapters(id) ON DELETE SET NULL,
  session_id      INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
  actor           TEXT NOT NULL CHECK (actor IN ('user','agent','system')),
  target_type     TEXT NOT NULL CHECK (target_type IN ('entity','relation','appearance_set',
                    'timeline_event','memory_note','foreshadowing','chapter_summary',
                    'setting_doc','character_design')),
  target_id       INTEGER NOT NULL,   -- appearance_set / chapter_summary: chapter id
  op              TEXT NOT NULL CHECK (op IN ('create','update','delete')),
  before          TEXT CHECK (before IS NULL OR json_valid(before)),     -- snapshot (Snapshot*)
  after           TEXT CHECK (after IS NULL OR json_valid(after)),
  note            TEXT NOT NULL DEFAULT '',            -- written delta ("修为：筑基初期→筑基中期"); '' when none (spec/06 §3)
  reverts_id      INTEGER REFERENCES change_log(id) ON DELETE SET NULL,
  reverted_by_id  INTEGER REFERENCES change_log(id) ON DELETE SET NULL,
  created_at      TEXT NOT NULL
);
CREATE INDEX change_log_book ON change_log(book_id, id DESC);
CREATE INDEX change_log_target ON change_log(target_type, target_id, id DESC);
CREATE INDEX change_log_chapter ON change_log(chapter_id);

-- ───────────────────────────── Knowledge ─────────────────────────────
CREATE TABLE knowledge_scopes (
  id           INTEGER PRIMARY KEY,
  name         TEXT NOT NULL UNIQUE,
  description  TEXT NOT NULL DEFAULT '',
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL
);

CREATE TABLE book_scopes (
  book_id   INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  scope_id  INTEGER NOT NULL REFERENCES knowledge_scopes(id) ON DELETE CASCADE,
  PRIMARY KEY (book_id, scope_id)
);

CREATE TABLE research_tasks (
  id               INTEGER PRIMARY KEY,
  book_id          INTEGER REFERENCES books(id) ON DELETE CASCADE,
  scope_id         INTEGER REFERENCES knowledge_scopes(id) ON DELETE SET NULL,
  scope_name       TEXT NOT NULL,                       -- target scope name (resolved/created on run)
  topic            TEXT NOT NULL,
  requirements     TEXT NOT NULL DEFAULT '',
  language         TEXT NOT NULL,                       -- BCP-47 language of the entries (spec/07 §8)
  status           TEXT NOT NULL CHECK (status IN ('draft','ready','deferred','running','done','failed','cancelled')),
  session_id       INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
  report           TEXT NOT NULL DEFAULT '',
  entries_created  INTEGER NOT NULL DEFAULT 0,
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL
);

CREATE TABLE knowledge_entries (
  id                INTEGER PRIMARY KEY,
  scope_id          INTEGER NOT NULL REFERENCES knowledge_scopes(id) ON DELETE CASCADE,
  title             TEXT NOT NULL,
  content           TEXT NOT NULL,                      -- Markdown
  source            TEXT NOT NULL CHECK (source IN ('manual','research')),
  source_urls       TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(source_urls)),
  tags              TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags)),
  research_task_id  INTEGER REFERENCES research_tasks(id) ON DELETE SET NULL,
  created_at        TEXT NOT NULL,
  updated_at        TEXT NOT NULL,
  UNIQUE (scope_id, title)                              -- error.entry_exists / conflict.exists
);
CREATE INDEX knowledge_entries_scope ON knowledge_entries(scope_id);

CREATE TABLE knowledge_chunks (
  id            INTEGER PRIMARY KEY,
  entry_id      INTEGER NOT NULL REFERENCES knowledge_entries(id) ON DELETE CASCADE,
  scope_id      INTEGER NOT NULL REFERENCES knowledge_scopes(id) ON DELETE CASCADE,
  ord           INTEGER NOT NULL,
  heading       TEXT NOT NULL DEFAULT '',               -- nearest Markdown heading path
  text          TEXT NOT NULL,
  content_hash  TEXT NOT NULL,
  embed_state   TEXT NOT NULL DEFAULT 'pending' CHECK (embed_state IN ('pending','done','failed')),
  UNIQUE (entry_id, ord)
);
CREATE INDEX knowledge_chunks_pending ON knowledge_chunks(embed_state) WHERE embed_state = 'pending';

-- ───────────────────────────── Search index (book corpus) ─────────────────────────────
CREATE TABLE search_docs (
  id            INTEGER PRIMARY KEY,
  book_id       INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  track_id      INTEGER REFERENCES tracks(id) ON DELETE CASCADE,       -- visibility
  kind          TEXT NOT NULL CHECK (kind IN ('entity','note','foreshadowing','event','summary','passage')),
  ref_id        INTEGER NOT NULL,                       -- source row id (chapter id for summary/passage)
  ref_part      INTEGER NOT NULL DEFAULT 0,             -- passage chunk index
  title         TEXT NOT NULL DEFAULT '',
  body          TEXT NOT NULL,
  meta          TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(meta)),   -- e.g. {"pids":["p3","p7"]}
  content_hash  TEXT NOT NULL,
  embed_state   TEXT NOT NULL DEFAULT 'pending' CHECK (embed_state IN ('pending','done','failed')),
  updated_at    TEXT NOT NULL,
  UNIQUE (kind, ref_id, ref_part)
);
CREATE INDEX search_docs_book ON search_docs(book_id, kind);
CREATE INDEX search_docs_pending ON search_docs(embed_state) WHERE embed_state = 'pending';

-- rowid = search_docs.id / knowledge_chunks.id; columns hold jieba-segmented, space-joined tokens
CREATE VIRTUAL TABLE fts_book      USING fts5(title, body, tokenize = 'unicode61 remove_diacritics 2');
CREATE VIRTUAL TABLE fts_knowledge USING fts5(title, body, tokenize = 'unicode61 remove_diacritics 2');
-- Virtual tables are not covered by foreign keys. These triggers also fire for cascaded deletes
-- (book, track, chapter, entry, scope), so no index row outlives its source.
CREATE TRIGGER search_docs_fts_ad AFTER DELETE ON search_docs BEGIN
  DELETE FROM fts_book WHERE rowid = old.id;
END;
CREATE TRIGGER knowledge_chunks_fts_ad AFTER DELETE ON knowledge_chunks BEGIN
  DELETE FROM fts_knowledge WHERE rowid = old.id;
END;
-- summary and passage docs reference chapters by ref_id (no foreign key) and may be shared
-- (track_id NULL), so a cascaded chapter delete (e.g. deleting a canon-mode side track) removes them here
CREATE TRIGGER chapters_search_docs_ad AFTER DELETE ON chapters BEGIN
  DELETE FROM search_docs WHERE kind IN ('summary','passage') AND ref_id = old.id;
END;

CREATE TABLE index_meta (
  key    TEXT PRIMARY KEY,                              -- 'embedding_model', 'embedding_dim'
  value  TEXT NOT NULL
);
-- vec0 tables and their cleanup triggers are (re)created at runtime by the index manager,
-- because the dimension depends on the active embedding model (spec/07 §3):
--   CREATE VIRTUAL TABLE vec_book USING vec0(
--     doc_id INTEGER PRIMARY KEY, embedding float[{dim}] distance_metric=cosine,
--     book_id INTEGER PARTITION KEY, corpus TEXT);            -- corpus: 'canon' | 'passage'
--   CREATE VIRTUAL TABLE vec_knowledge USING vec0(
--     chunk_id INTEGER PRIMARY KEY, embedding float[{dim}] distance_metric=cosine,
--     scope_id INTEGER PARTITION KEY);
--   CREATE TRIGGER search_docs_vec_ad AFTER DELETE ON search_docs BEGIN
--     DELETE FROM vec_book WHERE doc_id = old.id; END;
--   CREATE TRIGGER knowledge_chunks_vec_ad AFTER DELETE ON knowledge_chunks BEGIN
--     DELETE FROM vec_knowledge WHERE chunk_id = old.id; END;

-- ───────────────────────────── Agents ─────────────────────────────
CREATE TABLE agent_sessions (
  id                   INTEGER PRIMARY KEY,
  book_id              INTEGER REFERENCES books(id) ON DELETE CASCADE,
  role                 TEXT NOT NULL CHECK (role IN ('conception','researcher','planner','writer','reviewer','archivist')),
  kind                 TEXT NOT NULL CHECK (kind IN ('conception','research','chapter_outline','chapter_write',
                         'chapter_revise','chapter_review','chapter_maintenance','canon_seed','chapter_resync')),
  chapter_id           INTEGER REFERENCES chapters(id) ON DELETE CASCADE,
  track_id             INTEGER REFERENCES tracks(id) ON DELETE CASCADE,
  research_task_id     INTEGER REFERENCES research_tasks(id) ON DELETE CASCADE,
  model_profile_id     INTEGER REFERENCES model_profiles(id) ON DELETE SET NULL,
  model_snapshot       TEXT NOT NULL CHECK (json_valid(model_snapshot)),     -- ModelSnapshot
  system_prompt        TEXT NOT NULL,                                         -- frozen at creation
  tools                TEXT NOT NULL CHECK (json_valid(tools)),               -- ToolDef[] frozen at creation
  status               TEXT NOT NULL CHECK (status IN ('idle','running','held','awaiting_input','paused',
                         'completed','stopped','failed','interrupted','superseded','abandoned')),
  phase                TEXT NOT NULL DEFAULT 'none'
                       CHECK (phase IN ('none','planning','drafting','revision_planning','revising')),
  granularity          TEXT CHECK (granularity IS NULL OR granularity IN ('sentence','paragraph','free','chapter')),
  auto_pause           INTEGER NOT NULL DEFAULT 0,
  pause_after_outline  INTEGER NOT NULL DEFAULT 0,
  pause_requested      INTEGER NOT NULL DEFAULT 0,
  held_tool_call_id    INTEGER REFERENCES agent_tool_calls(id) ON DELETE SET NULL,
  base_content_version INTEGER,                        -- chapters.content_version when the session started
  pending_comment      TEXT,                           -- reviewer: the author's re-review comment awaiting a report
  view_snapshot        TEXT CHECK (view_snapshot IS NULL OR json_valid(view_snapshot)),   -- ViewSnapshot
  counters             TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(counters)),          -- SessionCounters
  usage                TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(usage)),             -- Usage (cumulative)
  error                TEXT CHECK (error IS NULL OR json_valid(error)),                  -- SessionError
  parent_session_id    INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,         -- rollover / redo
  created_at           TEXT NOT NULL,
  updated_at           TEXT NOT NULL,
  finished_at          TEXT
);
CREATE INDEX agent_sessions_book ON agent_sessions(book_id, id DESC);
CREATE INDEX agent_sessions_chapter ON agent_sessions(chapter_id, id DESC);
CREATE INDEX agent_sessions_status ON agent_sessions(status);

CREATE TABLE agent_messages (
  id                INTEGER PRIMARY KEY,
  session_id        INTEGER NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
  seq               INTEGER NOT NULL,                   -- 0, 1, 2 … contiguous
  role              TEXT NOT NULL CHECK (role IN ('user','assistant')),
  kind              TEXT NOT NULL CHECK (kind IN ('kickoff','user','notice','nudge','assistant')),
  content           TEXT NOT NULL CHECK (json_valid(content)),               -- StoredBlock[]
  provider_payload  TEXT CHECK (provider_payload IS NULL OR json_valid(provider_payload)),
  request_meta      TEXT CHECK (request_meta IS NULL OR json_valid(request_meta)),     -- RequestMeta
  stop_reason       TEXT,
  usage             TEXT CHECK (usage IS NULL OR json_valid(usage)),
  created_at        TEXT NOT NULL,
  UNIQUE (session_id, seq)
);

CREATE TABLE agent_tool_calls (
  id                INTEGER PRIMARY KEY,
  session_id        INTEGER NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
  message_id        INTEGER NOT NULL REFERENCES agent_messages(id) ON DELETE CASCADE,
  call_index        INTEGER NOT NULL,                   -- order inside the assistant message
  provider_call_id  TEXT NOT NULL,
  name              TEXT NOT NULL,
  input             TEXT NOT NULL,                      -- raw argument text as received (may be invalid)
  status            TEXT NOT NULL CHECK (status IN ('pending','executing','held','awaiting_input','done','error')),
  raw_result_text   TEXT,                               -- the tool's own result
  appendices        TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(appendices)),         -- Appendix[]
  result_text       TEXT,                               -- delivered text (raw + appendices); immutable once sealed
  is_error          INTEGER NOT NULL DEFAULT 0,
  ui                TEXT CHECK (ui IS NULL OR json_valid(ui)),               -- ToolUi
  effects           TEXT CHECK (effects IS NULL OR json_valid(effects)),     -- ToolEffects (deferred)
  sealed            INTEGER NOT NULL DEFAULT 0,         -- 1 once included in a model request
  created_at        TEXT NOT NULL,
  started_at        TEXT,
  finished_at       TEXT,
  UNIQUE (message_id, call_index)
);
CREATE INDEX agent_tool_calls_session ON agent_tool_calls(session_id, id);

CREATE TABLE agent_inbox (
  id                       INTEGER PRIMARY KEY,
  session_id               INTEGER NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
  kind                     TEXT NOT NULL CHECK (kind IN ('user_message','notice')),
  content                  TEXT NOT NULL,   -- user_message: raw author text; notice: rendered text
  created_at               TEXT NOT NULL,
  consumed_at              TEXT,
  consumed_by_tool_call_id INTEGER REFERENCES agent_tool_calls(id) ON DELETE SET NULL,
  consumed_by_message_id   INTEGER REFERENCES agent_messages(id) ON DELETE SET NULL
);
CREATE INDEX agent_inbox_pending ON agent_inbox(session_id) WHERE consumed_at IS NULL;

-- ───────────────────────────── Reviews ─────────────────────────────
CREATE TABLE review_reports (
  id                  INTEGER PRIMARY KEY,
  chapter_id          INTEGER NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
  content_version     INTEGER NOT NULL,                 -- chapters.content_version reviewed
  session_id          INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
  attempt             INTEGER NOT NULL,                 -- 1, 2, … per chapter
  verdict             TEXT NOT NULL CHECK (verdict IN ('pass','revise')),
  summary             TEXT NOT NULL,
  issues              TEXT NOT NULL CHECK (json_valid(issues)),              -- ReviewIssue[]
  feedback            TEXT NOT NULL,                    -- consolidated feedback for the writer
  user_comment        TEXT NOT NULL DEFAULT '',         -- the author's comment that triggered this report
  created_at          TEXT NOT NULL
);
CREATE INDEX review_reports_chapter ON review_reports(chapter_id, id DESC);

CREATE TABLE chapter_feedback (
  id                INTEGER PRIMARY KEY,
  chapter_id        INTEGER NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
  content_version   INTEGER NOT NULL,
  review_report_id  INTEGER REFERENCES review_reports(id) ON DELETE SET NULL,
  text              TEXT NOT NULL,
  issues            TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(issues)),   -- ReviewIssue[] ticked by the author
  mode              TEXT NOT NULL CHECK (mode IN ('revise','redraft')),
  session_id        INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,  -- revision session started
  created_at        TEXT NOT NULL
);

-- ───────────────────────────── Prompts ─────────────────────────────
CREATE TABLE prompt_overrides (
  id          INTEGER PRIMARY KEY,
  scope       TEXT NOT NULL CHECK (scope IN ('global','book')),
  book_id     INTEGER REFERENCES books(id) ON DELETE CASCADE,
  key         TEXT NOT NULL,
  content     TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  CHECK ((scope = 'global' AND book_id IS NULL) OR (scope = 'book' AND book_id IS NOT NULL))
);
CREATE UNIQUE INDEX prompt_overrides_global ON prompt_overrides(key) WHERE scope = 'global';
CREATE UNIQUE INDEX prompt_overrides_book   ON prompt_overrides(book_id, key) WHERE scope = 'book';
```

## 3. Enumerations

| Enum | Values |
|---|---|
| Protocol | `openai`, `anthropic`, `mock` |
| BookStatus | `conceiving`, `active`, `archived` |
| TrackKind / CanonMode / AnchorKind | `main`,`side` / `canon`,`isolated` / `none`,`before_start`,`after_chapter`,`after_end` |
| ChapterStatus | `planned`, `drafting`, `in_review`, `revising`, `official` |
| CanonStatus | `none`, `pending`, `syncing`, `synced`, `stale`, `failed` |
| VersionKind | `draft` (draft finished), `revision` (revision finished), `official` (approved), `manual` (author snapshot), `backup` (before revise, redraft, or restore) |
| SettingDocKind | `outline`, `world`, `protagonist` |
| EntityKind | `character`, `organization`, `location`, `item`, `other` |
| EntityStatus | `active`, `inactive`, `dead`, `destroyed`, `missing`, `unknown` |
| AppearanceRole | `pov`, `major`, `minor`, `mentioned` |
| NoteKind / NoteStatus | `fact`,`world_rule`,`other` / `active`,`retired` |
| ForeshadowingStatus | `open`, `resolved`, `abandoned` |
| ChangeActor / ChangeOp | `user`,`agent`,`system` / `create`,`update`,`delete` |
| AgentRole | `conception`, `researcher`, `planner`, `writer`, `reviewer`, `archivist` |
| SessionKind | `conception`, `research`, `chapter_outline`, `chapter_write`, `chapter_revise`, `chapter_review`, `chapter_maintenance`, `canon_seed`, `chapter_resync` |
| SessionStatus | `idle`, `running`, `held`, `awaiting_input`, `paused`, `completed`, `stopped`, `failed`, `interrupted`, `superseded`, `abandoned` |
| Phase | `none`, `planning` (legacy only: a `chapter_write` session started before the outline split), `drafting`, `revision_planning`, `revising` |
| Granularity | `sentence`, `paragraph`, `free`, `chapter` |
| ToolCallStatus | `pending`, `executing`, `held`, `awaiting_input`, `done`, `error` |
| ReviewVerdict / IssueSeverity | `pass`,`revise` / `critical`,`major`,`minor` |
| FeedbackMode | `revise`, `redraft` |
| RelationStatus | `active`, `ended` |
| ChangeTargetType | `entity`, `relation`, `appearance_set`, `timeline_event`, `memory_note`, `foreshadowing`, `chapter_summary`, `setting_doc`, `character_design` |
| ProposalStatus / ProposalTargetType | `pending`,`accepted`,`rejected`,`conflict` / `setting_doc`,`character_design`,`new_character` |
| ResearchTaskStatus | `draft`, `ready`, `deferred`, `running`, `done`, `failed`, `cancelled` |
| EntrySource | `manual`, `research` |
| SearchDocKind / EmbedState | `entity`,`note`,`foreshadowing`,`event`,`summary`,`passage` / `pending`,`done`,`failed` |
| MessageRole / MessageKind | `user`,`assistant` / `kickoff`,`user`,`notice`,`nudge`,`assistant` |
| InboxKind | `user_message`, `notice` |
| PromptScope | `global`, `book` |

Every CHECK-constrained `TEXT` column above maps to one of these Rust enums.

## 4. JSON column types

TypeScript-style notation. Rust structs derive `Serialize, Deserialize, Clone, Debug, TS` with
`#[serde(rename_all = "snake_case")]`. Missing optional fields deserialize to defaults
(`#[serde(default)]`) so old rows keep loading.

```ts
// books.settings
type BookSettings = {
  granularity: Granularity;            // default "paragraph"
  auto_pause: boolean;                 // default false
  pause_after_outline: boolean;        // default true; holds the outline session's submit_outline
  target_words_min: number;            // default 3000 (UAX#29 words; CJK chars count 1 each)
  target_words_max: number;            // default 4000
  recent_summary_count: number;        // N, default 5
  midterm_window: number;              // M chapters, default 10
  auto_maintenance: boolean;           // default true
};

// books.model_overrides and app_settings["roles.defaults"]
// planner is optional; an unset planner resolves through the writer's entries (spec/03 §4).
type RoleModels = Partial<Record<AgentRole, number /* model_profile_id */>>;

// books.conception
type ConceptionState = {
  session_id?: number;
  characters_revision: number; characters_approved_revision?: number;
  topics_revision: number;     topics_approved_revision?: number;
  finalized_at?: string;
};

// chapters.content, chapter_versions.content
type ChapterContent = {
  paragraphs: { pid: string /* "p<n>" */; text: string }[];
  next_pid: number;        // next number to assign; pids are never reused within a chapter
  tail_open: boolean;      // last paragraph is unfinished (sentence/free granularity)
};

// agent_sessions.view_snapshot — what the agent last saw when the session stopped running (spec/03 §7)
type ViewSnapshot =
  | { kind: "chapter"; content: ChapterContent; outline: string; reference_notes: string }   // writer
  | { kind: "conception";                                                                     // conception
      docs: Record<SettingDocKind, { revision: number; content: string }>;
      characters: { id: number; rank: number; name: string; role_label: string; summary: string; profile: string }[];
      topics: { topic: string; requirements: string; scope: string }[] };

// agent_messages.content
type StoredBlock =
  | { type: "text"; text: string; cache?: boolean }          // cache = place a cache breakpoint after this block
  | { type: "reasoning"; text: string }                      // display only; replay uses provider_payload
  | { type: "tool_call"; call_id: string; name: string; input_text: string };

// agent_messages.provider_payload (assistant messages; exact replay material)
type ProviderPayload =
  | { protocol: "openai"; content: string | null; reasoning_content?: string; tool_calls: OpenAiToolCall[] }
  | { protocol: "anthropic"; content: unknown[] /* raw content blocks incl. thinking + signatures */ }
  | { protocol: "mock"; content: unknown };

// agent_sessions.model_snapshot (frozen at session creation; API key resolved at call time)
type ModelSnapshot = {
  provider_id: number; protocol: Protocol; base_url: string; extra_headers: Record<string,string>;
  model: string; context_window: number; max_output_tokens: number;
  options: ModelOptions;               // cache_ttl "auto" is resolved to "5m" or "1h" here (spec/04 §3)
};

type SessionCounters = {
  steps: number; tool_calls: number; retrieval_calls: number;
  consecutive_tool_errors: number; consecutive_nudges: number;
  finish_warned: boolean;              // a completion was withheld once for confirmation (spec/05 §4, §6)
};
type Usage = {
  requests: number; input_tokens: number; cached_input_tokens: number;
  cache_write_tokens: number; output_tokens: number; reasoning_tokens: number;
  last_prompt_tokens: number;          // total prompt size of the latest request (rollover check)
  estimated: boolean;                  // the provider sent no usage; counts are estimates (spec/04 §2)
};
type SessionError = { code: string; message: string; retryable: boolean; at: string };

// agent_messages.request_meta (assistant messages) — state in effect when the model generated it
type RequestMeta = { phase: Phase; granularity: Granularity | null };

// agent_tool_calls.appendices — composed into result_text at sealing (canonical order: spec/03 §5.3)
type Appendix = {
  kind: "effect_notice" | "edit_report" | "user_feedback" | "inbox";
  text: string;                        // already rendered from its prompt template
  inbox_id?: number;
};

// agent_tool_calls.effects — applied when the call is finalized (not when executed)
type ToolEffects = {
  phase_to?: Phase;                    // e.g. submit_revision_plan → "revising"; legacy submit_outline → "drafting"
  complete?: "draft" | "revision" | "review" | "maintenance" | "research" | "outline";
  await_input?: "ask_user" | "request_review";
  pending_content?: PendingContent;    // a staging writer call's text (spec/03 §5.2)
  pending_outline?: PendingOutline;    // an outline session's staged outline (spec/03 §5.2)
};

// agent_tool_calls.effects.pending_outline — what an accepted submit_outline will write
type PendingOutline = {
  outline: string;
  reference_notes: string;
  title?: string;                      // applied only while the chapter's title is still empty
};

// review_reports.issues
type ReviewIssue = {
  severity: "critical" | "major" | "minor";
  category: "canon" | "continuity" | "outline" | "character" | "logic" | "pacing" | "prose" | "other";
  pids: string[]; quote: string; problem: string; suggestion: string;
};

// setting_proposals.edits
type ProposalEdits =
  | { kind: "edits"; edits: { find: string; replace: string }[] }   // each find must match exactly once
  | { kind: "append"; text: string }
  | { kind: "create_character"; character: { name: string; role_label: string; summary: string; profile: string } };

// change_log.before / after — snapshot of the target (only domain fields, no timestamps)
type SnapshotEntity = { kind, name, aliases, summary, description, state, attributes, importance, status, is_protagonist, track_id };
type SnapshotRelation = { source_id, target_id, kind, inverse_kind, description, status, since_chapter_id, until_chapter_id, track_id };
type SnapshotTimelineEvent = { position, story_time, title, description, chapter_id, importance, entity_ids, track_id };
type SnapshotNote = { kind, content, chapter_id, importance, status, entity_ids, track_id };
type SnapshotForeshadowing = { title, description, planned_payoff, status, planted_chapter_id, resolved_chapter_id, resolution, importance, entity_ids, track_id };
type SnapshotAppearanceSet = { appearances: { entity_id: number; role: AppearanceRole; note: string }[] };
type SnapshotChapterSummary = { summary: string };
type SnapshotSettingDoc = { content: string };
type SnapshotCharacterDesign = { rank, name, role_label, summary, profile, entity_id };
```

`ModelOptions` and `ToolUi` are defined in spec/04 §5 and spec/05 §1.6.

### 4.1 `app_settings` keys

| Key | Value |
|---|---|
| `roles.defaults` | `RoleModels` |
| `book_defaults` | `BookSettings` used for new books |
| `search` | `{ provider: "none" \| "tavily" \| "deepseek" \| "searxng"; tavily_api_key?: string /* literal or env:NAME, like provider keys */; deepseek_api_key?: string /* separate secret, same literal-or-env:NAME rule */; searxng_base_url?: string; fetch_via?: "direct" \| "deepseek" /* default "direct"; "deepseek" needs deepseek_api_key */; max_results: number /* default 6 */ }` |
| `embedding` | `{ model: "multilingual-e5-small" \| "bge-small-zh-v1.5" \| "embeddinggemma-300m-q" }` (default `multilingual-e5-small`) |

## 5. Invariants (enforced by services; covered by tests)

1. Every book has exactly one main track (created with the book; never deleted).
2. Every book has exactly three `setting_docs` rows (created with the book, empty content).
3. Pids are unique within a chapter and never reused (`next_pid` only grows).
4. `chapters.content_version` increments on every content change, whether by the author or an
   agent. `word_count` is recomputed in the same transaction.
5. At most one **open** session across the **planner and writer roles** per chapter — the two
   roles form one lock group, so an open outline session excludes a write or revise session and
   the other way round (status not in `completed`, `superseded`, `abandoned`).
   At most one open reviewer session and one open archivist session per chapter. At most one
   open conception session and one open `canon_seed` session per book. At most one open
   research session per task.
6. `agent_messages.seq` is contiguous per session. Sealed tool calls (`sealed = 1`) and all
   persisted messages are immutable (append-only history, spec/03 §1.3).
7. At most one entity per book has `is_protagonist = 1`.
8. Canon rows (entities, relations, timeline events, memory notes, foreshadowing) and their
   `change_log` and `search_docs` rows with a non-NULL `track_id` reference a side track with
   `canon_mode = 'isolated'`. (`agent_sessions.track_id` is simply the session's track.)
9. `change_log` rows are immutable except for setting `reverted_by_id`.
10. Deleting an entity deletes its relations, appearances and join rows (cascade) and records one
    `delete` change with the full snapshot. Revert of that delete recreates the entity with a
    new id; the relations are not restored (documented limitation in the UI).
11. No `fts_*` or `vec_*` row exists without its `search_docs` / `knowledge_chunks` row (the
    delete triggers above; tests delete a book and a scope and count the index rows).
