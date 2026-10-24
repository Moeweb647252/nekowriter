//! The tool framework (spec/05 §1–§2).
//!
//! A tool is a [`ToolHandler`] with two phases (spec/05 §1.2, D-024): `prepare` does slow,
//! read-only work **outside** any transaction and is cancellable by Stop; `execute` does atomic
//! database work **inside** the call's transaction (D-033). Model-facing definitions come from
//! `prompts/tools.toml` (spec/09 §1) and are frozen into the session; results and errors are
//! rendered from `prompts/results.toml` — code never writes model-facing text.

pub mod registry;
pub mod resolve;
pub mod retrieval;
pub mod spec;
pub mod test_tools;
pub mod writer;

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::db::entities::{agent_sessions, books, chapters, tracks};
use crate::db::json::{AwaitInputKind, RequestMeta, ToolEffects};
use crate::events::ServerEvent;
use crate::prompts::PromptSet;

pub use registry::{
    ARCHIVIST_TOOLS, CONCEPTION_TOOLS, DEBUG_TOOLS, RESEARCHER_TOOLS, RETRIEVAL_TOOLS,
    REVIEWER_TOOLS, ToolRegistry, WRITE_TOOLS, WRITER_TOOLS, allowed_tools, initial_phase,
    is_permitted, is_write_tool, phase_label, role_of_kind, tools_for_role,
};
pub use resolve::EntityResolver;
pub use spec::{ToolSpec, ToolSpecs, validate_input};

/// What a tool does, for grouping in the UI and in the logs (spec/05 §1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Retrieval,
    Draft,
    Canon,
    Interactive,
    Web,
    Knowledge,
    Control,
    /// Test-only tools of the debug session (spec/10 §2.7).
    Debug,
}

/// A user-correctable tool failure (spec/05 §1.2). Rendered with `result.error` from its
/// [`ToolErrorKind`] plus [`ToolError::fields`]; internal failures roll the transaction back.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{kind:?}: {fields:?}")]
pub struct ToolError {
    pub kind: ToolErrorKind,
    pub fields: BTreeMap<String, Value>,
}

/// One variant per `result.error` branch (spec/09 §2). The code strings are stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorKind {
    InvalidJson,
    Validation,
    EntityNotFound,
    EntityAmbiguous,
    EntityExists,
    PidNotFound,
    TextNotFound,
    TextAmbiguous,
    Granularity,
    DraftEmpty,
    IsolatedReadonly,
    ChapterNotFound,
    ChapterNotOfficial,
    NoKnowledgeScope,
    ScopeNotFound,
    NotFound,
    EntryExists,
    FindFailed,
    MaintenanceIncomplete,
    SearchNotConfigured,
    SearchFailed,
    FetchFailed,
    UnsupportedContentType,
    CharacterNotFound,
    CharacterExists,
    ReorderMismatch,
    RelationNotFound,
    SeedOnly,
    ExactlyOne,
    MutuallyExclusive,
    ToolNotAllowed,
    OutputTruncated,
    AfterCompletion,
    /// The provider refused the turn; its calls are finished unexecuted (spec/03 §5.5).
    Refusal,
    /// Anything else: `result.error` falls back to its internal-error text.
    Internal,
}

impl ToolErrorKind {
    /// The `code` passed to `result.error` (spec/09 §2).
    pub fn code(self) -> &'static str {
        match self {
            ToolErrorKind::InvalidJson => "invalid_json",
            ToolErrorKind::Validation => "validation",
            ToolErrorKind::EntityNotFound => "entity_not_found",
            ToolErrorKind::EntityAmbiguous => "entity_ambiguous",
            ToolErrorKind::EntityExists => "entity_exists",
            ToolErrorKind::PidNotFound => "pid_not_found",
            ToolErrorKind::TextNotFound => "text_not_found",
            ToolErrorKind::TextAmbiguous => "text_ambiguous",
            ToolErrorKind::Granularity => "granularity",
            ToolErrorKind::DraftEmpty => "draft_empty",
            ToolErrorKind::IsolatedReadonly => "isolated_readonly",
            ToolErrorKind::ChapterNotFound => "chapter_not_found",
            ToolErrorKind::ChapterNotOfficial => "chapter_not_official",
            ToolErrorKind::NoKnowledgeScope => "no_knowledge_scope",
            ToolErrorKind::ScopeNotFound => "scope_not_found",
            ToolErrorKind::NotFound => "not_found",
            ToolErrorKind::EntryExists => "entry_exists",
            ToolErrorKind::FindFailed => "find_failed",
            ToolErrorKind::MaintenanceIncomplete => "maintenance_incomplete",
            ToolErrorKind::SearchNotConfigured => "search_not_configured",
            ToolErrorKind::SearchFailed => "search_failed",
            ToolErrorKind::FetchFailed => "fetch_failed",
            ToolErrorKind::UnsupportedContentType => "unsupported_content_type",
            ToolErrorKind::CharacterNotFound => "character_not_found",
            ToolErrorKind::CharacterExists => "character_exists",
            ToolErrorKind::ReorderMismatch => "reorder_mismatch",
            ToolErrorKind::RelationNotFound => "relation_not_found",
            ToolErrorKind::SeedOnly => "seed_only",
            ToolErrorKind::ExactlyOne => "exactly_one",
            ToolErrorKind::MutuallyExclusive => "mutually_exclusive",
            ToolErrorKind::ToolNotAllowed => "tool_not_allowed",
            ToolErrorKind::OutputTruncated => "output_truncated",
            ToolErrorKind::AfterCompletion => "after_completion",
            ToolErrorKind::Refusal => "refusal",
            ToolErrorKind::Internal => "internal",
        }
    }
}

