//! Session creation's domain side effects are broadcast (PLAN 11 hardening, task-21).
//!
//! Creating a session changes domain rows in the same transaction: a writer session moves its
//! chapter to `drafting`/`revising`, a redraft clears the text, a maintenance/re-sync session sets
//! `canon_status = syncing`, and a research session makes its task `running`. Those changes used to
//! be invisible to a client that relies on SSE: it saw `session.updated` but kept a stale chapter
//! or task until its next refetch. They are now published with the creation events, after the
//! commit (D-115).

use axum::http::StatusCode;
use nekowriter::api::dto::ContentOp;
use nekowriter::db::json::{Granularity, SessionKind};
use nekowriter::events::{EventEnvelope, ServerEvent};
use nekowriter::services::sessions::{self, CreateSession};
use serde_json::json;
use tokio::sync::broadcast;

#[path = "agent_write_harness/mod.rs"]
mod agent_write_harness;

use agent_write_harness::runtime;

/// Drains everything the event bus buffered during the call that published it.
fn drain(receiver: &mut broadcast::Receiver<EventEnvelope>) -> Vec<ServerEvent> {
    let mut events = Vec::new();
    while let Ok(envelope) = receiver.try_recv() {
        events.push(envelope.event);
    }
    events
}

/// The `chapter.updated` payload of a chapter, when the events carry one.
fn chapter_updated(
    events: &[ServerEvent],
    chapter_id: i32,
) -> Option<&nekowriter::api::dto::ChapterListItemDto> {
    events.iter().find_map(|event| match event {
        ServerEvent::ChapterUpdated { chapter, .. } if chapter.id == chapter_id => Some(chapter),
        _ => None,
    })
}

/// The `chapter.content` payload of a chapter, when the events carry one.
fn chapter_content(
    events: &[ServerEvent],
    chapter_id: i32,
) -> Option<(i32, &Vec<ContentOp>, i32, bool)> {
    events.iter().find_map(|event| match event {
        ServerEvent::ChapterContentChanged {
            chapter_id: id,
            version,
            ops,
            word_count,
            tail_open,
            ..
        } if *id == chapter_id => Some((*version, ops, *word_count, *tail_open)),
        _ => None,
    })
}

/// The create request of a chapter session.
fn chapter_session(
    kind: SessionKind,
    book_id: i32,
    chapter_id: i32,
    redraft: bool,
) -> CreateSession {
    CreateSession {
        kind,
        book_id: Some(book_id),
        chapter_id: Some(chapter_id),
        research_task_id: None,
        granularity: Some(Granularity::Paragraph),
        auto_pause: None,
        pause_after_outline: None,
        redraft,
        guidance: None,
        kickoff_extra: None,
        pending_comment: None,
        chapter_guard: false,
        force_previous: false,
    }
}

#[tokio::test]
async fn a_writer_session_announces_the_chapter_it_moved_to_drafting() {
    let runtime = runtime(Vec::new()).await;
    let mut events = runtime.app.state().events().subscribe();
    sessions::create(
        runtime.app.state(),
        chapter_session(
            SessionKind::ChapterWrite,
            runtime.book_id,
            runtime.chapter_id,
            false,
        ),
    )
    .await
    .expect("create the writer session");
    let events = drain(&mut events);
    let chapter = chapter_updated(&events, runtime.chapter_id)
        .expect("the creation publishes chapter.updated");
    assert_eq!(chapter.status.as_str(), "drafting");
}

#[tokio::test]
async fn a_revise_session_announces_the_chapter_it_moved_to_revising() {
    let runtime = runtime(Vec::new()).await;
    runtime
        .app
        .put_text(runtime.chapter_id, 0, "第一段。")
        .await;
    let mut events = runtime.app.state().events().subscribe();
    sessions::create(
        runtime.app.state(),
        chapter_session(
            SessionKind::ChapterRevise,
            runtime.book_id,
            runtime.chapter_id,
            false,
        ),
    )
    .await
    .expect("create the revise session");
    let events = drain(&mut events);
    let chapter = chapter_updated(&events, runtime.chapter_id)
        .expect("the creation publishes chapter.updated");
    assert_eq!(chapter.status.as_str(), "revising");
}

