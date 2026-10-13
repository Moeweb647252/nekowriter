//! The event stream (server-sent events).
//!
//! A client reads the current cursor, loads its data, then subscribes with `after` set to
//! that cursor, so no change between loading and subscribing is missed. Every message is
//! one JSON [`ServerMessage`]; persisted events also carry their sequence number as the
//! SSE `id`, so a reconnecting browser resumes through `Last-Event-ID`. Previews carry no
//! `id`: they are not part of the log.

use std::{collections::VecDeque, convert::Infallible, time::Duration};

use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
};
use futures_util::{Stream, stream};
use nekowriter_core::event::Event;
use nekowriter_store::{EventBatch, EventRecord, Store};
use serde::{Deserialize, Serialize};
use tokio::{sync::watch, time::Instant};
use ts_rs::TS;

use crate::{
    http::{
        AppState,
        error::{ApiError, ApiProblem},
        extract::Query,
    },
    lifecycle::Shutdown,
    preview::Preview,
};

/// How many events are read from the database at a time.
const BATCH_SIZE: u64 = 256;

/// How often an idle stream sends a comment so proxies and browsers keep it open.
const HEARTBEAT: Duration = Duration::from_secs(15);

/// The shortest time between two previews on one stream.
const PREVIEW_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventCursorView {
    /// The sequence number of the newest event; subscribe after it.
    pub cursor: i64,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventStreamQuery {
    /// Stream the events after this cursor. A non-empty `Last-Event-ID` header takes
    /// precedence, because browsers send it when they reconnect on their own.
    #[ts(optional)]
    pub after: Option<i64>,
}

/// One message of the event stream.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ServerMessage {
    /// A committed change.
    Event { seq: i64, event: Event },
    /// The latest preview of the request in flight; `null` once it ended.
    Preview { preview: Option<Preview> },
    /// The cursor is outside the retained event log. Reload all data, read a new cursor
    /// and subscribe again. The stream ends after this message.
    ResetRequired,
    /// The service is stopping. The stream ends after this message.
    ShuttingDown,
}

pub(crate) async fn cursor(
    State(state): State<AppState>,
) -> Result<Json<EventCursorView>, ApiError> {
    Ok(Json(EventCursorView {
        cursor: state.store.event_cursor().await?,
    }))
}

pub(crate) async fn stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<EventStreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    let cursor = start_cursor(&headers, query.after)?;
    let messages = Messages {
        notifications: state.store.subscribe_events(),
        previews: Some(state.previews.subscribe()),
        next_preview: Instant::now(),
        store: state.store,
        shutdown: state.shutdown,
        cursor,
        pending: VecDeque::new(),
        finished: false,
    };
    let stream = stream::unfold(messages, |mut messages| async move {
        let message = messages.next().await?;
        Some((Ok(message), messages))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(HEARTBEAT)))
}

fn start_cursor(headers: &HeaderMap, after: Option<i64>) -> Result<i64, ApiError> {
    let malformed = |cause: &str| ApiError::new(ApiProblem::MalformedRequest, cause.to_owned());
    let last_event_id = headers
        .get("last-event-id")
        .map(|value| value.to_str().map(str::trim))
        .transpose()
        .map_err(|_| malformed("Last-Event-ID is not text"))?
        .filter(|value| !value.is_empty());
    let cursor = match last_event_id {
        Some(value) => value
            .parse()
            .map_err(|_| malformed("Last-Event-ID is not a sequence number"))?,
        None => after.ok_or_else(|| malformed("either Last-Event-ID or `after` is required"))?,
    };
    if cursor < 0 {
        return Err(malformed("the cursor is negative"));
    }
    Ok(cursor)
}

/// The state of one subscriber's stream.
struct Messages {
    store: Store,
    notifications: watch::Receiver<i64>,
    /// `None` once the preview channel closed.
    previews: Option<watch::Receiver<Option<Preview>>>,
    /// The earliest time the next preview may be sent.
    next_preview: Instant,
    shutdown: Shutdown,
    cursor: i64,
    pending: VecDeque<EventRecord>,
    finished: bool,
}

impl Messages {
    async fn next(&mut self) -> Option<SseEvent> {
        loop {
            if self.finished {
                return None;
            }
            if let Some(record) = self.pending.pop_front() {
                self.cursor = record.seq;
                return Some(
                    sse(&ServerMessage::Event {
                        seq: record.seq,
                        event: record.event,
                    })
                    .id(record.seq.to_string()),
                );
            }
            // Mark the current notification as seen before reading, so a commit that lands
            // after the read still wakes the wait below.
            self.notifications.borrow_and_update();
            match self.store.events_after(self.cursor, BATCH_SIZE).await {
                Ok(EventBatch::ResetRequired) => {
                    return Some(self.last(&ServerMessage::ResetRequired));
                }
                Ok(EventBatch::Events(records)) if !records.is_empty() => {
                    self.pending.extend(records);
                    continue;
                }
                Ok(EventBatch::Events(_)) => {}
                Err(error) => {
                    tracing::error!(%error, "the event stream could not read events");
                    return None;
                }
            }
            tokio::select! {
                changed = self.notifications.changed() => {
                    if changed.is_err() {
                        return None;
                    }
                }
                changed = preview_changed(self.previews.as_mut(), self.next_preview) => {
                    if changed {
                        return Some(self.preview());
                    }
                    self.previews = None;
                }
                () = self.shutdown.triggered() => return Some(self.last(&ServerMessage::ShuttingDown)),
            }
        }
    }

    fn preview(&mut self) -> SseEvent {
        let preview = self
            .previews
            .as_mut()
            .and_then(|previews| previews.borrow_and_update().clone());
        self.next_preview = Instant::now() + PREVIEW_INTERVAL;
        sse(&ServerMessage::Preview { preview })
    }

    fn last(&mut self, message: &ServerMessage) -> SseEvent {
        self.finished = true;
        sse(message)
    }
}

/// Completes with `true` once the preview changed and `not_before` has passed, or with
/// `false` if the channel closed. Without a channel it never completes.
async fn preview_changed(
    previews: Option<&mut watch::Receiver<Option<Preview>>>,
    not_before: Instant,
) -> bool {
    let Some(previews) = previews else {
        return std::future::pending().await;
    };
    tokio::time::sleep_until(not_before).await;
    previews.changed().await.is_ok()
}

fn sse(message: &ServerMessage) -> SseEvent {
    SseEvent::default()
        .json_data(message)
        .expect("server messages always serialize")
}
