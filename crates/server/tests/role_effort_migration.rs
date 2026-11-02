//! Migration 0009 — thinking effort per role (spec/02 §1–§2, spec/04 §5.1, D-162).
//!
//! The fixture is a database at the pre-0009 shape: `model_profiles` has neither effort column,
//! every removed option combination is present (a non-blank OpenAI `reasoning_effort`, a stranded
//! `effort` on an OpenAI profile, a blank string, an explicit Anthropic level, an Anthropic profile
//! with neither field, and a mock profile), plus an Anthropic session snapshot whose effort is
//! null and an OpenAI one that must not move.
//!
//! The migration must move each profile's old value into `effort_levels`/`default_effort`, remove
//! both keys from `options` without touching anything else, make the old implicit Anthropic `high`
//! explicit in the snapshot, and leave a failed attempt byte-identical and retryable. A fresh
//! database is compared against the upgraded one so the 0009 path cannot diverge.

use nekowriter::db::Db;
use sea_orm::{ConnectionTrait, DbBackend, Statement};

/// `model_profiles` exactly as 0001 created it, before 0009 adds the two columns.
const OLD_MODEL_PROFILES: &str = r#"
DROP TABLE IF EXISTS model_profiles;
CREATE TABLE model_profiles (
  id                 INTEGER PRIMARY KEY,
  provider_id        INTEGER NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  display_name       TEXT NOT NULL,
  model              TEXT NOT NULL,
  context_window     INTEGER NOT NULL CHECK (context_window > 0),
  max_output_tokens  INTEGER NOT NULL CHECK (max_output_tokens > 0),
  options            TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(options)),
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL,
  UNIQUE (provider_id, display_name)
);
"#;

/// Six profiles, one per option shape the plan names, plus three session snapshots.
const FIXTURE: &str = r#"
INSERT INTO providers(id, name, protocol, base_url, api_key, extra_headers, created_at, updated_at)
VALUES
  (1, 'OpenAI', 'openai', 'https://api.openai.test/v1', NULL, '{}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 'Anthropic', 'anthropic', 'https://api.anthropic.test', NULL, '{}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (3, 'Demo', 'mock', 'mock://demo', NULL, '{}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO model_profiles
  (id, provider_id, display_name, model, context_window, max_output_tokens, options,
   created_at, updated_at)
VALUES
  (1, 1, 'OpenAI explicit', 'gpt-test', 128000, 16384,
   '{"reasoning_replay":"always","reasoning_effort":"max","max_tokens_field":"max_completion_tokens","cache_ttl":"1h"}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 1, 'OpenAI stranded', 'gpt-test-2', 128000, 16384,
   '{"effort":"high","thinking_display":"omitted"}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (3, 1, 'OpenAI blank', 'gpt-test-3', 128000, 16384,
   '{"reasoning_effort":"","temperature":0.5}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (4, 2, 'Anthropic explicit', 'claude-test', 200000, 16384,
   '{"effort":"xhigh","thinking_display":"omitted"}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (5, 2, 'Anthropic implicit', 'claude-test-2', 200000, 16384,
   '{"reasoning_replay":"never"}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (6, 3, 'Demo', 'demo-model', 200000, 16384, '{}',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO agent_sessions
  (id, role, kind, model_profile_id, model_snapshot, system_prompt, tools, status, counters,
   usage, created_at, updated_at)
VALUES
  (1, 'writer', 'chapter_write', 4,
   '{"provider_id":2,"protocol":"anthropic","base_url":"https://api.anthropic.test","extra_headers":{},"model":"claude-test","context_window":200000,"max_output_tokens":16384,"options":{"effort":null,"thinking_display":"omitted"}}',
   '', '[]', 'paused', '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, 'writer', 'chapter_write', 1,
   '{"provider_id":1,"protocol":"openai","base_url":"https://api.openai.test/v1","extra_headers":{},"model":"gpt-test","context_window":128000,"max_output_tokens":16384,"options":{"reasoning_effort":"max","reasoning_replay":"always"}}',
   '', '[]', 'paused', '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (3, 'writer', 'chapter_write', 4,
   '{"provider_id":2,"protocol":"anthropic","base_url":"https://api.anthropic.test","extra_headers":{},"model":"claude-test","context_window":200000,"max_output_tokens":16384,"options":{"effort":"low"}}',
   '', '[]', 'paused', '{}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
"#;

const PROFILE_COLUMNS: &str = "id, provider_id, display_name, model, context_window, \
                               max_output_tokens, options, effort_levels, default_effort, \
                               created_at, updated_at";
/// The pre-0009 column list, used before the migration has added the two effort columns.
const PRE_PROFILE_COLUMNS: &str = "id, provider_id, display_name, model, context_window, \
                                   max_output_tokens, options, created_at, updated_at";
const SESSION_COLUMNS: &str = "id, role, kind, model_profile_id, model_snapshot, system_prompt, \
                               tools, status, counters, usage, created_at, updated_at";

async fn scalar(conn: &sea_orm::DatabaseConnection, sql: &str) -> i64 {
    let row = conn
        .query_one_raw(Statement::from_string(DbBackend::Sqlite, sql))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i64>("", "n").unwrap_or(-1)
}

async fn column_count(conn: &sea_orm::DatabaseConnection, table: &str, column: &str) -> i64 {
    scalar(
        conn,
        &format!("SELECT count(*) AS n FROM pragma_table_info('{table}') WHERE name = '{column}'"),
    )
    .await
}

async fn table_rows(
    conn: &sea_orm::DatabaseConnection,
    table: &str,
    columns: &str,
) -> Vec<(String, String)> {
    conn.query_all_raw(Statement::from_string(
        DbBackend::Sqlite,
        format!("SELECT {columns} FROM {table} ORDER BY id"),
    ))
    .await
    .expect("table rows")
    .into_iter()
    .map(|row| {
        let mut values = Vec::new();
        for column in columns.split(',').map(str::trim) {
            let value = row.try_get::<String>("", column).unwrap_or_default();
            values.push(value);
        }
        (table.to_owned(), values.join("\u{1f}"))
    })
    .collect()
}

/// SQLite preserves comments and whitespace differently after a rebuild; compare the statements
/// themselves. `IF NOT EXISTS` is normalized because 0001 omits it and migrations may use it.
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
        }
    }
    details
}

