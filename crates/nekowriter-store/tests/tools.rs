//! Knowledge queries and the auxiliary budget (`docs/product-spec.md` §8.3, §12.1).

mod common;

use common::{answer_with, call, dispatch_next, next_results, open_store, started, workspace};
use nekowriter_core::{
    call::CallRequest,
    embedding::EmbeddingModel,
    ids::{CommandId, RunId, VersionId},
    knowledge::{SearchUnavailable, SpaceName},
    model::{ApiBaseUrl, ModelId},
    project::AuxiliaryBudget,
    run_message::ToolOutcome,
};
use nekowriter_store::{ApiKeyChange, EmbeddingSettingsInput, ProjectSettings, RoundRecord, Store};

/// A preparation run that asked the knowledge base, with the call waiting to execute.
async fn queried(store: &Store, spaces: &[&str]) -> (VersionId, RunId) {
    let version = started(store).await;
    let (_, request) = dispatch_next(store).await;
    answer_with(
        store,
        request,
        RoundRecord::Call {
            call: call("query_knowledge"),
            request: CallRequest::QueryKnowledge {
                query: "lamp oil".to_owned(),
                spaces: spaces.iter().map(|&space| space.to_owned()).collect(),
            },
        },
    )
    .await;
    let run = workspace(store, version).await.run.unwrap().id;
    (version, run)
}

/// Embedding settings whose profile is ready at once: there are no entries to index.
async fn searchable(store: &Store) {
    store
        .save_embedding_settings(
            CommandId::generate(),
            0,
            EmbeddingSettingsInput {
                api_base_url: Some(ApiBaseUrl::parse("http://127.0.0.1:9/v1").unwrap()),
                api_key: ApiKeyChange::Keep,
                model: Some(EmbeddingModel {
                    model_id: ModelId::parse("embedder").unwrap(),
                    dimensions: 2,
                }),
            },
        )
        .await
        .unwrap();
    store
        .create_space(
            CommandId::generate(),
            SpaceName::parse("Lighthouses").unwrap(),
        )
        .await
        .unwrap();
}

async fn set_budget(store: &Store, version: VersionId, budget: u32) {
    let project = workspace(store, version).await.project;
    store
        .update_project_settings(
            CommandId::generate(),
            project.id,
            project.revision,
            ProjectSettings {
                writing_language: project.writing_language,
                model: project.model,
                thinking_level: project.thinking_level,
                auxiliary_budget: AuxiliaryBudget::new(budget).unwrap(),
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_query_is_charged_once_and_its_hits_become_the_result() {
    let (_dir, store) = open_store().await;
    searchable(&store).await;
    let (_, run) = queried(&store, &["Lighthouses"]).await;

    let search = store.begin_tool(run).await.unwrap().unwrap();
    assert_eq!(search.remaining_budget, 9);
    assert_eq!(search.spaces.len(), 1);
    // Interrupted before it finished, the call is carried out again without a new charge.
    let again = store.begin_tool(run).await.unwrap().unwrap();
    assert_eq!(again.remaining_budget, 9);

    store
        .finish_tool(
            search.call_id,
            ToolOutcome::KnowledgeFound {
                hits: Vec::new(),
                remaining_budget: search.remaining_budget,
            },
        )
        .await
        .unwrap();
    assert!(store.begin_tool(run).await.unwrap().is_none());
    assert!(matches!(
        next_results(&store).await[0].0,
        ToolOutcome::KnowledgeFound {
            remaining_budget: 9,
            ..
        }
    ));
}

#[tokio::test]
async fn queries_that_cannot_run_are_not_charged() {
    let (_dir, store) = open_store().await;
    let (version, run) = queried(&store, &[]).await;
    // Without an embedding model, search is unavailable.
    assert!(store.begin_tool(run).await.unwrap().is_none());
    assert_eq!(
        next_results(&store).await[0].0,
        ToolOutcome::KnowledgeUnavailable {
            reason: SearchUnavailable::NotConfigured
        }
    );

    searchable(&store).await;
    set_budget(&store, version, 0).await;
    let (_, request) = dispatch_next(&store).await;
    answer_with(
        &store,
        request,
        RoundRecord::Call {
            call: call("query_knowledge"),
            request: CallRequest::QueryKnowledge {
                query: "lamp oil".to_owned(),
                spaces: vec!["Gone".to_owned()],
            },
        },
    )
    .await;
    assert!(store.begin_tool(run).await.unwrap().is_none());
    assert_eq!(
        next_results(&store).await[0].0,
        ToolOutcome::KnowledgeSpacesGone {
            names: vec!["Gone".to_owned()]
        }
    );

    let (_, request) = dispatch_next(&store).await;
    answer_with(
        &store,
        request,
        RoundRecord::Call {
            call: call("query_knowledge"),
            request: CallRequest::QueryKnowledge {
                query: "lamp oil".to_owned(),
                spaces: Vec::new(),
            },
        },
    )
    .await;
    assert!(store.begin_tool(run).await.unwrap().is_none());
    assert_eq!(
        next_results(&store).await[0].0,
        ToolOutcome::QuotaExhausted { budget: 0 }
    );
}
