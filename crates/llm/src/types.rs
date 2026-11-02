//! Neutral request/response types shared by every protocol adapter (spec/04 §1).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One model request.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    /// Fixed order, frozen per session.
    pub tools: Vec<ToolDef>,
    pub max_output_tokens: u32,
    /// `false` → at most one tool call per response.
    pub parallel_tool_calls: bool,
    pub cache: CacheHints,
    pub options: ModelOptions,
    /// Only read by the demo provider (spec/04 §7.2).
    pub demo_hints: Option<DemoHints>,
}

impl ChatRequest {
    /// A minimal request for connection tests (spec/04 §8).
    pub fn ping(model: impl Into<String>, system: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system: system.into(),
            messages: vec![Message::User {
                blocks: vec![TextBlock {
                    text: "ping".to_owned(),
                    cache: false,
                }],
            }],
            tools: Vec::new(),
            max_output_tokens: 32,
            parallel_tool_calls: true,
            cache: CacheHints {
                ttl: CacheTtl::FiveMinutes,
            },
            options: ModelOptions::default(),
            demo_hints: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// Kickoff, author messages, notices and nudges.
    User { blocks: Vec<TextBlock> },
    /// Exact replay of what the provider returned.
    Assistant { payload: ProviderPayload },
    /// Always directly after an Assistant message.
    ToolResults { results: Vec<ToolResult> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBlock {
    pub text: String,
    /// Place a cache breakpoint after this block.
    pub cache: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub call_id: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheTtl {
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "1h")]
    OneHour,
}

impl CacheTtl {
    /// The wire form used in `cache_control` (Anthropic).
    pub fn as_str(self) -> &'static str {
        match self {
            CacheTtl::FiveMinutes => "5m",
            CacheTtl::OneHour => "1h",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheHints {
    pub ttl: CacheTtl,
}

/// Exact replay material, mirroring `db::json::ProviderPayload` (spec/02 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "protocol", rename_all = "snake_case")]
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

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// `index` = provider content-block index (display grouping).
    TextDelta {
        index: u32,
        text: String,
    },
    ReasoningDelta {
        index: u32,
        text: String,
    },
    /// `call_index` = 0-based ordinal among this turn's tool calls.
    ToolCallStart {
        call_index: u32,
        id: String,
        name: String,
    },
    ToolCallArgsDelta {
        call_index: u32,
        fragment: String,
    },
    /// May arrive more than once; the adapter merges.
    Usage(Usage),
}

/// Token usage of one request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Uncached input tokens.
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    /// No usage was reported; the numbers are estimates (spec/04 §2).
    pub estimated: bool,
}

impl Usage {
    /// Merges a later usage event into this one, keeping the larger value per field. Adapters
    /// emit usage once at `message_start` (input side) and once at `message_delta` (output side).
    pub fn merge(&mut self, other: Usage) {
        self.input_tokens = self.input_tokens.max(other.input_tokens);
        self.cached_input_tokens = self.cached_input_tokens.max(other.cached_input_tokens);
        self.cache_write_tokens = self.cache_write_tokens.max(other.cache_write_tokens);
        self.output_tokens = self.output_tokens.max(other.output_tokens);
        self.reasoning_tokens = self.reasoning_tokens.max(other.reasoning_tokens);
        self.estimated |= other.estimated;
    }
}

/// One completed assistant turn.
#[derive(Debug, Clone)]
pub struct AssistantTurn {
    /// Visible text (concatenated).
    pub text: String,
    /// Reasoning/thinking summary for display.
    pub reasoning: String,
    pub tool_calls: Vec<ToolCallOut>,
    pub stop_reason: StopReason,
    pub usage: Usage,
    /// Replay material (spec/02 §4).
    pub payload: ProviderPayload,
    /// Provider request id, for error reports.
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallOut {
    pub id: String,
    pub name: String,
    /// Raw argument text exactly as received.
    pub input_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Refusal {
        category: Option<String>,
        explanation: Option<String>,
    },
    PauseTurn,
    Other(String),
}

impl StopReason {
    /// Lowercase wire-ish name, used in logs and in the persisted `stop_reason` column.
    pub fn as_str(&self) -> &str {
        match self {
            StopReason::EndTurn => "end_turn",
            StopReason::ToolUse => "tool_use",
            StopReason::MaxTokens => "max_tokens",
            StopReason::Refusal { .. } => "refusal",
            StopReason::PauseTurn => "pause_turn",
            StopReason::Other(other) => other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Openai,
    Anthropic,
    Mock,
}

/// Configuration for one provider connection, built by the server from the `providers` row at
/// call time (spec/04 §1).
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub protocol: Protocol,
    pub base_url: String,
    pub api_key: Option<String>,
    pub extra_headers: BTreeMap<String, String>,
}

/// Model behavior options (`model_profiles.options`, spec/04 §5).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelOptions {
    // Read by both adapters
    pub reasoning_replay: ReasoningReplay,
    pub cache_ttl: CacheTtlOption,
    // OpenAI-compatible only
    pub reasoning_effort: Option<String>,
    pub max_tokens_field: MaxTokensField,
    pub send_parallel_tool_calls: Option<bool>,
    pub stream_usage: Option<bool>,
    pub temperature: Option<f64>,
    pub extra_body: Option<BTreeMap<String, Value>>,
    // Anthropic only
    pub effort: Option<Effort>,
    pub thinking_display: Option<ThinkingDisplay>,
    pub refusal_fallback: Option<bool>,
    pub eager_input_streaming: Option<bool>,
}

impl ModelOptions {
    pub fn send_parallel_tool_calls(&self) -> bool {
        self.send_parallel_tool_calls.unwrap_or(true)
    }

    pub fn stream_usage(&self) -> bool {
        self.stream_usage.unwrap_or(true)
    }

    pub fn eager_input_streaming(&self) -> bool {
        self.eager_input_streaming.unwrap_or(true)
    }

    /// The options this value sets that `protocol` cannot send (spec/04 §5).
    ///
    /// This is the one operation save-time validation, the read-time report and the startup scan
    /// all need, so they cannot disagree about what "stranded" means. Defaults are deliberately not
    /// reported: `ModelOptions` is `Default`-initialized and stored with every field serialized, so
    /// treating a default as a deviation would flag every profile.
    pub fn unreachable_keys(&self, protocol: Protocol) -> Vec<ModelOptionKey> {
        self.set_keys()
            .into_iter()
            .filter(|key| !key.reachable_by(protocol))
            .collect()
    }

    /// Every option this value sets, without judging whether a protocol can send it.
    pub fn set_keys(&self) -> Vec<ModelOptionKey> {
        let mut keys = Vec::new();
        if self.reasoning_replay != ReasoningReplay::default() {
            keys.push(ModelOptionKey::ReasoningReplay);
        }
        if self.cache_ttl != CacheTtlOption::default() {
            keys.push(ModelOptionKey::CacheTtl);
        }
        if self.reasoning_effort.is_some() {
            keys.push(ModelOptionKey::ReasoningEffort);
        }
        if self.max_tokens_field != MaxTokensField::default() {
            keys.push(ModelOptionKey::MaxTokensField);
        }
        if self.send_parallel_tool_calls.is_some() {
            keys.push(ModelOptionKey::SendParallelToolCalls);
        }
        if self.stream_usage.is_some() {
            keys.push(ModelOptionKey::StreamUsage);
        }
        if self.temperature.is_some() {
            keys.push(ModelOptionKey::Temperature);
        }
        if self
            .extra_body
            .as_ref()
            .is_some_and(|extra| !extra.is_empty())
        {
            keys.push(ModelOptionKey::ExtraBody);
        }
        if self.effort.is_some() {
            keys.push(ModelOptionKey::Effort);
        }
        if self.thinking_display.is_some() {
            keys.push(ModelOptionKey::ThinkingDisplay);
        }
        if self.refusal_fallback.is_some() {
            keys.push(ModelOptionKey::RefusalFallback);
        }
        if self.eager_input_streaming.is_some() {
            keys.push(ModelOptionKey::EagerInputStreaming);
        }
        keys
    }

    /// Removes every option the protocol cannot send, and reports what was removed.
    ///
    /// Used only where a protocol change makes stored options strand (a model profile whose
    /// provider switched protocol); the API validates on save, so this is the cleanup that keeps
    /// an already-stored mismatch from surviving the next edit (spec/04 §5).
    pub fn clear_unreachable(&mut self, protocol: Protocol) -> Vec<ModelOptionKey> {
        let unreachable = self.unreachable_keys(protocol);
        for key in &unreachable {
            match key {
                ModelOptionKey::ReasoningEffort => self.reasoning_effort = None,
                ModelOptionKey::MaxTokensField => self.max_tokens_field = MaxTokensField::default(),
                ModelOptionKey::SendParallelToolCalls => self.send_parallel_tool_calls = None,
                ModelOptionKey::StreamUsage => self.stream_usage = None,
                ModelOptionKey::Temperature => self.temperature = None,
                ModelOptionKey::ExtraBody => self.extra_body = None,
                ModelOptionKey::Effort => self.effort = None,
                ModelOptionKey::ThinkingDisplay => self.thinking_display = None,
                ModelOptionKey::RefusalFallback => self.refusal_fallback = None,
                ModelOptionKey::EagerInputStreaming => self.eager_input_streaming = None,
                // Read by both adapters, so never unreachable; listed for exhaustiveness.
                ModelOptionKey::ReasoningReplay | ModelOptionKey::CacheTtl => {}
            }
        }
        unreachable
    }
}

/// One entry of `ModelOptions`, so a protocol can be asked about it by name (spec/04 §5).
///
/// The classification is *verified against the adapters*, not assumed from the doc comments: a
/// test in `crates/llm/tests/model_options.rs` builds both request bodies for every key and
/// asserts that the key reaches the wire exactly for the protocols listed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelOptionKey {
    ReasoningReplay,
    CacheTtl,
    ReasoningEffort,
    MaxTokensField,
    SendParallelToolCalls,
    StreamUsage,
    Temperature,
    ExtraBody,
    Effort,
    ThinkingDisplay,
    RefusalFallback,
    EagerInputStreaming,
}

impl ModelOptionKey {
    /// Every key, in the order of `ModelOptions` (spec/04 §5).
    pub const ALL: [ModelOptionKey; 12] = [
        ModelOptionKey::ReasoningReplay,
        ModelOptionKey::CacheTtl,
        ModelOptionKey::ReasoningEffort,
        ModelOptionKey::MaxTokensField,
        ModelOptionKey::SendParallelToolCalls,
        ModelOptionKey::StreamUsage,
        ModelOptionKey::Temperature,
        ModelOptionKey::ExtraBody,
        ModelOptionKey::Effort,
        ModelOptionKey::ThinkingDisplay,
        ModelOptionKey::RefusalFallback,
        ModelOptionKey::EagerInputStreaming,
    ];

    /// The options a model profile may hold (spec/04 §5, D-162).
    ///
    /// [`Self::ALL`] minus `reasoning_effort` and `effort`: those two stay in `ModelOptions` as
    /// the wire carriers for the thinking effort a role chooses, but they are not profile
    /// options. [`Self::ALL`] is kept for the adapter wire test; this list is what
    /// `GET /provider-presets/options` publishes and what a profile's `options` may hold.
    pub const PROFILE: [ModelOptionKey; 10] = [
        ModelOptionKey::ReasoningReplay,
        ModelOptionKey::CacheTtl,
        ModelOptionKey::MaxTokensField,
        ModelOptionKey::SendParallelToolCalls,
        ModelOptionKey::StreamUsage,
        ModelOptionKey::Temperature,
        ModelOptionKey::ExtraBody,
        ModelOptionKey::ThinkingDisplay,
        ModelOptionKey::RefusalFallback,
        ModelOptionKey::EagerInputStreaming,
    ];

    /// The JSON field name in `model_profiles.options` (spec/04 §5).
    pub fn as_str(self) -> &'static str {
        match self {
            ModelOptionKey::ReasoningReplay => "reasoning_replay",
            ModelOptionKey::CacheTtl => "cache_ttl",
            ModelOptionKey::ReasoningEffort => "reasoning_effort",
            ModelOptionKey::MaxTokensField => "max_tokens_field",
            ModelOptionKey::SendParallelToolCalls => "send_parallel_tool_calls",
            ModelOptionKey::StreamUsage => "stream_usage",
            ModelOptionKey::Temperature => "temperature",
            ModelOptionKey::ExtraBody => "extra_body",
            ModelOptionKey::Effort => "effort",
            ModelOptionKey::ThinkingDisplay => "thinking_display",
            ModelOptionKey::RefusalFallback => "refusal_fallback",
            ModelOptionKey::EagerInputStreaming => "eager_input_streaming",
        }
    }

    /// Whether the protocol's adapter sends this option at all.
    ///
    /// Verified against the adapters, not against the doc comments: `reasoning_replay` and
    /// `cache_ttl` are read on both sides — the former selects the replayed assistant payload in
    /// `openai.rs` and `anthropic.rs`, the latter is resolved into `CacheHints.ttl` by the server
    /// and consumed by both (the OpenAI body has no explicit breakpoints by design, spec/04 §2).
    /// Every other field is read by exactly one adapter.
    ///
    /// The mock protocol sends **nothing**: its client is the in-process Demo provider, which
    /// ignores the request it is handed, so every option is inert there. Accepting them would be
    /// the stored-but-inert defect this table exists to prevent, merely on a dev-only provider.
    pub fn reachable_by(self, protocol: Protocol) -> bool {
        match protocol {
            Protocol::Openai => !matches!(
                self,
                ModelOptionKey::Effort
                    | ModelOptionKey::ThinkingDisplay
                    | ModelOptionKey::RefusalFallback
                    | ModelOptionKey::EagerInputStreaming
            ),
            Protocol::Anthropic => !matches!(
                self,
                ModelOptionKey::ReasoningEffort
                    | ModelOptionKey::MaxTokensField
                    | ModelOptionKey::SendParallelToolCalls
                    | ModelOptionKey::StreamUsage
                    | ModelOptionKey::Temperature
                    | ModelOptionKey::ExtraBody
            ),
            Protocol::Mock => false,
        }
    }

    /// The options a protocol can send, in `ModelOptions` order.
    pub fn reachable(protocol: Protocol) -> Vec<ModelOptionKey> {
        ModelOptionKey::ALL
            .into_iter()
            .filter(|key| key.reachable_by(protocol))
            .collect()
    }

    /// The protocols that can send this option. The mock protocol appears only when it can send
    /// the option, which it never does (see [`Self::reachable_by`]), so callers see the two real
    /// adapters.
    pub fn protocols(self) -> Vec<Protocol> {
        [Protocol::Openai, Protocol::Anthropic, Protocol::Mock]
            .into_iter()
            .filter(|protocol| self.reachable_by(*protocol))
            .collect()
    }

    /// The closed set of values this option accepts, or an empty slice when it is free-form
    /// (spec/04 §5). Mirrors the serde representation of the corresponding enum, which is what
    /// the adapters send.
    pub fn values(self) -> &'static [&'static str] {
        match self {
            ModelOptionKey::ReasoningReplay => &["never", "always"],
            ModelOptionKey::MaxTokensField => &["max_tokens", "max_completion_tokens"],
            ModelOptionKey::Effort => EffortRule::ANTHROPIC_LEVELS,
            ModelOptionKey::ThinkingDisplay => &["summarized", "omitted"],
            ModelOptionKey::CacheTtl => &["auto", "5m", "1h"],
            // Free-form or non-scalar: the UI edits these with the matching widget.
            ModelOptionKey::ReasoningEffort
            | ModelOptionKey::SendParallelToolCalls
            | ModelOptionKey::StreamUsage
            | ModelOptionKey::Temperature
            | ModelOptionKey::ExtraBody
            | ModelOptionKey::RefusalFallback
            | ModelOptionKey::EagerInputStreaming => &[],
        }
    }

