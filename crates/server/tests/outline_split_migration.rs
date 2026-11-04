//! The `planner` role and `chapter_outline` session kind in the schema (spec/02 §1–§3, D-146).
//!
//! Two things have to hold:
//!
//! 1. A fresh database accepts the new role and kind (the CHECK constraints of `0001_init.sql`).
//! 2. A database created **before** the split upgrades with every row and every session reference
//!    intact. `0004_outline_sessions` rebuilds `agent_sessions`, which ten foreign keys reference;
//!    with `foreign_keys=ON` (which `Db::connect` sets) the `DROP TABLE` would cascade-delete
//!    messages, tool calls and inbox rows and null version/proposal/change/report/feedback links.
//!
//! The fixture holds one row of every old session kind, a held tool call, a two-level parent
//! chain, inbox rows, reports, feedback, proposals, versions, change log entries and research
//! tasks — every referencing table. The broken-copy test injects an orphan foreign key and proves
//! the final `foreign_key_check` fails loudly while the data stays.

use nekowriter::db::json::{CompletionKind, PendingOutline, RoleModels, ToolEffects};
use nekowriter::testing::TestApp;
use serde_json::json;

/// The `agent_sessions` shape before 0004 (the one `0001_init.sql` created before the split).
const OLD_AGENT_SESSIONS: &str = r#"
PRAGMA foreign_keys = OFF;
DROP TABLE agent_sessions;
CREATE TABLE agent_sessions (
  id                   INTEGER PRIMARY KEY,
  book_id              INTEGER REFERENCES books(id) ON DELETE CASCADE,
  role                 TEXT NOT NULL CHECK (role IN ('conception','researcher','writer','reviewer','archivist')),
  kind                 TEXT NOT NULL CHECK (kind IN ('conception','research','chapter_write','chapter_revise',
                         'chapter_review','chapter_maintenance','canon_seed','chapter_resync')),
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
PRAGMA foreign_keys = ON;
"#;

/// Book, track, chapter and one row of every old session kind, plus every referencing child.
const FIXTURE_ROWS: &str = r#"
INSERT INTO books(id, title, writing_language, status, settings, created_at, updated_at)
VALUES (1, 'Book', 'zh-CN', 'active', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO tracks(id, book_id, kind, title, canon_mode, ord, created_at, updated_at)
VALUES (1, 1, 'main', 'Main', 'canon', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO chapters(id, book_id, track_id, ord, status, content, created_at, updated_at)
VALUES (1, 1, 1, 1, 'drafting', '{"paragraphs":[],"next_pid":1,"tail_open":false}',
        '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO research_tasks(id, book_id, scope_name, topic, language, status, created_at, updated_at)
VALUES (1, 1, 'S', 't', 'en', 'running', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

-- Every old kind (and every old role).
INSERT INTO agent_sessions
  (id, book_id, role, kind, chapter_id, track_id, research_task_id, model_snapshot, system_prompt,
   tools, status, phase, granularity, auto_pause, pause_after_outline, pause_requested,
   base_content_version, counters, usage, parent_session_id, created_at, updated_at)
VALUES
  (1, 1, 'conception',  'conception',          NULL, NULL, NULL, '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'completed', 'none', NULL, 0, 0, 0, NULL, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 1, 'researcher',  'research',            NULL, NULL, 1,    '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'running',   'none', NULL, 0, 0, 0, NULL, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (3, 1, 'writer',      'chapter_write',       1,    1,    NULL, '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'held',      'planning', 'paragraph', 0, 1, 0, 0, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (4, 1, 'writer',      'chapter_revise',      1,    1,    NULL, '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'paused',    'revision_planning', 'paragraph', 0, 0, 0, 0, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (5, 1, 'reviewer',    'chapter_review',      1,    1,    NULL, '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'completed', 'none', NULL, 0, 0, 0, 0, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (6, 1, 'archivist',   'chapter_maintenance', 1,    1,    NULL, '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'completed', 'none', NULL, 0, 0, 0, 0, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (7, 1, 'archivist',   'canon_seed',          NULL, 1,    NULL, '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'completed', 'none', NULL, 0, 0, 0, NULL, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (8, 1, 'archivist',   'chapter_resync',      1,    1,    NULL, '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'failed',    'none', NULL, 0, 0, 0, 0, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (10, 1, 'writer',     'chapter_write',       1,    1,    NULL, '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'completed', 'drafting', 'paragraph', 0, 0, 0, 0, '{}', '{}', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (11, 1, 'writer',     'chapter_write',       1,    1,    NULL, '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'running',   'drafting', 'paragraph', 0, 0, 0, 0, '{}', '{}', 10, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO agent_messages(id, session_id, seq, role, kind, content, created_at)
VALUES (1, 3, 0, 'user', 'kickoff', '[]', '2026-01-01T00:00:00Z'),
       (2, 10, 0, 'user', 'kickoff', '[]', '2026-01-01T00:00:00Z'),
       (3, 11, 0, 'user', 'kickoff', '[]', '2026-01-01T00:00:00Z');

INSERT INTO agent_tool_calls(id, session_id, message_id, call_index, provider_call_id, name, input,
                             status, created_at)
VALUES (1, 3, 1, 0, 'call_1', 'submit_outline', '{}', 'held', '2026-01-01T00:00:00Z'),
       (2, 10, 2, 0, 'call_2', 'insert_text', '{}', 'done', '2026-01-01T00:00:00Z');

UPDATE agent_sessions SET held_tool_call_id = 1 WHERE id = 3;

INSERT INTO agent_inbox(session_id, kind, content, created_at)
VALUES (3, 'user_message', 'keep going', '2026-01-01T00:00:00Z'),
       (3, 'notice', 'notice', '2026-01-01T00:00:00Z');

INSERT INTO chapter_versions(chapter_id, version_no, kind, content, word_count, session_id, created_at)
VALUES (1, 1, 'draft', '{"paragraphs":[],"next_pid":1,"tail_open":false}', 0, 10,
        '2026-01-01T00:00:00Z');

INSERT INTO setting_proposals(book_id, target_type, target_kind, edits, status, session_id, created_at)
VALUES (1, 'setting_doc', 'outline', '{"kind":"append","text":"x"}', 'pending', 7,
        '2026-01-01T00:00:00Z');

INSERT INTO change_log(book_id, track_id, session_id, actor, target_type, target_id, op, before, after, created_at)
VALUES (1, 1, 7, 'agent', 'setting_doc', 1, 'update', '{}', '{}', '2026-01-01T00:00:00Z');

INSERT INTO review_reports(chapter_id, content_version, session_id, attempt, verdict, summary, issues, feedback, created_at)
VALUES (1, 0, 5, 1, 'pass', 'ok', '[]', 'fine', '2026-01-01T00:00:00Z');

INSERT INTO chapter_feedback(chapter_id, content_version, review_report_id, text, mode, session_id, created_at)
VALUES (1, 0, 1, 'redo', 'revise', 4, '2026-01-01T00:00:00Z');

UPDATE research_tasks SET session_id = 2 WHERE id = 1;
"#;

/// The prompt overrides the migration must classify: a global old kickoff with no planner row at
/// that scope (moves), a book old kickoff with a planner row already there (stays), and the
/// retired redraft key (untouched).
const OVERRIDES: &str = r#"
INSERT INTO prompt_overrides(scope, book_id, key, content, updated_at) VALUES
  ('global', NULL, 'writer.kickoff.chapter', 'OLD GLOBAL', '2026-01-01T00:00:00Z'),
  ('book',   1,    'writer.kickoff.chapter', 'BOOK OLD',   '2026-01-01T00:00:00Z'),
  ('book',   1,    'planner.kickoff',        'BOOK NEW',   '2026-01-01T00:00:00Z'),
  ('global', NULL, 'writer.kickoff.redraft', 'REDRAFT',    '2026-01-01T00:00:00Z');
"#;

/// Builds a database whose `agent_sessions` is the pre-0004 shape with the full fixture, and whose
/// migration record stops at `m0003` — the state a database created before the split is in.
async fn pre_outline_database() -> (tempfile::TempDir, std::path::PathBuf) {
    use sea_orm::ConnectionTrait;

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("old.db");
    let db = nekowriter::db::Db::connect(&path).await.expect("connect");
    nekowriter_migration::up(db.conn()).await.expect("migrate");

    // Replace the (empty) migrated table with the pre-split shape, then fill it.
    db.conn()
        .execute_unprepared(OLD_AGENT_SESSIONS)
        .await
        .expect("the pre-split agent_sessions shape");
    db.conn()
        .execute_unprepared(FIXTURE_ROWS)
        .await
        .expect("the fixture rows");
    db.conn()
        .execute_unprepared(OVERRIDES)
        .await
        .expect("the prompt overrides");
    // Rewind the migration record: only a later file can reach this database, which is why an
    // edit to 0001 alone would not. 0006 is rewound too so its `canon_systems` column is added
    // again after 0004's rebuild; 0005 stays recorded so its seeded row is not inserted twice.
    db.conn()
        .execute_unprepared(
            "DELETE FROM seaql_migrations WHERE version IN \
             ('m0004_outline_sessions','m0006_goals_and_switches','m0010_tool_error_limit','m0012_book_agent')",
        )
        .await
        .expect("rewind to 0003");
    // The old CHECK really does refuse the new values; otherwise this test would prove nothing.
    assert!(
        db.conn()
            .execute_unprepared(
                "INSERT INTO agent_sessions(book_id, role, kind, model_snapshot, system_prompt, \
                 tools, status, created_at, updated_at) \
                 VALUES (1, 'planner', 'chapter_outline', '{\"provider_id\":0,\"protocol\":\"mock\",\"base_url\":\"\",\"extra_headers\":{},\"model\":\"\",\"context_window\":0,\"max_output_tokens\":0,\"options\":null}', '', '[]', 'completed', \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
            )
            .await
            .is_err(),
        "the pre-split CHECK must refuse the values the upgrade adds"
    );
    (dir, path)
}

async fn scalar_i64(conn: &sea_orm::DatabaseConnection, sql: &str) -> i64 {
    use sea_orm::ConnectionTrait;
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
    use sea_orm::ConnectionTrait;
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
async fn a_fresh_database_accepts_the_planner_role_and_outline_kind() {
    let app = TestApp::new().await;
    // The CHECK constraints are the real gate: a direct row proves the schema carries the values,
    // not only the Rust enums (spec/02 §2, §3).
    app.execute(
        "INSERT INTO agent_sessions(book_id, role, kind, model_snapshot, system_prompt, tools, \
         status, created_at, updated_at) \
         VALUES (NULL, 'planner', 'chapter_outline', '{\"provider_id\":0,\"protocol\":\"mock\",\"base_url\":\"\",\"extra_headers\":{},\"model\":\"\",\"context_window\":0,\"max_output_tokens\":0,\"options\":null}', '', '[]', 'completed', \
         '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .await;
    let (status, body) = app.get("/api/health").await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_upgrade_keeps_every_row_and_every_session_reference() {
    use sea_orm::ConnectionTrait;

    let (_dir, path) = pre_outline_database().await;
    let db = nekowriter::db::Db::connect(&path).await.expect("reconnect");
    nekowriter_migration::up(db.conn()).await.expect("upgrade");

    // Every row of every referencing table survived the rebuild.
    for (table, expected) in [
        ("agent_sessions", 10),
        ("agent_messages", 3),
        ("agent_tool_calls", 2),
        ("agent_inbox", 2),
        ("chapter_versions", 1),
        ("setting_proposals", 1),
        ("change_log", 1),
        ("research_tasks", 1),
        ("review_reports", 1),
        ("chapter_feedback", 1),
    ] {
        let n = scalar_i64(db.conn(), &format!("SELECT count(*) AS n FROM {table}")).await;
        assert_eq!(n, expected, "{table} row count after the rebuild");
    }
    // The self-reference and the held call survived too.
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM agent_sessions WHERE id = 11 AND parent_session_id = 10"
        )
        .await,
        1
    );
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM agent_sessions WHERE id = 3 AND status = 'held' \
             AND held_tool_call_id = 1"
        )
        .await,
        1
    );
    // Every SET NULL reference still points at its session.
    for check in [
        "SELECT count(*) AS n FROM research_tasks WHERE id = 1 AND session_id = 2",
        "SELECT count(*) AS n FROM review_reports WHERE id = 1 AND session_id = 5",
        "SELECT count(*) AS n FROM chapter_feedback WHERE id = 1 AND session_id = 4",
        "SELECT count(*) AS n FROM setting_proposals WHERE id = 1 AND session_id = 7",
        "SELECT count(*) AS n FROM change_log WHERE id = 1 AND session_id = 7",
        "SELECT count(*) AS n FROM chapter_versions WHERE id = 1 AND session_id = 10",
    ] {
        assert_eq!(scalar_i64(db.conn(), check).await, 1, "{check}");
    }
    // Every old kind and role is still there, and the new values are accepted now.
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(DISTINCT kind) AS n FROM agent_sessions"
        )
        .await,
        8
    );
    db.conn()
        .execute_unprepared(
            "INSERT INTO agent_sessions(book_id, role, kind, model_snapshot, system_prompt, tools, \
             status, created_at, updated_at) \
             VALUES (1, 'planner', 'chapter_outline', '{\"provider_id\":0,\"protocol\":\"mock\",\"base_url\":\"\",\"extra_headers\":{},\"model\":\"\",\"context_window\":0,\"max_output_tokens\":0,\"options\":null}', '', '[]', 'completed', \
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .await
        .expect("the upgraded CHECK accepts planner/chapter_outline");
    // The migration is recorded exactly once and stays repeatable.
    nekowriter_migration::up(db.conn())
        .await
        .expect("second up");
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0004_outline_sessions'"
        )
        .await,
        1
    );
    // The prompt overrides moved exactly where the rules say.
    assert_eq!(
        scalar_text(
            db.conn(),
            "SELECT content AS v FROM prompt_overrides WHERE scope = 'global' \
             AND key = 'planner.kickoff'"
        )
        .await,
        "OLD GLOBAL"
    );
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM prompt_overrides WHERE scope = 'global' \
             AND key = 'writer.kickoff.chapter'"
        )
        .await,
        0
    );
    assert_eq!(
        scalar_text(
            db.conn(),
            "SELECT content AS v FROM prompt_overrides WHERE scope = 'book' AND book_id = 1 \
             AND key = 'writer.kickoff.chapter'"
        )
        .await,
        "BOOK OLD"
    );
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM prompt_overrides WHERE key = 'writer.kickoff.redraft'"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn a_broken_copy_fails_loudly_and_keeps_its_data() {
    use sea_orm::ConnectionTrait;

    let (_dir, path) = pre_outline_database().await;
    let db = nekowriter::db::Db::connect(&path).await.expect("reconnect");
    // An orphan child row makes `PRAGMA foreign_key_check` report a row after the rebuild. The
    // FK is off for the insert so the broken copy can exist at all.
    db.conn()
        .execute_unprepared(
            "PRAGMA foreign_keys = OFF; \
             INSERT INTO agent_messages(id, session_id, seq, role, kind, content, created_at) \
             VALUES (99, 999, 0, 'user', 'notice', '[]', '2026-01-01T00:00:00Z'); \
             PRAGMA foreign_keys = ON;",
        )
        .await
        .expect("inject an orphan");

    let failed = nekowriter_migration::up(db.conn()).await;
    assert!(failed.is_err(), "the broken copy must fail loudly");
    // No data was lost by the failed attempt, and it is not recorded as applied.
    assert_eq!(
        scalar_i64(db.conn(), "SELECT count(*) AS n FROM agent_sessions").await,
        10
    );
    assert_eq!(
        scalar_i64(db.conn(), "SELECT count(*) AS n FROM agent_messages").await,
        4
    );
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM research_tasks WHERE id = 1 AND session_id = 2"
        )
        .await,
        1
    );
    assert_eq!(
        scalar_i64(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0004_outline_sessions'"
        )
        .await,
        0
    );
}

#[test]
fn tool_effects_and_role_models_round_trip_with_the_new_fields() {
    let effects = ToolEffects {
        complete: Some(CompletionKind::Outline),
        pending_outline: Some(PendingOutline {
            outline: "o".to_owned(),
            reference_notes: "n".to_owned(),
            title: Some("t".to_owned()),
        }),
        ..ToolEffects::default()
    };
    let raw = serde_json::to_string(&effects).expect("serialize");
    let back: ToolEffects = serde_json::from_str(&raw).expect("deserialize");
    assert_eq!(back, effects);

    // Old rows load: no pending_outline, no planner.
    let old: ToolEffects =
        serde_json::from_str(r#"{"phase_to":"drafting"}"#).expect("old effects JSON");
    assert_eq!(old.pending_outline, None);
    let roles: RoleModels = serde_json::from_str(r#"{"writer":3}"#).expect("old roles JSON");
    assert_eq!(roles.writer, Some(3));
    assert_eq!(roles.planner, None);
    // The new field serializes with the role name the UI uses.
    let roles = RoleModels {
        planner: Some(7),
        ..RoleModels::default()
    };
    let raw = serde_json::to_value(&roles).expect("serialize roles");
    assert_eq!(raw["planner"], json!(7));
}

/// The upgraded pre-0004 schema must be object-for-object identical to the full fresh migration
/// path. The direct 0001-vs-chain guard for the tables 0004 rebuilds lives in
/// `knowledge_first_migration::edited_0001_matches_the_chain_for_directly_created_tables`; this
/// test keeps the end-to-end migration path honest.
#[tokio::test]
async fn the_upgraded_schema_matches_a_fresh_database() {
    use sea_orm::ConnectionTrait;

    fn normalize_schema_sql(sql: &str) -> String {
        // 0004's rebuild drops the inline comments 0001 carries; comments and whitespace are not
        // schema, so compare the statements themselves (quoted identifiers normalized too).
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
                    // A rebuild leaves the table name quoted in sqlite_master; SQLite treats that
                    // identically to the unquoted fresh form, so normalize it before comparing.
                    normalize_schema_sql(&row.try_get::<String>("", "sql").unwrap_or_default()),
                ),
            )
        })
        .collect()
    }

    let (_dir, path) = pre_outline_database().await;
    let old = nekowriter::db::Db::connect(&path).await.expect("connect");
    nekowriter_migration::up(old.conn()).await.expect("upgrade");
    let upgraded = schema_dump(old.conn()).await;
    let fresh_dir = tempfile::tempdir().expect("fresh temp dir");
    let fresh = nekowriter::db::Db::connect(&fresh_dir.path().join("fresh.db"))
        .await
        .expect("fresh connect");
    // Compare against the final fresh path: 0001 now carries the 0008 additions too, so the
    // whole migration chain must reach the same shape on both paths.
    nekowriter_migration::up(fresh.conn())
        .await
        .expect("migrate the fresh database");
    let fresh = schema_dump(fresh.conn()).await;

    assert_eq!(
        upgraded.keys().collect::<Vec<_>>(),
        fresh.keys().collect::<Vec<_>>(),
        "the 0004 rebuild must leave the same object set as the fresh migration path"
    );
    for (name, object) in &upgraded {
        assert_eq!(
            Some(object),
            fresh.get(name),
            "the 0004 rebuild changed {name} on the fresh migration path"
        );
    }
}
