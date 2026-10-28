-- 0004 — the `planner` role and the `chapter_outline` session kind (spec/02 §1–§3, D-146)
--
-- `agent_sessions.role` and `.kind` are CHECK-constrained, and SQLite cannot alter a CHECK in
-- place. The table is rebuilt with the SQLite-documented create/copy/drop/rename procedure.
--
-- The danger is larger than in 0002: **ten** foreign keys reference `agent_sessions` —
--   ON DELETE CASCADE: agent_messages, agent_tool_calls, agent_inbox;
--   ON DELETE SET NULL: chapter_versions, setting_proposals, change_log, research_tasks,
--     review_reports, chapter_feedback, and the table's own parent_session_id.
-- `Db::connect` turns `foreign_keys` on for every connection, so a `DROP TABLE` with the pragma on
-- would cascade-delete every message, tool call and inbox row of every session and null every
-- version/proposal/change/report/feedback link. The pragma is a no-op inside a transaction, so the
-- file switches it before `BEGIN`; the migrator runs this migration outside its own transaction
-- (`use_transaction` is `Some(false)` in `m0004_outline_sessions.rs`).
--
-- The guards after the rebuild turn a silent loss into a hard failure: every table that references
-- a session is counted before and after, and `PRAGMA foreign_key_check` must report no row. The
-- override move is part of the same migration: `writer.kickoff.chapter` no longer means what it
-- did (it is now the drafting kickoff), so its planning instructions move to `planner.kickoff`
-- where that scope has no planner override yet.

CREATE TEMP TABLE outline_rebuild_counts (
  table_name  TEXT PRIMARY KEY,
  before_count INTEGER NOT NULL
);
INSERT INTO outline_rebuild_counts (table_name, before_count) VALUES
  ('agent_sessions',    (SELECT count(*) FROM agent_sessions)),
  ('agent_messages',    (SELECT count(*) FROM agent_messages)),
  ('agent_tool_calls',  (SELECT count(*) FROM agent_tool_calls)),
  ('agent_inbox',       (SELECT count(*) FROM agent_inbox)),
  ('chapter_versions',  (SELECT count(*) FROM chapter_versions WHERE session_id IS NOT NULL)),
  ('setting_proposals', (SELECT count(*) FROM setting_proposals WHERE session_id IS NOT NULL)),
  ('change_log',        (SELECT count(*) FROM change_log WHERE session_id IS NOT NULL)),
  ('research_tasks',    (SELECT count(*) FROM research_tasks WHERE session_id IS NOT NULL)),
  ('review_reports',    (SELECT count(*) FROM review_reports WHERE session_id IS NOT NULL)),
  ('chapter_feedback',  (SELECT count(*) FROM chapter_feedback WHERE session_id IS NOT NULL)),
  ('parent_sessions',   (SELECT count(*) FROM agent_sessions WHERE parent_session_id IS NOT NULL));

PRAGMA foreign_keys = OFF;
BEGIN;
CREATE TABLE agent_sessions_outline_rebuild (
  id                   INTEGER PRIMARY KEY,
  book_id              INTEGER REFERENCES books(id) ON DELETE CASCADE,
  role                 TEXT NOT NULL CHECK (role IN ('conception','researcher','planner','writer','reviewer','archivist')),
  kind                 TEXT NOT NULL CHECK (kind IN ('conception','research','chapter_outline','chapter_write',
                         'chapter_revise','chapter_review','chapter_maintenance','canon_seed','chapter_resync')),
  chapter_id           INTEGER REFERENCES chapters(id) ON DELETE CASCADE,
  track_id             INTEGER REFERENCES tracks(id) ON DELETE CASCADE,
  research_task_id     INTEGER REFERENCES research_tasks(id) ON DELETE CASCADE,
  model_profile_id     INTEGER REFERENCES model_profiles(id) ON DELETE SET NULL,
  model_snapshot       TEXT NOT NULL CHECK (json_valid(model_snapshot)),
  system_prompt        TEXT NOT NULL,
  tools                TEXT NOT NULL CHECK (json_valid(tools)),
  status               TEXT NOT NULL CHECK (status IN ('idle','running','held','awaiting_input','paused',
                         'completed','stopped','failed','interrupted','superseded','abandoned')),
  phase                TEXT NOT NULL DEFAULT 'none'
                       CHECK (phase IN ('none','planning','drafting','revision_planning','revising')),
  granularity          TEXT CHECK (granularity IS NULL OR granularity IN ('sentence','paragraph','free','chapter')),
  auto_pause           INTEGER NOT NULL DEFAULT 0,
  pause_after_outline  INTEGER NOT NULL DEFAULT 0,
  pause_requested      INTEGER NOT NULL DEFAULT 0,
  held_tool_call_id    INTEGER REFERENCES agent_tool_calls(id) ON DELETE SET NULL,
  base_content_version INTEGER,
  pending_comment      TEXT,
  view_snapshot        TEXT CHECK (view_snapshot IS NULL OR json_valid(view_snapshot)),
  counters             TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(counters)),
  usage                TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(usage)),
  error                TEXT CHECK (error IS NULL OR json_valid(error)),
  parent_session_id    INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
  created_at           TEXT NOT NULL,
  updated_at           TEXT NOT NULL,
  finished_at          TEXT
);
INSERT INTO agent_sessions_outline_rebuild
  (id, book_id, role, kind, chapter_id, track_id, research_task_id, model_profile_id,
   model_snapshot, system_prompt, tools, status, phase, granularity, auto_pause,
   pause_after_outline, pause_requested, held_tool_call_id, base_content_version, pending_comment,
   view_snapshot, counters, usage, error, parent_session_id, created_at, updated_at, finished_at)
  SELECT id, book_id, role, kind, chapter_id, track_id, research_task_id, model_profile_id,
         model_snapshot, system_prompt, tools, status, phase, granularity, auto_pause,
         pause_after_outline, pause_requested, held_tool_call_id, base_content_version,
         pending_comment, view_snapshot, counters, usage, error, parent_session_id, created_at,
         updated_at, finished_at
  FROM agent_sessions;