    /// The value `ModelOptions::default` uses, as the JSON string it serializes to (spec/04 §5).
    ///
    /// A field equal to its default is not "set": `set_keys` does not report it, so neither may a
    /// client that mirrors this table. Only the three enum-backed options have such a default; the
    /// rest are absent unless set.
    pub fn default_value(self) -> Option<&'static str> {
        match self {
            ModelOptionKey::ReasoningReplay => Some("never"),
            ModelOptionKey::MaxTokensField => Some("max_tokens"),
            ModelOptionKey::CacheTtl => Some("auto"),
            _ => None,
        }
    }

    /// How the settings page edits this option (spec/11 §9).
    pub fn kind(self) -> ModelOptionKind {
        match self {
            ModelOptionKey::ReasoningReplay
            | ModelOptionKey::MaxTokensField
            | ModelOptionKey::Effort
            | ModelOptionKey::ThinkingDisplay
            | ModelOptionKey::CacheTtl => ModelOptionKind::Choice,
            ModelOptionKey::SendParallelToolCalls
            | ModelOptionKey::StreamUsage
            | ModelOptionKey::RefusalFallback
            | ModelOptionKey::EagerInputStreaming => ModelOptionKind::Boolean,
            ModelOptionKey::Temperature => ModelOptionKind::Number,
            // A free-form string such as OpenAI's `reasoning_effort`: gateways differ, so the UI
            // must not restrict it to the values OpenAI documents.
            ModelOptionKey::ReasoningEffort => ModelOptionKind::Text,
            ModelOptionKey::ExtraBody => ModelOptionKind::Json,
        }
    }
}

