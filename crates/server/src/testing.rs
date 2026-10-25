//! Test helpers (spec/01 §11): a migrated temporary database, a router and fixtures.
//!
//! Only compiled for unit tests or with the `testing` feature. `crates/server` lists itself as a
//! dev-dependency with that feature, so integration tests in `tests/` can use these helpers
//! (DECISIONS D-049).

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::Value;
use tower::ServiceExt;

use crate::config::Config;
use crate::db::Db;
use crate::events::EventBus;
use crate::state::AppState;

/// A migrated temporary database plus the app built over it.
pub struct TestApp {
    _dir: tempfile::TempDir,
    db: Db,
    events: EventBus,
    state: AppState,
}

impl TestApp {
    /// Creates a temp data dir, a migrated database and the app state.
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = Db::connect(&dir.path().join("test.db"))
            .await
            .expect("connect");
        nekowriter_migration::up(db.conn()).await.expect("migrate");
        let vec_version = db.vec_version().await.expect("vec_version");
        let config = Config {
            bind: "127.0.0.1:0".parse().expect("bind"),
            data_dir: Some(dir.path().to_path_buf()),
            web_dir: None,
            enable_demo_provider: true,
            allow_remote: false,
        };
        let events = EventBus::new();
        let state = AppState::new(db.clone(), config, vec_version).with_events(events.clone());
        // Build the runtime vec schema at the test embedder's dimension and load the fake model
        // (spec/07 §3). Integration tests drive the indexer with `services::indexer::run_once`.
        crate::search::initialize(&state)
            .await
            .expect("search initialization");
        Self {
            _dir: dir,
            db,
            events,
            state,
        }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn events(&self) -> &EventBus {
        &self.events
    }

    /// The temporary data directory that holds `test.db`, so a test can reopen the same database
    /// and drive the production startup sequence over it (spec/01 §5 steps 7–8).
    pub fn data_dir(&self) -> &std::path::Path {
        self._dir.path()
    }

    /// The config the app was built with (its `data_dir` points at [`Self::data_dir`]).
    pub fn config(&self) -> &Config {
        self.state.config()
    }

    pub fn app(&self) -> Router {
        crate::api::build_app(self.state.clone())
    }

    /// Replaces the provider client of every runner with a scripted one (spec/04 §7.1) so runtime
    /// tests touch no network. The returned handle shares the queue and the request log.
    pub fn set_provider(
        &mut self,
        provider: nekowriter_llm::mock::ScriptedProvider,
    ) -> nekowriter_llm::mock::ScriptedProvider {
        let handle = provider.handle();
        let state = self.state.clone();
        self.state = state.with_provider(nekowriter_llm::ProviderClient::Scripted(
            std::sync::Arc::new(provider),
        ));
        handle
    }

    /// Removes the embedding model: search must fall back to keywords (spec/07 §5).
    pub fn without_embedder(&mut self) {
        let search = self.state.search().with_events(self.events.clone());
        search.uninstall_embedder();
        self.state = self.state.clone().with_search(search);
    }

