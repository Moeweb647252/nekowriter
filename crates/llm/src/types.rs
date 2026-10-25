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
    // OpenAI-compatible
    pub reasoning_replay: ReasoningReplay,
    pub reasoning_effort: Option<String>,
    pub max_tokens_field: MaxTokensField,
    pub send_parallel_tool_calls: Option<bool>,
    pub stream_usage: Option<bool>,
    pub temperature: Option<f64>,
    pub extra_body: Option<BTreeMap<String, Value>>,
    // Anthropic
    pub effort: Option<Effort>,
    pub thinking_display: Option<ThinkingDisplay>,
    pub cache_ttl: CacheTtlOption,
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
