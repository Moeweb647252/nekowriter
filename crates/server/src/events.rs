//! The event bus and `ServerEvent` (spec/10 §3, spec/01 §7).
//!
//! Services return the events their committed transaction produced; the HTTP layer publishes
//! them through [`EventBus`] **after** the commit (spec/01 §4.2). `GET /api/events` subscribes
//! and streams them as SSE (spec/10 §3). `seq` is a process-wide monotonic counter; a lagging
//! subscriber receives `resync`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use ts_rs::TS;

use crate::api::dto::{ChapterListItemDto, ContentOp};
use crate::db::json::SettingDocKind;

/// Broadcast capacity for subscribers (spec/01 §7).
pub const EVENT_CHANNEL_CAPACITY: usize = 4096;

/// What happened to a book row (spec/10 §3 `book.updated`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BookOp {
    Create,
    Update,
    Delete,
}

/// Which setting material changed (spec/10 §3 `settings.changed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SettingsKind {
    SettingDoc,
    Characters,
    ResearchTopics,
}

/// Who changed chapter content (spec/10 §3 `chapter.content`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ContentActor {
    Agent,
    User,
}

/// Which live buffer a delta belongs to (spec/10 §3 `session.delta`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DeltaKind {
    Text,
    Reasoning,
}

/// A server event (spec/10 §3). Variants arrive with the milestone that produces them; M1 emits
/// `book.updated`, `chapter.updated`, `chapter.content` and `settings.changed`, M3 adds the
/// session events, and any lagging subscriber receives `resync`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ServerEvent {
    /// A session row changed (status, phase, usage, counters, error).
    #[serde(rename = "session.updated")]
    SessionUpdated {
        session: crate::api::dto::SessionDto,
        /// Codes the UI localizes (spec/10 §1), e.g. `context.large_kickoff`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        warnings: Vec<String>,
    },
    /// A message was persisted. The client appends it to the activity stream and drops the
    /// matching live buffers (spec/10 §3).
    #[serde(rename = "session.message")]
    SessionMessage {
        session_id: i32,
        message: crate::api::dto::MessageDto,
    },
    /// A streamed text or reasoning delta (spec/10 §3).
    #[serde(rename = "session.delta")]
    SessionDelta {
        session_id: i32,
        seq: i32,
        kind: DeltaKind,
        index: u32,
        text: String,
    },
    /// A tool call row changed: created, executing, held, done or error (spec/10 §3).
    #[serde(rename = "session.tool_call")]
    SessionToolCall {
        session_id: i32,
        tool_call: crate::api::dto::ToolCallDto,
    },
    /// A preview of a long tool argument while it is generated (spec/03 §12).
    #[serde(rename = "session.tool_preview")]
    SessionToolPreview {
        session_id: i32,
        /// The `seq` of the assistant message being generated.
        seq: i32,
        call_index: u32,
        name: String,
        field: String,
        delta: String,
    },
    /// A book row or its tracks changed: create, edit, finalize, archive, delete.
    #[serde(rename = "book.updated")]
    BookUpdated { book_id: i32, op: BookOp },
    /// A chapter's metadata changed (title, status, outline, word count).
    #[serde(rename = "chapter.updated")]
    ChapterUpdated {
        chapter: ChapterListItemDto,
        outline_changed: bool,
    },
    /// Chapter text changed by the author or an agent.
    #[serde(rename = "chapter.content")]
    ChapterContentChanged {
        chapter_id: i32,
        version: i32,
        ops: Vec<ContentOp>,
        word_count: i32,
        tail_open: bool,
        actor: ContentActor,
    },
    /// Setting materials changed (the book's `settings` column emits `book.updated`).
    #[serde(rename = "settings.changed")]
    SettingsChanged {
        book_id: i32,
        kind: SettingsKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        doc_kind: Option<SettingDocKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<i32>,
    },
    /// A lagging subscriber missed events; refetch everything visible.
    #[serde(rename = "resync")]
    Resync,
}

/// One event with its process-wide sequence number.
///
/// Only [`ServerEvent`] is serialized into the SSE `data:` field; `seq` becomes the `id:` field
/// (spec/10 §3: `id: <seq>`, `data: <JSON ServerEvent>`).
#[derive(Debug, Clone)]
pub struct EventEnvelope {
    pub seq: u64,
    pub event: ServerEvent,
}

/// The process-wide broadcast bus (spec/01 §7: capacity 4096).
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<EventEnvelope>,
    seq: Arc<AtomicU64>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _receiver) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            sender,
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Assigns the next sequence number and broadcasts one event.
    pub fn publish(&self, event: ServerEvent) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        tracing::debug!(seq, event = ?event, "publishing server event");
        // No subscriber is not an error.
        let _ = self.sender.send(EventEnvelope { seq, event });
    }

    /// Publishes the events of one committed transaction, in order.
    pub fn publish_all(&self, events: Vec<ServerEvent>) {
        for event in events {
            self.publish(event);
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.sender.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_numbers_increase_and_events_reach_subscribers() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        bus.publish(ServerEvent::BookUpdated {
            book_id: 1,
            op: BookOp::Create,
        });
        bus.publish(ServerEvent::Resync);
        let first = receiver.try_recv().expect("first event");
        let second = receiver.try_recv().expect("second event");
        assert_eq!(first.seq, 1);
        assert_eq!(second.seq, 2);
        assert!(matches!(second.event, ServerEvent::Resync));
    }

    #[test]
    fn publishing_without_subscribers_is_not_an_error() {
        let bus = EventBus::new();
        bus.publish(ServerEvent::Resync);
        let mut receiver = bus.subscribe();
        bus.publish(ServerEvent::Resync);
        // The subscriber only sees events published after it subscribed.
        assert_eq!(receiver.try_recv().expect("event").seq, 2);
    }
}