/// Builds the pre-0009 database: apply the current chain, replace `model_profiles` with its 0001
/// shape, insert the fixture, and rewind the migration record. `PRAGMA foreign_keys = OFF` is
/// scoped to the raw connection call because `Db::connect` turns it on for every new connection.
async fn pre_0009_database() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Db::connect(&dir.path().join("old.db"))
        .await
        .expect("connect");
    nekowriter_migration::up(db.conn())
        .await
        .expect("migrate to head");
    db.conn()
        .execute_unprepared(&format!(
            "PRAGMA foreign_keys = OFF;\n{OLD_MODEL_PROFILES}\nPRAGMA foreign_keys = ON;"
        ))
        .await
        .expect("pre-0009 model_profiles shape");
    db.conn()
        .execute_unprepared(FIXTURE)
        .await
        .expect("fixture");
    db.conn()
        .execute_unprepared("DELETE FROM seaql_migrations WHERE version = 'm0009_role_effort'")
        .await
        .expect("rewind to 0008");
    assert_eq!(
        column_count(db.conn(), "model_profiles", "effort_levels").await,
        0,
        "the fixture really is pre-0009"
    );
    (dir, db)
}

fn options_without_effort_keys(value: &serde_json::Value) -> serde_json::Value {
    let mut options = value.clone();
    if let Some(object) = options.as_object_mut() {
        object.remove("reasoning_effort");
        object.remove("effort");
    }
    options
}