/// How a `ModelOptions` entry is edited, and therefore which widget the settings page renders
/// (spec/04 §5, spec/11 §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelOptionKind {
    /// One of [`ModelOptionKey::values`].
    Choice,
    Boolean,
    Number,
    /// A free-form string.
    Text,
    /// A JSON object, edited as text and validated before saving.
    Json,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningReplay {
    #[default]
    Never,
    Always,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaxTokensField {
    #[default]
    MaxTokens,
    MaxCompletionTokens,
}

impl MaxTokensField {
    pub fn field_name(self) -> &'static str {
        match self {
            MaxTokensField::MaxTokens => "max_tokens",
            MaxTokensField::MaxCompletionTokens => "max_completion_tokens",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
            Effort::Max => "max",
        }
    }
}

/// How a protocol accepts the thinking effort (spec/04 §5.1, D-162).
///
/// The protocol owns both halves of the rule: the body field the level lands in, and the levels
/// it accepts. Validating a level through this type keeps save-time validation, the stored
/// mismatch report, session creation and the test connection on one source of truth, instead of
/// each repeating the protocol's rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortRule {
    /// The protocol has no effort parameter; nothing is sent (mock).
    None,
    /// Any level matching [`EffortRule::is_valid_level`] (OpenAI-compatible gateways differ).
    Free,
    /// A closed set of levels in canonical order, lowest first (Anthropic).
    Closed(&'static [&'static str]),
}

