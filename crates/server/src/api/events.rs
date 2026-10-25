//! `GET /api/events` — the Server-Sent Events stream (spec/10 §3).
//!
//! Framing is `id: <seq>`, `event: <type>`, `data: <JSON ServerEvent>`, with a `: keep-alive`
//! comment every 15 seconds. Events are not replayed: a client refetches on reconnect, and a
//! subscriber that fell behind the broadcast channel receives `resync` (spec/10 §3).
//!
//! The subscription is created **before** the first poll, so no event published after the
//! response is built can be lost.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{mpsc, oneshot};

use crate::events::{EventEnvelope, ServerEvent};
use crate::state::AppState;

/// keep-alive interval fixed by spec/10 §3.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// `?book_id=` narrows the stream to that book plus global events (spec/10 §3).
#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    pub book_id: Option<i32>,
}

/// Streams one connection's events.
pub async fn stream(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Subscribe first: events published from now on reach this connection.
    let receiver = state.events().subscribe();
    let (sender, outbox) = mpsc::channel::<Result<Event, Infallible>>(64);
    let (cancel, cancelled) = oneshot::channel::<()>();
    let handle = tokio::spawn(pump_events(receiver, query.book_id, sender, cancelled));
    let guard = CancelOnDrop {
        cancel: Some(cancel),
        handle: Some(handle),
    };

    let stream = futures::stream::unfold((outbox, guard), |(mut outbox, guard)| async move {
        outbox.recv().await.map(|item| (item, (outbox, guard)))
    });
    // `stream` owns `guard`, so dropping the response drops it and stops the pump.
    Sse::new(stream).keep_alive(KeepAlive::new().interval(KEEP_ALIVE).text("keep-alive"))
}

/// True when an event belongs to the connection's filter (spec/10 §3).
///
/// `chapter.content` and `resync` carry no book id, so they go to every connection; the client
/// ignores events for resources it does not show. Being conservative here would silently drop
/// content updates from an open chapter (DECISIONS D-066).
fn matches_filter(event: &ServerEvent, book_filter: Option<i32>) -> bool {
    let Some(wanted) = book_filter else {
        return true;
    };
    match event_book(event) {
        None => true,
        Some(book_id) => book_id == wanted,
    }
}

/// The book an event's payload names, when it names one.
fn event_book(event: &ServerEvent) -> Option<i32> {
    match event {
        ServerEvent::BookUpdated { book_id, .. } => Some(*book_id),
        ServerEvent::ChapterUpdated { chapter, .. } => Some(chapter.book_id),
        ServerEvent::SettingsChanged { book_id, .. } => Some(*book_id),
        ServerEvent::CanonChanged { book_id, .. } => Some(*book_id),
        ServerEvent::ProposalChanged { book_id, .. } => Some(*book_id),
        ServerEvent::SessionUpdated { session, .. } => session.book_id,
        // No book id in the payload (spec/10 §3): `knowledge.changed` names a scope, not a book,
        // and `index.status` is global.
        ServerEvent::ChapterContentChanged { .. }
        | ServerEvent::KnowledgeChanged { .. }
        | ServerEvent::IndexStatus { .. }
        | ServerEvent::SessionMessage { .. }
        | ServerEvent::SessionDelta { .. }
        | ServerEvent::SessionToolCall { .. }
        | ServerEvent::SessionToolPreview { .. }
        | ServerEvent::Resync => None,
    }
}

