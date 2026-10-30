//! The tool framework (spec/05 §1–§2).
//!
//! A tool is a [`ToolHandler`] with two phases (spec/05 §1.2, D-024): `prepare` does slow,
//! read-only work **outside** any transaction and is cancellable by Stop; `execute` does atomic
//! database work **inside** the call's transaction (D-033). Model-facing definitions come from
//! `prompts/tools.toml` (spec/09 §1) and are frozen into the session; results and errors are
//! rendered from `prompts/results.toml` — code never writes model-facing text.

pub mod archivist;
pub mod conception;
pub mod goal_proposals;
pub mod proposals;
pub mod registry;
pub mod research;
pub mod resolve;
pub mod retrieval;
pub mod reviewer;
pub mod search;
pub mod spec;
pub mod test_tools;
pub mod web;
pub mod writer;

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::db::entities::{agent_sessions, books, chapters, tracks};
use crate::db::json::{AwaitInputKind, CanonMode, PendingContent, RequestMeta, ToolEffects};
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
            Some(track) if track.canon_mode == CanonMode::Isolated => {
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

    /// Replaces the deferred effects, keeping any content already staged for the write.
    ///
    /// Staged content is not an effect a tool chooses to set, it is a record of the work the call
    /// did; dropping it because a later call site replaced the effect set would silently lose the
    /// write (D-139), so it is carried across.
    pub fn with_effects(mut self, effects: ToolEffects) -> Self {
        let pending = self.effects.pending_content.take();
        self.effects = effects;
        if self.effects.pending_content.is_none() {
            self.effects.pending_content = pending;
        }
        self
    }

    /// Stages a chapter write for the transaction that finalizes this call (spec/03 §5.2, §5.4).
    ///
    /// A text-changing tool never writes `chapters.content` itself: it carries the new content on
    /// the outcome, so the write commits exactly when the call's effects do. That is what makes a
    /// held call's text invisible until it is approved, and discarded when it is rejected.
    pub fn with_pending_content(mut self, content: PendingContent) -> Self {
        self.effects.pending_content = Some(content);
        self
    }

    /// Marks the call as needing the author's input before it can finish (spec/05 §1.2, §7).
    /// Both this field and the effect are set by the tool: the effect is what the transaction
    /// that finalizes the call stores, the field is what the deferral check reads.
    pub fn with_await_input(mut self, kind: AwaitInputKind) -> Self {
        self.await_input = Some(kind);
        self.effects.await_input = Some(kind);
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

// ───────────────────────────── result cap (spec/05 §1.5) ─────────────────────────────

/// The default cap of a tool result, in characters (spec/05 §1.5).
pub const RESULT_CHAR_LIMIT: usize = 12_000;

/// Tools that enforce their own `max_chars` cap, so the global cap must not cut a second time:
/// `read_chapter` and `read_draft` page with `max_chars`, `web_fetch` with its own conversion
/// cap, and `read_knowledge_entry` reads from an `offset` in 6,000-character slices (spec/05 §3).
const TOOL_OWNED_RESULT_CAPS: [&str; 4] = [
    "read_chapter",
    "read_draft",
    "web_fetch",
    "read_knowledge_entry",
];

/// Caps one tool result at [`RESULT_CHAR_LIMIT`] characters (spec/05 §1.5).
///
/// The text is cut at the last paragraph (or, failing that, line) boundary that fits, so a model
/// never receives half a sentence when the result has structure. The cut is counted in
/// characters (not bytes) so CJK prose keeps its full budget, and the `result.truncated` prompt
/// template is appended — the model-facing wording stays in `prompts/` (D-017). Tools that own a
/// `max_chars` cap are left untouched (their own bound is the documented contract).
pub fn cap_result(prompts: &PromptSet, text: &str, tool: &str) -> String {
    if TOOL_OWNED_RESULT_CAPS.contains(&tool) || text.chars().count() <= RESULT_CHAR_LIMIT {
        return text.to_owned();
    }
    let cut = boundary_at_or_before(text, RESULT_CHAR_LIMIT);
    let head = &text[..cut];
    let notice = prompts
        .render(
            "result.truncated",
            &serde_json::json!({ "next_offset": serde_json::Value::Null }),
        )
        .map(|rendered| rendered.text)
        .unwrap_or_default();
    if notice.trim().is_empty() {
        return head.to_owned();
    }
    format!("{head}\n{notice}")
}

/// The byte index where `text` can be cut after at most `max_chars` characters: the last
/// paragraph break, else the last line break, else the character boundary itself. Always a char
/// boundary, so the result is valid UTF-8 (spec/05 §1.5).
fn boundary_at_or_before(text: &str, max_chars: usize) -> usize {
    // `nth(max_chars)` is the first character past the budget; its byte index is the cut.
    let end = text
        .char_indices()
        .nth(max_chars)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let window = &text[..end];
    // A break at index 0 would leave an empty head; keep looking for a usable one.
    if let Some(position) = window.rfind("\n\n").filter(|position| *position > 0) {
        return position;
    }
    if let Some(position) = window.rfind('\n').filter(|position| *position > 0) {
        return position;
    }
    end
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

    /// Every `ToolErrorKind` must render a **non-empty** model-facing text.
    ///
    /// The renderer is strict (`UndefinedBehavior::Strict`): a branch that reaches a variable its
    /// fields do not carry throws, and [`ToolOutcome::error`] then returns `String::new()` — an
    /// error the model cannot read, with `is_error = true` and no explanation. This test would have
    /// caught that class (a tool passing `entry_id` where the branch reads `title`), so it renders
    /// every kind with representative fields and asserts the branch's own wording appears.
    #[test]
    fn every_tool_error_renders_a_non_empty_text() {
        let prompts = prompts();
        // One representative error per kind, with the fields that kind's branch reads.
        let cases: Vec<ToolError> = vec![
            ToolError::new(ToolErrorKind::InvalidJson).with("raw", "{\"title\": "),
            ToolError::new(ToolErrorKind::Validation)
                .with("errors", vec!["title must not be empty".to_owned()]),
            ToolError::new(ToolErrorKind::EntityNotFound)
                .with("query", "林昭")
                .with("suggestions", Vec::<Value>::new()),
            ToolError::new(ToolErrorKind::EntityAmbiguous)
                .with("query", "林")
                .with("candidates", Vec::<Value>::new()),
            // `entity_exists` reads name + id (the branch does not use `query`).
            ToolError::new(ToolErrorKind::EntityExists)
                .with("name", "林昭")
                .with("id", 1),
            // `pid_not_found` branches on `tool` to word its hint.
            ToolError::new(ToolErrorKind::PidNotFound)
                .with("pids", vec!["p1".to_owned()])
                .with("tool", "read_draft"),
            ToolError::new(ToolErrorKind::TextNotFound)
                .with("pid", "p1")
                .with("text", "abc"),
            ToolError::new(ToolErrorKind::TextAmbiguous)
                .with("pid", "p1")
                .with("count", 2),
            // `granularity` reads mode_label, mode, reason, tool, length and limit.
            ToolError::new(ToolErrorKind::Granularity)
                .with("mode_label", "段落")
                .with("mode", "paragraph")
                .with("reason", "too_long")
                .with("tool", "insert_text")
                .with("length", 500)
                .with("limit", 300),
            ToolError::new(ToolErrorKind::DraftEmpty),
            ToolError::new(ToolErrorKind::IsolatedReadonly).with("target", "林昭"),
            ToolError::new(ToolErrorKind::ChapterNotFound).with("query", "12"),
            ToolError::new(ToolErrorKind::ChapterNotOfficial).with("label", "第12章"),
            ToolError::new(ToolErrorKind::NoKnowledgeScope),
            ToolError::new(ToolErrorKind::ScopeNotFound)
                .with("name", "宋代社会")
                .with("available", vec!["背景设定".to_owned()]),
            ToolError::new(ToolErrorKind::NotFound)
                .with("what_label", "知识条目")
                .with("id", 7),
            ToolError::new(ToolErrorKind::EntryExists)
                .with("title", "宋代茶馆")
                .with("id", 7),
            ToolError::new(ToolErrorKind::FindFailed)
                .with("doc_label", "知识条目")
                .with("find", "约三百家")
                .with("count", 2),
            ToolError::new(ToolErrorKind::MaintenanceIncomplete)
                .with("missing", vec!["summary".to_owned()]),
            ToolError::new(ToolErrorKind::SearchNotConfigured),
            ToolError::new(ToolErrorKind::SearchFailed).with("reason", "timeout"),
            ToolError::new(ToolErrorKind::FetchFailed)
                .with("reason", "blocked")
                .with("url", "http://127.0.0.1/"),
            ToolError::new(ToolErrorKind::UnsupportedContentType)
                .with("content_type", "application/pdf")
                .with("url", "https://example.com/a.pdf"),
            ToolError::new(ToolErrorKind::CharacterNotFound).with("name", "林昭"),
            ToolError::new(ToolErrorKind::CharacterExists).with("name", "林昭"),
            ToolError::new(ToolErrorKind::ReorderMismatch)
                .with("expected", vec!["林昭".to_owned(), "沈砚".to_owned()]),
            ToolError::new(ToolErrorKind::RelationNotFound)
                .with("source", "甲")
                .with("target", "乙")
                .with("kind", "ally"),
            ToolError::new(ToolErrorKind::SeedOnly)
                .with("tool", "update_entity")
                .with("field", "summary"),
            ToolError::new(ToolErrorKind::ExactlyOne).with("fields", vec!["name", "id"]),
            ToolError::new(ToolErrorKind::MutuallyExclusive)
                .with("fields", vec!["content", "edits"]),
            ToolError::tool_not_allowed(
                "write_chapter",
                "正文阶段".to_owned(),
                vec!["read_chapter".to_owned()],
            ),
            ToolError::new(ToolErrorKind::OutputTruncated),
            ToolError::new(ToolErrorKind::AfterCompletion),
            ToolError::new(ToolErrorKind::Refusal),
            ToolError::new(ToolErrorKind::Internal),
        ];
        for case in cases {
            let outcome = ToolOutcome::error(&prompts, &case);
            assert!(outcome.is_error, "{} must be an error", case.kind.code());
            assert!(
                !outcome.text.trim().is_empty(),
                "{} rendered an EMPTY text: its fields do not match the result.error branch",
                case.kind.code()
            );
            assert!(
                !outcome.text.contains("{{"),
                "{} left a template placeholder: {}",
                case.kind.code(),
                outcome.text
            );
            // The fallback text is the internal-error branch's; only `internal` may use it.
            if case.kind != ToolErrorKind::Internal {
                assert!(
                    !outcome.text.contains("内部错误"),
                    "{} fell through to the internal-error text: {}",
                    case.kind.code(),
                    outcome.text
                );
            }
        }
    }

    fn prompts() -> PromptSet {
        crate::prompts::PromptRegistry::embedded()
            .expect("the embedded prompt pack")
            .defaults_only()
    }

    fn truncated_notice(prompts: &PromptSet) -> String {
        prompts
            .render(
                "result.truncated",
                &serde_json::json!({ "next_offset": serde_json::Value::Null }),
            )
            .expect("result.truncated")
            .text
    }

    #[test]
    fn the_result_cap_leaves_a_result_at_exactly_the_limit_untouched() {
        let prompts = prompts();
        let text = "x".repeat(RESULT_CHAR_LIMIT);
        assert_eq!(cap_result(&prompts, &text, "get_entity"), text);
    }

    #[test]
    fn the_result_cap_cuts_at_a_line_boundary_and_appends_the_template() {
        let prompts = prompts();
        let line = format!("{} {}", "y".repeat(64), "z".repeat(8));
        let text = (0..400)
            .map(|index| format!("{line} {index:03}"))
            .collect::<Vec<_>>()
            .join("\n");
        let capped = cap_result(&prompts, &text, "get_entity");
        let notice = truncated_notice(&prompts);
        assert!(capped.ends_with(&notice), "the template is appended");
        let head = capped
            .strip_suffix(&notice)
            .expect("the notice suffix")
            .trim_end_matches('\n');
        assert!(head.chars().count() <= RESULT_CHAR_LIMIT);
        let full_line = format!("{line} ");
        assert!(
            head.lines()
                .all(|candidate| candidate.starts_with(&full_line)),
            "every line in the head is complete"
        );
    }

    #[test]
    fn the_result_cap_counts_characters_not_bytes() {
        let prompts = prompts();
        // 11,999 CJK characters are ~36 KB but still within a character budget.
        let cjk = "字".repeat(RESULT_CHAR_LIMIT - 1);
        assert_eq!(cap_result(&prompts, &cjk, "get_entity"), cjk);
        // One character past the cap truncates; the cut is a valid UTF-8 boundary.
        let longer = "字".repeat(RESULT_CHAR_LIMIT + 1);
        let capped = cap_result(&prompts, &longer, "get_entity");
        assert!(capped.contains('（'), "the rendered notice is there");
        assert!(capped.chars().count() >= RESULT_CHAR_LIMIT);
    }

    #[test]
    fn a_tool_owned_cap_is_not_cut_by_the_global_one() {
        let prompts = prompts();
        let text = "x".repeat(RESULT_CHAR_LIMIT + 500);
        for tool in [
            "read_chapter",
            "read_draft",
            "web_fetch",
            "read_knowledge_entry",
        ] {
            assert_eq!(
                cap_result(&prompts, &text, tool),
                text,
                "{tool} owns its cap"
            );
        }
    }
}
