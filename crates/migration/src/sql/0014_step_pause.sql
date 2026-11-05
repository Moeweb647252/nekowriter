-- 0014 — a pause that waits for the current step to finish (spec/02 §1–§2; D-173)
--
-- The new flag sits beside `pause_requested`: a step pause lets the current step finish and
-- pauses before the next model request. The ALTER is not idempotent, so the module runs this file
-- in one Rust-managed transaction; the guards roll the column back if any session row is lost or
-- any existing row is unexpectedly non-zero. 0001 is deliberately not edited.

CREATE TEMP TABLE step_pause_session_count (
  before_count INTEGER NOT NULL
);
INSERT INTO step_pause_session_count (before_count)
  SELECT count(*) FROM agent_sessions;

ALTER TABLE agent_sessions ADD COLUMN pause_after_step INTEGER NOT NULL DEFAULT 0;

CREATE TEMP TABLE step_pause_session_guard (
  bad INTEGER NOT NULL CHECK (bad = 0)
);
INSERT INTO step_pause_session_guard (bad)
  SELECT (SELECT count(*) FROM agent_sessions) - before_count
    FROM step_pause_session_count;

CREATE TEMP TABLE step_pause_value_guard (
  bad INTEGER NOT NULL CHECK (bad = 0)
);
INSERT INTO step_pause_value_guard (bad)
  SELECT count(*) FROM agent_sessions WHERE pause_after_step <> 0;

DROP TABLE step_pause_value_guard;
DROP TABLE step_pause_session_guard;
DROP TABLE step_pause_session_count;
