//! Migration 0006 — writing goals and per-book canon system switches (spec/02 §1–§5, D-150).
//!
//! Two things have to hold:
//!
//! 1. A fresh database accepts the new `change_log` / `setting_proposals` target types, the goal
//!    tables and `agent_sessions.canon_systems` (the CHECK constraints of `0001_init.sql`).
//! 2. A database created **before** 0006 upgrades with every row and every revert link intact.
//!    `0006_goals_and_switches` rebuilds `change_log` and `setting_proposals` with
//!    `PRAGMA foreign_keys` off, like 0004; the guards must fail loudly instead of losing rows.
//!
//! The fixture holds changes with both revert links, one row of every old proposal type and one
//! row of every session kind/role; the broken-copy test injects an orphan foreign key and proves
//! the final `foreign_key_check` fails while the data stays.

use nekowriter::db::json::{BookSettings, CanonSystems, GoalLevel, GoalStatus};
use nekowriter::testing::TestApp;
use sea_orm::ConnectionTrait;

/// `change_log` as 0001 created it before 0006 (the old `target_type` CHECK list).
const OLD_CHANGE_LOG: &str = r#"
DROP TABLE change_log;
CREATE TABLE change_log (
  id              INTEGER PRIMARY KEY,
  book_id         INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  track_id        INTEGER REFERENCES tracks(id) ON DELETE CASCADE,
  chapter_id      INTEGER REFERENCES chapters(id) ON DELETE SET NULL,
  session_id      INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
  actor           TEXT NOT NULL CHECK (actor IN ('user','agent','system')),
  target_type     TEXT NOT NULL CHECK (target_type IN ('entity','relation','appearance_set',
                    'timeline_event','memory_note','foreshadowing','chapter_summary',
                    'setting_doc','character_design')),
  target_id       INTEGER NOT NULL,
  op              TEXT NOT NULL CHECK (op IN ('create','update','delete')),
  before          TEXT CHECK (before IS NULL OR json_valid(before)),
  after           TEXT CHECK (after IS NULL OR json_valid(after)),
  note            TEXT NOT NULL DEFAULT '',
  reverts_id      INTEGER REFERENCES change_log(id) ON DELETE SET NULL,
  reverted_by_id  INTEGER REFERENCES change_log(id) ON DELETE SET NULL,
  created_at      TEXT NOT NULL
);
CREATE INDEX change_log_book ON change_log(book_id, id DESC);
CREATE INDEX change_log_target ON change_log(target_type, target_id, id DESC);
CREATE INDEX change_log_chapter ON change_log(chapter_id);
"#;

/// `setting_proposals` as 0001 created it before 0006.
const OLD_SETTING_PROPOSALS: &str = r#"
DROP TABLE setting_proposals;
CREATE TABLE setting_proposals (
  id           INTEGER PRIMARY KEY,
  book_id      INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  target_type  TEXT NOT NULL CHECK (target_type IN ('setting_doc','character_design','new_character')),
  target_kind  TEXT CHECK (target_kind IS NULL OR target_kind IN ('outline','world','protagonist')),
  target_id    INTEGER,
  edits        TEXT NOT NULL CHECK (json_valid(edits)),
  rationale    TEXT NOT NULL DEFAULT '',
  status       TEXT NOT NULL CHECK (status IN ('pending','accepted','rejected','conflict')),
  chapter_id   INTEGER REFERENCES chapters(id) ON DELETE SET NULL,
  session_id   INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
  created_at   TEXT NOT NULL,
  decided_at   TEXT
);
CREATE INDEX setting_proposals_book ON setting_proposals(book_id, status);
"#;

