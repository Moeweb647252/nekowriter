//! Typed JSON column wrappers (spec/02 §4).
//!
//! Stored as `TEXT` with `json_valid` CHECK constraints and mapped through serde. Every struct
//! derives `Serialize, Deserialize, Clone, Debug, TS` with `#[serde(rename_all = "snake_case")]`
//! and defaults on optional field groups so old rows keep loading.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use sea_orm::{DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

// ───────────────────────────── Enumerations (spec/02 §3) ─────────────────────────────

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
            TS, EnumIter, DeriveActiveEnum,
        )]
        #[serde(rename_all = "snake_case")]
        #[sea_orm(rs_type = "String", db_type = "Text")]
        #[ts(export)]
        pub enum $name {
            $(
                #[serde(rename = $value)]
                #[ts(rename = $value)]
                #[sea_orm(string_value = $value)]
                $variant,
            )+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value,)+
                }
            }

            pub fn from_str_opt(value: &str) -> Option<Self> {
                match value {
                    $($value => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::from_str_opt(value).ok_or_else(|| format!("unknown {}: {value}", stringify!($name)))
            }
        }
    };
}

string_enum! {
    /// Wire protocol of a provider (spec/02 §3).
    Protocol {
        Openai => "openai",
        Anthropic => "anthropic",
        Mock => "mock",
    }
}

string_enum! {
    BookStatus {
        Conceiving => "conceiving",
        Active => "active",
        Archived => "archived",
    }
}

string_enum! {
    TrackKind {
        Main => "main",
        Side => "side",
    }
}

string_enum! {
    CanonMode {
        Canon => "canon",
        Isolated => "isolated",
    }
}

string_enum! {
    AnchorKind {
        None => "none",
        BeforeStart => "before_start",
        AfterChapter => "after_chapter",
        AfterEnd => "after_end",
    }
}

string_enum! {
    ChapterStatus {
        Planned => "planned",
        Drafting => "drafting",
        InReview => "in_review",
        Revising => "revising",
        Official => "official",
    }
}

string_enum! {
    CanonStatus {
        None => "none",
        Pending => "pending",
        Syncing => "syncing",
        Synced => "synced",
        Stale => "stale",
        Failed => "failed",
    }
}

string_enum! {
    /// Chapter version kinds (spec/02 §3).
    VersionKind {
        Draft => "draft",
        Revision => "revision",
        Official => "official",
        Manual => "manual",
        Backup => "backup",
    }
}

string_enum! {
    SettingDocKind {
        Outline => "outline",
        World => "world",
        Protagonist => "protagonist",
    }
}

string_enum! {
    EntityKind {
        Character => "character",
        Organization => "organization",
        Location => "location",
        Item => "item",
        Other => "other",
    }
}

string_enum! {
    EntityStatus {
        Active => "active",
        Inactive => "inactive",
        Dead => "dead",
        Destroyed => "destroyed",
        Missing => "missing",
        Unknown => "unknown",
    }
}

string_enum! {
    AppearanceRole {
        Pov => "pov",
        Major => "major",
        Minor => "minor",
        Mentioned => "mentioned",
    }
}

string_enum! {
    NoteKind {
        Fact => "fact",
        WorldRule => "world_rule",
        Other => "other",
    }
}

string_enum! {
    NoteStatus {
        Active => "active",
        Retired => "retired",
    }
}

string_enum! {
    ForeshadowingStatus {
        Open => "open",
        Resolved => "resolved",
        Abandoned => "abandoned",
    }
}

string_enum! {
    ChangeActor {
        User => "user",
        Agent => "agent",
        System => "system",
    }
}

string_enum! {
    ChangeOp {
        Create => "create",
        Update => "update",
        Delete => "delete",
    }
}

string_enum! {
    AgentRole {
        Conception => "conception",
        Researcher => "researcher",
        Planner => "planner",
        Writer => "writer",
        Reviewer => "reviewer",
        Archivist => "archivist",
    }
}

string_enum! {
    SessionKind {
        Conception => "conception",
        Research => "research",
        ChapterOutline => "chapter_outline",
        ChapterWrite => "chapter_write",
        ChapterRevise => "chapter_revise",
        ChapterReview => "chapter_review",
        ChapterMaintenance => "chapter_maintenance",
        CanonSeed => "canon_seed",
        ChapterResync => "chapter_resync",
    }
}

