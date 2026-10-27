-- 0003 — one active relation per ordered pair (spec/06 §1.2, spec/02 §2, D-143)
--
-- Why this is a new migration and not only an edit to 0001_init.sql
-- ----------------------------------------------------------------
-- spec/02 §1 permits editing 0001 while the repository has no tagged release, so 0001_init.sql
-- carries the new indexes too and a fresh database is correct from the start. That alone is not
-- enough: the migrator runs only the *pending* files, so a database that already records
-- `m0001_init` — the author's, with the duplicate relations this change is about — keeps the old,
-- `kind`-keyed indexes. There the pair rule would exist only in Rust and any path that bypasses
-- the service (a merge, a revert, a hand-written UPDATE) could still create two active relations
-- for one pair.
--
-- `kind` is data about a bond, not part of its identity: at most one **active** relation per
-- ordered `(source, target)` pair in shared canon, and one per isolated track. The pair stays
-- ordered, so `(A, B, 父亲)` and `(B, A, 嫡长子)` are two legitimate rows.
--
-- Only the two partial unique indexes change; the table and its rows are untouched, so this file
-- needs no `foreign_keys=OFF` and no table rebuild, and it runs inside the migrator's transaction
-- (a failed index creation rolls the drop back, leaving the previous indexes in place).
--
-- Creating `relations_active_shared` fails if the database already holds two active relations for
-- one pair. That is deliberate: the alternative is to pick a survivor and rewrite the author's
-- canon, which this project never does silently. `m0003_relation_pair_unique.rs` checks for those
-- pairs first and fails with the offending ids instead of SQLite's bare constraint message.

DROP INDEX IF EXISTS relations_active_shared;
DROP INDEX IF EXISTS relations_active_track;
CREATE UNIQUE INDEX relations_active_shared ON relations(source_id, target_id)
  WHERE status = 'active' AND track_id IS NULL;
CREATE UNIQUE INDEX relations_active_track ON relations(track_id, source_id, target_id)
  WHERE status = 'active' AND track_id IS NOT NULL;
