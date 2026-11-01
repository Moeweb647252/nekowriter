-- 0008 — knowledge-first visibility and research-task ownership (spec/02 §1–§5, D-158)
--
-- Opt-out visibility replaces book_scopes with book_scope_exclusions, so every existing book
-- sees every scope after the upgrade. The two ALTERs are not idempotent, so the module runs this
-- file in one Rust-managed transaction. The research-task row count is guarded before and after
-- the back-fill; any loss rolls the whole migration back.

CREATE TEMP TABLE knowledge_first_task_count (
  before_count INTEGER NOT NULL
);
INSERT INTO knowledge_first_task_count (before_count)
  SELECT count(*) FROM research_tasks;

CREATE TABLE IF NOT EXISTS book_scope_exclusions (
  book_id   INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  scope_id  INTEGER NOT NULL REFERENCES knowledge_scopes(id) ON DELETE CASCADE,
  PRIMARY KEY (book_id, scope_id)
);
CREATE INDEX IF NOT EXISTS book_scope_exclusions_scope ON book_scope_exclusions(scope_id);

DROP TABLE IF EXISTS book_scopes;

ALTER TABLE research_tasks ADD COLUMN kind TEXT NOT NULL DEFAULT 'topic'
  CHECK (kind IN ('topic','preparation'));
ALTER TABLE research_tasks ADD COLUMN origin TEXT NOT NULL DEFAULT 'author'
  CHECK (origin IN ('author','conception'));

-- Who created an old draft is not recorded. Treat drafts as the agent's so in-flight
-- conceptions keep today's replace behaviour; every other row belongs to the author.
UPDATE research_tasks SET origin = 'conception' WHERE status = 'draft';

CREATE TEMP TABLE knowledge_first_task_guard (
  lost INTEGER NOT NULL CHECK (lost = 0)
);
INSERT INTO knowledge_first_task_guard (lost)
  SELECT (SELECT count(*) FROM research_tasks)
       - (SELECT before_count FROM knowledge_first_task_count);
DROP TABLE knowledge_first_task_guard;
DROP TABLE knowledge_first_task_count;
