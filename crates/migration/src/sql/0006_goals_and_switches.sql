-- 0006 — writing goals and per-book canon system switches (spec/02 §1–§5, D-150)
--
-- Two CHECK-constrained tables gain values:
--   change_log.target_type       += 'goal', 'plan', 'task'
--   setting_proposals.target_type += 'goal_node', 'new_goal_task'
-- SQLite cannot alter a CHECK in place, so both are rebuilt with the create/copy/drop/rename
-- procedure of 0004. `PRAGMA foreign_keys = OFF` cannot be used here: it is a no-op inside the
-- Rust-managed transaction that makes this migration atomic, and a `PRAGMA` on a pooled connection
-- is not guaranteed to see the same connection as the following statements. Instead the rebuild is
-- safe with enforcement left on: neither rebuilt table is referenced by another table, and the
-- only self-reference in `change_log_goals_rebuild` points at itself until the final rename, so
-- dropping the old `change_log` cannot cascade into the copied rows. The migration module opens a
-- `DatabaseTransaction`: every statement below, including the guards, runs on one connection and
-- rolls back together if one fails. That keeps the non-idempotent `ALTER TABLE` retryable.
--
-- The guards turn a silent loss into a hard failure: both tables and both revert links are
-- counted before and after, and `PRAGMA foreign_key_check` must report no row.
--
-- The four goal tables are created empty, and `agent_sessions.canon_systems` is a nullable JSON
-- column added with ALTER TABLE (no rebuild). Nothing is back-filled: a `BookSettings` without the
-- field loads with all five systems on, and a session created before this migration reads NULL,
-- which means all on (spec/03 §4).

CREATE TEMP TABLE goals_rebuild_counts (
  table_name   TEXT PRIMARY KEY,
  before_count INTEGER NOT NULL
);
INSERT INTO goals_rebuild_counts (table_name, before_count) VALUES
  ('change_log',                (SELECT count(*) FROM change_log)),
  ('change_log_reverts_id',     (SELECT count(*) FROM change_log WHERE reverts_id IS NOT NULL)),
  ('change_log_reverted_by_id', (SELECT count(*) FROM change_log WHERE reverted_by_id IS NOT NULL)),
  ('setting_proposals',         (SELECT count(*) FROM setting_proposals));

CREATE TABLE change_log_goals_rebuild (
  id              INTEGER PRIMARY KEY,
  book_id         INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  track_id        INTEGER REFERENCES tracks(id) ON DELETE CASCADE,
  chapter_id      INTEGER REFERENCES chapters(id) ON DELETE SET NULL,
  session_id      INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
  actor           TEXT NOT NULL CHECK (actor IN ('user','agent','system')),
  target_type     TEXT NOT NULL CHECK (target_type IN ('entity','relation','appearance_set',
                    'timeline_event','memory_note','foreshadowing','chapter_summary',
                    'setting_doc','character_design','goal','plan','task')),
  target_id       INTEGER NOT NULL,
  op              TEXT NOT NULL CHECK (op IN ('create','update','delete')),
  before          TEXT CHECK (before IS NULL OR json_valid(before)),
  after           TEXT CHECK (after IS NULL OR json_valid(after)),
  note            TEXT NOT NULL DEFAULT '',
  reverts_id      INTEGER REFERENCES change_log_goals_rebuild(id) ON DELETE SET NULL,
  reverted_by_id  INTEGER REFERENCES change_log_goals_rebuild(id) ON DELETE SET NULL,
  created_at      TEXT NOT NULL
);
INSERT INTO change_log_goals_rebuild
  SELECT id, book_id, track_id, chapter_id, session_id, actor, target_type, target_id, op,
         before, after, note, reverts_id, reverted_by_id, created_at
  FROM change_log;
DROP TABLE change_log;
ALTER TABLE change_log_goals_rebuild RENAME TO change_log;
CREATE INDEX change_log_book ON change_log(book_id, id DESC);
CREATE INDEX change_log_target ON change_log(target_type, target_id, id DESC);
CREATE INDEX change_log_chapter ON change_log(chapter_id);

CREATE TABLE setting_proposals_goals_rebuild (
  id           INTEGER PRIMARY KEY,
  book_id      INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  target_type  TEXT NOT NULL CHECK (target_type IN ('setting_doc','character_design','new_character',
                 'goal_node','new_goal_task')),
  target_kind  TEXT CHECK (target_kind IS NULL OR target_kind IN ('outline','world','protagonist')),
  target_id    INTEGER,
  edits        TEXT NOT NULL CHECK (json_valid(edits)),
  rationale    TEXT NOT NULL DEFAULT '',
  status       TEXT NOT NULL CHECK (status IN ('pending','accepted','rejected','conflict')),
  chapter_id   INTEGER REFERENCES chapters(id) ON DELETE SET NULL,
  session_id   INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
  created_at   TEXT NOT NULL,
  decided_at   TEXT
);
INSERT INTO setting_proposals_goals_rebuild
  SELECT id, book_id, target_type, target_kind, target_id, edits, rationale, status, chapter_id,
         session_id, created_at, decided_at
  FROM setting_proposals;
DROP TABLE setting_proposals;
ALTER TABLE setting_proposals_goals_rebuild RENAME TO setting_proposals;
CREATE INDEX setting_proposals_book ON setting_proposals(book_id, status);

