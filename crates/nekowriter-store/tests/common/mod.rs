//! Helpers shared by the store's integration tests.

// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use nekowriter_core::{
    ids::CommandId,
    model::{ApiBaseUrl, ModelId, ModelProfile},
    project::{NovelTitle, ProjectName, WritingLanguage},
};
use nekowriter_store::{
    ApiKeyChange, DATABASE_FILE, NewProject, ProjectView, Store, SystemSettingsInput,
};
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

/// Saves a model service and a default model, so model work can start.
pub async fn configure_model(store: &Store) {
    let revision = store.system_settings().await.unwrap().revision;
    store
        .save_system_settings(
            CommandId::generate(),
            revision,
            SystemSettingsInput {
                api_base_url: Some(ApiBaseUrl::parse("http://127.0.0.1:9/v1").unwrap()),
                api_key: ApiKeyChange::Keep,
                default_model: Some(ModelProfile {
                    model_id: ModelId::parse("writer").unwrap(),
                    context_window: 128_000,
                    max_output_tokens: 8_000,
                    supports_thinking: false,
                    reports_stream_usage: true,
                }),
            },
        )
        .await
        .expect("the model settings are saved");
}
