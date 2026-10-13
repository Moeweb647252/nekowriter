//! The event stream over a real port.

mod common;

use std::time::Duration;

use common::TestServer;
use serde_json::{Value, json};

/// Reads server-sent events from a streaming response.
struct SseReader {
    response: reqwest::Response,
    buffer: String,
}

/// One parsed server-sent event.
struct SseMessage {
    id: Option<String>,
    data: Value,
}

impl SseReader {
    fn new(response: reqwest::Response) -> Self {
        Self {
            response,
            buffer: String::new(),
        }
    }

    /// The next message with data, skipping heartbeat comments. `None` when the stream ends.
    async fn next(&mut self) -> Option<SseMessage> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let block: String = self.buffer.drain(..end + 2).collect();
                let mut id = None;
                let mut data = String::new();
                for line in block.lines() {
                    if let Some(value) = line.strip_prefix("id:") {
                        id = Some(value.trim().to_owned());
                    } else if let Some(value) = line.strip_prefix("data:") {
                        data.push_str(value.trim());
                    }
                }
                if !data.is_empty() {
                    return Some(SseMessage {
                        id,
                        data: serde_json::from_str(&data).unwrap(),
                    });
                }
                continue;
            }
            let chunk = tokio::time::timeout(Duration::from_secs(10), self.response.chunk())
                .await
                .expect("the stream produces data in time")
                .unwrap()?;
            self.buffer.push_str(std::str::from_utf8(&chunk).unwrap());
        }
    }
}

async fn subscribe(server: &TestServer, after: i64) -> SseReader {
    let response = server
        .get(&format!("/api/events?after={after}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    SseReader::new(response)
}

#[tokio::test]
async fn subscribers_receive_committed_events_after_their_cursor() {
    let server = TestServer::start().await;
    let cursor: Value = server
        .get("/api/events/cursor")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cursor["cursor"], 0);
    let mut events = subscribe(&server, 0).await;

    let project = server.create_project("Rain").await;

    let message = events.next().await.unwrap();
    assert_eq!(message.id.as_deref(), Some("1"));
    assert_eq!(
        message.data,
        json!({
            "type": "event",
            "seq": 1,
            "event": { "type": "project_changed", "projectId": project["id"] }
        })
    );
}

#[tokio::test]
async fn a_reconnecting_client_resumes_from_last_event_id() {
    let server = TestServer::start().await;
    server.create_project("First").await;
    server.create_project("Second").await;

    let response = server
        .get("/api/events?after=0")
        .header("last-event-id", "1")
        .send()
        .await
        .unwrap();
    let mut events = SseReader::new(response);

    let message = events.next().await.unwrap();
    assert_eq!(message.data["seq"], 2);
}

#[tokio::test]
async fn a_cursor_from_the_future_requires_a_reset() {
    let server = TestServer::start().await;
    let mut events = subscribe(&server, 99).await;

    let message = events.next().await.unwrap();

    assert_eq!(message.data, json!({ "type": "reset_required" }));
    assert!(events.next().await.is_none());
}

#[tokio::test]
async fn streams_end_with_a_notice_when_the_service_stops() {
    let server = TestServer::start().await;
    let mut events = subscribe(&server, 0).await;

    server.server.shutdown().trigger();

    let message = events.next().await.unwrap();
    assert_eq!(message.data, json!({ "type": "shutting_down" }));
    assert!(events.next().await.is_none());
}
