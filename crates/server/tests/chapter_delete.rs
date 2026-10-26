//! Deleting a chapter: the events it publishes and the rows that cascade (spec/08 §1.1).
//!
//! The other rules of the Delete row are covered elsewhere and were already green:
//! - the open-session refusal (`chapter.session_exists` + `details.session_id`) and the renumbering
//!   of the later chapters: `api_chapters::delete_renumbers_and_refuses_when_a_session_is_open`;
//! - the anchor refusal (`chapter.is_anchor` + `details.track_ids`) and the fact that the anchor
//!   becomes deletable once the side track is gone:
//!   `side_story_tracks::an_anchor_chapter_is_pinned_and_a_side_track_is_deletable`;
//! - the first/last appearance recomputation: `canon_services::deleting_a_chapter_recomputes_the_bounds`;
//! - the search index cascade: `m7_review_orphans` (chapter delete leaves no doc, fts or vec row).
//!
//! What none of them pinned is the pair of events the delete publishes — `book.updated` (the
//! chapter counts and word totals change) and `chapter.updated` — or that a chapter's own rows
//! cascade with it. The UI relies on both: an open client that never hears about the delete keeps
//! rendering the row, and a cascade that left a version or an appearance behind would resurface the
//! chapter through its entity.

use axum::http::StatusCode;
use nekowriter::events::{BookOp, EventEnvelope, ServerEvent};
use nekowriter::testing::{TestApp, assert_error};
use serde_json::json;
use tokio::sync::broadcast::error::TryRecvError;

/// Drains the events buffered while the call that published them ran (spec/10 §3: after commit).
async fn drain(receiver: &mut tokio::sync::broadcast::Receiver<EventEnvelope>) -> Vec<ServerEvent> {
    let mut events = Vec::new();
    tokio::task::yield_now().await;
    loop {
        match receiver.try_recv() {
            Ok(envelope) => events.push(envelope.event),
            Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
            Err(TryRecvError::Lagged(_)) => continue,
        }
    }
    events
}

/// Creates a book plus its main track id and returns `(book_id, track_id)`.
async fn book_with_track(app: &TestApp) -> (i32, i32) {
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    (book_id, track_id)
}

#[tokio::test]
async fn delete_publishes_book_and_chapter_events_and_cascades_its_rows() {
    let app = TestApp::new().await;
    let (book_id, track_id) = book_with_track(&app).await;
    let kept = app.create_chapter(track_id, "One").await;
    let doomed = app.create_chapter(track_id, "Two").await;
    let kept_id = kept["id"].as_i64().expect("kept id") as i32;
    let doomed_id = doomed["id"].as_i64().expect("doomed id") as i32;

    // Give the chapter rows to cascade: a version (submit creates a `draft` one), an appearance
    // and an official status with a synced canon would be overkill — the point is the cascade, and
    // every one of these rows references `chapters(id) ON DELETE CASCADE` (spec/02 §2).
    app.put_text(doomed_id, 0, "Lin Fan enters the tomb.").await;
    let (status, body) = app
        .post(&format!("/api/chapters/{doomed_id}/submit"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "submit: {body}");
    let (_, entity) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "Lin Fan" }),
        )
        .await;
    let entity_id = entity["id"].as_i64().expect("entity id") as i32;
    let (status, body) = app
        .put(
            &format!("/api/chapters/{doomed_id}/appearances"),
            json!([{ "entity_id": entity_id, "role": "major", "note": "" }]),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "appearances: {body}");
    let (_, versions) = app
        .get(&format!("/api/chapters/{doomed_id}/versions"))
        .await;
    assert!(
        !versions.as_array().expect("versions").is_empty(),
        "the chapter has a version to cascade: {versions}"
    );

    // Subscribe after the fixtures so only the delete's events appear.
    let mut receiver = app.events().subscribe();
    let (status, body) = app.delete(&format!("/api/chapters/{doomed_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let events = drain(&mut receiver).await;
    assert!(
        events.iter().any(|event| matches!(
            event,
            ServerEvent::BookUpdated { book_id: id, op: BookOp::Update } if *id == book_id
        )),
        "the book's chapter and word counts changed, so book.updated is published: {events:?}"
    );
    // The list is renumbered by the delete, so the event is a signal for the whole list rather than
    // a patch of one row; it still names the deleted chapter, which is what the client invalidates.
    let updated = events.iter().find_map(|event| match event {
        ServerEvent::ChapterUpdated {
            chapter,
            outline_changed,
        } if chapter.id == doomed_id => Some((chapter.number, *outline_changed)),
        _ => None,
    });
    assert_eq!(
        updated,
        Some((2, false)),
        "chapter.updated carries the deleted chapter's old number: {events:?}"
    );

    // The chapter and everything that hung off it are gone.
    let (status, _) = app.get(&format!("/api/chapters/{doomed_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // `GET /versions` lists by chapter id without loading the chapter, so an empty list is the
    // cascade: a version row that survived would resurface through the history tab of the id.
    let (status, versions) = app
        .get(&format!("/api/chapters/{doomed_id}/versions"))
        .await;
    assert_eq!(status, StatusCode::OK, "{versions}");
    assert!(
        versions.as_array().expect("versions").is_empty(),
        "versions cascade with the chapter: {versions}"
    );
    let (_, entity) = app.get(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(
        entity["first_chapter_id"],
        json!(null),
        "the entity that only appeared in the deleted chapter has no bound left: {entity}"
    );
    assert_eq!(entity["last_chapter_id"], json!(null));

    // The rest of the track is untouched and still contiguous.
    let (_, list) = app.get(&format!("/api/tracks/{track_id}/chapters")).await;
    let remaining: Vec<(i64, i64)> = list
        .as_array()
        .expect("list")
        .iter()
        .map(|chapter| {
            (
                chapter["id"].as_i64().expect("id"),
                chapter["number"].as_i64().expect("number"),
            )
        })
        .collect();
    assert_eq!(remaining, vec![(i64::from(kept_id), 1)]);
}

/// A chapter that exists on a `conceiving` book is still deletable (spec/08 §1.1).
///
/// Such a row is exactly what the reported bug left behind: `chapters::create_inner` did not check
/// `BookStatus`, so a book whose conception was never finalized could gain a chapter. The create
/// side is now gated, but deletion must **not** be: gating it on `active` would strand the author
/// with a chapter they cannot remove, which is the state the bug created in their database
/// (a real `conceiving` book with one `planned` chapter).
#[tokio::test]
async fn a_chapter_on_a_conceiving_book_can_still_be_deleted() {
    let app = TestApp::new().await;
    let (book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "Stray").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;

    // The exact shape found in the author's data: the book is still `conceiving` and owns a chapter.
    app.execute(&format!(
        "UPDATE books SET status = 'conceiving' WHERE id = {book_id}"
    ))
    .await;
    let (_, book) = app.get(&format!("/api/books/{book_id}")).await;
    assert_eq!(book["status"], "conceiving");

    // Creating another one is refused now (spec/08 §1.1, D-129)...
    let (status, body) = app
        .post(
            &format!("/api/tracks/{track_id}/chapters"),
            json!({ "title": "Another", "guidance": "" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "book.invalid_state");

    // ...but the row the bug created is still deletable, so the author can clean it up.
    let (status, body) = app.delete(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "deletion is not gated on the book status: {body}"
    );
    let (_, list) = app.get(&format!("/api/tracks/{track_id}/chapters")).await;
    assert!(
        list.as_array().expect("list").is_empty(),
        "the stray chapter is gone: {list}"
    );
}