string_enum! {
    SessionStatus {
        Idle => "idle",
        Running => "running",
        Held => "held",
        AwaitingInput => "awaiting_input",
        Paused => "paused",
        Completed => "completed",
        Stopped => "stopped",
        Failed => "failed",
        Interrupted => "interrupted",
        Superseded => "superseded",
        Abandoned => "abandoned",
    }
}

string_enum! {
    Phase {
        None => "none",
        Planning => "planning",
        Drafting => "drafting",
        RevisionPlanning => "revision_planning",
        Revising => "revising",
    }
}

string_enum! {
    Granularity {
        Sentence => "sentence",
        Paragraph => "paragraph",
        Free => "free",
        Chapter => "chapter",
    }
}

string_enum! {
    ToolCallStatus {
        Pending => "pending",
        Executing => "executing",
        Held => "held",
        AwaitingInput => "awaiting_input",
        Done => "done",
        Error => "error",
    }
}

string_enum! {
    ReviewVerdict {
        Pass => "pass",
        Revise => "revise",
    }
}

string_enum! {
    IssueSeverity {
        Critical => "critical",
        Major => "major",
        Minor => "minor",
    }
}

string_enum! {
    IssueCategory {
        Canon => "canon",
        Continuity => "continuity",
        Outline => "outline",
        Character => "character",
        Logic => "logic",
        Pacing => "pacing",
        Prose => "prose",
        Other => "other",
    }
}

string_enum! {
    FeedbackMode {
        Revise => "revise",
        Redraft => "redraft",
    }
}

string_enum! {
    RelationStatus {
        Active => "active",
        Ended => "ended",
    }
}

string_enum! {
    ChangeTargetType {
        Entity => "entity",
        Relation => "relation",
        AppearanceSet => "appearance_set",
        TimelineEvent => "timeline_event",
        MemoryNote => "memory_note",
        Foreshadowing => "foreshadowing",
        ChapterSummary => "chapter_summary",
        SettingDoc => "setting_doc",
        CharacterDesign => "character_design",
    }
}

string_enum! {
    ProposalStatus {
        Pending => "pending",
        Accepted => "accepted",
        Rejected => "rejected",
        Conflict => "conflict",
    }
}

string_enum! {
    ProposalTargetType {
        SettingDoc => "setting_doc",
        CharacterDesign => "character_design",
        NewCharacter => "new_character",
    }
}

string_enum! {
    /// The lifecycle of a research task (spec/02 §3, spec/07 §8).
    ///
    /// `Deferred` (wire `deferred`) is the author's "not now" (D-138): the task keeps its topic,
    /// requirements and scope but drops out of the runnable set, so it no longer makes
    /// `run_research` the overview's next action (D-136). It is a state of the task, not of its
    /// session — deferring never touches the session columns, and `run` accepts a deferred task
    /// directly. It is entered from `ready` only, and left by `resume` (back to `ready`) or by a
    /// run (to `running`).
    ResearchTaskStatus {
        Draft => "draft",
        Ready => "ready",
        Deferred => "deferred",
        Running => "running",
        Done => "done",
        Failed => "failed",
        Cancelled => "cancelled",
    }
}

string_enum! {
    EntrySource {
        Manual => "manual",
        Research => "research",
    }
}

string_enum! {
    SearchDocKind {
        Entity => "entity",
        Note => "note",
        Foreshadowing => "foreshadowing",
        Event => "event",
        Summary => "summary",
        Passage => "passage",
    }
}

string_enum! {
    EmbedState {
        Pending => "pending",
        Done => "done",
        Failed => "failed",
    }
}

string_enum! {
    MessageRole {
        User => "user",
        Assistant => "assistant",
    }
}

string_enum! {
    MessageKind {
        Kickoff => "kickoff",
        User => "user",
        Notice => "notice",
        Nudge => "nudge",
        Assistant => "assistant",
    }
}

string_enum! {
    InboxKind {
        UserMessage => "user_message",
        Notice => "notice",
    }
}

string_enum! {
    PromptScope {
        Global => "global",
        Book => "book",
    }
}

// ───────────────────────────── books ─────────────────────────────

/// `books.settings` (spec/02 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "snake_case")]
#[ts(export)]
pub struct BookSettings {
    pub granularity: Granularity,
    pub auto_pause: bool,
    pub pause_after_outline: bool,
    /// Target length in UAX #29 words (CJK characters count one each).
    pub target_words_min: i32,
    pub target_words_max: i32,
    pub recent_summary_count: i32,
    /// Number of chapters in the mid-term window.
    pub midterm_window: i32,
    /// How many prior official chapters `block.recent_chapters` injects in full (design §6.4).
    /// `0` disables the block.
    pub recent_chapter_count: i32,
    pub auto_maintenance: bool,
}

