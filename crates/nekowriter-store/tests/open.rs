//! Opening databases: creation, reopening, and refusing databases this build cannot read.

mod common;

use nekowriter_store::{DATABASE_FILE, OpenError, ProjectListFilter, Store};
use sea_orm::{ConnectionTrait, Database};

#[tokio::test]
async fn data_survives_reopening() {
    let (dir, store) = common::open_store().await;
    let project = common::create_project(&store, "Rain").await;
    store.close().await.unwrap();

    let reopened = Store::open(&dir.path().join(DATABASE_FILE), "test")
        .await
        .unwrap();

    assert_eq!(
        reopened
            .list_projects(ProjectListFilter::Active)
            .await
            .unwrap(),
        vec![project]
    );
}

#[tokio::test]
async fn a_database_with_a_different_schema_is_refused_untouched() {
    let (dir, store) = common::open_store().await;
    common::create_project(&store, "Rain").await;
    store.close().await.unwrap();
    let path = dir.path().join(DATABASE_FILE);
    let db = Database::connect(format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    db.execute_unprepared("UPDATE schema_info SET fingerprint = 'other'")
        .await
        .unwrap();
    db.close().await.unwrap();

    let error = Store::open(&path, "test").await.unwrap_err();

    assert!(
        matches!(error, OpenError::SchemaMismatch { created_by_version } if created_by_version == "test")
    );
    let db = Database::connect(format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    let projects = db
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DbBackend::Sqlite,
            "SELECT COUNT(*) FROM projects",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(projects.try_get_by_index::<i64>(0).unwrap(), 1);
}

#[tokio::test]
async fn a_database_created_by_another_program_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(DATABASE_FILE);
    let db = Database::connect(format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .unwrap();
    db.execute_unprepared("CREATE TABLE notes (id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    db.close().await.unwrap();

    let error = Store::open(&path, "test").await.unwrap_err();

    assert!(matches!(error, OpenError::Foreign));
}
