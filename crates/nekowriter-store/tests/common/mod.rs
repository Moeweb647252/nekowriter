//! Helpers shared by the store's integration tests.

// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use nekowriter_core::{
    ids::CommandId,
    project::{NovelTitle, ProjectName, WritingLanguage},
};
use nekowriter_store::{DATABASE_FILE, NewProject, ProjectView, Store};
use tempfile::TempDir;

/// A store in a fresh temporary directory, which is removed when the guard drops.
pub async fn open_store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().expect("a temporary directory can be created");
    let store = Store::open(&dir.path().join(DATABASE_FILE), "test")
        .await
        .expect("a new database opens");
    (dir, store)
}

pub fn new_project(name: &str) -> NewProject {
    NewProject {
        name: ProjectName::parse(name).unwrap(),
        novel_title: NovelTitle::parse("Rain Letters").unwrap(),
        writing_language: WritingLanguage::En,
    }
}

pub async fn create_project(store: &Store, name: &str) -> ProjectView {
    store
        .create_project(CommandId::generate(), new_project(name))
        .await
        .expect("the project is created")
}
