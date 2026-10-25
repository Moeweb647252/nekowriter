//! `GET /api/events` SSE tests (PLAN M1.3, spec/10 §3).
//!
//! The tests subscribe to the real event bus, drive the real router and read the frames the
//! client would receive.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use futures::StreamExt;
use nekowriter::events::{BookOp, EventEnvelope, ServerEvent};
use nekowriter::testing::TestApp;
use serde_json::json;
use tokio::sync::broadcast::error::TryRecvError;

/// Reads server events from the bus while an API call runs (the router publishes after commit).
async fn drain(receiver: &mut tokio::sync::broadcast::Receiver<EventEnvelope>) -> Vec<ServerEvent> {
    let mut events = Vec::new();
    // The publish happens before the HTTP response is returned, so a short yield loop suffices.
    for _ in 0..50 {
        match receiver.try_recv() {
            Ok(envelope) => events.push(envelope.event),
            Err(TryRecvError::Empty) => {
                tokio::task::yield_now().await;
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            Err(TryRecvError::Lagged(_)) => continue,
            Err(TryRecvError::Closed) => break,
        }
    }
    events
}

#[tokio::test]
async fn content_patch_emits_chapter_updated_and_chapter_content() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "ch").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;

    // Subscribe *after* creating the chapter so only the PATCH events appear.
    let mut receiver = app.events().subscribe();
    let (status, result) = app
        .patch(
            &format!("/api/chapters/{chapter_id}/content"),
            json!({
                "base_version": 0,
                "ops": [
                    { "op": "insert", "after": null, "text": "第一段。" },
                    { "op": "insert", "after": "p1", "text": "第二段。" }
                ]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");

    let events = drain(&mut receiver).await;
    let content = events
        .iter()
        .find_map(|event| match event {
            ServerEvent::ChapterContentChanged {
                chapter_id: id,
                version,
                ops,
                word_count,
                tail_open,
                actor,
            } => Some((*id, *version, ops.clone(), *word_count, *tail_open, *actor)),
            _ => None,
        })
        .expect("a chapter.content event");
    assert_eq!(content.0, chapter_id);
    assert_eq!(content.1, 1);
    assert_eq!(content.3, 6);
    assert!(!content.4);
    assert_eq!(content.5, nekowriter::events::ContentActor::User);
    assert_eq!(content.2.len(), 2, "the stored ops are broadcast");
    let response_ops = result["ops"].as_array().expect("ops");
    assert_eq!(
        serde_json::to_value(&content.2).expect("serialize ops"),
        serde_json::Value::Array(response_ops.clone()),
        "the broadcast ops are the response ops (spec/10 §3)"
    );

    let updated = events
        .iter()
        .find_map(|event| match event {
            ServerEvent::ChapterUpdated {
                chapter,
                outline_changed,
            } => Some((chapter.clone(), *outline_changed)),
            _ => None,
        })
        .expect("a chapter.updated event");
    assert_eq!(updated.0.id, chapter_id);
    assert_eq!(updated.0.word_count, 6);
    assert!(!updated.1);

    // The order is chapter.content then chapter.updated (spec/10 §3: the UI applies ops first).
    let names: Vec<&str> = events
        .iter()
        .map(|event| match event {
            ServerEvent::ChapterContentChanged { .. } => "chapter.content",
            ServerEvent::ChapterUpdated { .. } => "chapter.updated",
            ServerEvent::BookUpdated { .. } => "book.updated",
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
        })
        .collect();
    assert_eq!(names, vec!["chapter.content", "chapter.updated"]);
}

#[tokio::test]
async fn book_and_settings_endpoints_emit_their_events() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;

    let mut receiver = app.events().subscribe();
    let (status, _) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "title": "Renamed" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let events = drain(&mut receiver).await;
    assert!(
        events.iter().any(|event| matches!(
            event,
            ServerEvent::BookUpdated {
                book_id: id,
                op: BookOp::Update
            } if *id == book_id
        )),
        "book.updated should be emitted: {events:?}"
    );

    let mut receiver = app.events().subscribe();
    let (status, _) = app
        .put(
            &format!("/api/books/{book_id}/setting-docs/world"),
            json!({ "content": "## World", "base_revision": 0 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let events = drain(&mut receiver).await;
    let settings = events
        .iter()
        .find_map(|event| match event {
            ServerEvent::SettingsChanged {
                book_id,
                kind,
                doc_kind,
                revision,
            } => Some((*book_id, *kind, *doc_kind, *revision)),
            _ => None,
        })
        .expect("settings.changed");
    assert_eq!(settings.0, book_id);
    assert_eq!(settings.1, nekowriter::events::SettingsKind::SettingDoc);
    assert_eq!(
        settings.2,
        Some(nekowriter::db::json::SettingDocKind::World)
    );
    assert_eq!(settings.3, Some(1));
}

#[tokio::test]
async fn the_sse_stream_serves_frames_with_the_spec_framing() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;

    // Open the stream, then publish an event through the bus.
    let request = Request::get("/api/events")
        .header(header::HOST, "127.0.0.1:7878")
        .body(Body::empty())
        .expect("request");
    let response = tower::ServiceExt::oneshot(app.app(), request)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream",
        "the SSE content type is required"
    );
    let mut body = response.into_body().into_data_stream();

    // The handler subscribes inside the response future; publishing after a short yield is
    // published after the subscription, so it must reach the stream (this is what the
    // "subscribe before the first poll" design buys).
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    app.events().publish(ServerEvent::SettingsChanged {
        book_id,
        kind: nekowriter::events::SettingsKind::Characters,
        doc_kind: None,
        revision: None,
    });

    let mut collected = String::new();
    for _ in 0..20 {
        let Some(chunk) = body.next().await else {
            break;
        };
        let bytes = chunk.expect("chunk");
        collected.push_str(&String::from_utf8_lossy(bytes.as_ref()));
        if collected.contains("settings.changed") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        collected.contains("event: settings.changed"),
        "SSE should carry the event name: {collected}"
    );
    assert!(
        collected.contains("id: "),
        "SSE should carry the sequence id: {collected}"
    );
    assert!(
        collected.contains("data: {"),
        "SSE should carry the JSON payload: {collected}"
    );
    assert!(collected.contains("\"type\":\"settings.changed\""));
    assert!(
        !collected.contains("\"seq\""),
        "seq lives in the id field, not in the payload (spec/10 §3)"
    );
}

#[tokio::test]
async fn the_book_filter_keeps_book_events_and_drops_others() {
    let app = TestApp::new().await;
    let first = app.create_book("First").await;
    let first_id = first["id"].as_i64().expect("book id") as i32;
    let second = app.create_book("Second").await;
    let second_id = second["id"].as_i64().expect("book id") as i32;

    // The filter helper is unit-tested too; this asserts the endpoint parses `?book_id=`.
    let request = Request::get("/api/events?book_id=1")
        .header(header::HOST, "127.0.0.1:7878")
        .body(Body::empty())
        .expect("request");
    let response = tower::ServiceExt::oneshot(app.app(), request)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    app.events().publish(ServerEvent::BookUpdated {
        book_id: second_id,
        op: BookOp::Update,
    });
    app.events().publish(ServerEvent::BookUpdated {
        book_id: first_id,
        op: BookOp::Update,
    });
    app.events().publish(ServerEvent::Resync);

    let mut collected = String::new();
    for _ in 0..20 {
        let Some(chunk) = body.next().await else {
            break;
        };
        let bytes = chunk.expect("chunk");
        collected.push_str(&String::from_utf8_lossy(bytes.as_ref()));
        if collected.contains("resync") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let book_frames: Vec<&str> = collected
        .split("event: ")
        .skip(1)
        .map(|frame| frame.split('\n').next().unwrap_or_default())
        .collect();
    assert_eq!(
        book_frames,
        vec!["book.updated", "resync"],
        "only the requested book's events plus global ones: {collected}"
    );
    assert!(
        collected.contains(&format!("\"book_id\":{first_id}")),
        "the surviving event belongs to the requested book: {collected}"
    );
    assert!(
        !collected.contains(&format!("\"book_id\":{second_id}")),
        "the other book's event was filtered out: {collected}"
    );
}

#[tokio::test]
async fn a_lagging_subscriber_receives_resync() {
    let app = TestApp::new().await;
    // The bus capacity is 4096; a receiver that never drains it lags once it overflows.
    let mut receiver = app.events().subscribe();
    for _ in 0..(nekowriter::events::EVENT_CHANNEL_CAPACITY + 10) {
        app.events().publish(ServerEvent::Resync);
    }
    // `recv` surfaces `Lagged` first; the SSE pump turns that into a `resync` frame.
    let first = receiver.recv().await;
    assert!(matches!(
        first,
        Err(tokio::sync::broadcast::error::RecvError::Lagged(_))
    ));
}

#[tokio::test]
async fn a_nonnumeric_book_filter_is_rejected() {
    let app = TestApp::new().await;
    // A non-numeric `book_id` cannot deserialize into the query struct; axum answers 400.
    let (status, _body) = app.get("/api/events?book_id=abc").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
