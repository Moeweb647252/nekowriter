//! Migration 0008 — knowledge-first visibility and research-task ownership (spec/02 §1–§5,
//! D-158).
//!
//! The fixture is a database at the pre-0008 shape: `book_scopes` exists, `research_tasks` has
//! no `kind`/`origin`, and a row of every research status plus one research session is present.
//! The migration must preserve every task, back-fill only `draft` rows as `conception`, drop the
//! old attachment table and leave the new exclusion table empty. A fresh database is compared
//! against the upgraded one so the edited `0001` and the migration path cannot diverge.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use nekowriter::config::Config;
use nekowriter::db::Db;
use nekowriter::state::AppState;
use sea_orm::{ConnectionTrait, DbBackend, EntityTrait, Statement};
use tower::ServiceExt;

const OLD_RESEARCH_TASKS: &str = r#"
DROP TABLE IF EXISTS research_tasks;
CREATE TABLE research_tasks (
  id               INTEGER PRIMARY KEY,
  book_id          INTEGER REFERENCES books(id) ON DELETE CASCADE,
  scope_id         INTEGER REFERENCES knowledge_scopes(id) ON DELETE SET NULL,
  scope_name       TEXT NOT NULL,
  topic            TEXT NOT NULL,
  requirements     TEXT NOT NULL DEFAULT '',
  language         TEXT NOT NULL,
  status           TEXT NOT NULL CHECK (status IN ('draft','ready','deferred','running','done','failed','cancelled')),
  session_id       INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
  report           TEXT NOT NULL DEFAULT '',
  entries_created  INTEGER NOT NULL DEFAULT 0,
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL
);
"#;

const FIXTURE: &str = r#"
INSERT INTO books(id, title, idea, genre, writing_language, status, settings, model_overrides,
                  conception, created_at, updated_at)
