//! Searching the knowledge base by meaning (`docs/product-spec.md` §12.2). The search
//! page and the agents' knowledge tool share it.

use nekowriter_agent::{client::ChatClient, embedding::EmbeddingError};
use nekowriter_core::{
    ids::KnowledgeSpaceId,
    knowledge::{SearchQuery, SearchUnavailable, TopK},
};
use nekowriter_store::{SearchHit, Store, StoreError};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum SearchError {
    #[error("search is unavailable: {0:?}")]
    Unavailable(SearchUnavailable),
    #[error(transparent)]
    Embedding(#[from] EmbeddingError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Embeds the query once with the current profile's model and ranks the chunks of the
/// entries in `spaces` (all spaces when empty). The embedding request is recorded for the
/// dashboard whether it succeeds or not.
pub(crate) async fn search(
    store: &Store,
    client: &ChatClient,
    query: &SearchQuery,
    spaces: &[KnowledgeSpaceId],
    top_k: TopK,
) -> Result<Vec<SearchHit>, SearchError> {
    let profile = store
        .search_profile()
        .await?
        .map_err(SearchError::Unavailable)?;
    let embedded = client
        .embed_one(&profile.service, &profile.model, query.as_str().to_owned())
        .await;
    let input_tokens = embedded.as_ref().ok().and_then(|(_, tokens)| *tokens);
    store
        .record_search_embedding(profile.profile_id, embedded.is_ok(), input_tokens)
        .await?;
    let (vector, _) = embedded?;
    Ok(store
        .search_chunks(profile.profile_id, spaces, &vector, top_k)
        .await?)
}
