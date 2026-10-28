-- 0005 — writing skills (design §4, D-148)
--
-- Storage is "B": a skill's body lives in the database, not in the prompt pack. Built-in rows are
-- read-only and their bodies ship empty; the author creates their own. `key` is built-in-only and
-- permanent (UNIQUE on a nullable column lets user rows carry NULL without ever colliding with a
-- later built-in key). `author` has no default, so a row is never silently reconciler-owned.
--
-- `retired` replaces deletion for built-ins: a built-in that disappears from the pack is marked
-- and stops being offered, while existing selections keep working.
--
-- No reconciler ships with this migration (design §4.2). The `basic-writing` row is seeded here
-- with a literal, empty body. When a built-in body is first shipped, that change must either seed
-- it in its own migration or add the reconciler; it is deliberately deferred until then.

CREATE TABLE writing_skills (
  id                 INTEGER PRIMARY KEY,
  key                TEXT UNIQUE,                -- built-in only; NULL for user rows
  name               TEXT NOT NULL,
  category           TEXT NOT NULL DEFAULT '',
  author             TEXT NOT NULL CHECK (author IN ('builtin','user')),
  description        TEXT NOT NULL DEFAULT '',
  body               TEXT NOT NULL DEFAULT '',
  enabled_by_default INTEGER NOT NULL DEFAULT 0,
  retired            INTEGER NOT NULL DEFAULT 0,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL
);

CREATE TABLE book_writing_skills (
  book_id    INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  skill_id   INTEGER NOT NULL REFERENCES writing_skills(id) ON DELETE CASCADE,
  created_at TEXT NOT NULL,
  PRIMARY KEY (book_id, skill_id)
);
CREATE INDEX book_writing_skills_skill ON book_writing_skills(skill_id);

INSERT INTO writing_skills
  (key, name, category, author, description, body, enabled_by_default, retired, created_at, updated_at)
VALUES
  ('basic-writing', '基础写作 / Basic Writing', '写作 / Writing', 'builtin', '', '', 0, 0,
   strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
