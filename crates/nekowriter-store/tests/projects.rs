mod common;

use nekowriter_core::{
    conflict::{Resource, StateConflict},
    ids::CommandId,
    project::ProjectName,
};
use nekowriter_store::{ProjectListFilter, StoreError};

#[tokio::test]
async fn a_new_project_has_a_selected_first_version() {
    let (_dir, store) = common::open_store().await;

    let project = common::create_project(&store, "Rain").await;

    assert_eq!(project.name.as_str(), "Rain");
    assert_eq!(project.selected_version.number, 1);
    assert_eq!(
        project.selected_version.novel_title.as_str(),
        "Rain Letters"
    );
    assert_eq!(project.revision, 1);
    assert_eq!(project.deleted_at, None);
}

#[tokio::test]
async fn renaming_advances_the_revision_and_rejects_a_stale_one() {
    let (_dir, store) = common::open_store().await;
    let project = common::create_project(&store, "Rain").await;

    let renamed = store
        .rename_project(
            CommandId::generate(),
            project.id,
            project.revision,
            ProjectName::parse("Snow").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(renamed.name.as_str(), "Snow");
    assert_eq!(renamed.revision, project.revision + 1);

    let error = store
        .rename_project(
            CommandId::generate(),
            project.id,
            project.revision,
            ProjectName::parse("Hail").unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::RevisionConflict {
            resource: Resource::Project,
            expected: 1,
            actual: 2
        }
    ));
}

#[tokio::test]
async fn deleting_moves_a_project_to_the_deleted_list_and_restoring_brings_it_back() {
    let (_dir, store) = common::open_store().await;
    let project = common::create_project(&store, "Rain").await;

    let deleted = store
        .delete_project(CommandId::generate(), project.id, project.revision)
        .await
        .unwrap();
    assert!(deleted.deleted_at.is_some());
    assert_eq!(
        store
            .list_projects(ProjectListFilter::Active)
            .await
            .unwrap(),
        Vec::new()
    );
    assert_eq!(
        store
            .list_projects(ProjectListFilter::Deleted)
            .await
            .unwrap(),
        vec![deleted.clone()]
    );

    let restored = store
        .restore_project(CommandId::generate(), project.id, deleted.revision)
        .await
        .unwrap();
    assert_eq!(restored.deleted_at, None);
    assert_eq!(
        store
            .list_projects(ProjectListFilter::Active)
            .await
            .unwrap(),
        vec![restored]
    );
}

#[tokio::test]
async fn a_deleted_project_cannot_be_renamed_and_an_active_one_cannot_be_restored() {
    let (_dir, store) = common::open_store().await;
    let project = common::create_project(&store, "Rain").await;

    let restore = store
        .restore_project(CommandId::generate(), project.id, project.revision)
        .await
        .unwrap_err();
    assert!(matches!(
        restore,
        StoreError::StateConflict(StateConflict::ProjectNotDeleted)
    ));

    let deleted = store
        .delete_project(CommandId::generate(), project.id, project.revision)
        .await
        .unwrap();
    let rename = store
        .rename_project(
            CommandId::generate(),
            project.id,
            deleted.revision,
            ProjectName::parse("Snow").unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(rename, StoreError::NotFound(Resource::Project)));
}

#[tokio::test]
async fn active_projects_are_listed_most_recently_updated_first() {
    let (_dir, store) = common::open_store().await;
    let first = common::create_project(&store, "First").await;
    let second = common::create_project(&store, "Second").await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    store
        .rename_project(
            CommandId::generate(),
            first.id,
            first.revision,
            ProjectName::parse("First again").unwrap(),
        )
        .await
        .unwrap();

    let ids: Vec<_> = store
        .list_projects(ProjectListFilter::Active)
        .await
        .unwrap()
        .into_iter()
        .map(|project| project.id)
        .collect();

    assert_eq!(ids, vec![first.id, second.id]);
}
