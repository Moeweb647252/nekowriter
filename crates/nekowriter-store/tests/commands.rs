//! The idempotent command envelope, exercised through project commands.

mod common;

use nekowriter_core::{ids::CommandId, project::ProjectName};
use nekowriter_store::{ProjectListFilter, StoreError};

#[tokio::test]
async fn retrying_a_command_returns_the_recorded_result_without_running_it_again() {
    let (_dir, store) = common::open_store().await;
    let command = CommandId::generate();

    let first = store
        .create_project(command, common::new_project("Rain"))
        .await
        .unwrap();
    let retry = store
        .create_project(command, common::new_project("Rain"))
        .await
        .unwrap();

    assert_eq!(first, retry);
    let projects = store
        .list_projects(ProjectListFilter::Active)
        .await
        .unwrap();
    assert_eq!(projects.len(), 1);
}

#[tokio::test]
async fn reusing_a_command_id_for_different_input_is_a_conflict() {
    let (_dir, store) = common::open_store().await;
    let command = CommandId::generate();
    store
        .create_project(command, common::new_project("Rain"))
        .await
        .unwrap();

    let error = store
        .create_project(command, common::new_project("Snow"))
        .await
        .unwrap_err();

    assert!(matches!(error, StoreError::IdempotencyConflict(id) if id == command));
}

#[tokio::test]
async fn a_failed_command_records_nothing_and_can_be_retried() {
    let (_dir, store) = common::open_store().await;
    let project = common::create_project(&store, "Rain").await;
    let command = CommandId::generate();
    let name = ProjectName::parse("Snow").unwrap();

    let stale = store
        .rename_project(command, project.id, project.revision + 1, name.clone())
        .await
        .unwrap_err();
    assert!(matches!(stale, StoreError::RevisionConflict { .. }));

    // The same command ID with the same input still fails the same way: nothing was recorded.
    let again = store
        .rename_project(command, project.id, project.revision + 1, name)
        .await
        .unwrap_err();
    assert!(matches!(again, StoreError::RevisionConflict { .. }));
}

#[tokio::test]
async fn concurrent_attempts_of_one_command_apply_it_once() {
    let (_dir, store) = common::open_store().await;
    let command = CommandId::generate();

    let attempts = (0..8).map(|_| {
        let store = store.clone();
        tokio::spawn(async move {
            store
                .create_project(command, common::new_project("Rain"))
                .await
                .unwrap()
        })
    });
    let results = join_all_tasks(attempts).await;

    assert!(results.windows(2).all(|pair| pair[0] == pair[1]));
    let projects = store
        .list_projects(ProjectListFilter::Active)
        .await
        .unwrap();
    assert_eq!(projects.len(), 1);
}

async fn join_all_tasks<T>(handles: impl Iterator<Item = tokio::task::JoinHandle<T>>) -> Vec<T> {
    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.expect("the task does not panic"));
    }
    results
}
