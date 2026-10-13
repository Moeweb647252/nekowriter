mod common;

use nekowriter_core::{event::Event, ids::CommandId};
use nekowriter_store::{EventBatch, EventRecord};

#[tokio::test]
async fn commands_append_events_that_follow_the_cursor() {
    let (_dir, store) = common::open_store().await;
    let cursor = store.event_cursor().await.unwrap();
    assert_eq!(cursor, 0);

    let project = common::create_project(&store, "Rain").await;
    store
        .delete_project(CommandId::generate(), project.id, project.revision)
        .await
        .unwrap();

    let EventBatch::Events(events) = store.events_after(cursor, 100).await.unwrap() else {
        panic!("a current cursor never needs a reset");
    };
    let expected = Event::ProjectChanged {
        project_id: project.id,
    };
    assert_eq!(
        events,
        vec![
            EventRecord {
                seq: 1,
                event: expected.clone()
            },
            EventRecord {
                seq: 2,
                event: expected
            },
        ]
    );
    assert_eq!(store.event_cursor().await.unwrap(), 2);
    assert_eq!(
        store.events_after(2, 100).await.unwrap(),
        EventBatch::Events(Vec::new())
    );
}

#[tokio::test]
async fn a_replayed_command_appends_no_event() {
    let (_dir, store) = common::open_store().await;
    let command = CommandId::generate();
    store
        .create_project(command, common::new_project("Rain"))
        .await
        .unwrap();
    store
        .create_project(command, common::new_project("Rain"))
        .await
        .unwrap();

    assert_eq!(store.event_cursor().await.unwrap(), 1);
}

#[tokio::test]
async fn a_cursor_beyond_the_newest_event_requires_a_reset() {
    let (_dir, store) = common::open_store().await;
    common::create_project(&store, "Rain").await;

    assert_eq!(
        store.events_after(5, 100).await.unwrap(),
        EventBatch::ResetRequired
    );
}

#[tokio::test]
async fn subscribers_are_woken_by_commits() {
    let (_dir, store) = common::open_store().await;
    let mut subscription = store.subscribe_events();

    common::create_project(&store, "Rain").await;

    subscription.changed().await.unwrap();
    assert_eq!(*subscription.borrow(), 1);
}
