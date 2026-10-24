//! Typed JSON column wrappers (spec/02 §4).
//!
//! Stored as `TEXT` with `json_valid` CHECK constraints and mapped through serde. Every struct
//! derives `Serialize, Deserialize, Clone, Debug, TS` with `#[serde(rename_all = "snake_case")]`
//! and defaults on optional field groups so old rows keep loading.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

// ───────────────────────────── Enumerations (spec/02 §3) ─────────────────────────────

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
        #[serde(rename_all = "snake_case")]
        #[ts(export)]
        pub enum $name {
            $(
                #[serde(rename = $value)]
                #[ts(rename = $value)]
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
        Writer => "writer",
        Reviewer => "reviewer",
        Archivist => "archivist",
    }
}

string_enum! {
    SessionKind {
        Conception => "conception",
        Research => "research",
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
    ResearchTaskStatus {
        Draft => "draft",
        Ready => "ready",
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
            auto_maintenance: true,
        }
    }
}

/// `books.model_overrides` and `app_settings["roles.defaults"]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "snake_case")]
#[ts(export)]
pub struct RoleModels {
    pub conception: Option<i32>,
    pub researcher: Option<i32>,
    pub writer: Option<i32>,
    pub reviewer: Option<i32>,
    pub archivist: Option<i32>,
}

impl RoleModels {
    pub fn get(&self, role: AgentRole) -> Option<i32> {
        match role {
            AgentRole::Conception => self.conception,
            AgentRole::Researcher => self.researcher,
            AgentRole::Writer => self.writer,
            AgentRole::Reviewer => self.reviewer,
            AgentRole::Archivist => self.archivist,
        }
    }

    pub fn set(&mut self, role: AgentRole, value: Option<i32>) {
        match role {
            AgentRole::Conception => self.conception = value,
            AgentRole::Researcher => self.researcher = value,
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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "snake_case")]
#[ts(export)]
pub struct Usage {
    pub requests: i32,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    /// Total prompt size of the latest request (rollover check).
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