impl Default for BookSettings {
    fn default() -> Self {
        Self {
            granularity: Granularity::Paragraph,
            auto_pause: false,
            pause_after_outline: true,
            target_words_min: 3000,
            target_words_max: 4000,
            recent_summary_count: 5,
            midterm_window: 10,
            recent_chapter_count: 2,
            auto_maintenance: true,
        }
    }
}

/// `books.model_overrides` and `app_settings["roles.defaults"]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "snake_case")]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct RoleModels {
    pub conception: Option<i32>,
    pub researcher: Option<i32>,
    /// The planner's own model; unset means "follow the writer" (spec/03 §4).
    pub planner: Option<i32>,
    pub writer: Option<i32>,
    pub reviewer: Option<i32>,
    pub archivist: Option<i32>,
}

impl RoleModels {
    pub fn get(&self, role: AgentRole) -> Option<i32> {
        match role {
            AgentRole::Conception => self.conception,
            AgentRole::Researcher => self.researcher,
            AgentRole::Planner => self.planner,
            AgentRole::Writer => self.writer,
            AgentRole::Reviewer => self.reviewer,
            AgentRole::Archivist => self.archivist,
        }
    }

    pub fn set(&mut self, role: AgentRole, value: Option<i32>) {
        match role {
            AgentRole::Conception => self.conception = value,
            AgentRole::Researcher => self.researcher = value,
            AgentRole::Planner => self.planner = value,
            AgentRole::Writer => self.writer = value,
            AgentRole::Reviewer => self.reviewer = value,
            AgentRole::Archivist => self.archivist = value,
        }
    }
}

/// `books.conception` (spec/02 §4).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "snake_case")]
#[ts(export)]
pub struct ConceptionState {
    pub session_id: Option<i32>,
    pub characters_revision: i32,
    pub characters_approved_revision: Option<i32>,
    pub topics_revision: i32,
    pub topics_approved_revision: Option<i32>,
    pub finalized_at: Option<DateTime<Utc>>,
}

// ───────────────────────────── chapters ─────────────────────────────

/// One paragraph of chapter text. `pid` is `p<n>`, unique within a chapter and never reused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct Paragraph {
    pub pid: String,
    pub text: String,
}

/// `chapters.content` and `chapter_versions.content` (spec/02 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "snake_case")]
#[ts(export)]
pub struct ChapterContent {
    pub paragraphs: Vec<Paragraph>,
    /// Next number to assign; pids are never reused within a chapter.
    pub next_pid: i32,
    /// The last paragraph is unfinished (sentence/free granularity).
    pub tail_open: bool,
}

impl Default for ChapterContent {
    fn default() -> Self {
        Self {
            paragraphs: Vec::new(),
            next_pid: 1,
            tail_open: false,
        }
    }
}

// ───────────────────────────── agent sessions ─────────────────────────────