#[tokio::test]
async fn migration_backfills_every_profile_and_removes_the_old_keys() {
    let (_dir, db) = pre_0009_database().await;

    let before_options: Vec<serde_json::Value> = db
        .conn()
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT options FROM model_profiles ORDER BY id",
        ))
        .await
        .expect("options before")
        .into_iter()
        .map(|row| {
            serde_json::from_str(&row.try_get::<String>("", "options").expect("options"))
                .expect("valid options JSON")
        })
        .collect();
    let before_count = scalar(db.conn(), "SELECT count(*) AS n FROM model_profiles").await;

    nekowriter_migration::up(db.conn())
        .await
        .expect("the 0009 migration applies");

    assert_eq!(
        scalar(db.conn(), "SELECT count(*) AS n FROM model_profiles").await,
        before_count,
        "the profile count guard holds"
    );
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM pragma_foreign_key_check"
        )
        .await,
        0,
        "the upgraded database has no dangling foreign keys"
    );

    let rows = db
        .conn()
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            format!("SELECT {PROFILE_COLUMNS} FROM model_profiles ORDER BY id"),
        ))
        .await
        .expect("profiles");
    let expected = [
        (
            1,
            "OpenAI explicit",
            vec!["max".to_owned()],
            Some("max".to_owned()),
        ),
        (2, "OpenAI stranded", Vec::new(), None),
        (3, "OpenAI blank", Vec::new(), None),
        (
            4,
            "Anthropic explicit",
            vec!["xhigh".to_owned()],
            Some("xhigh".to_owned()),
        ),
        (
            5,
            "Anthropic implicit",
            vec!["high".to_owned()],
            Some("high".to_owned()),
        ),
        (6, "Demo", Vec::new(), None),
    ];
    assert_eq!(rows.len(), expected.len());
    for (index, (row, expected)) in rows.iter().zip(expected).enumerate() {
        assert_eq!(row.try_get::<i32>("", "id").expect("id"), expected.0);
        assert_eq!(
            row.try_get::<String>("", "display_name").expect("name"),
            expected.1
        );
        let levels: Vec<String> =
            serde_json::from_str(&row.try_get::<String>("", "effort_levels").expect("levels"))
                .expect("levels JSON");
        assert_eq!(levels, expected.2, "profile {} levels", expected.0);
        assert_eq!(
            row.try_get::<Option<String>>("", "default_effort")
                .expect("default"),
            expected.3,
            "profile {} default",
            expected.0
        );

        // No options blob still holds either removed key, and every other option is byte-equal
        // after decoding.
        let options: serde_json::Value =
            serde_json::from_str(&row.try_get::<String>("", "options").expect("options"))
                .expect("options JSON");
        assert!(options.get("reasoning_effort").is_none(), "{}", expected.0);
        assert!(options.get("effort").is_none(), "{}", expected.0);
        assert_eq!(
            options,
            options_without_effort_keys(&before_options[index]),
            "profile {} lost an unrelated option",
            expected.0
        );
    }
}

#[tokio::test]
async fn migration_makes_the_implicit_anthropic_effort_explicit_in_snapshots() {
    let (_dir, db) = pre_0009_database().await;
    let before: Vec<serde_json::Value> = db
        .conn()
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT model_snapshot FROM agent_sessions ORDER BY id",
        ))
        .await
        .expect("snapshots before")
        .into_iter()
        .map(|row| {
            serde_json::from_str(
                &row.try_get::<String>("", "model_snapshot")
                    .expect("snapshot"),
            )
            .expect("snapshot JSON")
        })
        .collect();

    nekowriter_migration::up(db.conn()).await.expect("migrate");

    let after: Vec<serde_json::Value> = db
        .conn()
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT model_snapshot FROM agent_sessions ORDER BY id",
        ))
        .await
        .expect("snapshots after")
        .into_iter()
        .map(|row| {
            serde_json::from_str(
                &row.try_get::<String>("", "model_snapshot")
                    .expect("snapshot"),
            )
            .expect("snapshot JSON")
        })
        .collect();

    // Session 1: the Anthropic snapshot whose effort was null gets the old implicit high.
    let mut expected = before[0].clone();
    expected["options"]["effort"] = serde_json::json!("high");
    assert_eq!(after[0], expected, "the null Anthropic effort becomes high");
    // Session 2: the OpenAI snapshot is byte-identical.
    assert_eq!(after[1], before[1], "the OpenAI snapshot must not change");
    // Session 3: an Anthropic snapshot that already had a level keeps it.
    assert_eq!(after[2], before[2], "an explicit effort must not change");

    // The rest of the snapshot (provider, model, cache hints, other options) is untouched.
    assert_eq!(
        after[0]["options"]["thinking_display"],
        serde_json::json!("omitted")
    );
}