-- ───────────────────────────── Writing goals (spec/02 §2, spec/06 §11) ─────────────────────────────
-- `0001_init.sql` already carries this final schema for fresh databases. A fresh database still
-- runs 0006 (the migrator runs every pending file), so the create statements are IF NOT EXISTS and
-- are a no-op there; an existing database gets the tables here for the first time.
CREATE TABLE IF NOT EXISTS goal_nodes (
  id            INTEGER PRIMARY KEY,
  book_id       INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  track_id      INTEGER REFERENCES tracks(id) ON DELETE CASCADE,      -- NULL = shared (main story); T = isolated side track T
  level         TEXT NOT NULL CHECK (level IN ('goal','plan','task')),
  parent_id     INTEGER REFERENCES goal_nodes(id) ON DELETE CASCADE,  -- services refuse to delete a node with children;
                                                                       -- CASCADE only acts in a book or track delete
  title         TEXT NOT NULL CHECK (length(trim(title)) > 0),
  purpose       TEXT NOT NULL CHECK (length(trim(purpose)) > 0),       -- why the node exists (spec/06 §11.1)
  status        TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','waiting','done','dropped')),
  conditions    TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(conditions)),   -- GoalCondition[]
  chapter_from  INTEGER CHECK (chapter_from IS NULL OR chapter_from >= 1),   -- chapter numbers in the node's storyline
  chapter_to    INTEGER CHECK (chapter_to IS NULL OR chapter_to >= 1),
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  CHECK (level <> 'goal' OR parent_id IS NULL),
  CHECK (chapter_from IS NULL OR chapter_to IS NULL OR chapter_to >= chapter_from),
  CHECK (status <> 'waiting' OR json_array_length(conditions) > 0)
);
CREATE INDEX IF NOT EXISTS goal_nodes_book ON goal_nodes(book_id, status);
CREATE INDEX IF NOT EXISTS goal_nodes_parent ON goal_nodes(parent_id);
CREATE TABLE IF NOT EXISTS goal_node_entities (
  node_id    INTEGER NOT NULL REFERENCES goal_nodes(id) ON DELETE CASCADE,
  entity_id  INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
  PRIMARY KEY (node_id, entity_id)
);
CREATE INDEX IF NOT EXISTS goal_node_entities_entity ON goal_node_entities(entity_id);
CREATE TABLE IF NOT EXISTS goal_node_events (
  node_id    INTEGER NOT NULL REFERENCES goal_nodes(id) ON DELETE CASCADE,
  event_id   INTEGER NOT NULL REFERENCES timeline_events(id) ON DELETE CASCADE,
  PRIMARY KEY (node_id, event_id)
);
CREATE INDEX IF NOT EXISTS goal_node_events_event ON goal_node_events(event_id);
CREATE TABLE IF NOT EXISTS goal_node_foreshadowing (
  node_id           INTEGER NOT NULL REFERENCES goal_nodes(id) ON DELETE CASCADE,
  foreshadowing_id  INTEGER NOT NULL REFERENCES foreshadowing(id) ON DELETE CASCADE,
  PRIMARY KEY (node_id, foreshadowing_id)
);
CREATE INDEX IF NOT EXISTS goal_node_foreshadowing_target ON goal_node_foreshadowing(foreshadowing_id);

ALTER TABLE agent_sessions ADD COLUMN canon_systems TEXT CHECK (canon_systems IS NULL OR json_valid(canon_systems));

-- Every row and both revert links must have survived the rebuild. One row per guard, each with a
-- CHECK(lost = 0): the first mismatch aborts the migration instead of leaving a database that
-- silently lost rows. The guards run **inside** the transaction: the ALTER above is not
-- idempotent, so a failed guard must roll the rebuilt tables, the goal tables and the new column
-- back together and leave the pre-0006 database retryable.
CREATE TEMP TABLE goals_rebuild_guard (
  table_name TEXT NOT NULL,
  lost       INTEGER NOT NULL CHECK (lost = 0)
);
INSERT INTO goals_rebuild_guard (table_name, lost)
  SELECT 'change_log', (SELECT count(*) FROM change_log)
                     - (SELECT before_count FROM goals_rebuild_counts WHERE table_name = 'change_log');
INSERT INTO goals_rebuild_guard (table_name, lost)
  SELECT 'change_log_reverts_id', (SELECT count(*) FROM change_log WHERE reverts_id IS NOT NULL)
                               - (SELECT before_count FROM goals_rebuild_counts WHERE table_name = 'change_log_reverts_id');
INSERT INTO goals_rebuild_guard (table_name, lost)
  SELECT 'change_log_reverted_by_id', (SELECT count(*) FROM change_log WHERE reverted_by_id IS NOT NULL)
                                   - (SELECT before_count FROM goals_rebuild_counts WHERE table_name = 'change_log_reverted_by_id');
INSERT INTO goals_rebuild_guard (table_name, lost)
  SELECT 'setting_proposals', (SELECT count(*) FROM setting_proposals)
                            - (SELECT before_count FROM goals_rebuild_counts WHERE table_name = 'setting_proposals');
DROP TABLE goals_rebuild_guard;
DROP TABLE goals_rebuild_counts;

-- No dangling foreign key may remain after the rebuild.
CREATE TEMP TABLE goals_rebuild_fk_guard (bad INTEGER NOT NULL CHECK (bad = 0));
INSERT INTO goals_rebuild_fk_guard (bad) SELECT count(*) FROM pragma_foreign_key_check;
DROP TABLE goals_rebuild_fk_guard;

