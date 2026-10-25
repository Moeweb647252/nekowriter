//! Shared harness for the runtime tests (PLAN M3.1, spec/03 §15).
//!
//! Every test uses a temporary database, a `ScriptedProvider` and the real API/router, so the
//! assertions cover the persisted rows the spec names rather than internal state.

#![allow(dead_code)]

use std::time::{Duration, Instant};

use nekowriter::api::dto::providers::ModelProfileInput;
use nekowriter::db::json::{
    AgentRole, Granularity, Phase, SessionKind, SessionStatus, ToolCallStatus,
};
use nekowriter::events::{EventEnvelope, ServerEvent};
use nekowriter::services::sessions;
use nekowriter::testing::TestApp;
use nekowriter_llm::StopReason;
use nekowriter_llm::mock::{ScriptStep, ScriptedProvider};
use serde_json::{Value, json};

/// The wait budget of one status change.
///
/// A fixed 10 s was exceeded when the whole suite ran under load (PLAN M11 hardening): with
/// dozens of test binaries and tokio runtimes on one machine a runner can be scheduled late, and
/// a deadline that assumes an idle box turns that delay into a flake. The budget scales with the
/// machine's parallelism and is bounded, so a real hang still fails the test instead of hanging
/// the run. Callers may still pass a larger explicit timeout to `wait_status_within`.
pub fn wait_budget() -> Duration {
    let parallelism = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
        .min(32) as u64;
    Duration::from_secs(10 + parallelism)
}

/// A harness: the app, its scripted provider and the ids of the fixtures.
pub struct Runtime {
    pub app: TestApp,
    pub scripted: ScriptedProvider,
    pub book_id: i32,
    pub track_id: i32,
    pub chapter_id: i32,
}

/// Creates a book with a main track, a chapter and a Demo model wired to every role, plus a
/// scripted provider.
pub async fn runtime(steps: Vec<ScriptStep>) -> Runtime {
    let mut app = TestApp::new().await;
    // The runtime gets one handle; the test keeps the other to inspect the requests.
    let scripted = app.set_provider(ScriptedProvider::new(steps));

    let book = app.create_book("Runtime Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;

    // One model on a Demo provider, assigned to every role.
    let (status, provider) = app
        .post(
            "/api/providers",
            json!({
                "name": "Demo",
                "protocol": "mock",
                "base_url": "mock://demo",
                "extra_headers": {},
            }),
        )
        .await;
    assert_eq!(status, 201, "create provider: {provider}");
    let provider_id = provider["id"].as_i64().expect("provider id") as i32;

    let profile = ModelProfileInput {
        provider_id,
        display_name: "Demo model".to_owned(),
        model: "demo-model".to_owned(),
        context_window: 200_000,
        max_output_tokens: 8_192,
        options: Default::default(),
    };
    let (status, profile) = app
        .post(
            "/api/model-profiles",
            serde_json::to_value(&profile).expect("profile"),
        )
        .await;
    assert_eq!(status, 201, "create model profile: {profile}");
    let profile_id = profile["id"].as_i64().expect("profile id") as i32;

    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({
                "roles_defaults": {
                    "conception": profile_id,
                    "researcher": profile_id,
                    "writer": profile_id,
                    "reviewer": profile_id,
                    "archivist": profile_id,
                },
            }),
        )
        .await;
    assert_eq!(status, 200, "assign roles: {body}");

    Runtime {
        app,
        scripted,
        book_id,
        track_id,
        chapter_id,
    }
}

impl Runtime {
    /// Starts a test-only session (the debug session of spec/10 §2.7) whose frozen tool list is
    /// `echo`, `test_write` (a write tool, so `auto_pause` holds it), `test_finish` (completes)
    /// and `test_slow` (a cancellable `prepare`), so the runtime is exercised without the writer
    /// tools of M4 (PLAN M3.1).
    pub async fn start_debug(&self) -> i32 {
        self.start_debug_with(Granularity::Paragraph, false, false)
            .await
    }