#[tokio::test]
async fn a_redraft_announces_the_cleared_text_with_ops() {
    let runtime = runtime(Vec::new()).await;
    let written = runtime
        .app
        .put_text(runtime.chapter_id, 0, "第一段。\n\n第二段。")
        .await;
    let version = written["version"].as_i64().expect("version") as i32;
    let old_pids: Vec<String> = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await
        .1["content"]["paragraphs"]
        .as_array()
        .expect("paragraphs")
        .iter()
        .map(|paragraph| paragraph["pid"].as_str().expect("pid").to_owned())
        .collect();
    assert_eq!(old_pids.len(), 2);

    let mut events = runtime.app.state().events().subscribe();
    sessions::create(
        runtime.app.state(),
        chapter_session(
            SessionKind::ChapterWrite,
            runtime.book_id,
            runtime.chapter_id,
            true,
        ),
    )
    .await
    .expect("create the redraft session");
    let events = drain(&mut events);

    let chapter = chapter_updated(&events, runtime.chapter_id)
        .expect("the creation publishes chapter.updated");
    assert_eq!(chapter.status.as_str(), "drafting");
    assert_eq!(chapter.word_count, 0, "the cleared chapter has no words");

    let (event_version, ops, word_count, tail_open) = chapter_content(&events, runtime.chapter_id)
        .expect("the redraft publishes chapter.content");
    assert_eq!(event_version, version + 1);
    assert_eq!(word_count, 0);
    assert!(!tail_open);
    let expected: Vec<ContentOp> = old_pids
        .iter()
        .map(|pid| ContentOp::Delete { pid: pid.clone() })
        .chain(std::iter::once(ContentOp::Tail { open: false }))
        .collect();
    assert_eq!(
        ops, &expected,
        "every old paragraph is deleted, then the tail closes"
    );
}

#[tokio::test]
async fn maintenance_and_resync_announce_the_chapters_canon_status() {
    for kind in [SessionKind::ChapterMaintenance, SessionKind::ChapterResync] {
        let runtime = runtime(Vec::new()).await;
        runtime
            .app
            .execute(&format!(
                "UPDATE chapters SET status = 'official', canon_status = 'synced' WHERE id = {}",
                runtime.chapter_id
            ))
            .await;
        let mut events = runtime.app.state().events().subscribe();
        sessions::create(
            runtime.app.state(),
            chapter_session(kind, runtime.book_id, runtime.chapter_id, false),
        )
        .await
        .expect("create the maintenance session");
        let events = drain(&mut events);
        let chapter = chapter_updated(&events, runtime.chapter_id)
            .unwrap_or_else(|| panic!("{kind:?} publishes chapter.updated"));
        assert_eq!(
            chapter.canon_status.as_str(),
            "syncing",
            "{kind:?} moves the chapter's canon to syncing"
        );
    }
}

#[tokio::test]
async fn a_research_run_announces_its_task_as_running() {
    let runtime = runtime(Vec::new()).await;
    let (status, task) = runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": runtime.book_id,
                "scope_name": "背景设定",
                "topic": "边城城防",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create task: {task}");
    let task_id = task["id"].as_i64().expect("task id") as i32;

    let mut events = runtime.app.state().events().subscribe();
    nekowriter::services::research::run(runtime.app.state(), task_id)
        .await
        .expect("start the researcher");
    let events = drain(&mut events);
    let updated = events
        .iter()
        .find_map(|event| match event {
            ServerEvent::ResearchUpdated { task } if task.id == task_id => Some(task),
            _ => None,
        })
        .expect("the run publishes research.updated");
    assert_eq!(updated.status.as_str(), "running");
    assert!(updated.session_id.is_some(), "the task names its session");
}

#[tokio::test]
async fn no_duplicate_chapter_event_for_a_session_without_a_chapter() {
    // A book-wide session (conception) has no chapter side effect, so it publishes no chapter
    // event; the test also pins that the creation events stay a closed set for this path.
    let runtime = runtime(Vec::new()).await;
    runtime
        .app
        .execute(&format!(
            "UPDATE books SET status = 'conceiving' WHERE id = {}",
            runtime.book_id
        ))
        .await;
    let mut events = runtime.app.state().events().subscribe();
    sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::Conception,
            book_id: Some(runtime.book_id),
            chapter_id: None,
            research_task_id: None,
            granularity: None,
            auto_pause: None,
            pause_after_outline: None,
            redraft: false,
            guidance: None,
            kickoff_extra: None,
            pending_comment: None,
            chapter_guard: false,
            force_previous: false,
        },
    )
    .await
    .expect("start the conception session");
    let events = drain(&mut events);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ServerEvent::ChapterUpdated { .. })),
        "a chapter-less session publishes no chapter event"
    );
    // The session itself is still announced.
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ServerEvent::SessionUpdated { .. }))
    );
}