impl EffortRule {
    /// The Anthropic levels, lowest first (spec/04 §5.1).
    pub const ANTHROPIC_LEVELS: &'static [&'static str] =
        &["low", "medium", "high", "xhigh", "max"];

    /// Whether `level` matches the free-form token pattern `^[A-Za-z0-9_-]{1,32}$`.
    ///
    /// This is the one pattern shared by the OpenAI protocol's levels and the role-effort save
    /// validation (spec/04 §5.1, spec/03 §4). It is checked as ASCII rather than through a regex
    /// engine so the rule cannot drift between the two call sites.
    pub fn is_valid_level(level: &str) -> bool {
        let bytes = level.as_bytes();
        (1..=32).contains(&bytes.len())
            && bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-')
    }

    /// The rule for a protocol (spec/04 §5.1).
    pub fn for_protocol(protocol: Protocol) -> Self {
        match protocol {
            Protocol::Openai => EffortRule::Free,
            Protocol::Anthropic => EffortRule::Closed(Self::ANTHROPIC_LEVELS),
            Protocol::Mock => EffortRule::None,
        }
    }

    /// The provider body field the level lands in, or `None` when the protocol takes no effort.
    pub fn wire_key(self) -> Option<&'static str> {
        match self {
            EffortRule::None => None,
            EffortRule::Free => Some("reasoning_effort"),
            EffortRule::Closed(_) => Some("output_config.effort"),
        }
    }

    /// The closed levels in canonical order, or an empty slice for a free-form or absent rule.
    pub fn levels(self) -> &'static [&'static str] {
        match self {
            EffortRule::Closed(levels) => levels,
            EffortRule::Free | EffortRule::None => &[],
        }
    }

    /// Whether this protocol accepts `level`.
    pub fn accepts(self, level: &str) -> bool {
        match self {
            EffortRule::None => false,
            EffortRule::Free => EffortRule::is_valid_level(level),
            EffortRule::Closed(levels) => levels.contains(&level),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingDisplay {
    Summarized,
    Omitted,
}

impl ThinkingDisplay {
    pub fn as_str(self) -> &'static str {
        match self {
            ThinkingDisplay::Summarized => "summarized",
            ThinkingDisplay::Omitted => "omitted",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheTtlOption {
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "1h")]
    OneHour,
    #[default]
    Auto,
}

/// Hints the demo provider reads to decide what to do (spec/04 §7.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DemoHints {
    pub kind: String,
    pub phase: String,
    pub granularity: Option<String>,
    pub step_in_phase: u32,
    pub language: String,
    /// The tools the session froze, so the Demo provider can drive the test-only tools of the
    /// debug session (spec/04 §7.2, spec/10 §2.7).
    #[serde(default)]
    pub tools: Vec<String>,
    /// The name of the protagonist the writer may look up, when the book has one (spec/04 §7.2:
    /// planning starts with `get_entity` on the protagonist *if any*).
    #[serde(default)]
    pub protagonist: Option<String>,
    /// The review attempt the reviewer session is about to write: 1 for a first review, 2+ for a
    /// re-review (spec/04 §7.2: the Demo revises on the first attempt and passes later).
    #[serde(default)]
    pub review_attempt: u32,
    /// The track's canon mode (`canon` | `isolated`), so the Demo archivist does not propose
    /// setting changes for an isolated side story (spec/06 §6).
    #[serde(default)]
    pub canon_mode: String,
    /// The chapter's current paragraphs, so the Demo provider can build a `replace_text` whose
    /// `old` occurs exactly once (spec/04 §7.2, D-077).
    #[serde(default)]
    pub draft: Vec<DemoParagraph>,
    /// The knowledge entries the research task already has, so the demo researcher saves the
    /// corpus's remaining entries instead of duplicating them (spec/04 §7.2).
    #[serde(default)]
    pub saved_entries: u32,
    /// The book's character designs, so the Demo's `canon_seed` plan can create one entity per
    /// design with the rank-1 one as the protagonist (spec/04 §7.2, spec/06 §7).
    #[serde(default)]
    pub characters: Vec<DemoCharacter>,
    /// The setting-document kinds whose current revision is approved, so the Demo's conception
    /// plan revises exactly the documents the author asked to change (spec/04 §7.2).
    #[serde(default)]
    pub approved_docs: Vec<String>,
    /// The first relevant task id of a maintenance chapter, so the Demo proposes `complete` for
    /// it when `propose_goal_change` is frozen (spec/04 §7.2, plan §4.6).
    #[serde(default)]
    pub goal_task_id: Option<i32>,
}

/// One character design the Demo provider seeds canon from (spec/04 §7.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DemoCharacter {
    pub name: String,
    pub role_label: String,
    pub summary: String,
    pub profile: String,
}

/// One paragraph of the chapter the Demo provider is working on (D-077).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DemoParagraph {
    pub pid: String,
    pub text: String,
}

/// A model advertised by the provider's model-list endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteModel {
    pub id: String,
    pub display_name: Option<String>,
    pub context_window: Option<i64>,
    pub max_output_tokens: Option<i64>,
}
