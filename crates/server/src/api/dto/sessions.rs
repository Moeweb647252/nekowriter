//! Session DTOs (spec/10 §2.7, §2.8).
//!
//! A `SessionDto` carries data only: the UI composes the session title from `kind`,
//! `chapter_number` and `research_topic` through i18n (DECISIONS D-054).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::db::json::{
    AgentRole, Appendix, Granularity, MessageKind, PendingContent, PendingOutline, Phase,
    SessionCounters, SessionError, SessionKind, SessionStatus, ToolCallStatus, Usage,
};

/// One session row (spec/10 §2.8).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct SessionDto {
    pub id: i32,
    pub book_id: Option<i32>,
    pub role: AgentRole,
    pub kind: SessionKind,
    pub chapter_id: Option<i32>,
    pub track_id: Option<i32>,
    pub research_task_id: Option<i32>,
    /// The chapter's number in its track, for the UI-composed title (D-054).
    pub chapter_number: Option<i32>,
    pub research_topic: Option<String>,
    pub model_label: String,
    pub status: SessionStatus,
    pub phase: Phase,
    pub granularity: Option<Granularity>,
    pub auto_pause: bool,
    pub pause_after_outline: bool,
    pub pause_requested: bool,
    pub held_tool_call_id: Option<i32>,
    pub base_content_version: Option<i32>,
    pub error: Option<SessionError>,
    pub usage: Usage,
    pub counters: SessionCounters,
    pub parent_session_id: Option<i32>,
    /// `parent_session_id` of the child session, when this one was superseded by a rollover.
    pub child_session_id: Option<i32>,
    /// Present while the provider is being retried (spec/04 §6).
    pub retrying: Option<RetryingDto>,
    pub warnings: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// The retry state of a model request (spec/10 §2.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RetryingDto {
    /// 1-based retry number within this request (spec/04 §6).
    pub attempt: i32,
    pub wait_seconds: i32,
    pub reason: String,
}

/// One text or reasoning block of a message (spec/10 §2.8). Tool calls are `ToolCallDto`s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum MessageBlockDto {
    Text { text: String },
    Reasoning { text: String },
}

/// One persisted message (spec/10 §2.8).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct MessageDto {
    pub id: i32,
    pub seq: i32,
    /// `user` | `assistant`.
    pub role: String,
    pub kind: MessageKind,
    pub blocks: Vec<MessageBlockDto>,
    pub stop_reason: Option<String>,
    pub usage: Option<Usage>,
    pub created_at: DateTime<Utc>,
}

/// One tool call (spec/10 §2.8).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct ToolCallDto {
    pub id: i32,
    pub message_id: i32,
    pub seq: i32,
    pub call_index: i32,
    pub name: String,
    /// Parsed input when it is valid JSON (spec/10 §2.8).
    pub input: Option<Value>,
    /// The raw argument text, truncated for display.
    pub input_text: String,
    pub status: ToolCallStatus,
    pub is_error: bool,
    /// `raw_result_text` before sealing, `result_text` after (spec/03 §1.3).
    pub result_text: Option<String>,
    pub appendices: Vec<Appendix>,
    pub ui: Option<Value>,
    pub sealed: bool,
    /// The chapter text a held text-changing call has staged but not applied (spec/03 §5.2, D-139).
    ///
    /// Present only while the call is deferred: it is what approval will commit, so the agent panel
    /// can preview a held write against the confirmed draft without a second endpoint. `None` once
    /// the call is finalized, because the text is then in the chapter.
    pub pending_content: Option<PendingContent>,
    /// The staged outline of a held outline session's `submit_outline` (spec/03 §5.2, D-146).
    /// Present only while the call is deferred; approval writes exactly this proposal.
    pub pending_outline: Option<PendingOutline>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// A message with its tool calls (spec/10 §2.7 message list).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MessageWithCallsDto {
    pub message: MessageDto,
    pub tool_calls: Vec<ToolCallDto>,
}

/// One page of messages, newest last (spec/10 §2.7).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct MessagePageDto {
    pub items: Vec<MessageWithCallsDto>,
    /// Pass as `?cursor=` to load the next older page; `None` at the start of the session.
    pub next_cursor: Option<String>,
}

/// One unconsumed author message queued in `agent_inbox` (spec/10 §2.7).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InboxItemDto {
    pub id: i32,
    pub kind: String,
    pub content: String,
}

/// `GET /api/sessions/:id` (spec/10 §2.7).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct SessionSnapshotDto {
    pub session: SessionDto,
    /// The last 200 messages, oldest first.
    pub messages: Vec<MessageWithCallsDto>,
    /// The tool calls of those messages.
    pub tool_calls: Vec<ToolCallDto>,
    /// Unconsumed author messages, oldest first.
    pub inbox: Vec<InboxItemDto>,
    pub inbox_pending: i32,
    /// The frozen system prompt and tool list of the session (spec/03 §1.3).
    pub system_prompt: String,
    pub tool_names: Vec<String>,
    pub next_cursor: Option<String>,
}

/// `POST /api/sessions/:id/resume` body (spec/03 §6).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResumeInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
}

/// `POST /api/sessions/:id/message` body (spec/03 §6).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionMessageInput {
    pub text: String,
}

/// `POST /api/sessions/:id/answer` body (spec/03 §6).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionAnswerInput {
    pub tool_call_id: i32,
    pub payload: Value,
}

/// `PATCH /api/sessions/:id` body (spec/03 §6). Omitted fields are unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields = nullable)]
pub struct SessionPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granularity: Option<Granularity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_pause: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_after_outline: Option<bool>,
}

/// `POST /api/debug/sessions/echo` body (spec/10 §2.7, debug builds only).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugEchoInput {
    pub book_id: i32,
}

/// `POST /api/debug/sessions/echo` response.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugEchoDto {
    pub session_id: i32,
}