VALUES
  (1, 'First', '', '', 'zh-CN', 'conceiving', '{}', '{}', '{}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 'Second', '', '', 'en', 'active', '{}', '{}', '{}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO knowledge_scopes(id, name, description, created_at, updated_at)
VALUES
  (1, 'History', 'Historical facts', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 'Lore', 'World lore', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (3, 'Private', 'Not attached', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

-- Attachments for the first book only; book 2 has none.
INSERT INTO book_scopes(book_id, scope_id) VALUES (1, 1), (1, 2);

INSERT INTO research_tasks
  (id, book_id, scope_id, scope_name, topic, requirements, language, status, session_id, report,
   entries_created, created_at, updated_at)
VALUES
  (1, 1, 1, 'History', 'Draft topic', '', 'zh-CN', 'draft', NULL, '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 1, 1, 'History', 'Ready topic', '', 'zh-CN', 'ready', NULL, '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (3, 1, 1, 'History', 'Deferred topic', '', 'zh-CN', 'deferred', NULL, '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (4, 1, 1, 'History', 'Running topic', '', 'zh-CN', 'running', NULL, '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (5, 1, 1, 'History', 'Done topic', '', 'zh-CN', 'done', NULL, 'done', 1,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (6, 1, 1, 'History', 'Failed topic', '', 'zh-CN', 'failed', NULL, '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (7, 1, 1, 'History', 'Cancelled topic', '', 'zh-CN', 'cancelled', NULL, '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (8, 2, 3, 'Private', 'Book two draft', 'book2 requirements', 'en', 'draft', NULL, '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (9, 1, 1, 'History', 'Second draft topic', 'second requirements', 'zh-CN', 'draft', NULL, '', 0,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO agent_sessions
  (id, book_id, role, kind, research_task_id, model_snapshot, system_prompt, tools, status, phase,
   counters, usage, view_snapshot, created_at, updated_at)
VALUES
  (1, 1, 'researcher', 'research', 1, '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'running', 'none', '{}', '{}', NULL,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 1, 'conception', 'conception', NULL, '{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}', '', '[]', 'completed', 'none', '{}', '{}',
   '{"kind":"conception","docs":{},"characters":[],"topics":[{"topic":"Draft topic","requirements":"","scope":"History"},{"topic":"Second draft topic","requirements":"second requirements","scope":"History"}]}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

UPDATE research_tasks SET session_id = 1 WHERE id = 1;

"#;

async fn scalar(conn: &sea_orm::DatabaseConnection, sql: &str) -> i64 {
    let row = conn
        .query_one_raw(Statement::from_string(DbBackend::Sqlite, sql))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i64>("", "n").unwrap_or(-1)
}

async fn table_exists(conn: &sea_orm::DatabaseConnection, table: &str) -> bool {
    scalar(
        conn,
        &format!(
            "SELECT count(*) AS n FROM sqlite_master WHERE type = 'table' AND name = '{}'",
            table
        ),
    )
    .await
        == 1
}

async fn pre_0008_database() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Db::connect(&dir.path().join("old.db"))
        .await
        .expect("connect");
    nekowriter_migration::up(db.conn())
        .await
        .expect("migrate to 0008");
    db.conn()
        .execute_unprepared(&format!(
            "PRAGMA foreign_keys = OFF; \
             DROP TABLE IF EXISTS book_scope_exclusions; \
             DROP TABLE IF EXISTS book_scopes; \
             {OLD_RESEARCH_TASKS} \
             CREATE TABLE book_scopes ( \
               book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE, \
               scope_id INTEGER NOT NULL REFERENCES knowledge_scopes(id) ON DELETE CASCADE, \
               PRIMARY KEY (book_id, scope_id) \
             ); \
             PRAGMA foreign_keys = ON;"
        ))
        .await
        .expect("pre-0008 shape");
    db.conn()
        .execute_unprepared(FIXTURE)
        .await
        .expect("fixture");
    db.conn()
        .execute_unprepared(
            "DELETE FROM seaql_migrations WHERE version IN \
             ('m0008_knowledge_first', 'm0011_check_topics')",
        )
        .await
        .expect("rewind to 0007");
    assert!(!table_exists(db.conn(), "book_scope_exclusions").await);
    assert!(table_exists(db.conn(), "book_scopes").await);
    (dir, db)
}

/// SQLite preserves comments and whitespace differently after a rebuild; compare the statements
/// themselves. `IF NOT EXISTS` is normalized because 0001 omits it and the migration uses it.
fn normalize_schema_sql(sql: &str) -> String {
    let without_comments: String = sql
        .lines()
        .map(|line| line.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    without_comments
        .replace('"', "")
        .replace("IF NOT EXISTS ", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" ,", ",")
        .replace(" )", ")")
}

/// The full object list and SQL from `sqlite_master`, as in `goals_migration::schema_dump`.
async fn schema_dump(
    conn: &sea_orm::DatabaseConnection,
) -> std::collections::BTreeMap<String, (String, String)> {
    conn.query_all_raw(Statement::from_string(
        DbBackend::Sqlite,
        "SELECT type, name, sql FROM sqlite_master \
         WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' \
           AND name <> 'seaql_migrations' ORDER BY type, name",
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

/// Everything `schema_dump`'s SQL does not expose directly: defaults and nullability, FK actions,
/// index columns and partial-index flags, plus trigger SQL (already in `schema_dump`).
async fn schema_details(conn: &sea_orm::DatabaseConnection) -> Vec<String> {
    let tables = conn
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type = 'table' \
             AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'seaql_%' ORDER BY name",
        ))
        .await
        .expect("tables");
    let mut details = Vec::new();
    for table in tables {
        let name: String = table.try_get("", "name").expect("name");
        let columns = conn
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("PRAGMA table_info(\"{name}\")"),
            ))
            .await
            .expect("columns");
        for column in columns {
            details.push(format!(
                "{name}.column:{}:{}:{}:{}:{}",
                column.try_get::<String>("", "name").expect("column"),
                column.try_get::<String>("", "type").expect("type"),
                column.try_get::<i64>("", "notnull").expect("notnull"),
                column
                    .try_get::<String>("", "dflt_value")
                    .unwrap_or_default(),
                column.try_get::<i64>("", "pk").expect("pk"),
            ));
        }
        let foreign_keys = conn
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("PRAGMA foreign_key_list(\"{name}\")"),
            ))
            .await
            .expect("foreign keys");
        for key in foreign_keys {
            details.push(format!(
                "{name}.fk:{}:{}:{}:{}:{}",
                key.try_get::<String>("", "from").expect("from"),
                key.try_get::<String>("", "table").expect("table"),
                key.try_get::<String>("", "to").expect("to"),
                key.try_get::<String>("", "on_update").expect("on_update"),
                key.try_get::<String>("", "on_delete").expect("on_delete"),
            ));
        }
        let indexes = conn
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("PRAGMA index_list(\"{name}\")"),
            ))
            .await
            .expect("indexes");
        for index in indexes {
            let index_name: String = index.try_get("", "name").expect("index name");
            details.push(format!(
                "{name}.index:{index_name}:{}:{}",
                index.try_get::<i64>("", "unique").expect("unique"),
                index.try_get::<i64>("", "partial").expect("partial"),
            ));
            let info = conn
                .query_all_raw(Statement::from_string(
                    DbBackend::Sqlite,
                    format!("PRAGMA index_info(\"{index_name}\")"),
                ))
                .await
                .expect("index info");
            for column in info {
                details.push(format!(
                    "{name}.index-column:{index_name}:{}:{}",
                    column.try_get::<i64>("", "seqno").expect("seqno"),
                    column.try_get::<String>("", "name").unwrap_or_default(),
                ));
            }
        }
    }
    details
}

#[tokio::test]
async fn migration_upgrades_a_pre_0008_database_and_preserves_research_tasks() {
    let (_dir, db) = pre_0008_database().await;
    let before = scalar(db.conn(), "SELECT count(*) AS n FROM research_tasks").await;

    nekowriter_migration::up(db.conn())
        .await
        .expect("the 0008 migration applies");

    let after = scalar(db.conn(), "SELECT count(*) AS n FROM research_tasks").await;
    assert_eq!(before, 9);
    assert_eq!(after, before, "the research_tasks count guard holds");
    assert!(
        !table_exists(db.conn(), "book_scopes").await,
        "book_scopes must be gone"
    );
    assert!(
        table_exists(db.conn(), "book_scope_exclusions").await,
        "the exclusion table must exist"
    );
    assert_eq!(
        scalar(db.conn(), "SELECT count(*) AS n FROM book_scope_exclusions").await,
        0,
        "the upgrade creates no exclusions"
    );
    assert!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM pragma_foreign_key_check"
        )
        .await
            == 0,
        "the upgraded database has no dangling foreign keys"
    );

    let rows = db
        .conn()
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT id, book_id, scope_id, scope_name, topic, kind, origin, requirements, language, \
             status, session_id, report, entries_created, created_at, updated_at \
             FROM research_tasks ORDER BY id",
        ))
        .await
        .expect("tasks");
    let expected = [
        (
            1,
            Some(1),
            Some(1),
            "History",
            "Draft topic",
            "topic",
            "conception",
            "",
            "zh-CN",
            "draft",
            Some(1),
            "",
            0,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        ),
        (
            2,
            Some(1),
            Some(1),
            "History",
            "Ready topic",
            "topic",
            "author",
            "",
            "zh-CN",
            "ready",
            None,
            "",
            0,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        ),
        (
            3,
            Some(1),
            Some(1),
            "History",
            "Deferred topic",
            "topic",
            "author",
            "",
            "zh-CN",
            "deferred",
            None,
            "",
            0,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        ),
        (
            4,
            Some(1),
            Some(1),
            "History",
            "Running topic",
            "topic",
            "author",
            "",
            "zh-CN",
            "running",
            None,
            "",
            0,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        ),
        (
            5,
            Some(1),
            Some(1),
            "History",
            "Done topic",
            "topic",
            "author",
            "",
            "zh-CN",
            "done",
            None,
            "done",
            1,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        ),
        (
            6,
            Some(1),
            Some(1),
            "History",
            "Failed topic",
            "topic",
            "author",
            "",
            "zh-CN",
            "failed",
            None,
            "",
            0,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        ),
        (
            7,
            Some(1),
            Some(1),
            "History",
            "Cancelled topic",
            "topic",
            "author",
            "",
            "zh-CN",
            "cancelled",
            None,
            "",
            0,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        ),
        (
            8,
            Some(2),
            Some(3),
            "Private",
            "Book two draft",
            "topic",
            "conception",
            "book2 requirements",
            "en",
            "draft",
            None,
            "",
            0,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        ),
        (
            9,
            Some(1),
            Some(1),
            "History",
            "Second draft topic",
            "topic",
            "conception",
            "second requirements",
            "zh-CN",
            "draft",
            None,
            "",
            0,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        ),
    ];
    assert_eq!(rows.len(), expected.len());
    for (row, expected) in rows.iter().zip(expected) {
        assert_eq!(row.try_get::<i32>("", "id").unwrap(), expected.0);
        assert_eq!(
            row.try_get::<Option<i32>>("", "book_id").unwrap(),
            expected.1
        );
        assert_eq!(
            row.try_get::<Option<i32>>("", "scope_id").unwrap(),
            expected.2
        );
        assert_eq!(row.try_get::<String>("", "scope_name").unwrap(), expected.3);
        assert_eq!(row.try_get::<String>("", "topic").unwrap(), expected.4);
        assert_eq!(row.try_get::<String>("", "kind").unwrap(), expected.5);
        assert_eq!(row.try_get::<String>("", "origin").unwrap(), expected.6);
        assert_eq!(
            row.try_get::<String>("", "requirements").unwrap(),
            expected.7
        );
        assert_eq!(row.try_get::<String>("", "language").unwrap(), expected.8);
        assert_eq!(row.try_get::<String>("", "status").unwrap(), expected.9);
        assert_eq!(
            row.try_get::<Option<i32>>("", "session_id").unwrap(),
            expected.10
        );
        assert_eq!(row.try_get::<String>("", "report").unwrap(), expected.11);
        assert_eq!(
            row.try_get::<i32>("", "entries_created").unwrap(),
            expected.12
        );
        assert_eq!(
            row.try_get::<String>("", "created_at").unwrap(),
            expected.13
        );
        assert_eq!(
            row.try_get::<String>("", "updated_at").unwrap(),
            expected.14
        );
    }
}