    /// The same with explicit writer switches and phase.
    pub async fn start_debug_with(
        &self,
        granularity: Granularity,
        auto_pause: bool,
        pause_after_outline: bool,
    ) -> i32 {
        sessions::create_debug_session(
            self.app.state(),
            sessions::DebugSessionSpec {
                book_id: self.book_id,
                chapter_id: Some(self.chapter_id),
                kind: SessionKind::ChapterWrite,
                phase: Phase::Drafting,
                granularity: Some(granularity),
                auto_pause,
                pause_after_outline,
            },
        )
        .await
        .expect("create debug session")
    }

    /// A test-only session with no chapter (the debug echo endpoint's shape).
    pub async fn start_debug_book_level(&self) -> i32 {
        sessions::create_debug_session(
            self.app.state(),
            sessions::DebugSessionSpec {
                book_id: self.book_id,
                chapter_id: None,
                kind: SessionKind::ChapterWrite,
                phase: Phase::Drafting,
                granularity: Some(Granularity::Paragraph),
                auto_pause: false,
                pause_after_outline: false,
            },
        )
        .await
        .expect("create debug session")
    }

    /// The raw session row fields the assertions need.
    pub async fn session_row(&self, session_id: i32) -> SessionRow {
        use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
        let row = self
            .app
            .db()
            .conn()
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT status, phase, pause_requested, held_tool_call_id, counters, usage, error, \
                 view_snapshot, granularity, auto_pause, pause_after_outline, finished_at \
                 FROM agent_sessions WHERE id = ?",
                vec![session_id.into()],
            ))
            .await
            .expect("query session")
            .expect("session row");
        SessionRow {
            status: row.try_get::<String>("", "status").expect("status"),
            phase: row.try_get::<String>("", "phase").expect("phase"),
            pause_requested: row.try_get::<i64>("", "pause_requested").expect("pause"),
            held_tool_call_id: row
                .try_get::<Option<i32>>("", "held_tool_call_id")
                .expect("held"),
            counters: row.try_get::<String>("", "counters").expect("counters"),
            usage: row.try_get::<String>("", "usage").expect("usage"),
            error: row.try_get::<Option<String>>("", "error").expect("error"),
            view_snapshot: row
                .try_get::<Option<String>>("", "view_snapshot")
                .expect("snapshot"),
            granularity: row
                .try_get::<Option<String>>("", "granularity")
                .expect("granularity"),
            auto_pause: row.try_get::<i64>("", "auto_pause").expect("auto"),
            pause_after_outline: row
                .try_get::<i64>("", "pause_after_outline")
                .expect("pause_outline"),
            finished_at: row
                .try_get::<Option<String>>("", "finished_at")
                .expect("finished"),
        }
    }

    /// Waits until the session reaches `status`, or panics after [`wait_budget`].
    pub async fn wait_status(&self, session_id: i32, status: SessionStatus) -> SessionRow {
        self.wait_status_within(session_id, status, wait_budget())
            .await
    }

    pub async fn wait_status_within(
        &self,
        session_id: i32,
        status: SessionStatus,
        timeout: Duration,
    ) -> SessionRow {
        // A caller's explicit timeout is a floor, not a ceiling: under a full parallel suite a
        // runner can be scheduled late, and a fixed 10 s then fails for load rather than for a
        // defect (PLAN M11 hardening). A genuinely stuck session still fails at the budget.
        let timeout = timeout.max(wait_budget());
        let deadline = Instant::now() + timeout;
        loop {
            let row = self.session_row(session_id).await;
            if row.status == status.as_str() {
                return row;
            }
            if Instant::now() > deadline {
                panic!(
                    "session {session_id} did not reach {} within {timeout:?}; it is {}",
                    status.as_str(),
                    row.status
                );
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }

    /// Waits until the session leaves `running` (any non-running status).
    pub async fn wait_any_status(&self, session_id: i32) -> SessionRow {
        let deadline = Instant::now() + wait_budget();
        loop {
            let row = self.session_row(session_id).await;
            if row.status != "running" {
                return row;
            }
            if Instant::now() > deadline {
                panic!("session {session_id} stayed running");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Waits until no runner is active for the session.
    pub async fn wait_idle(&self, session_id: i32) {
        let deadline = Instant::now() + wait_budget();
        while self.app.state().manager().is_running(session_id) {
            if Instant::now() > deadline {
                panic!("the runner of session {session_id} did not exit");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// One call row by id.
    pub async fn call_by_id(&self, call_id: i32) -> CallRow {
        use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
        let row = self
            .app
            .db()
            .conn()
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT id, message_id, call_index, name, input, status, raw_result_text, \
                 appendices, result_text, is_error, sealed, effects FROM agent_tool_calls \
                 WHERE id = ?",
                vec![call_id.into()],
            ))
            .await
            .expect("query call")
            .expect("call row");
        CallRow {
            id: row.try_get::<i32>("", "id").expect("id"),
            message_id: row.try_get::<i32>("", "message_id").expect("message"),
            call_index: row.try_get::<i32>("", "call_index").expect("index"),
            name: row.try_get::<String>("", "name").expect("name"),
            input: row.try_get::<String>("", "input").expect("input"),
            status: row.try_get::<String>("", "status").expect("status"),
            raw_result_text: row
                .try_get::<Option<String>>("", "raw_result_text")
                .expect("raw"),
            appendices: row.try_get::<String>("", "appendices").expect("appendices"),
            result_text: row
                .try_get::<Option<String>>("", "result_text")
                .expect("result"),
            is_error: row.try_get::<i64>("", "is_error").expect("is_error") != 0,
            sealed: row.try_get::<i64>("", "sealed").expect("sealed") != 0,
            effects: row
                .try_get::<Option<String>>("", "effects")
                .expect("effects"),
        }
    }

    /// The calls of a session in `call_index` order.
    pub async fn calls(&self, session_id: i32) -> Vec<CallRow> {
        use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
        let rows = self
            .app
            .db()
            .conn()
            .query_all_raw(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT id, message_id, call_index, name, input, status, raw_result_text, \
                 appendices, result_text, is_error, seeded_status, sealed, effects FROM (\
                   SELECT id, message_id, call_index, name, input, status, raw_result_text, \
                          appendices, result_text, is_error, status AS seeded_status, sealed, \
                          effects \
                   FROM agent_tool_calls WHERE session_id = ? ORDER BY id)",
                vec![session_id.into()],
            ))
            .await
            .expect("query calls");
        rows.into_iter()
            .map(|row| CallRow {
                id: row.try_get::<i32>("", "id").expect("id"),
                message_id: row.try_get::<i32>("", "message_id").expect("message"),
                call_index: row.try_get::<i32>("", "call_index").expect("index"),
                name: row.try_get::<String>("", "name").expect("name"),
                input: row.try_get::<String>("", "input").expect("input"),
                status: row.try_get::<String>("", "status").expect("status"),
                raw_result_text: row
                    .try_get::<Option<String>>("", "raw_result_text")
                    .expect("raw"),
                appendices: row.try_get::<String>("", "appendices").expect("appendices"),
                result_text: row
                    .try_get::<Option<String>>("", "result_text")
                    .expect("result"),
                is_error: row.try_get::<i64>("", "is_error").expect("is_error") != 0,
                sealed: row.try_get::<i64>("", "sealed").expect("sealed") != 0,
                effects: row
                    .try_get::<Option<String>>("", "effects")
                    .expect("effects"),
            })
            .collect()
    }

    /// The messages of a session.
    pub async fn messages(&self, session_id: i32) -> Vec<MessageRow> {
        use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
        self.app
            .db()
            .conn()
            .query_all_raw(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT id, seq, role, kind, content, request_meta, provider_payload \
                 FROM agent_messages WHERE session_id = ? ORDER BY seq",
                vec![session_id.into()],
            ))
            .await
            .expect("query messages")
            .into_iter()
            .map(|row| MessageRow {
                id: row.try_get::<i32>("", "id").expect("id"),
                seq: row.try_get::<i32>("", "seq").expect("seq"),
                role: row.try_get::<String>("", "role").expect("role"),
                kind: row.try_get::<String>("", "kind").expect("kind"),
                content: row.try_get::<String>("", "content").expect("content"),
                request_meta: row
                    .try_get::<Option<String>>("", "request_meta")
                    .expect("meta"),
                provider_payload: row
                    .try_get::<Option<String>>("", "provider_payload")
                    .expect("payload"),
            })
            .collect()
    }

    /// The frozen model snapshot of a session, decoded.
    pub async fn model_snapshot(&self, session_id: i32) -> serde_json::Value {
        use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
        let row = self
            .app
            .db()
            .conn()
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT model_snapshot FROM agent_sessions WHERE id = ?",
                vec![session_id.into()],
            ))
            .await
            .expect("query snapshot")
            .expect("session row");
        let raw: String = row.try_get("", "model_snapshot").expect("snapshot column");
        serde_json::from_str(&raw).expect("json")
    }

    /// The chapter's status and canon status.
    pub async fn chapter_status(&self) -> (String, String, i32) {
        use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
        let row = self
            .app
            .db()
            .conn()
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT status, canon_status, content_version FROM chapters WHERE id = ?",
                vec![self.chapter_id.into()],
            ))
            .await
            .expect("query chapter")
            .expect("chapter");
        (
            row.try_get::<String>("", "status").expect("status"),
            row.try_get::<String>("", "canon_status").expect("canon"),
            row.try_get::<i32>("", "content_version").expect("version"),
        )
    }

    /// Every version row kind of the chapter.
    pub async fn version_kinds(&self) -> Vec<String> {
        use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
        self.app
            .db()
            .conn()
            .query_all_raw(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT kind FROM chapter_versions WHERE chapter_id = ? ORDER BY id",
                vec![self.chapter_id.into()],
            ))
            .await
            .expect("query versions")
            .into_iter()
            .map(|row| row.try_get::<String>("", "kind").expect("kind"))
            .collect()
    }

    /// The pending inbox rows of a session.
    pub async fn inbox_pending(&self, session_id: i32) -> i64 {
        nekowriter::agent::session::inbox_pending(self.app.state().db_conn(), session_id)
            .await
            .expect("inbox count")
    }

    /// The stored appendices of one call, decoded.
    pub fn appendices(call: &CallRow) -> Vec<Value> {
        serde_json::from_str(&call.appendices).unwrap_or_default()
    }

    /// The last request the scripted provider received.
    pub fn last_request(&self) -> nekowriter_llm::ChatRequest {
        self.scripted.requests().last().cloned().expect("a request")
    }

    /// The last serialized request body of the scripted provider.
    pub fn last_body(&self) -> Value {
        self.scripted.bodies().last().cloned().expect("a body")
    }

    /// Waits for an event matching `predicate`, with a timeout.
    pub async fn wait_event(
        &self,
        receiver: &mut tokio::sync::broadcast::Receiver<EventEnvelope>,
        timeout: Duration,
        predicate: impl Fn(&ServerEvent) -> bool,
    ) -> ServerEvent {
        let deadline = Instant::now() + timeout;
        loop {
            match tokio::time::timeout_at(deadline.into(), receiver.recv()).await {
                Ok(Ok(envelope)) if predicate(&envelope.event) => return envelope.event,
                Ok(Ok(_)) => continue,
                Ok(Err(_)) => panic!("the event channel closed"),
                Err(_) => panic!("no matching event within {timeout:?}"),
            }
        }
    }
}