impl ToolError {
    pub fn new(kind: ToolErrorKind) -> Self {
        Self {
            kind,
            fields: BTreeMap::new(),
        }
    }

    /// Attaches one template field.
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.fields.insert(key.to_owned(), value.into());
        self
    }

    /// `error.tool_not_allowed`, listing the tools the current phase permits (spec/03 §1.2).
    pub fn tool_not_allowed(tool: &str, phase_label: String, allowed: Vec<String>) -> Self {
        Self::new(ToolErrorKind::ToolNotAllowed)
            .with("tool", tool.to_owned())
            .with("phase_label", phase_label)
            .with("allowed", allowed)
    }
}

/// Slow read-only work done by `prepare`, handed to `execute` (spec/05 §1.2).
#[derive(Debug, Clone, Default)]
pub enum Prepared {
    #[default]
    None,
    QueryVector(Vec<f32>),
    WebSearch(Vec<WebHit>),
    WebPage(Box<FetchedPage>),
}

/// One web search hit (spec/07 §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub published: Option<String>,
}

/// One fetched, extracted page (spec/07 §7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedPage {
    pub url: String,
    pub title: String,
    pub markdown: String,
    /// True when the page came from the cache (no request was made).
    pub cached: bool,
}

/// The services a `prepare` phase may use (spec/05 §1.2). M3 has no web or embedding clients
/// yet; later milestones add them here.
pub struct PrepareCtx<'a> {
    pub db: &'a sea_orm::DatabaseConnection,
    pub session: &'a agent_sessions::Model,
    pub state: &'a crate::state::AppState,
}

/// Which canon rows a tool may see (spec/06 §2): shared canon, or shared plus one isolated
/// track's private rows. The canon services of M5 own the queries; the runtime carries the
/// visibility of the session so the tools never re-derive it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    /// `track_id IS NULL` (the main story and canon-mode side stories).
    #[default]
    Shared,
    /// `track_id IS NULL OR track_id = <track>`: an isolated side track may read shared canon
    /// but not modify it (D-014, D-036).
    Isolated { track_id: i32 },
}

impl Visibility {
    /// The visibility of a session's track.
    pub fn of(session: &agent_sessions::Model, track: Option<&tracks::Model>) -> Self {
        match track {
            Some(track) if track.canon_mode == "isolated" => {
                Visibility::Isolated { track_id: track.id }
            }
            _ => {
                let _ = session;
                Visibility::Shared
            }
        }
    }

    /// True when writes to shared canon are allowed (spec/06 §2).
    pub fn can_write_shared(self) -> bool {
        matches!(self, Visibility::Shared)
    }
}

/// The context of one `execute`: the call's transaction plus everything the tool may read.
pub struct ToolCtx<'a> {
    pub tx: &'a sea_orm::DatabaseTransaction,
    pub session: &'a agent_sessions::Model,
    pub book: Option<&'a books::Model>,
    pub chapter: Option<&'a chapters::Model>,
    pub track: Option<&'a tracks::Model>,
    /// Which canon rows the session may read and write (spec/06 §2).
    pub visibility: Visibility,
    /// The state in effect when the model generated the call (spec/03 §8, §11).
    pub request_meta: RequestMeta,
    pub prompts: &'a PromptSet,
    pub state: &'a crate::state::AppState,
    /// Events to publish after the commit (spec/01 §4.2).
    pub events: Vec<ServerEvent>,
}

impl ToolCtx<'_> {
    /// The session's kind as its enum.
    pub fn kind(&self) -> crate::db::json::SessionKind {
        crate::agent::session::kind_of(self.session)
    }

    /// The book's writing language (BCP-47), defaulting to `en` when the session has no book.
    pub fn writing_language(&self) -> String {
        self.book
            .map(|book| book.writing_language.clone())
            .unwrap_or_else(|| "en".to_owned())
    }

    /// Queues one event for after the commit.
    pub fn emit(&mut self, event: ServerEvent) {
        self.events.push(event);
    }
}

