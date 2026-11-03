-- 0010 — configurable agent limits (spec/03 §5.7, spec/05 §8, D-164)
--
-- The tool-error breaker was the hardcoded `TOOL_ERROR_STREAK_LIMIT = 5`; the researcher budget
-- was the hardcoded 40 calls / 25 entries. All three are now frozen onto each session at creation,
-- like `auto_pause`/`pause_after_outline`, so a running session never changes behaviour under the
-- author's feet. Existing sessions keep the old behaviour with the defaults 5/40/25.
-- 0001 is deliberately not edited: no later migration rebuilds `agent_sessions`, so a column added
-- to 0001 would make these ALTERs fail on a fresh database.

ALTER TABLE agent_sessions ADD COLUMN tool_error_streak_limit INTEGER NOT NULL DEFAULT 5
  CHECK (tool_error_streak_limit >= 0);
ALTER TABLE agent_sessions ADD COLUMN research_max_tool_calls INTEGER NOT NULL DEFAULT 40
  CHECK (research_max_tool_calls >= 1);
ALTER TABLE agent_sessions ADD COLUMN research_max_entries INTEGER NOT NULL DEFAULT 25
  CHECK (research_max_entries >= 1);