/// The session columns assertions read.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub status: String,
    pub phase: String,
    pub pause_requested: i64,
    pub held_tool_call_id: Option<i32>,
    pub counters: String,
    pub usage: String,
    pub error: Option<String>,
    pub view_snapshot: Option<String>,
    pub granularity: Option<String>,
    pub auto_pause: i64,
    pub pause_after_outline: i64,
    pub finished_at: Option<String>,
}

impl SessionRow {
    pub fn counters(&self) -> nekowriter::db::json::SessionCounters {
        nekowriter::db::json::from_json_or_default(&self.counters)
    }

    pub fn usage(&self) -> nekowriter::db::json::Usage {
        nekowriter::db::json::from_json_or_default(&self.usage)
    }

    pub fn error(&self) -> Option<nekowriter::db::json::SessionError> {
        self.error
            .as_deref()
            .and_then(|raw| nekowriter::db::json::from_json(raw).ok())
    }
}

/// The tool-call columns assertions read.
#[derive(Debug, Clone)]
pub struct CallRow {
    pub id: i32,
    pub message_id: i32,
    pub call_index: i32,
    pub name: String,
    pub input: String,
    pub status: String,
    pub raw_result_text: Option<String>,
    pub appendices: String,
    pub result_text: Option<String>,
    pub is_error: bool,
    pub sealed: bool,
    pub effects: Option<String>,
}