/// `agent_sessions.view_snapshot` (spec/02 §4, spec/03 §7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ViewSnapshot {
    Chapter {
        content: ChapterContent,
        outline: String,
        reference_notes: String,
    },
    Conception {
        docs: BTreeMap<String, ConceptionDocSnapshot>,
        characters: Vec<ConceptionCharacterSnapshot>,
        topics: Vec<ConceptionTopicSnapshot>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct ConceptionDocSnapshot {
    pub revision: i32,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct ConceptionCharacterSnapshot {
    pub id: i32,
    pub rank: i32,
    pub name: String,
    pub role_label: String,
    pub summary: String,
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct ConceptionTopicSnapshot {
    pub topic: String,
    pub requirements: String,
    pub scope: String,
}

/// `agent_messages.content` (spec/02 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum StoredBlock {
    Text {
        text: String,
        /// Place a cache breakpoint after this block.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache: Option<bool>,
    },
    /// Display only; replay uses `provider_payload`.
    Reasoning { text: String },
    ToolCall {
        call_id: String,
        name: String,
        input_text: String,
    },
}

/// `agent_messages.provider_payload` — exact replay material (spec/02 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "protocol", rename_all = "snake_case")]
#[ts(export)]
pub enum ProviderPayload {
    Openai {
        content: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
        tool_calls: Vec<Value>,
    },
    Anthropic {
        content: Vec<Value>,
    },
    Mock {
        content: Value,
    },
}

/// `agent_sessions.model_snapshot` (spec/02 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct ModelSnapshot {
    pub provider_id: i32,
    pub protocol: Protocol,
    pub base_url: String,
    pub extra_headers: BTreeMap<String, String>,
    pub model: String,
    pub context_window: i32,
    pub max_output_tokens: i32,
    pub options: Value,
}

impl Default for ModelSnapshot {
    fn default() -> Self {
        Self {
            provider_id: 0,
            protocol: Protocol::Mock,
            base_url: String::new(),
            extra_headers: BTreeMap::new(),
            model: String::new(),
            context_window: 0,
            max_output_tokens: 0,
            options: Value::Null,
        }
    }
}

/// `agent_sessions.counters` (spec/02 §4).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "snake_case")]
#[ts(export)]
pub struct SessionCounters {
    pub steps: i32,
    pub tool_calls: i32,
    pub retrieval_calls: i32,
    pub consecutive_tool_errors: i32,
    pub consecutive_nudges: i32,
    /// A completion was withheld once for confirmation (spec/05 §4, §6).
    pub finish_warned: bool,
}

/// Cumulative token usage of a session or a single request (spec/02 §4).
///
/// The counters are `i64` in Rust but cross the wire as JSON numbers, which `JSON.parse` always
/// yields as `number`, so the exported TS type is `number` rather than `bigint`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "snake_case")]
#[ts(export)]
pub struct Usage {
    pub requests: i32,
    #[ts(type = "number")]
    pub input_tokens: i64,
    #[ts(type = "number")]
    pub cached_input_tokens: i64,
    #[ts(type = "number")]
    pub cache_write_tokens: i64,
    #[ts(type = "number")]
    pub output_tokens: i64,
    #[ts(type = "number")]
    pub reasoning_tokens: i64,
    /// Total prompt size of the latest request (rollover check).
    #[ts(type = "number")]
    pub last_prompt_tokens: i64,
    /// The provider sent no usage; counts are estimates (spec/04 §2).
    pub estimated: bool,
}

impl Usage {
    /// Accumulate one request's usage into this cumulative total.
    pub fn accumulate(&mut self, other: &Usage) {
        self.requests += other.requests.max(1);
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.cache_write_tokens += other.cache_write_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_tokens += other.reasoning_tokens;
        self.last_prompt_tokens =
            other.input_tokens + other.cached_input_tokens + other.cache_write_tokens;
        self.estimated |= other.estimated;
    }
}

/// `agent_sessions.error` (spec/02 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct SessionError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub at: DateTime<Utc>,
}

/// `agent_messages.request_meta` — state in effect when the model generated the message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct RequestMeta {
    pub phase: Phase,
    pub granularity: Option<Granularity>,
}

/// `agent_tool_calls.appendices` — composed into `result_text` at sealing (spec/03 §5.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct Appendix {
    pub kind: AppendixKind,
    /// Already rendered from its prompt template.
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbox_id: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AppendixKind {
    EffectNotice,
    EditReport,
    UserFeedback,
    Inbox,
}

impl AppendixKind {
    /// Canonical composition order (spec/03 §5.3).
    pub fn canonical_rank(self) -> u8 {
        match self {
            AppendixKind::EffectNotice => 0,
            AppendixKind::EditReport => 1,
            AppendixKind::UserFeedback => 2,
            AppendixKind::Inbox => 3,
        }
    }
}

/// `agent_tool_calls.effects` — applied when the call is finalized, not when executed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "snake_case")]
#[ts(export)]
pub struct ToolEffects {
    pub phase_to: Option<Phase>,
    pub complete: Option<CompletionKind>,
    pub await_input: Option<AwaitInputKind>,
    /// A text-changing writer call's staged chapter write, applied by the transaction that
    /// finalizes the call (spec/03 §5.2, §5.4). `None` for every other tool.
    ///
    /// This is an effect, not domain state: `chapters.content` stays untouched by `execute`, so a
    /// held call's text is not in the chapter until the call is approved. Feedback on the held
    /// call revokes the effects (spec/03 §5.4), which is what discards the write.
    pub pending_content: Option<PendingContent>,
    /// An outline session's staged outline, applied by the transaction that finalizes an accepted
    /// `submit_outline` (spec/03 §5.2, D-146). `None` for every other tool and session kind.
    ///
    /// Like `pending_content` this is an effect, not domain state: `chapters.outline` holds only
    /// an accepted or author-written outline, so a held or rejected proposal never reaches it.
    pub pending_outline: Option<PendingOutline>,
}