#[tokio::test]
async fn a_legacy_view_snapshot_survives_the_upgrade_and_reads_through_the_api() {
    let (dir, old) = pre_0008_database().await;
    nekowriter_migration::up(old.conn())
        .await
        .expect("upgrade old database");
    let row = nekowriter::db::entities::agent_sessions::Entity::find_by_id(2)
        .one(old.conn())
        .await
        .expect("session row loads")
        .expect("session 2");
    let snapshot = row
        .view_snapshot
        .as_ref()
        .expect("legacy snapshot")
        .0
        .clone();
    let nekowriter::db::json::ViewSnapshot::Conception { ref topics, .. } = snapshot else {
        panic!("conception snapshot");
    };
    assert_eq!(topics.len(), 2);
    assert_eq!(topics[0].topic, "Draft topic");
    assert_eq!(topics[1].topic, "Second draft topic");
    assert_eq!(topics[0].id, None, "legacy topics have no real id");
    assert_eq!(
        topics[0].origin,
        nekowriter::db::json::ResearchTaskOrigin::Conception
    );
    let current = nekowriter::agent::report::current_conception_state(old.conn(), 1)
        .await
        .expect("current conception state");
    let prompts = nekowriter::prompts::PromptRegistry::embedded()
        .expect("pack")
        .defaults_only();
    let report = nekowriter::agent::report::conception_report(&snapshot, &current, &prompts);
    assert!(
        report.is_empty(),
        "an unchanged legacy snapshot must not look like an author edit: {report:?}"
    );
    old.write_unprepared("UPDATE research_tasks SET requirements = 'author changed' WHERE id = 9")
        .await
        .expect("author edit");
    let current = nekowriter::agent::report::current_conception_state(old.conn(), 1)
        .await
        .expect("edited conception state");
    let report = nekowriter::agent::report::conception_report(&snapshot, &current, &prompts);
    assert!(
        !report.is_empty(),
        "a genuine author edit still shows in notice.setting_edits"
    );
    let encoded = serde_json::to_string(&snapshot).expect("serialize snapshot");
    let decoded: nekowriter::db::json::ViewSnapshot =
        serde_json::from_str(&encoded).expect("round-trip snapshot");
    assert_eq!(
        decoded, snapshot,
        "a new save keeps the normalized snapshot"
    );

    let config = Config {
        bind: "127.0.0.1:0".parse().expect("bind"),
        data_dir: Some(dir.path().to_path_buf()),
        web_dir: None,
        enable_demo_provider: false,
        allow_remote: false,
    };
    let state = AppState::new(old.clone(), config, old.vec_version().await.expect("vec"));
    let app = nekowriter::api::build_app(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/sessions/2")
                .header("host", "localhost")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/books/1/sessions")
                .header("host", "localhost")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn fresh_and_upgraded_databases_have_the_same_schema_shape() {
    let (_old_dir, old) = pre_0008_database().await;
    nekowriter_migration::up(old.conn())
        .await
        .expect("upgrade old database");
    let old_dump = schema_dump(old.conn()).await;
    let old_details = schema_details(old.conn()).await;

    let fresh_dir = tempfile::tempdir().expect("fresh dir");
    let fresh = Db::connect(&fresh_dir.path().join("fresh.db"))
        .await
        .expect("connect fresh");
    nekowriter_migration::up(fresh.conn())
        .await
        .expect("migrate fresh database");
    let fresh_dump = schema_dump(fresh.conn()).await;
    let fresh_details = schema_details(fresh.conn()).await;

    assert_eq!(
        fresh_dump.keys().collect::<Vec<_>>(),
        old_dump.keys().collect::<Vec<_>>(),
        "the edited 0001 and the 0008 upgrade path must have the same objects"
    );
    for (name, object) in &fresh_dump {
        assert_eq!(
            Some(object),
            old_dump.get(name),
            "the edited 0001 and the 0008 upgrade path disagree about {name}"
        );
    }
    assert_eq!(
        fresh_details, old_details,
        "the edited 0001 and the 0008 upgrade path disagree about defaults, FK actions or index columns"
    );
}

/// The order-insensitive semantic shape of `research_tasks`: `0001` puts `kind`/`origin` inline,
/// while the chain appends them with `ALTER TABLE` in 0008, so column order and full table SQL
/// cannot be compared byte-for-byte. Compare the column map and the normalized CHECK fragments.
async fn research_tasks_semantics(
    conn: &sea_orm::DatabaseConnection,
) -> (
    std::collections::BTreeMap<String, (String, String, String)>,
    Vec<String>,
) {
    let sql: String = conn
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'research_tasks'",
        ))
        .await
        .expect("research_tasks sql")
        .expect("research_tasks exists")
        .try_get("", "sql")
        .expect("sql");
    let mut columns = std::collections::BTreeMap::new();
    for column in conn
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA table_info(\"research_tasks\")",
        ))
        .await
        .expect("research_tasks columns")
    {
        columns.insert(
            column.try_get::<String>("", "name").expect("name"),
            (
                column.try_get::<String>("", "type").expect("type"),
                column
                    .try_get::<i64>("", "notnull")
                    .expect("notnull")
                    .to_string(),
                column
                    .try_get::<String>("", "dflt_value")
                    .unwrap_or_default(),
            ),
        );
    }
    let normalized = normalize_schema_sql(&sql);
    let mut checks = Vec::new();
    let bytes = normalized.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let Some(relative) = normalized[index..].find("CHECK") else {
            break;
        };
        let start = index + relative;
        let after = start + "CHECK".len();
        let Some(open_relative) = normalized[after..].find('(') else {
            break;
        };
        let open = after + open_relative;
        let mut depth = 0i32;
        let mut end = open;
        for (position, byte) in normalized[open..].bytes().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + position + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        checks.push(normalized[start..end].to_owned());
        index = end;
    }
    checks.sort();
    (columns, checks)
}