/// `agent_sessions` after 0004 and before 0006: same as 0004's rebuilt table, without the
/// `canon_systems` column 0006 adds.
const OLD_AGENT_SESSIONS: &str = r#"
DROP TABLE agent_sessions;
CREATE TABLE agent_sessions (
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
CREATE INDEX agent_sessions_book ON agent_sessions(book_id, id DESC);
CREATE INDEX agent_sessions_chapter ON agent_sessions(chapter_id, id DESC);
CREATE INDEX agent_sessions_status ON agent_sessions(status);
"#;

/// Book, track, chapter, one row of every old session kind, changes with both revert links, and
/// one row of every old proposal type.
const FIXTURE_ROWS: &str = r#"
INSERT INTO books(id, title, writing_language, status, settings, created_at, updated_at)
VALUES (1, 'Book', 'zh-CN', 'active', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO tracks(id, book_id, kind, title, canon_mode, ord, created_at, updated_at)
VALUES (1, 1, 'main', '', 'canon', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO chapters(id, book_id, track_id, ord, status, content, created_at, updated_at)
VALUES (1, 1, 1, 1, 'official', '{"paragraphs":[],"next_pid":1,"tail_open":false}',
        '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO agent_sessions
  (id, book_id, role, kind, chapter_id, track_id, model_snapshot, system_prompt, tools, status,
   phase, granularity, auto_pause, pause_after_outline, pause_requested, base_content_version,
   counters, usage, parent_session_id, created_at, updated_at)
VALUES
  (1, 1, 'conception', 'conception',         NULL, NULL, '{}', '', '[]', 'completed', 'none', NULL, 0, 0, 0, NULL, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 1, 'researcher', 'research',           NULL, NULL, '{}', '', '[]', 'running',   'none', NULL, 0, 0, 0, NULL, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (3, 1, 'planner',    'chapter_outline',    1,    1,    '{}', '', '[]', 'paused',    'none', NULL, 0, 0, 0, 0, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (4, 1, 'writer',     'chapter_write',      1,    1,    '{}', '', '[]', 'held',      'drafting', 'paragraph', 0, 0, 0, 0, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (5, 1, 'reviewer',   'chapter_review',     1,    1,    '{}', '', '[]', 'completed', 'none', NULL, 0, 0, 0, 0, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (6, 1, 'archivist',  'chapter_maintenance', 1,   1,    '{}', '', '[]', 'failed',    'none', NULL, 0, 0, 0, 0, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (7, 1, 'archivist',  'canon_seed',          NULL, 1,    '{}', '', '[]', 'completed', 'none', NULL, 0, 0, 0, NULL, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (8, 1, 'archivist',  'chapter_resync',      1,    1,    '{}', '', '[]', 'completed', 'none', NULL, 0, 0, 0, 0, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (9, 1, 'writer',     'chapter_write',       1,    1,    '{}', '', '[]', 'running',   'drafting', 'paragraph', 0, 0, 0, 0, '{}', '{}', 4, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO change_log
  (id, book_id, track_id, chapter_id, session_id, actor, target_type, target_id, op, before, after,
   note, reverts_id, reverted_by_id, created_at)
VALUES
  (1, 1, 1, 1, 2, 'agent', 'entity', 1, 'create', NULL, NULL, 'created', NULL, 2, '2026-01-01T00:00:00Z'),
  (2, 1, 1, 1, 2, 'agent', 'entity', 1, 'delete', NULL, NULL, 'deleted', 1, NULL, '2026-01-01T00:00:00Z'),
  (3, 1, 1, 1, 2, 'user', 'relation', 1, 'create', NULL, NULL, '', NULL, NULL, '2026-01-01T00:00:00Z'),
  (4, 1, NULL, NULL, NULL, 'system', 'memory_note', 1, 'update', NULL, NULL, 'note', NULL, 5, '2026-01-01T00:00:00Z'),
  (5, 1, NULL, NULL, NULL, 'system', 'memory_note', 1, 'update', NULL, NULL, '', 4, NULL, '2026-01-01T00:00:00Z'),
  (6, 1, NULL, NULL, NULL, 'user', 'appearance_set', 1, 'update', NULL, NULL, '', NULL, NULL, '2026-01-01T00:00:00Z'),
  (7, 1, NULL, NULL, NULL, 'user', 'chapter_summary', 1, 'update', NULL, NULL, '', NULL, NULL, '2026-01-01T00:00:00Z'),
  (8, 1, NULL, NULL, NULL, 'user', 'foreshadowing', 1, 'create', NULL, NULL, '', NULL, NULL, '2026-01-01T00:00:00Z'),
  (9, 1, NULL, NULL, NULL, 'user', 'timeline_event', 1, 'create', NULL, NULL, '', NULL, NULL, '2026-01-01T00:00:00Z'),
  (10, 1, NULL, NULL, NULL, 'user', 'setting_doc', 1, 'update', NULL, NULL, '', NULL, NULL, '2026-01-01T00:00:00Z'),
  (11, 1, NULL, NULL, NULL, 'user', 'character_design', 1, 'update', NULL, NULL, '', NULL, NULL, '2026-01-01T00:00:00Z');

INSERT INTO setting_proposals
  (id, book_id, target_type, target_kind, target_id, edits, rationale, status, chapter_id, session_id,
   created_at, decided_at)
VALUES
  (1, 1, 'setting_doc', 'outline', NULL, '{"kind":"append","text":"appended"}', 'why', 'pending',
   NULL, NULL, '2026-01-01T00:00:00Z', NULL),
  (2, 1, 'character_design', NULL, 1,
   '{"kind":"edits","edits":[{"find":"a","replace":"b"}]}', 'why', 'accepted', NULL, 1,
   '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z'),
  (3, 1, 'new_character', NULL, NULL,
   '{"kind":"create_character","character":{"name":"N","role_label":"","summary":"s","profile":"p"}}',
   'why', 'rejected', NULL, NULL, '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z'),
  (4, 1, 'setting_doc', 'world', NULL, '{"kind":"append","text":"x"}', 'why', 'conflict',
   NULL, NULL, '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z');
"#;

/// Builds a database whose goal objects are removed, whose two rebuilt tables have their pre-0006
/// shape, whose `agent_sessions` lacks `canon_systems`, and whose migration record stops at
/// `m0005` — the state a database created before 0006 is in.
async fn pre_goals_database() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("old.db");
    let db = nekowriter::db::Db::connect(&path).await.expect("connect");
    nekowriter_migration::up(db.conn()).await.expect("migrate");

    // Replace the 0006 objects with the pre-0006 shapes. This runs on the raw connection because
    // `PRAGMA foreign_keys = OFF` is a no-op inside the write transaction `TestApp::execute` uses.
    let shape = format!(
        "PRAGMA foreign_keys = OFF;\n\
         DROP TABLE IF EXISTS goal_node_foreshadowing;\n\
         DROP TABLE IF EXISTS goal_node_events;\n\
         DROP TABLE IF EXISTS goal_node_entities;\n\
         DROP TABLE IF EXISTS goal_nodes;\n\
         {OLD_CHANGE_LOG}\n{OLD_SETTING_PROPOSALS}\n{OLD_AGENT_SESSIONS}\n\
         PRAGMA foreign_keys = ON;"
    );
    db.conn()
        .execute_unprepared(&shape)
        .await
        .expect("the pre-0006 shapes");
    db.conn()
        .execute_unprepared(FIXTURE_ROWS)
        .await
        .expect("the fixture rows");
    // Rewind the migration record: only a later file can reach this database, which is why an
    // edit to 0001 alone would not.
    db.conn()
        .execute_unprepared(
            "DELETE FROM seaql_migrations WHERE version = 'm0006_goals_and_switches'",
        )
        .await
        .expect("rewind to 0005");
    // The old CHECKs really do refuse the new values; otherwise this test would prove nothing.
    assert!(
        db.conn()
            .execute_unprepared(
                "INSERT INTO change_log(book_id, actor, target_type, target_id, op, created_at) \
                 VALUES (1, 'user', 'goal', 1, 'create', '2026-01-01T00:00:00Z')"
            )
            .await
            .is_err(),
        "the pre-0006 change_log CHECK must refuse `goal`"
    );
    assert!(
        db.conn()
            .execute_unprepared(
                "INSERT INTO setting_proposals(book_id, target_type, edits, status, created_at) \
                 VALUES (1, 'goal_node', '{}', 'pending', '2026-01-01T00:00:00Z')"
            )
            .await
            .is_err(),
        "the pre-0006 setting_proposals CHECK must refuse `goal_node`"
    );
    (dir, path)
}

async fn scalar_i64(conn: &sea_orm::DatabaseConnection, sql: &str) -> i64 {
    let row = conn
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql,
        ))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i64>("", "n").unwrap_or(-1)
}

async fn scalar_text(conn: &sea_orm::DatabaseConnection, sql: &str) -> String {
    let row = conn
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql,
        ))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<String>("", "v").unwrap_or_default()
}

#[tokio::test]
async fn a_fresh_database_accepts_goal_rows_and_computed_systems() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id");
    // The CHECK constraints are the real gate: direct rows prove the schema carries the values,
    // not only the Rust enums (spec/02 §2, §3).
    app.execute(&format!(
        "INSERT INTO goal_nodes(id, book_id, level, title, purpose, status, conditions, \
         chapter_from, chapter_to, created_at, updated_at) VALUES \
         (1, {book_id}, 'goal', 'Main goal', 'Why', 'active', '[]', 1, 5, \
          '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    app.execute(&format!(
        "INSERT INTO goal_nodes(id, book_id, parent_id, level, title, purpose, status, conditions, \
         created_at, updated_at) VALUES \
         (2, {book_id}, 1, 'plan', 'Plan', 'Why', 'waiting', \
          '[{{\"kind\":\"chapter_at_least\",\"chapter\":3}}]', \
          '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    app.execute(&format!(
        "INSERT INTO change_log(book_id, actor, target_type, target_id, op, created_at) \
         VALUES ({book_id}, 'user', 'goal', 1, 'create', '2026-01-01T00:00:00Z')"
    ))
    .await;
    app.execute(&format!(
        "INSERT INTO setting_proposals(book_id, target_type, target_id, edits, status, created_at) \
         VALUES ({book_id}, 'goal_node', 1, '{{\"kind\":\"set_goal_status\",\"status\":\"done\"}}', \
                 'pending', '2026-01-01T00:00:00Z')"
    ))
    .await;
    app.execute(&format!(
        "INSERT INTO setting_proposals(book_id, target_type, target_id, edits, status, created_at) \
         VALUES ({book_id}, 'new_goal_task', 1, '{{\"kind\":\"create_goal_task\",\"task\":{{\"parent_id\":1,\
         \"title\":\"T\",\"purpose\":\"P\",\"chapter_from\":null,\"chapter_to\":null,\
         \"entity_ids\":[],\"event_ids\":[],\"foreshadowing_ids\":[],\"track_id\":null}}}}', \
         'pending', '2026-01-01T00:00:00Z')"
    ))
    .await;
    app.execute(&format!(
        "INSERT INTO agent_sessions(book_id, role, kind, model_snapshot, system_prompt, tools, \
         status, canon_systems, created_at, updated_at) VALUES \
         ({book_id}, 'writer', 'chapter_write', '{{}}', '', '[]', 'idle', \
          '{{\"entities\":false,\"goals\":true}}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    let (status, body) = app.get("/api/health").await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");

    // The cheap CHECK constraints catch the structural errors the service also enforces.
    for invalid in [
        // A goal cannot have a parent.
        "INSERT INTO goal_nodes(book_id, parent_id, level, title, purpose, created_at, updated_at) \
         VALUES (1, 2, 'goal', 'G', 'P', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        // `waiting` needs a condition.
        "INSERT INTO goal_nodes(book_id, level, title, purpose, status, created_at, updated_at) \
         VALUES (1, 'goal', 'G', 'P', 'waiting', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        // A blank title is refused.
        "INSERT INTO goal_nodes(book_id, level, title, purpose, created_at, updated_at) \
         VALUES (1, 'goal', '   ', 'P', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        // An inverted range is refused.
        "INSERT INTO goal_nodes(book_id, level, title, purpose, chapter_from, chapter_to, \
         created_at, updated_at) VALUES (1, 'goal', 'G', 'P', 5, 3, \
         '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        // `canon_systems` must be valid JSON when present.
        "INSERT INTO agent_sessions(book_id, role, kind, model_snapshot, system_prompt, tools, \
         status, canon_systems, created_at, updated_at) VALUES \
         (1, 'writer', 'chapter_write', '{}', '', '[]', 'idle', 'not json', \
          '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    ] {
        assert!(
            app.try_execute(invalid).await.is_err(),
            "the schema must refuse: {invalid}"
        );
    }
}

#[tokio::test]
async fn the_upgrade_keeps_every_row_and_every_revert_link() {
    let (_dir, path) = pre_goals_database().await;
    let db = nekowriter::db::Db::connect(&path).await.expect("reconnect");
    nekowriter_migration::up(db.conn())
        .await
        .expect("upgrade to 0006");

    // Every row survived the two rebuilds.
    assert_eq!(
        scalar_i64(db.conn(), "SELECT count(*) AS n FROM change_log").await,
        11
    );
    assert_eq!(
        scalar_i64(db.conn(), "SELECT count(*) AS n FROM setting_proposals").await,
        4
    );
    assert_eq!(
        scalar_i64(db.conn(), "SELECT count(*) AS n FROM agent_sessions").await,
        9
    );
    // Both revert links still point at their partner.
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM change_log WHERE id = 1 AND reverted_by_id = 2"
        )
        .await,
        1
    );
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM change_log WHERE id = 2 AND reverts_id = 1"
        )
        .await,
        1
    );
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM change_log WHERE id = 4 AND reverted_by_id = 5"
        )
        .await,
        1
    );
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM change_log WHERE id = 5 AND reverts_id = 4"
        )
        .await,
        1
    );
    // Every old target type and every old proposal type is still there, byte for byte.
    for (target_type, expected) in [
        ("entity", 2),
        ("relation", 1),
        ("appearance_set", 1),
        ("timeline_event", 1),
        ("memory_note", 2),
        ("foreshadowing", 1),
        ("chapter_summary", 1),
        ("setting_doc", 1),
        ("character_design", 1),
    ] {
        assert_eq!(
            scalar_i64(
                db.conn(),
                &format!(
                    "SELECT count(*) AS n FROM change_log WHERE target_type = '{target_type}'"
                )
            )
            .await,
            expected,
            "{target_type}"
        );
    }
    assert_eq!(
        scalar_text(
            db.conn(),
            "SELECT edits AS v FROM setting_proposals WHERE id = 2"
        )
        .await,
        "{\"kind\":\"edits\",\"edits\":[{\"find\":\"a\",\"replace\":\"b\"}]}"
    );
    assert_eq!(
        scalar_text(
            db.conn(),
            "SELECT status AS v FROM setting_proposals WHERE id = 3"
        )
        .await,
        "rejected"
    );
    // Sessions created before 0006 read `canon_systems = NULL`, which means all five systems on.
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM agent_sessions WHERE canon_systems IS NULL"
        )
        .await,
        9
    );
    // The new values are accepted now.
    db.conn()
        .execute_unprepared(
            "INSERT INTO change_log(book_id, actor, target_type, target_id, op, created_at) \
             VALUES (1, 'user', 'goal', 1, 'create', '2026-01-01T00:00:00Z')",
        )
        .await
        .expect("the upgraded change_log CHECK accepts `goal`");
    db.conn()
        .execute_unprepared(
            "INSERT INTO setting_proposals(book_id, target_type, target_id, edits, status, created_at) \
             VALUES (1, 'goal_node', 1, '{\"kind\":\"set_goal_status\",\"status\":\"done\"}', \
                     'pending', '2026-01-01T00:00:00Z')",
        )
        .await
        .expect("the upgraded setting_proposals CHECK accepts `goal_node`");
    db.conn()
        .execute_unprepared(
            "INSERT INTO goal_nodes(id, book_id, level, title, purpose, created_at, updated_at) \
             VALUES (1, 1, 'goal', 'G', 'P', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .await
        .expect("the goal table accepts a row");
    for table in [
        "goal_node_entities",
        "goal_node_events",
        "goal_node_foreshadowing",
    ] {
        assert_eq!(
            scalar_i64(db.conn(), &format!("SELECT count(*) AS n FROM {table}")).await,
            0,
            "{table} is empty"
        );
    }

    // The migration is recorded exactly once and is repeatable.
    nekowriter_migration::up(db.conn())
        .await
        .expect("second up");
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0006_goals_and_switches'"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn a_broken_copy_fails_loudly_and_keeps_its_data() {
    let (_dir, path) = pre_goals_database().await;
    let db = nekowriter::db::Db::connect(&path).await.expect("reconnect");
    // An orphan row makes `PRAGMA foreign_key_check` report a row after the rebuild. The FK is off
    // for the insert so the broken copy can exist at all.
    db.conn()
        .execute_unprepared(
            "PRAGMA foreign_keys = OFF; \
             INSERT INTO change_log(book_id, actor, target_type, target_id, op, created_at) \
             VALUES (999, 'user', 'entity', 1, 'create', '2026-01-01T00:00:00Z'); \
             PRAGMA foreign_keys = ON;",
        )
        .await
        .expect("inject an orphan");

    let failed = nekowriter_migration::up(db.conn()).await;
    assert!(failed.is_err(), "the broken copy must fail loudly");
    // No data was lost by the failed attempt, and it is not recorded as applied.
    assert_eq!(
        scalar_i64(db.conn(), "SELECT count(*) AS n FROM change_log").await,
        12
    );
    assert_eq!(
        scalar_i64(db.conn(), "SELECT count(*) AS n FROM setting_proposals").await,
        4
    );
    assert_eq!(
        scalar_i64(db.conn(), "SELECT count(*) AS n FROM agent_sessions").await,
        9
    );
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0006_goals_and_switches'"
        )
        .await,
        0
    );
}