/// A writer call's staged chapter write (spec/03 §5.2): the content `execute` computed but did not
/// store, so that only the transaction which finalizes the call can put it in the chapter.
///
/// `expected_version` is the `chapters.content_version` the tool read. The author may edit the
/// chapter freely while a call is held (spec/08 §1.4), so approval compares it against the current
/// version: while it still matches, the staged content applies exactly; once the author has moved
/// the chapter, the staged write is dropped rather than reverting the author's work.
///
/// The ops and word count that `chapter.content` and `chapters.word_count` need are derived from
/// the state the content is applied to (`text::transform_ops`, `text::content_word_count`), so they
/// cannot fall out of sync with it and are not stored here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct PendingContent {
    pub content: ChapterContent,
    pub expected_version: i32,
}

/// An outline session's staged proposal (spec/03 §5.2, D-146). `title` is applied only while the
/// chapter's title is still empty at apply time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct PendingOutline {
    pub outline: String,
    pub reference_notes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CompletionKind {
    Draft,
    Revision,
    Review,
    Maintenance,
    Research,
    /// `submit_outline` in a `chapter_outline` session (the staged outline is applied first).
    Outline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AwaitInputKind {
    AskUser,
    RequestReview,
}

// ───────────────────────────── reviews ─────────────────────────────

/// `review_reports.issues` (spec/02 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct ReviewIssue {
    pub severity: IssueSeverity,
    pub category: IssueCategory,
    pub pids: Vec<String>,
    pub quote: String,
    pub problem: String,
    pub suggestion: String,
}

// ───────────────────────────── proposals ─────────────────────────────

/// `setting_proposals.edits` (spec/02 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ProposalEdits {
    Edits { edits: Vec<FindReplace> },
    Append { text: String },
    CreateCharacter { character: NewCharacter },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct FindReplace {
    /// Must match exactly once.
    pub find: String,
    pub replace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct NewCharacter {
    pub name: String,
    pub role_label: String,
    pub summary: String,
    pub profile: String,
}

// ───────────────────────────── snapshots (change_log) ─────────────────────────────

/// `change_log.before` / `after` — only domain fields, no timestamps (spec/02 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum Snapshot {
    Entity {
        kind: EntityKind,
        name: String,
        aliases: Vec<String>,
        summary: String,
        description: String,
        state: String,
        attributes: BTreeMap<String, String>,
        importance: i32,
        status: EntityStatus,
        is_protagonist: bool,
        track_id: Option<i32>,
    },
    Relation {
        source_id: i32,
        target_id: i32,
        kind: String,
        inverse_kind: String,
        description: String,
        status: RelationStatus,
        since_chapter_id: Option<i32>,
        until_chapter_id: Option<i32>,
        track_id: Option<i32>,
    },
    TimelineEvent {
        position: f64,
        story_time: String,
        title: String,
        description: String,
        chapter_id: Option<i32>,
        importance: i32,
        entity_ids: Vec<i32>,
        track_id: Option<i32>,
    },
    Note {
        kind: NoteKind,
        content: String,
        chapter_id: Option<i32>,
        importance: i32,
        status: NoteStatus,
        entity_ids: Vec<i32>,
        track_id: Option<i32>,
    },
    Foreshadowing {
        title: String,
        description: String,
        planned_payoff: String,
        status: ForeshadowingStatus,
        planted_chapter_id: Option<i32>,
        resolved_chapter_id: Option<i32>,
        resolution: String,
        importance: i32,
        entity_ids: Vec<i32>,
        track_id: Option<i32>,
    },
    AppearanceSet {
        appearances: Vec<AppearanceSnapshot>,
    },
    ChapterSummary {
        summary: String,
    },
    SettingDoc {
        content: String,
    },
    CharacterDesign {
        rank: i32,
        name: String,
        role_label: String,
        summary: String,
        profile: String,
        entity_id: Option<i32>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct AppearanceSnapshot {
    pub entity_id: i32,
    pub role: AppearanceRole,
    pub note: String,
}

// ───────────────────────────── helpers ─────────────────────────────

/// Serializes a typed JSON column to the stored `TEXT` form.
pub fn to_json<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(value)
}

/// Deserializes a `TEXT` JSON column. A corrupt value is a database defect, never a user error.
pub fn from_json<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(raw)
}