/// Reads the broadcast channel and writes SSE frames into the outbox.
async fn pump_events(
    mut receiver: tokio::sync::broadcast::Receiver<EventEnvelope>,
    book_filter: Option<i32>,
    sender: mpsc::Sender<Result<Event, Infallible>>,
    mut cancelled: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut cancelled => break,
            received = receiver.recv() => {
                match received {
                    Ok(envelope) => {
                        if !matches_filter(&envelope.event, book_filter) {
                            continue;
                        }
                        match to_frame(&envelope) {
                            Ok(frame) => {
                                if sender.send(Ok(frame)).await.is_err() {
                                    break;
                                }
                            }
                            Err(err) => {
                                tracing::warn!(error = %err, "cannot serialize a server event");
                            }
                        }
                    }
                    // A lagging subscriber missed events: tell it to refetch (spec/10 §3).
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::debug!(skipped, "SSE subscriber lagged; sending resync");
                        let resync = EventEnvelope {
                            seq: 0,
                            event: ServerEvent::Resync,
                        };
                        match to_frame(&resync) {
                            Ok(frame) => {
                                if sender.send(Ok(frame)).await.is_err() {
                                    break;
                                }
                            }
                            Err(err) => tracing::warn!(error = %err, "cannot serialize resync"),
                        }
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }
}

/// Builds one SSE frame: `id: <seq>`, `event: <type>`, `data: <JSON ServerEvent>`.
pub fn to_frame(envelope: &EventEnvelope) -> Result<Event, serde_json::Error> {
    let data = serde_json::to_string(&envelope.event)?;
    let event_type = event_type(&envelope.event).to_owned();
    let mut frame = Event::default().event(event_type).data(data);
    if envelope.seq != 0 {
        frame = frame.id(envelope.seq.to_string());
    }
    Ok(frame)
}

/// The SSE event name of an event (spec/10 §3).
pub fn event_type(event: &ServerEvent) -> &'static str {
    match event {
        ServerEvent::BookUpdated { .. } => "book.updated",
        ServerEvent::ChapterUpdated { .. } => "chapter.updated",
        ServerEvent::ChapterContentChanged { .. } => "chapter.content",
        ServerEvent::CanonChanged { .. } => "canon.changed",
        ServerEvent::ProposalChanged { .. } => "proposal.changed",
        ServerEvent::SettingsChanged { .. } => "settings.changed",
        ServerEvent::SessionUpdated { .. } => "session.updated",
        ServerEvent::SessionMessage { .. } => "session.message",
        ServerEvent::SessionDelta { .. } => "session.delta",
        ServerEvent::SessionToolCall { .. } => "session.tool_call",
        ServerEvent::SessionToolPreview { .. } => "session.tool_preview",
        ServerEvent::KnowledgeChanged { .. } => "knowledge.changed",
        ServerEvent::IndexStatus { .. } => "index.status",
        ServerEvent::Resync => "resync",
    }
}

/// Stops the pump task when the response stream is dropped.
struct CancelOnDrop {
    cancel: Option<oneshot::Sender<()>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::dto::ChapterListItemDto;
    use crate::db::json::{CanonStatus, ChapterStatus};
    use chrono::Utc;

    fn chapter_event(book_id: i32) -> ServerEvent {
        ServerEvent::ChapterUpdated {
            chapter: ChapterListItemDto {
                id: 1,
                book_id,
                track_id: 2,
                number: 1,
                title: String::new(),
                status: ChapterStatus::Planned,
                canon_status: CanonStatus::None,
                word_count: 0,
                updated_at: Utc::now(),
            },
            outline_changed: false,
        }
    }

    #[test]
    fn event_names_match_the_spec_table() {
        assert_eq!(
            event_type(&ServerEvent::BookUpdated {
                book_id: 1,
                op: crate::events::BookOp::Create
            }),
            "book.updated"
        );
        assert_eq!(event_type(&chapter_event(1)), "chapter.updated");
        assert_eq!(
            event_type(&ServerEvent::ChapterContentChanged {
                chapter_id: 1,
                version: 2,
                ops: Vec::new(),
                word_count: 10,
                tail_open: false,
                actor: crate::events::ContentActor::User,
            }),
            "chapter.content"
        );
        assert_eq!(
            event_type(&ServerEvent::SettingsChanged {
                book_id: 1,
                kind: crate::events::SettingsKind::SettingDoc,
                doc_kind: None,
                revision: Some(1),
            }),
            "settings.changed"
        );
        assert_eq!(event_type(&ServerEvent::Resync), "resync");
    }

    #[test]
    fn filters_keep_the_book_and_global_events() {
        assert!(matches_filter(&chapter_event(7), Some(7)));
        assert!(!matches_filter(&chapter_event(8), Some(7)));
        assert!(matches_filter(&chapter_event(8), None));
        assert!(matches_filter(&ServerEvent::Resync, Some(7)));
    }
}