DROP TABLE agent_sessions;
ALTER TABLE agent_sessions_outline_rebuild RENAME TO agent_sessions;
CREATE INDEX agent_sessions_book ON agent_sessions(book_id, id DESC);
CREATE INDEX agent_sessions_chapter ON agent_sessions(chapter_id, id DESC);
CREATE INDEX agent_sessions_status ON agent_sessions(status);
COMMIT;
PRAGMA foreign_keys = ON;

-- Every reference must have survived the rebuild. One row per guard, each with a CHECK(lost = 0):
-- the first mismatch aborts the migration instead of leaving a database that silently lost rows.
CREATE TEMP TABLE outline_rebuild_guard (
  table_name TEXT NOT NULL,
  lost       INTEGER NOT NULL CHECK (lost = 0)
);
INSERT INTO outline_rebuild_guard (table_name, lost)
  SELECT 'agent_sessions', (SELECT count(*) FROM agent_sessions)
                        - (SELECT before_count FROM outline_rebuild_counts WHERE table_name = 'agent_sessions');
INSERT INTO outline_rebuild_guard (table_name, lost)
  SELECT 'agent_messages', (SELECT count(*) FROM agent_messages)
                        - (SELECT before_count FROM outline_rebuild_counts WHERE table_name = 'agent_messages');
INSERT INTO outline_rebuild_guard (table_name, lost)
  SELECT 'agent_tool_calls', (SELECT count(*) FROM agent_tool_calls)
                          - (SELECT before_count FROM outline_rebuild_counts WHERE table_name = 'agent_tool_calls');
INSERT INTO outline_rebuild_guard (table_name, lost)
  SELECT 'agent_inbox', (SELECT count(*) FROM agent_inbox)
                     - (SELECT before_count FROM outline_rebuild_counts WHERE table_name = 'agent_inbox');
INSERT INTO outline_rebuild_guard (table_name, lost)
  SELECT 'chapter_versions', (SELECT count(*) FROM chapter_versions WHERE session_id IS NOT NULL)
                           - (SELECT before_count FROM outline_rebuild_counts WHERE table_name = 'chapter_versions');
INSERT INTO outline_rebuild_guard (table_name, lost)
  SELECT 'setting_proposals', (SELECT count(*) FROM setting_proposals WHERE session_id IS NOT NULL)
                            - (SELECT before_count FROM outline_rebuild_counts WHERE table_name = 'setting_proposals');
INSERT INTO outline_rebuild_guard (table_name, lost)
  SELECT 'change_log', (SELECT count(*) FROM change_log WHERE session_id IS NOT NULL)
                     - (SELECT before_count FROM outline_rebuild_counts WHERE table_name = 'change_log');
INSERT INTO outline_rebuild_guard (table_name, lost)
  SELECT 'research_tasks', (SELECT count(*) FROM research_tasks WHERE session_id IS NOT NULL)
                         - (SELECT before_count FROM outline_rebuild_counts WHERE table_name = 'research_tasks');
INSERT INTO outline_rebuild_guard (table_name, lost)
  SELECT 'review_reports', (SELECT count(*) FROM review_reports WHERE session_id IS NOT NULL)
                         - (SELECT before_count FROM outline_rebuild_counts WHERE table_name = 'review_reports');
INSERT INTO outline_rebuild_guard (table_name, lost)
  SELECT 'chapter_feedback', (SELECT count(*) FROM chapter_feedback WHERE session_id IS NOT NULL)
                           - (SELECT before_count FROM outline_rebuild_counts WHERE table_name = 'chapter_feedback');
INSERT INTO outline_rebuild_guard (table_name, lost)
  SELECT 'parent_sessions', (SELECT count(*) FROM agent_sessions WHERE parent_session_id IS NOT NULL)
                          - (SELECT before_count FROM outline_rebuild_counts WHERE table_name = 'parent_sessions');
DROP TABLE outline_rebuild_guard;
DROP TABLE outline_rebuild_counts;

-- No dangling foreign key may remain after the rebuild.
CREATE TEMP TABLE outline_rebuild_fk_guard (bad INTEGER NOT NULL CHECK (bad = 0));
INSERT INTO outline_rebuild_fk_guard (bad) SELECT count(*) FROM pragma_foreign_key_check;
DROP TABLE outline_rebuild_fk_guard;

-- Move the old planning kickoff overrides to the planner's kickoff. `IS` compares NULLs as equal,
-- so the NOT EXISTS guard works for both the global row (book_id NULL) and a book row.
UPDATE prompt_overrides SET key = 'planner.kickoff'
WHERE key = 'writer.kickoff.chapter'
  AND NOT EXISTS (
    SELECT 1 FROM prompt_overrides o
    WHERE o.key = 'planner.kickoff'
      AND o.scope = prompt_overrides.scope
      AND o.book_id IS prompt_overrides.book_id
  );