/// Parses a JSON column, falling back to the type's default on invalid content. Used for
/// percent-new optional columns so a hand-edited database cannot break the API.
pub fn from_json_or_default<T: for<'de> Deserialize<'de> + Default>(raw: &str) -> T {
    serde_json::from_str(raw).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Round-trips one typed JSON column value through its stored `TEXT` form, exactly as
    /// `Db::write` stores it and as `from_json` reads it back (PLAN 0.3).
    fn round_trip<T>(value: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        let stored = to_json(value).expect("serialize to the column TEXT form");
        from_json(&stored).expect("deserialize back from the column TEXT form")
    }

    // Every typed JSON column of spec/02 §2 gets a round trip below.

    /// `books.settings`.
    #[test]
    fn book_settings_round_trip() {
        let value = BookSettings {
            granularity: Granularity::Sentence,
            auto_pause: true,
            pause_after_outline: false,
            target_words_min: 1500,
            target_words_max: 2500,
            recent_summary_count: 3,
            midterm_window: 7,
            recent_chapter_count: 1,
            auto_maintenance: false,
        };
        assert_eq!(round_trip(&value), value);
    }

    /// `books.model_overrides`, `app_settings["roles.defaults"]`.
    #[test]
    fn role_models_round_trip() {
        let value = RoleModels {
            conception: Some(1),
            researcher: None,
            planner: None,
            writer: Some(3),
            reviewer: Some(4),
            archivist: None,
        };
        assert_eq!(round_trip(&value), value);
    }

    /// `books.conception`.
    #[test]
    fn conception_state_round_trip() {
        let value = ConceptionState {
            session_id: Some(9),
            characters_revision: 2,
            characters_approved_revision: Some(1),
            topics_revision: 4,
            topics_approved_revision: None,
            finalized_at: Some(DateTime::from_timestamp_millis(1_767_225_600_000).expect("ts")),
        };
        assert_eq!(round_trip(&value), value);
    }

    /// `chapters.content`, `chapter_versions.content`.
    #[test]
    fn chapter_content_round_trip() {
        let value = ChapterContent {
            paragraphs: vec![
                Paragraph {
                    pid: "p1".to_owned(),
                    text: "First.".to_owned(),
                },
                Paragraph {
                    pid: "p2".to_owned(),
                    text: "Second.".to_owned(),
                },
            ],
            next_pid: 3,
            tail_open: true,
        };
        assert_eq!(round_trip(&value), value);
    }

    /// `agent_sessions.view_snapshot` (both variants).
    #[test]
    fn view_snapshot_round_trip() {
        let chapter = ViewSnapshot::Chapter {
            content: ChapterContent::default(),
            outline: "outline".to_owned(),
            reference_notes: "notes".to_owned(),
        };
        assert_eq!(round_trip(&chapter), chapter);

        let mut docs = BTreeMap::new();
        docs.insert(
            "outline".to_owned(),
            ConceptionDocSnapshot {
                revision: 2,
                content: "doc".to_owned(),
            },
        );
        let conception = ViewSnapshot::Conception {
            docs,
            characters: vec![ConceptionCharacterSnapshot {
                id: 1,
                rank: 1,
                name: "A".to_owned(),
                role_label: "lead".to_owned(),
                summary: "sum".to_owned(),
                profile: "prof".to_owned(),
            }],
            topics: vec![ConceptionTopicSnapshot {
                topic: "t".to_owned(),
                requirements: "r".to_owned(),
                scope: "s".to_owned(),
            }],
        };
        assert_eq!(round_trip(&conception), conception);
    }

    /// `agent_messages.content`.
    #[test]
    fn stored_block_round_trip() {
        let blocks = vec![
            StoredBlock::Text {
                text: "hello".to_owned(),
                cache: Some(true),
            },
            StoredBlock::Reasoning {
                text: "thinking".to_owned(),
            },
            StoredBlock::ToolCall {
                call_id: "call_1".to_owned(),
                name: "read_chapter".to_owned(),
                input_text: "{}".to_owned(),
            },
        ];
        assert_eq!(round_trip(&blocks), blocks);
    }

    /// `agent_messages.provider_payload` (all three protocols).
    #[test]
    fn provider_payload_round_trip() {
        let openai = ProviderPayload::Openai {
            content: Some("text".to_owned()),
            reasoning_content: Some("reasoning".to_owned()),
            tool_calls: vec![json!({"id": "call_1", "type": "function"})],
        };
        assert_eq!(round_trip(&openai), openai);

        let anthropic = ProviderPayload::Anthropic {
            content: vec![json!({"type": "thinking", "signature": "sig"})],
        };
        assert_eq!(round_trip(&anthropic), anthropic);

        let mock = ProviderPayload::Mock {
            content: json!({"anything": [1, 2, 3]}),
        };
        assert_eq!(round_trip(&mock), mock);
    }

    /// `agent_sessions.model_snapshot`.
    #[test]
    fn model_snapshot_round_trip() {
        let mut extra_headers = BTreeMap::new();
        extra_headers.insert("x-extra".to_owned(), "1".to_owned());
        let value = ModelSnapshot {
            provider_id: 2,
            protocol: Protocol::Anthropic,
            base_url: "https://example.test".to_owned(),
            extra_headers,
            model: "a-model".to_owned(),
            context_window: 200_000,
            max_output_tokens: 8192,
            options: json!({"cache_ttl": "5m"}),
        };
        assert_eq!(round_trip(&value), value);
    }

    /// `agent_sessions.counters`.
    #[test]
    fn session_counters_round_trip() {
        let value = SessionCounters {
            steps: 7,
            tool_calls: 3,
            retrieval_calls: 1,
            consecutive_tool_errors: 2,
            consecutive_nudges: 1,
            finish_warned: true,
        };
        assert_eq!(round_trip(&value), value);
    }

    /// `agent_sessions.usage`, `agent_messages.usage`.
    #[test]
    fn usage_round_trip() {
        let value = Usage {
            requests: 5,
            input_tokens: 1000,
            cached_input_tokens: 200,
            cache_write_tokens: 50,
            output_tokens: 300,
            reasoning_tokens: 20,
            last_prompt_tokens: 1250,
            estimated: true,
        };
        assert_eq!(round_trip(&value), value);
    }

    /// `agent_sessions.error`.
    #[test]
    fn session_error_round_trip() {
        let value = SessionError {
            code: "provider.unavailable".to_owned(),
            message: "upstream failed".to_owned(),
            retryable: true,
            at: DateTime::from_timestamp_millis(1_767_225_600_000).expect("ts"),
        };
        assert_eq!(round_trip(&value), value);
    }

    /// `agent_messages.request_meta`.
    #[test]
    fn request_meta_round_trip() {
        let value = RequestMeta {
            phase: Phase::Drafting,
            granularity: Some(Granularity::Free),
        };
        assert_eq!(round_trip(&value), value);

        let no_granularity = RequestMeta {
            phase: Phase::None,
            granularity: None,
        };
        assert_eq!(round_trip(&no_granularity), no_granularity);
    }

    /// `agent_tool_calls.appendices` (all kinds).
    #[test]
    fn appendix_round_trip() {
        let value = vec![
            Appendix {
                kind: AppendixKind::EffectNotice,
                text: "notice".to_owned(),
                inbox_id: None,
            },
            Appendix {
                kind: AppendixKind::EditReport,
                text: "report".to_owned(),
                inbox_id: None,
            },
            Appendix {
                kind: AppendixKind::UserFeedback,
                text: "feedback".to_owned(),
                inbox_id: None,
            },
            Appendix {
                kind: AppendixKind::Inbox,
                text: "inbox".to_owned(),
                inbox_id: Some(4),
            },
        ];
        assert_eq!(round_trip(&value), value);
    }

    /// `agent_tool_calls.effects`.
    #[test]
    fn tool_effects_round_trip() {
        let value = ToolEffects {
            phase_to: Some(Phase::Drafting),
            complete: Some(CompletionKind::Draft),
            await_input: Some(AwaitInputKind::RequestReview),
            pending_content: Some(PendingContent {
                content: ChapterContent {
                    paragraphs: vec![Paragraph {
                        pid: "p1".to_owned(),
                        text: "第一段。".to_owned(),
                    }],
                    next_pid: 2,
                    tail_open: false,
                },
                expected_version: 3,
            }),
            pending_outline: None,
        };
        assert_eq!(round_trip(&value), value);
        assert_eq!(round_trip(&ToolEffects::default()), ToolEffects::default());
    }

    /// A held call's row written before this field existed must still load (spec/02 §4: defaults on
    /// optional field groups so old rows keep loading).
    #[test]
    fn tool_effects_without_pending_content_still_loads() {
        let raw = r#"{"phase_to":null,"complete":null,"await_input":null}"#;
        let effects: ToolEffects = serde_json::from_str(raw).expect("old rows decode");
        assert!(effects.pending_content.is_none());
    }

    /// `review_reports.issues`, `chapter_feedback.issues`.
    #[test]
    fn review_issue_round_trip() {
        let value = vec![ReviewIssue {
            severity: IssueSeverity::Major,
            category: IssueCategory::Continuity,
            pids: vec!["p3".to_owned(), "p7".to_owned()],
            quote: "quote".to_owned(),
            problem: "problem".to_owned(),
            suggestion: "suggestion".to_owned(),
        }];
        assert_eq!(round_trip(&value), value);
    }

    /// `setting_proposals.edits` (all three variants).
    #[test]
    fn proposal_edits_round_trip() {
        let edits = ProposalEdits::Edits {
            edits: vec![FindReplace {
                find: "a".to_owned(),
                replace: "b".to_owned(),
            }],
        };
        assert_eq!(round_trip(&edits), edits);

        let append = ProposalEdits::Append {
            text: "more".to_owned(),
        };
        assert_eq!(round_trip(&append), append);

        let character = ProposalEdits::CreateCharacter {
            character: NewCharacter {
                name: "N".to_owned(),
                role_label: "lead".to_owned(),
                summary: "s".to_owned(),
                profile: "p".to_owned(),
            },
        };
        assert_eq!(round_trip(&character), character);
    }

    /// `change_log.before` / `after` (every snapshot variant, spec/02 §4).
    #[test]
    fn snapshot_round_trip() {
        let mut attributes = BTreeMap::new();
        attributes.insert("age".to_owned(), "30".to_owned());
        let snapshots = vec![
            Snapshot::Entity {
                kind: EntityKind::Character,
                name: "A".to_owned(),
                aliases: vec!["B".to_owned()],
                summary: "s".to_owned(),
                description: "d".to_owned(),
                state: "st".to_owned(),
                attributes,
                importance: 3,
                status: EntityStatus::Active,
                is_protagonist: true,
                track_id: None,
            },
            Snapshot::Relation {
                source_id: 1,
                target_id: 2,
                kind: "friend".to_owned(),
                inverse_kind: "friend".to_owned(),
                description: "d".to_owned(),
                status: RelationStatus::Active,
                since_chapter_id: Some(1),
                until_chapter_id: None,
                track_id: Some(2),
            },
            Snapshot::TimelineEvent {
                position: 1.5,
                story_time: "day 1".to_owned(),
                title: "t".to_owned(),
                description: "d".to_owned(),
                chapter_id: None,
                importance: 2,
                entity_ids: vec![1, 2],
                track_id: None,
            },
            Snapshot::Note {
                kind: NoteKind::Fact,
                content: "c".to_owned(),
                chapter_id: Some(3),
                importance: 2,
                status: NoteStatus::Active,
                entity_ids: vec![],
                track_id: None,
            },
            Snapshot::Foreshadowing {
                title: "t".to_owned(),
                description: "d".to_owned(),
                planned_payoff: "p".to_owned(),
                status: ForeshadowingStatus::Open,
                planted_chapter_id: Some(1),
                resolved_chapter_id: None,
                resolution: String::new(),
                importance: 4,
                entity_ids: vec![1],
                track_id: None,
            },
            Snapshot::AppearanceSet {
                appearances: vec![AppearanceSnapshot {
                    entity_id: 1,
                    role: AppearanceRole::Pov,
                    note: "n".to_owned(),
                }],
            },
            Snapshot::ChapterSummary {
                summary: "s".to_owned(),
            },
            Snapshot::SettingDoc {
                content: "c".to_owned(),
            },
            Snapshot::CharacterDesign {
                rank: 1,
                name: "A".to_owned(),
                role_label: "lead".to_owned(),
                summary: "s".to_owned(),
                profile: "p".to_owned(),
                entity_id: Some(3),
            },
        ];
        assert_eq!(round_trip(&snapshots), snapshots);
    }

    /// Untagged union columns: a JSON `string[]` and a `Record<string, string>`.
    #[test]
    fn scalar_collections_round_trip() {
        let aliases: Vec<String> = vec!["one".to_owned(), "two".to_owned()];
        assert_eq!(round_trip(&aliases), aliases);

        let mut attributes = BTreeMap::new();
        attributes.insert("k".to_owned(), "v".to_owned());
        assert_eq!(round_trip(&attributes), attributes);

        let arbitrary = json!({"pids": ["p3", "p7"]});
        assert_eq!(round_trip(&arbitrary), arbitrary);
    }

    /// `from_json_or_default` keeps a hand-edited database loadable (spec/02 §4).
    #[test]
    fn invalid_json_falls_back_to_default() {
        let parsed: BookSettings = from_json_or_default("{not json");
        assert_eq!(parsed, BookSettings::default());
    }

    /// A stored column is always valid JSON, so `from_json` never sees a raw SQL `NULL`.
    #[test]
    fn stored_form_is_a_json_object() {
        let stored = to_json(&BookSettings::default()).expect("serialize");
        assert!(stored.starts_with('{'), "stored form was {stored}");
    }
}
