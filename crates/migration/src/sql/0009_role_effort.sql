-- 0009 — thinking effort per role (spec/02 §1–§2, spec/04 §5.1, D-162)
--
-- The model profile declares the effort levels its model accepts and the default a role that
-- chooses none gets; the role's choice lives in `RoleModels.efforts`. This file creates the two
-- columns and moves every existing profile's old option into them, so nothing changes on the wire
-- at upgrade: roles start with no choice and resolve to the profile's default, which is the exact
-- value the adapter sent before.
--
-- Both ALTERs are not idempotent, so the module runs this file in one Rust-managed transaction and
-- the guards below roll everything back together. 0001 is deliberately not edited: no later
-- migration rebuilds `model_profiles`, so a column added to 0001 would make this ALTER fail on a
-- fresh database.

CREATE TEMP TABLE role_effort_profile_count (
  before_count INTEGER NOT NULL
);
INSERT INTO role_effort_profile_count (before_count)
  SELECT count(*) FROM model_profiles;

ALTER TABLE model_profiles ADD COLUMN effort_levels TEXT NOT NULL DEFAULT '[]'
  CHECK (json_valid(effort_levels));
ALTER TABLE model_profiles ADD COLUMN default_effort TEXT;

-- Q3: an OpenAI-protocol profile with a non-blank `reasoning_effort` gets exactly that level.
-- A blank string is not a level (it never reached the wire in a useful form) and stays empty.
UPDATE model_profiles
SET effort_levels = json_array(json_extract(options, '$.reasoning_effort')),
    default_effort = json_extract(options, '$.reasoning_effort')
WHERE provider_id IN (SELECT id FROM providers WHERE protocol = 'openai')
  AND json_type(options, '$.reasoning_effort') = 'text'
  AND trim(json_extract(options, '$.reasoning_effort')) <> '';

-- The Anthropic adapter sent `high` whenever `options.effort` was absent, so that implicit value
-- becomes the explicit default. A stored value is kept as-is.
UPDATE model_profiles
SET effort_levels = json_array(coalesce(json_extract(options, '$.effort'), 'high')),
    default_effort = coalesce(json_extract(options, '$.effort'), 'high')
WHERE provider_id IN (SELECT id FROM providers WHERE protocol = 'anthropic');

-- Both top-level keys and `extra_body.reasoning_effort` leave `options` on every profile.
-- `extra_body` is merged last (spec/04 §1), so a profile keeping that key would silently override
-- the role's resolved level. A key its protocol could not send (D-132) never reached the wire, so
-- dropping it changes no request.
UPDATE model_profiles
SET options = json_remove(
  json_remove(options, '$.reasoning_effort', '$.effort'),
  '$.extra_body.reasoning_effort'
);

-- The snapshot is frozen configuration, not message history (spec/03 §1.3). An in-flight
-- Anthropic session keeps the effort the old adapter sent implicitly, and with it its prompt
-- cache; without this step its first post-upgrade request would change the top-level effort.
UPDATE agent_sessions
SET model_snapshot = json_set(model_snapshot, '$.options.effort', 'high')
WHERE json_extract(model_snapshot, '$.protocol') = 'anthropic'
  AND json_extract(model_snapshot, '$.options.effort') IS NULL;

-- Guard 1: the back-fill never adds or removes a profile.
CREATE TEMP TABLE role_effort_profile_guard (
  lost INTEGER NOT NULL CHECK (lost = 0)
);
INSERT INTO role_effort_profile_guard (lost)
  SELECT (SELECT count(*) FROM model_profiles)
       - (SELECT before_count FROM role_effort_profile_count);
DROP TABLE role_effort_profile_guard;
DROP TABLE role_effort_profile_count;

-- Guard 2: no profile's options still holds either key, at the top level or inside `extra_body`
-- (json_type distinguishes a stored JSON null from a missing path, which json_extract would
-- conflate).
CREATE TEMP TABLE role_effort_options_guard (
  stranded INTEGER NOT NULL CHECK (stranded = 0)
);
INSERT INTO role_effort_options_guard (stranded)
  SELECT count(*) FROM model_profiles
  WHERE json_type(options, '$.reasoning_effort') IS NOT NULL
     OR json_type(options, '$.effort') IS NOT NULL
     OR json_type(options, '$.extra_body.reasoning_effort') IS NOT NULL;
DROP TABLE role_effort_options_guard;

-- Guard 3: every non-null default is one of its own levels.
CREATE TEMP TABLE role_effort_default_guard (
  invalid INTEGER NOT NULL CHECK (invalid = 0)
);
INSERT INTO role_effort_default_guard (invalid)
  SELECT count(*) FROM model_profiles
  WHERE default_effort IS NOT NULL
    AND NOT EXISTS (
      SELECT 1 FROM json_each(model_profiles.effort_levels)
      WHERE json_each.value = model_profiles.default_effort
    );
DROP TABLE role_effort_default_guard;
