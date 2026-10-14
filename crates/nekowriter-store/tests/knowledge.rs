//! Knowledge spaces and entries (`docs/product-spec.md` §12.2).

mod common;

use common::open_store;
use nekowriter_core::{
    conflict::StateConflict,
    ids::{CommandId, KnowledgeSpaceId},
    knowledge::{EntryContent, EntryTitle, Keywords, SpaceName},
};
use nekowriter_store::{NewEntry, SpaceView, Store, StoreError};

async fn space(store: &Store, name: &str) -> Result<SpaceView, StoreError> {
    store
        .create_space(CommandId::generate(), SpaceName::parse(name).unwrap())
        .await
}

fn entry(space_id: KnowledgeSpaceId, title: &str, keywords: &[&str]) -> NewEntry {
    NewEntry {
        space_id,
        title: EntryTitle::parse(title).unwrap(),
        keywords: Keywords::parse(
            &keywords
                .iter()
                .map(|&keyword| keyword.to_owned())
                .collect::<Vec<_>>(),
        )
        .unwrap(),
        content: EntryContent::parse("Lamp oil was carried up the stairs.").unwrap(),
    }
}

#[tokio::test]
async fn entries_are_kept_in_spaces_and_found_by_title_or_keyword() {
    let (_dir, store) = open_store().await;
    let lighthouses = space(&store, "Lighthouses").await.unwrap();
    assert!(matches!(
        space(&store, "Lighthouses").await,
        Err(StoreError::StateConflict(StateConflict::SpaceNameTaken))
    ));

    let oil = store
        .create_entry(
            CommandId::generate(),
            entry(lighthouses.id, "Lamp oil", &["Fuel", "lamp"]),
        )
        .await
        .unwrap();
    store
        .create_entry(
            CommandId::generate(),
            entry(lighthouses.id, "Fog signals", &["sound"]),
        )
        .await
        .unwrap();

    let titles = |entries: Vec<nekowriter_store::EntrySummary>| {
        entries
            .into_iter()
            .map(|entry| entry.title)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        titles(store.list_entries(Some(lighthouses.id), "").await.unwrap()),
        ["Fog signals", "Lamp oil"]
    );
    assert_eq!(
        titles(store.list_entries(None, "fuel").await.unwrap()),
        ["Lamp oil"]
    );
    assert_eq!(store.list_spaces().await.unwrap()[0].entry_count, 2);

    let updated = store
        .update_entry(
            CommandId::generate(),
            oil.id,
            oil.revision,
            entry(lighthouses.id, "Lamp oil and wicks", &["fuel"]),
        )
        .await
        .unwrap();
    assert_eq!(updated.revision, 2);
    assert!(matches!(
        store
            .update_entry(
                CommandId::generate(),
                oil.id,
                oil.revision,
                entry(lighthouses.id, "Stale", &[]),
            )
            .await,
        Err(StoreError::RevisionConflict { .. })
    ));
}

#[tokio::test]
async fn only_an_empty_space_can_be_deleted() {
    let (_dir, store) = open_store().await;
    let notes = space(&store, "Notes").await.unwrap();
    let only = store
        .create_entry(CommandId::generate(), entry(notes.id, "Tides", &[]))
        .await
        .unwrap();

    assert!(matches!(
        store
            .delete_space(CommandId::generate(), notes.id, notes.revision)
            .await,
        Err(StoreError::StateConflict(StateConflict::SpaceNotEmpty))
    ));
    store
        .delete_entry(CommandId::generate(), only.id, only.revision)
        .await
        .unwrap();
    store
        .delete_space(CommandId::generate(), notes.id, notes.revision)
        .await
        .unwrap();
    assert_eq!(store.list_spaces().await.unwrap(), Vec::new());
    // The name is free again.
    space(&store, "Notes").await.unwrap();
}

mod indexing {
    use nekowriter_core::{
        embedding::{EmbeddingFailure, EmbeddingModel},
        knowledge::IndexState,
        model::{ApiBaseUrl, ModelId},
    };
    use nekowriter_store::{ApiKeyChange, EmbeddingSettingsInput};

    use super::*;

    async fn embed_with(store: &Store, model: &str, dimensions: u32) {
        let revision = store.embedding_settings().await.unwrap().revision;
        store
            .save_embedding_settings(
                CommandId::generate(),
                revision,
                EmbeddingSettingsInput {
                    api_base_url: Some(ApiBaseUrl::parse("http://127.0.0.1:9/v1").unwrap()),
                    api_key: ApiKeyChange::Keep,
                    model: Some(EmbeddingModel {
                        model_id: ModelId::parse(model).unwrap(),
                        dimensions,
                    }),
                },
            )
            .await
            .unwrap();
    }

    async fn state(store: &Store, entry: nekowriter_core::ids::KnowledgeEntryId) -> IndexState {
        store.entry(entry).await.unwrap().index
    }

    #[tokio::test]
    async fn entries_are_indexed_under_the_current_model() {
        let (_dir, store) = open_store().await;
        let notes = space(&store, "Notes").await.unwrap();
        let tides = store
            .create_entry(CommandId::generate(), entry(notes.id, "Tides", &[]))
            .await
            .unwrap();
        assert_eq!(tides.index, IndexState::NotConfigured);
        assert!(store.next_index_task().await.unwrap().is_none());

        // Choosing a model queues every entry.
        embed_with(&store, "embedder", 2).await;
        assert_eq!(state(&store, tides.id).await, IndexState::Pending);
        let task = store.next_index_task().await.unwrap().unwrap();
        assert_eq!(state(&store, tides.id).await, IndexState::Running);
        store
            .finish_index_task(
                &task,
                Ok(vec![("Lamp oil.".to_owned(), vec![1.0, 0.0])]),
                Some(3),
            )
            .await
            .unwrap();
        assert_eq!(state(&store, tides.id).await, IndexState::Ready);

        // An edit while a task runs makes that task's outcome stale.
        let task = {
            let current = store.entry(tides.id).await.unwrap();
            store
                .update_entry(
                    CommandId::generate(),
                    tides.id,
                    current.revision,
                    entry(notes.id, "Tides", &["sea"]),
                )
                .await
                .unwrap();
            store.next_index_task().await.unwrap().unwrap()
        };
        let current = store.entry(tides.id).await.unwrap();
        store
            .update_entry(
                CommandId::generate(),
                tides.id,
                current.revision,
                entry(notes.id, "Tides and moons", &["sea"]),
            )
            .await
            .unwrap();
        store
            .finish_index_task(&task, Err(EmbeddingFailure::NotFinite), None)
            .await
            .unwrap();
        assert_eq!(state(&store, tides.id).await, IndexState::Pending);

        let task = store.next_index_task().await.unwrap().unwrap();
        store
            .finish_index_task(&task, Err(EmbeddingFailure::NotFinite), None)
            .await
            .unwrap();
        assert_eq!(
            state(&store, tides.id).await,
            IndexState::Failed {
                failure: EmbeddingFailure::NotFinite
            }
        );
        store
            .reindex_entry(CommandId::generate(), tides.id)
            .await
            .unwrap();
        assert_eq!(state(&store, tides.id).await, IndexState::Pending);

        // Another model indexes from scratch; its tasks are separate.
        embed_with(&store, "other", 3).await;
        let task = store.next_index_task().await.unwrap().unwrap();
        assert_eq!(task.model.model_id.as_str(), "other");
    }
}