impl CallRow {
    pub fn status_enum(&self) -> ToolCallStatus {
        ToolCallStatus::from_str_opt(&self.status).expect("status")
    }

    pub fn effects(&self) -> nekowriter::db::json::ToolEffects {
        nekowriter::db::json::from_json_or_default(self.effects.as_deref().unwrap_or("{}"))
    }
}

/// The message columns assertions read.
#[derive(Debug, Clone)]
pub struct MessageRow {
    pub id: i32,
    pub seq: i32,
    pub role: String,
    pub kind: String,
    pub content: String,
    pub request_meta: Option<String>,
    pub provider_payload: Option<String>,
}

impl MessageRow {
    pub fn blocks(&self) -> Vec<nekowriter::db::json::StoredBlock> {
        nekowriter::db::json::from_json_or_default(&self.content)
    }

    /// The joined text of the message's text blocks.
    pub fn text(&self) -> String {
        self.blocks()
            .into_iter()
            .filter_map(|block| match block {
                nekowriter::db::json::StoredBlock::Text { text, .. } => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn request_meta(&self) -> Option<nekowriter::db::json::RequestMeta> {
        self.request_meta
            .as_deref()
            .and_then(|raw| nekowriter::db::json::from_json(raw).ok())
    }
}

// ───────────────────────────── script helpers ─────────────────────────────

/// A step that calls one tool with the given arguments.
pub fn tool_step(name: &str, arguments: Value) -> ScriptStep {
    ScriptStep::new().tool(name, arguments)
}

/// A step that calls several tools in one response.
pub fn tools_step(calls: &[(&str, Value)]) -> ScriptStep {
    let mut step = ScriptStep::new();
    for (name, arguments) in calls {
        step = step.tool(name, arguments.clone());
    }
    step
}

/// A completing step: `test_finish` carries `complete: draft`, so the session ends.
pub fn finish_step() -> ScriptStep {
    ScriptStep::new().tool("test_finish", json!({ "text": "done" }))
}

/// A step with text only (no tool calls).
pub fn text_step(text: &str) -> ScriptStep {
    ScriptStep::new().text(text).stop(StopReason::EndTurn)
}

/// A step that stops with `max_tokens`.
pub fn truncated_step(name: &str, arguments: Value) -> ScriptStep {
    ScriptStep::new()
        .tool(name, arguments)
        .stop(StopReason::MaxTokens)
}

/// A step that refuses.
pub fn refusal_step(message: &str) -> ScriptStep {
    ScriptStep::new().text("").stop(StopReason::Refusal {
        category: Some("cyber".to_owned()),
        explanation: Some(message.to_owned()),
    })
}

/// Asserts a phase value.
pub fn assert_phase(row: &SessionRow, phase: Phase) {
    assert_eq!(row.phase, phase.as_str(), "phase");
}

/// Asserts a status value.
pub fn assert_status(row: &SessionRow, status: SessionStatus) {
    assert_eq!(row.status, status.as_str(), "status");
}

/// Asserts the role enum of a session kind maps as the spec says.
pub fn assert_role(kind: SessionKind, role: AgentRole) {
    assert_eq!(nekowriter::agent::tools::role_of_kind(kind), role.as_str());
}