/// `books.settings`, `app_settings["book_defaults"]` and `agent_sessions.canon_systems` without
/// the field mean all five systems on (spec/02 §4, plan §6).
#[test]
fn book_settings_without_canon_systems_defaults_to_all_on() {
    let settings: BookSettings = serde_json::from_str("{}").expect("old settings JSON");
    assert_eq!(settings.canon_systems, CanonSystems::default());
    assert!(settings.canon_systems.entities);
    assert!(settings.canon_systems.timeline);
    assert!(settings.canon_systems.memory);
    assert!(settings.canon_systems.foreshadowing);
    assert!(settings.canon_systems.goals);
}

/// A partial `canon_systems` object loads the missing fields as on (spec/02 §4).
#[test]
fn partial_canon_systems_defaults_the_missing_fields_on() {
    let settings: BookSettings =
        serde_json::from_str(r#"{"canon_systems":{"goals":false,"memory":false}}"#)
            .expect("partial settings JSON");
    assert!(!settings.canon_systems.goals);
    assert!(!settings.canon_systems.memory);
    assert!(settings.canon_systems.entities);
    assert!(settings.canon_systems.timeline);
    assert!(settings.canon_systems.foreshadowing);
    // The serialized form carries every key, so a saved settings row is explicit from now on.
    let stored = serde_json::to_value(&settings).expect("serialize settings");
    assert_eq!(stored["canon_systems"]["entities"], serde_json::json!(true));
    assert_eq!(stored["canon_systems"]["goals"], serde_json::json!(false));
}

/// The goal enums and conditions round-trip through their stored forms (spec/02 §4).
#[test]
fn goal_json_types_round_trip() {
    let condition: nekowriter::db::json::GoalCondition =
        serde_json::from_str(r#"{"kind":"entity_status_is","entity_id":7,"status":"dead"}"#)
            .expect("condition JSON");
    let raw = serde_json::to_value(&condition).expect("serialize condition");
    assert_eq!(raw["kind"], "entity_status_is");
    assert_eq!(raw["status"], "dead");

    let task = nekowriter::db::json::NewGoalTask {
        parent_id: Some(3),
        title: "T".to_owned(),
        purpose: "P".to_owned(),
        chapter_from: Some(4),
        chapter_to: Some(6),
        entity_ids: vec![1],
        event_ids: vec![2],
        foreshadowing_ids: vec![3],
        track_id: None,
    };
    let edits = nekowriter::db::json::ProposalEdits::CreateGoalTask { task };
    let raw = serde_json::to_value(&edits).expect("serialize edits");
    assert_eq!(raw["kind"], "create_goal_task");
    assert_eq!(raw["task"]["purpose"], "P");
    assert_eq!(GoalLevel::Task.as_str(), "task");
    assert_eq!(GoalStatus::Waiting.as_str(), "waiting");
}

/// The upgraded pre-0006 schema must be object-for-object identical to a fresh database built from
/// the edited `0001` (plus 0005, whose tables 0001 does not carry): a future edit to only one side
/// fails this test.
#[tokio::test]
async fn the_upgraded_schema_matches_a_fresh_database() {
    fn normalize_schema_sql(sql: &str) -> String {
        let without_comments: String = sql
            .lines()
            .map(|line| line.split("--").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        without_comments
            .replace('"', "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            // SQLite's ALTER TABLE ADD COLUMN keeps the separating punctuation's whitespace.
            .replace(" ,", ",")
            .replace(" )", ")")
    }

    async fn schema_dump(
        conn: &sea_orm::DatabaseConnection,
    ) -> std::collections::BTreeMap<String, (String, String)> {
        conn.query_all_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT type, name, sql FROM sqlite_master \
             WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' \
               AND name <> 'seaql_migrations' \
             ORDER BY type, name"
                .to_owned(),
        ))
        .await
        .expect("schema dump")
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "name").unwrap_or_default(),
                (
                    row.try_get::<String>("", "type").unwrap_or_default(),
                    normalize_schema_sql(&row.try_get::<String>("", "sql").unwrap_or_default()),
                ),
            )
        })
        .collect()
    }

    let (_dir, path) = pre_goals_database().await;
    let old = nekowriter::db::Db::connect(&path).await.expect("connect");
    nekowriter_migration::up(old.conn()).await.expect("upgrade");
    let upgraded = schema_dump(old.conn()).await;
    let fresh_dir = tempfile::tempdir().expect("fresh temp dir");
    let fresh = nekowriter::db::Db::connect(&fresh_dir.path().join("fresh.db"))
        .await
        .expect("fresh connect");
    // Build the fresh side from the edited 0001 itself, not through the migrator: applying 0006
    // there too would overwrite the rebuilt tables and hide a 0001 edit.
    fresh
        .conn()
        .execute_unprepared(include_str!("../../migration/src/sql/0001_init.sql"))
        .await
        .expect("apply the edited 0001");
    fresh
        .conn()
        .execute_unprepared(include_str!(
            "../../migration/src/sql/0005_writing_skills.sql"
        ))
        .await
        .expect("apply the writing-skills schema");
    let fresh = schema_dump(fresh.conn()).await;

    assert_eq!(
        upgraded.keys().collect::<Vec<_>>(),
        fresh.keys().collect::<Vec<_>>(),
        "0006 must leave the same object set as the edited 0001"
    );
    for (name, object) in &upgraded {
        assert_eq!(
            Some(object),
            fresh.get(name),
            "0006 and 0001 disagree about {name}"
        );
    }
}