/// F4 guard: the edited `0001` must match the full chain for the tables it owns directly. This is
/// deliberately a different comparison from the fresh/upgraded parity check: a one-sided edit to
/// `change_log`'s CHECK in 0001 must fail here even though every migration path rebuilds the table.
#[tokio::test]
async fn edited_0001_matches_the_chain_for_directly_created_tables() {
    let raw_dir = tempfile::tempdir().expect("raw dir");
    let raw = Db::connect(&raw_dir.path().join("raw.db"))
        .await
        .expect("connect raw");
    raw.conn()
        .execute_unprepared(include_str!("../../migration/src/sql/0001_init.sql"))
        .await
        .expect("apply edited 0001");
    // 0010 appends three columns to `agent_sessions`; 0001 is append-only and intentionally not
    // edited, so the direct-table comparison applies the same addition before comparing.
    raw.conn()
        .execute_unprepared(include_str!(
            "../../migration/src/sql/0010_tool_error_limit.sql"
        ))
        .await
        .expect("apply the 0010 agent_sessions additions");
    // 0011 appends `research_tasks.check_id` and its index; apply the same addition for the
    // direct-table semantics comparison.
    raw.conn()
        .execute_unprepared(include_str!(
            "../../migration/src/sql/0011_check_topics.sql"
        ))
        .await
        .expect("apply the 0011 research_tasks additions");
    // 0012 rebuilds `agent_sessions` into its final shape; apply it to the direct table too.
    raw.conn()
        .execute_unprepared(include_str!("../../migration/src/sql/0012_book_agent.sql"))
        .await
        .expect("apply the 0012 agent_sessions rebuild");
    let raw_dump = schema_dump(raw.conn()).await;

    let chain_dir = tempfile::tempdir().expect("chain dir");
    let chain = Db::connect(&chain_dir.path().join("chain.db"))
        .await
        .expect("connect chain");
    nekowriter_migration::up(chain.conn())
        .await
        .expect("migrate chain");
    let chain_dump = schema_dump(chain.conn()).await;

    for name in ["change_log", "setting_proposals", "agent_sessions"] {
        assert_eq!(
            raw_dump.get(name),
            chain_dump.get(name),
            "0001 and the migration chain disagree about {name}"
        );
    }
    assert_eq!(
        research_tasks_semantics(raw.conn()).await,
        research_tasks_semantics(chain.conn()).await,
        "0001 and the migration chain disagree semantically about research_tasks"
    );
}
