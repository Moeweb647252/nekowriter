-- 0002 — the `deferred` research status (spec/02 §2–§3, spec/07 §8, D-138)
--
-- Why this is a new migration and not only an edit to 0001_init.sql
-- ----------------------------------------------------------------
-- spec/02 §1 permits editing 0001 while the repository has no tagged release, and `git tag` is
-- empty, so 0001_init.sql carries the new value too: a fresh database, every test fixture and the
-- dev databases recreated by hand all get it from there. That alone is not enough. The migrator
-- records applied migrations by name in `seaql_migrations` and runs only the *pending* ones, so a
-- database that already has `m0001_init` recorded — the author's, with 8 `ready` tasks, real
-- researcher sessions and 0 knowledge entries — never sees the edited file, and every
-- `status = 'deferred'` write on it fails the old CHECK. Shipping only the 0001 edit would make the
-- feature work on a fresh database and fail on the only database that has the problem.
--
-- SQLite cannot alter a CHECK constraint in place (`PRAGMA writable_schema` would just corrupt the
-- file), so the table is rebuilt: create, copy, drop, rename — the procedure the SQLite manual
-- documents under "Making Other Kinds Of Table Schema Changes". Three details matter:
--
--   * `legacy_alter_table` stays at its default (off), so `RENAME` rewrites the references other
--     tables hold in their own DDL. Without that, `agent_sessions.research_task_id` and
--     `knowledge_entries.research_task_id` would keep pointing at a table that no longer exists.
--   * `foreign_keys` must be OFF around the rebuild. `Db::connect` sets `foreign_keys=ON` on every
--     connection and SQLite then applies `ON DELETE` actions to a DROP: dropping `research_tasks`
--     with it on silently DELETEs the researcher sessions (`ON DELETE CASCADE`) and NULLs every
--     `knowledge_entries.research_task_id` (`ON DELETE SET NULL`). The pragma is a no-op inside a
--     transaction, so the file switches it before `BEGIN`; the migrator therefore runs this
--     migration outside its own transaction (`use_transaction` is `Some(false)` in
--     `m0002_research_deferred.rs`).
--   * The row-count guard at the end turns that silent data loss into a hard failure instead of a
--     successful migration that lost the author's session history.
--
-- The pragma is restored before the guard so the connection matches every other one in the pool.

CREATE TEMP TABLE deferred_rebuild_sessions_before (n INTEGER NOT NULL);
INSERT INTO deferred_rebuild_sessions_before
  SELECT count(*) FROM agent_sessions WHERE research_task_id IS NOT NULL;

PRAGMA foreign_keys = OFF;
BEGIN;
CREATE TABLE research_tasks_deferred_rebuild (
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
INSERT INTO research_tasks_deferred_rebuild
  (id, book_id, scope_id, scope_name, topic, requirements, language, status, session_id, report,
   entries_created, created_at, updated_at)
  SELECT id, book_id, scope_id, scope_name, topic, requirements, language, status, session_id,
         report, entries_created, created_at, updated_at
  FROM research_tasks;
DROP TABLE research_tasks;
ALTER TABLE research_tasks_deferred_rebuild RENAME TO research_tasks;
COMMIT;
PRAGMA foreign_keys = ON;

CREATE TEMP TABLE deferred_rebuild_guard (lost INTEGER NOT NULL CHECK (lost = 0));
INSERT INTO deferred_rebuild_guard
  SELECT (SELECT count(*) FROM agent_sessions WHERE research_task_id IS NOT NULL)
       - (SELECT n FROM deferred_rebuild_sessions_before);
DROP TABLE deferred_rebuild_guard;
DROP TABLE deferred_rebuild_sessions_before;
