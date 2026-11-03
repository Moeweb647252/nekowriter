-- 0011 — knowledge-check research topics (spec/02 §1–§2, §5; spec/10 §2.5; D-168)
--
-- A knowledge check lists the gaps as ordinary topic tasks. `check_id` points each one at its
-- preparation task; deleting the check sets it to NULL, so the topic becomes an ordinary task.
-- The ALTER and the index are not idempotent, so the module runs this file in one Rust-managed
-- transaction. The row-count guard in the 0008 style rolls the column and index back with any
-- unexpected row loss. 0001 is deliberately not edited: 0011 is the only source of the column.

CREATE TEMP TABLE check_topics_task_count (
  before_count INTEGER NOT NULL
);
INSERT INTO check_topics_task_count (before_count)
  SELECT count(*) FROM research_tasks;

ALTER TABLE research_tasks ADD COLUMN check_id INTEGER
  REFERENCES research_tasks(id) ON DELETE SET NULL;
CREATE INDEX research_tasks_check ON research_tasks(check_id);

CREATE TEMP TABLE check_topics_task_guard (
  lost INTEGER NOT NULL CHECK (lost = 0)
);
INSERT INTO check_topics_task_guard (lost)
  SELECT (SELECT count(*) FROM research_tasks)
       - (SELECT before_count FROM check_topics_task_count);
DROP TABLE check_topics_task_guard;
DROP TABLE check_topics_task_count;
