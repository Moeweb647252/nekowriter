-- 0013 — the timeline as a tree of history and plan (spec/02 §1–§5; D-170)
--
-- The timeline becomes one tree per book: a real, contentless root anchors entries, and every
-- pre-0013 row becomes a top-level `occurred` entry under that root. Foreshadowing gains an
-- optional anchor to an entry. The ALTERs are not idempotent, so the module runs this file in one
-- Rust-managed transaction; the guards below roll the columns, roots and reparenting back with any
-- unexpected row loss. 0001 is deliberately not edited: 0013 is the only source of the columns and
-- the root rows.

CREATE TEMP TABLE timeline_tree_entry_count (
  before_count INTEGER NOT NULL
);
INSERT INTO timeline_tree_entry_count (before_count)
  SELECT count(*) FROM timeline_events;

ALTER TABLE timeline_events ADD COLUMN parent_id INTEGER REFERENCES timeline_events(id);
ALTER TABLE timeline_events ADD COLUMN kind TEXT NOT NULL DEFAULT 'entry'
  CHECK (kind IN ('root','entry'));
ALTER TABLE timeline_events ADD COLUMN status TEXT NOT NULL DEFAULT 'occurred'
  CHECK (status IN ('occurred','planned','abandoned'));

-- One root per book. The ids start above every id the timeline table or the change log has used,
-- because a reverted delete recreates a row with a fresh id (spec/02 §5.10) and a root must never
-- take an id a future entry could reuse.
INSERT INTO timeline_events
  (id, book_id, track_id, position, story_time, title, description, chapter_id, importance,
   kind, status, created_at, updated_at)
SELECT
  base.top + ROW_NUMBER() OVER (ORDER BY b.id),
  b.id, NULL, 0, '', '', '', NULL, 2, 'root', 'occurred',
  strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
FROM books b
CROSS JOIN (
  SELECT max(
           coalesce((SELECT max(id) FROM timeline_events), 0),
           coalesce((SELECT max(target_id) FROM change_log
                     WHERE target_type = 'timeline_event'), 0)
         ) AS top
) base;

-- Every existing row is a top-level entry under its book's root. The ALTER defaults already made
-- every old row `entry` / `occurred`.
UPDATE timeline_events
   SET parent_id = (SELECT r.id FROM timeline_events r
                     WHERE r.book_id = timeline_events.book_id AND r.kind = 'root')
 WHERE kind = 'entry';

CREATE UNIQUE INDEX timeline_events_root ON timeline_events(book_id) WHERE kind = 'root';
CREATE INDEX timeline_events_parent ON timeline_events(parent_id);

ALTER TABLE foreshadowing ADD COLUMN payoff_event_id INTEGER
  REFERENCES timeline_events(id) ON DELETE SET NULL;
ALTER TABLE foreshadowing ADD COLUMN payoff_position TEXT
  CHECK (payoff_position IN ('before','after','at'));

-- Guards: no entry is lost, every book has exactly one root, and every entry names a parent.
CREATE TEMP TABLE timeline_tree_entry_guard (
  lost INTEGER NOT NULL CHECK (lost = 0)
);
INSERT INTO timeline_tree_entry_guard (lost)
  SELECT (SELECT count(*) FROM timeline_events WHERE kind = 'entry')
       - (SELECT before_count FROM timeline_tree_entry_count);
DROP TABLE timeline_tree_entry_guard;
DROP TABLE timeline_tree_entry_count;

CREATE TEMP TABLE timeline_tree_root_guard (
  bad INTEGER NOT NULL CHECK (bad = 0)
);
INSERT INTO timeline_tree_root_guard (bad)
  SELECT (SELECT count(*) FROM timeline_events WHERE kind = 'root')
       - (SELECT count(*) FROM books);
DROP TABLE timeline_tree_root_guard;

CREATE TEMP TABLE timeline_tree_parent_guard (
  bad INTEGER NOT NULL CHECK (bad = 0)
);
INSERT INTO timeline_tree_parent_guard (bad)
  SELECT count(*) FROM timeline_events WHERE kind = 'entry' AND parent_id IS NULL;
DROP TABLE timeline_tree_parent_guard;