#[tokio::test]
async fn a_fresh_database_gets_each_column_exactly_once() {
    let fresh_dir = tempfile::tempdir().expect("fresh dir");
    let fresh = Db::connect(&fresh_dir.path().join("fresh.db"))
        .await
        .expect("connect fresh");
    nekowriter_migration::up(fresh.conn())
        .await
        .expect("migrate fresh database");

    assert_eq!(
        column_count(fresh.conn(), "model_profiles", "effort_levels").await,
        1,
        "0009 is the only source of effort_levels"
    );
    assert_eq!(
        column_count(fresh.conn(), "model_profiles", "default_effort").await,
        1,
        "0009 is the only source of default_effort"
    );
    let levels_type: String = fresh
        .conn()
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT type FROM pragma_table_info('model_profiles') WHERE name = 'effort_levels'",
        ))
        .await
        .expect("column type")
        .expect("effort_levels exists")
        .try_get("", "type")
        .expect("type");
    assert_eq!(levels_type, "TEXT");
    let default_not_null = scalar(
        fresh.conn(),
        "SELECT \"notnull\" AS n FROM pragma_table_info('model_profiles') WHERE name = 'default_effort'",
    )
    .await;
    assert_eq!(default_not_null, 0, "default_effort is nullable");
}

#[tokio::test]
async fn fresh_and_upgraded_databases_have_the_same_schema_shape() {
    let (_old_dir, old) = pre_0009_database().await;
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
        "the upgraded and fresh schemas must have the same objects"
    );
    for (name, object) in &fresh_dump {
        assert_eq!(
            Some(object),
            old_dump.get(name),
            "the upgraded and fresh schemas disagree about {name}"
        );
    }
    assert_eq!(
        fresh_details, old_details,
        "the upgraded and fresh schemas disagree about defaults, FK actions, indexes or column order"
    );
}

/// A pre-seeded `effort_levels` column makes the first `ALTER` fail. The transaction must roll
/// everything back — the fixture rows, both options keys and the snapshots stay exactly as they
/// were — and the repaired copy must migrate on the next attempt.
#[tokio::test]
async fn a_broken_copy_fails_loudly_and_can_be_retried() {
    let (_dir, db) = pre_0009_database().await;
    db.conn()
        .execute_unprepared(
            "ALTER TABLE model_profiles ADD COLUMN effort_levels TEXT NOT NULL DEFAULT '[]'",
        )
        .await
        .expect("pre-seed the conflicting column");
    let baseline_schema = schema_dump(db.conn()).await;
    let baseline_details = schema_details(db.conn()).await;
    let baseline_profiles = table_rows(db.conn(), "model_profiles", PRE_PROFILE_COLUMNS).await;
    let baseline_sessions = table_rows(db.conn(), "agent_sessions", SESSION_COLUMNS).await;

    let error = nekowriter_migration::up(db.conn())
        .await
        .expect_err("the broken copy must fail loudly");
    assert!(
        error.to_string().contains("duplicate column name"),
        "the first ALTER must fail on the pre-seeded column, got: {error}"
    );
    assert_eq!(
        schema_dump(db.conn()).await,
        baseline_schema,
        "the failed migration changed the schema"
    );
    assert_eq!(
        schema_details(db.conn()).await,
        baseline_details,
        "the failed migration changed the columns"
    );
    assert_eq!(
        table_rows(db.conn(), "model_profiles", PRE_PROFILE_COLUMNS).await,
        baseline_profiles,
        "the failed migration changed profile rows"
    );
    assert_eq!(
        table_rows(db.conn(), "agent_sessions", SESSION_COLUMNS).await,
        baseline_sessions,
        "the failed migration changed session snapshots"
    );
    assert_eq!(
        scalar(
            db.conn(),
            "SELECT count(*) AS n FROM seaql_migrations WHERE version = 'm0009_role_effort'"
        )
        .await,
        0,
        "the failed migration must not be recorded"
    );
    assert_eq!(
        column_count(db.conn(), "model_profiles", "default_effort").await,
        0,
        "the failed migration must not commit the second ALTER"
    );

    // Repair the copy: drop the pre-seeded column and retry.
    db.conn()
        .execute_unprepared("ALTER TABLE model_profiles DROP COLUMN effort_levels")
        .await
        .expect("repair the copy");
    nekowriter_migration::up(db.conn())
        .await
        .expect("retry after repairing the copy");
    assert_eq!(
        column_count(db.conn(), "model_profiles", "effort_levels").await,
        1
    );
    assert_eq!(
        column_count(db.conn(), "model_profiles", "default_effort").await,
        1
    );
    let levels: String = db
        .conn()
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT effort_levels FROM model_profiles WHERE id = 1",
        ))
        .await
        .expect("profile 1")
        .expect("row")
        .try_get("", "effort_levels")
        .expect("levels");
    assert_eq!(levels, r#"["max"]"#, "the retry back-filled the levels");
}