/// The result of one tool call (spec/05 §1.2).
#[derive(Debug, Clone)]
pub struct ToolOutcome {
    /// Rendered from a `result.*` template.
    pub text: String,
    pub is_error: bool,
    /// Structured data for the agent panel; carries data only (spec/05 §1.6, D-054).
    pub ui: Value,
    /// Deferred effects, applied when the call is finalized (spec/03 §5.4).
    pub effects: ToolEffects,
    /// `Some` when the call needs the author's input before it can finish.
    pub await_input: Option<AwaitInputKind>,
    /// Deferred counter flag applied by `finalize` in the same transaction (spec/03 §8: a short
    /// completion is withheld exactly once, remembered in `counters.finish_warned`). It travels on
    /// the outcome because `finalize` writes the counters from the row it loaded before `execute`
    /// ran, so a direct write inside `execute` would be overwritten.
    pub finish_warned: bool,
}

impl ToolOutcome {
    /// A successful outcome without effects.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
            ui: Value::Null,
            effects: ToolEffects::default(),
            await_input: None,
            finish_warned: false,
        }
    }

    pub fn with_ui(mut self, ui: Value) -> Self {
        self.ui = ui;
        self
    }

    pub fn with_effects(mut self, effects: ToolEffects) -> Self {
        self.effects = effects;
        self
    }

    /// The error outcome of a [`ToolError`], rendered through `result.error`.
    pub fn error(prompts: &PromptSet, error: &ToolError) -> Self {
        let mut fields = serde_json::Map::new();
        fields.insert(
            "code".to_owned(),
            Value::String(error.kind.code().to_owned()),
        );
        for (key, value) in &error.fields {
            fields.insert(key.clone(), value.clone());
        }
        let text = prompts
            .render_value("result.error", crate::prompts::template_value(&fields))
            .map(|rendered| rendered.text)
            .unwrap_or_else(|err| {
                tracing::error!(error = %err, "cannot render result.error");
                String::new()
            });
        Self {
            text,
            is_error: true,
            ui: Value::Null,
            effects: ToolEffects::default(),
            await_input: None,
            finish_warned: false,
        }
    }

    /// Records that the one withheld completion was used (spec/03 §8, D-029).
    pub fn with_finish_warned(mut self) -> Self {
        self.finish_warned = true;
        self
    }
}

/// One tool of a role (spec/05 §1.2, §1.3).
#[async_trait::async_trait]
pub trait ToolHandler: Send + Sync {
    fn name(&self) -> &'static str;

    fn kind(&self) -> ToolKind;

    /// The JSON field whose streaming value is previewed (spec/03 §12).
    fn preview_field(&self) -> Option<&'static str> {
        None
    }

    /// Slow, read-only work outside any transaction; cancellable by Stop (spec/05 §1.2).
    async fn prepare(&self, _ctx: &PrepareCtx<'_>, _input: &Value) -> Result<Prepared, ToolError> {
        Ok(Prepared::None)
    }

    /// Atomic work inside the call's transaction: database reads and writes only, no I/O waits.
    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_kind_has_a_code() {
        // The codes are the `result.error` branches of spec/09 §2.
        let codes: Vec<&str> = [
            ToolErrorKind::InvalidJson,
            ToolErrorKind::Validation,
            ToolErrorKind::EntityNotFound,
            ToolErrorKind::EntityAmbiguous,
            ToolErrorKind::EntityExists,
            ToolErrorKind::PidNotFound,
            ToolErrorKind::TextNotFound,
            ToolErrorKind::TextAmbiguous,
            ToolErrorKind::Granularity,
            ToolErrorKind::DraftEmpty,
            ToolErrorKind::IsolatedReadonly,
            ToolErrorKind::ChapterNotFound,
            ToolErrorKind::ChapterNotOfficial,
            ToolErrorKind::NoKnowledgeScope,
            ToolErrorKind::ScopeNotFound,
            ToolErrorKind::NotFound,
            ToolErrorKind::EntryExists,
            ToolErrorKind::FindFailed,
            ToolErrorKind::MaintenanceIncomplete,
            ToolErrorKind::SearchNotConfigured,
            ToolErrorKind::SearchFailed,
            ToolErrorKind::FetchFailed,
            ToolErrorKind::UnsupportedContentType,
            ToolErrorKind::CharacterNotFound,
            ToolErrorKind::CharacterExists,
            ToolErrorKind::ReorderMismatch,
            ToolErrorKind::RelationNotFound,
            ToolErrorKind::SeedOnly,
            ToolErrorKind::ExactlyOne,
            ToolErrorKind::MutuallyExclusive,
            ToolErrorKind::ToolNotAllowed,
            ToolErrorKind::OutputTruncated,
            ToolErrorKind::AfterCompletion,
            ToolErrorKind::Refusal,
            ToolErrorKind::Internal,
        ]
        .iter()
        .map(|kind| kind.code())
        .collect();
        assert_eq!(codes.len(), 35);
        let unique: std::collections::BTreeSet<&str> = codes.iter().copied().collect();
        assert_eq!(unique.len(), codes.len(), "codes must be unique");
        assert!(codes.contains(&"after_completion"));
        assert!(codes.contains(&"refusal"));
    }
}