    /// Sends one JSON request and returns the status and the parsed body.
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "127.0.0.1:7878");
        let request = match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.app().oneshot(request).await.expect("response");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .expect("body");
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, json)
    }

    /// `GET` shorthand.
    pub async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.request("GET", path, None).await
    }

    /// `POST` shorthand.
    pub async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", path, Some(body)).await
    }

    /// `PATCH` shorthand.
    pub async fn patch(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.request("PATCH", path, Some(body)).await
    }

    /// `PUT` shorthand.
    pub async fn put(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.request("PUT", path, Some(body)).await
    }

    /// `DELETE` shorthand.
    pub async fn delete(&self, path: &str) -> (StatusCode, Value) {
        self.request("DELETE", path, None).await
    }

    /// Creates a book through the API and returns its DTO.
    pub async fn create_book(&self, title: &str) -> Value {
        let (status, body) = self
            .post(
                "/api/books",
                serde_json::json!({
                    "title": title,
                    "idea": "an idea",
                    "genre": "xianxia",
                    "writing_language": "zh-CN",
                    "start_conception": false,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "create book: {body}");
        body["book"].clone()
    }

    /// Creates a chapter on a track and returns its DTO.
    pub async fn create_chapter(&self, track_id: i32, title: &str) -> Value {
        let (status, body) = self
            .post(
                &format!("/api/tracks/{track_id}/chapters"),
                serde_json::json!({ "title": title, "guidance": "" }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "create chapter: {body}");
        body
    }

    /// Writes a chapter's text through `PUT /content/text` and returns the result body.
    pub async fn put_text(&self, chapter_id: i32, base_version: i32, text: &str) -> Value {
        let (status, body) = self
            .put(
                &format!("/api/chapters/{chapter_id}/content/text"),
                serde_json::json!({ "base_version": base_version, "text": text }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "put text: {body}");
        body
    }
}

impl TestApp {
    /// Runs one raw SQL statement (fixtures for state later milestones create).
    ///
    /// It goes through [`Db::write_unprepared`], so a fixture obeys the same process-wide write
    /// lock as production writes: a spawned runner's transaction and this statement can never
    /// overlap (spec/01 §7).
    pub async fn execute(&self, sql: &str) {
        self.db
            .write_unprepared(sql)
            .await
            .unwrap_or_else(|err| panic!("fixture SQL failed: {err}\n{sql}"));
    }

    /// Sets a chapter's status directly (M4 turns status changes into workflow operations).
    pub async fn set_chapter_status(&self, chapter_id: i32, status: &str) {
        self.execute(&format!(
            "UPDATE chapters SET status = '{status}' WHERE id = {chapter_id}"
        ))
        .await;
    }

    /// Marks a chapter's canon as synced (M6 owns the maintenance endpoint).
    pub async fn mark_canon_synced(&self, chapter_id: i32) {
        self.execute(&format!(
            "UPDATE chapters SET canon_status = 'synced', synced_version_id = \
             official_version_id WHERE id = {chapter_id}"
        ))
        .await;
    }

    /// Inserts a writer session row in `status` for a chapter (M3 creates real sessions).
    pub async fn insert_writer_session(&self, chapter_id: i32, status: &str) {
        self.execute(&format!(
            "INSERT INTO agent_sessions(book_id, role, kind, chapter_id, track_id, model_snapshot, \
             system_prompt, tools, status, created_at, updated_at) \
             SELECT book_id, 'writer', 'chapter_write', id, track_id, '{{}}', '', '[]', \
             '{status}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z' \
             FROM chapters WHERE id = {chapter_id}"
        ))
        .await;
    }

    /// Forces a session's status (crash-recovery fixtures, spec/01 §5 step 8).
    pub async fn set_session_status(&self, session_id: i32, status: &str) {
        self.execute(&format!(
            "UPDATE agent_sessions SET status = '{status}' WHERE id = {session_id}"
        ))
        .await;
    }

    /// Inserts a tool call in `executing` with no result (a crash fixture, spec/03 §5.4).
    pub async fn insert_executing_call(
        &self,
        session_id: i32,
        name: &str,
        arguments: Value,
    ) -> i32 {
        // A message row is required by the foreign key.
        self.execute(&format!(
            "INSERT INTO agent_messages(session_id, seq, role, kind, content, created_at)              SELECT {session_id}, coalesce(max(seq), 0) + 1, 'assistant', 'assistant', '[]',              '2026-01-01T00:00:00Z' FROM agent_messages WHERE session_id = {session_id}"
        ))
        .await;
        let input = serde_json::to_string(&arguments).expect("arguments");
        self.execute(&format!(
            "INSERT INTO agent_tool_calls(session_id, message_id, call_index, provider_call_id,              name, input, status, created_at)              SELECT {session_id}, max(id), 0, 'call_recovered', '{name}', '{input}', 'executing',              '2026-01-01T00:00:00Z' FROM agent_messages WHERE session_id = {session_id}"
        ))
        .await;
        let row = {
            use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
            self.db
                .conn()
                .query_one_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "SELECT max(id) AS value FROM agent_tool_calls WHERE session_id = ?",
                    vec![session_id.into()],
                ))
                .await
                .expect("query call id")
                .expect("row")
        };
        row.try_get::<i32>("", "value").expect("call id")
    }

    /// Runs a read-only query returning one row as a JSON object (test fixtures).
    pub async fn query_json(&self, sql: &str, values: Vec<sea_orm::Value>) -> Value {
        use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
        let statement = Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values);
        let row = self
            .db
            .conn()
            .query_one_raw(statement)
            .await
            .expect("query")
            .expect("row");
        let mut map = serde_json::Map::new();
        for (index, column) in row.column_names().iter().enumerate() {
            let value: Value = row
                .try_get_by_index::<Option<Value>>(index)
                .ok()
                .flatten()
                .unwrap_or(Value::Null);
            map.insert((*column).to_owned(), value);
        }
        Value::Object(map)
    }

    /// Removes every session row (fixture cleanup).
    pub async fn clear_sessions(&self) {
        self.execute("DELETE FROM agent_sessions").await;
    }
}

/// Asserts an API error body has `code` (spec/10 §4).
pub fn assert_error(body: &Value, code: &str) {
    assert_eq!(body["error"]["code"], code, "error body: {body}");
}

/// Asserts an API error body has `code` and a `details` key.
pub fn assert_error_detail(body: &Value, code: &str, detail: &str) {
    assert_eq!(body["error"]["code"], code, "error body: {body}");
    assert!(
        body["error"]["details"].get(detail).is_some(),
        "error details should contain {detail}: {body}"
    );
}
