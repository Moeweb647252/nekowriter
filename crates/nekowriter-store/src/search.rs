//! Semantic knowledge search (`docs/product-spec.md` §12.2): exact cosine similarity
//! over the chunks of the current profile whose entries are indexed at their current
//! revision. The application embeds the query; this module ranks the chunks.

use std::collections::HashMap;

use nekowriter_core::{
    embedding::{EmbeddingModel, cosine, vector_from_bytes},
    ids::{EmbeddingProfileId, KnowledgeEntryId, KnowledgeSpaceId},
    knowledge::{EntryTitle, SearchUnavailable, TopK},
    model::ModelService,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DerivePartialModel, EntityTrait, PaginatorTrait, QueryFilter,
    Set,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Store, StoreError,
    entity::{
        embedding_request::{self, EmbeddingPurpose},
        embedding_settings::{self, ROW_ID},
        knowledge_chunk, knowledge_entry,
        knowledge_index::{self, IndexStatus},
        knowledge_space,
    },
    indexing::current_profile,
};

/// The current profile, ready to search, with the service that embeds the query.
#[derive(Clone, Debug)]
pub struct SearchProfile {
    pub profile_id: EmbeddingProfileId,
    pub service: ModelService,
    pub model: EmbeddingModel,
}

/// A chunk that matched, with where it comes from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchHit {
    pub entry_id: KnowledgeEntryId,
    pub entry_revision: i64,
    pub chunk_id: i64,
    pub title: String,
    pub space_id: KnowledgeSpaceId,
    pub space_name: String,
    pub text: String,
    /// Cosine similarity with the query, from -1 to 1; a relevance, not a confidence.
    pub relevance: f32,
}

#[derive(Debug, DerivePartialModel)]
#[sea_orm(entity = "knowledge_entry::Entity")]
struct EntryRow {
    id: KnowledgeEntryId,
    space_id: KnowledgeSpaceId,
    title: EntryTitle,
    revision: i64,
}

impl Store {
    /// The profile to search with, or why search cannot run now.
    pub async fn search_profile(
        &self,
    ) -> Result<Result<SearchProfile, SearchUnavailable>, StoreError> {
        let settings = embedding_settings::Entity::find_by_id(ROW_ID)
            .one(&self.db)
            .await?;
        let (Some(profile), Some(settings)) = (current_profile(&self.db).await?, settings) else {
            return Ok(Err(SearchUnavailable::NotConfigured));
        };
        let (Some(base_url), Some(model)) = (settings.api_base_url, settings.model) else {
            return Ok(Err(SearchUnavailable::NotConfigured));
        };
        if profile.ready_at.is_none() {
            let remaining = knowledge_index::Entity::find()
                .filter(knowledge_index::Column::ProfileId.eq(profile.id))
                .filter(
                    knowledge_index::Column::Status
                        .is_in([IndexStatus::Pending, IndexStatus::Running]),
                )
                .count(&self.db)
                .await?;
            return Ok(Err(SearchUnavailable::Indexing {
                remaining: u32::try_from(remaining).unwrap_or(u32::MAX),
            }));
        }
        Ok(Ok(SearchProfile {
            profile_id: profile.id,
            service: ModelService {
                base_url,
                api_key: settings.api_key,
            },
            model,
        }))
    }

    /// Records an embedding request made for a search, for the dashboard (§15).
    pub async fn record_search_embedding(
        &self,
        profile: EmbeddingProfileId,
        answered: bool,
        input_tokens: Option<i64>,
    ) -> Result<(), StoreError> {
        let tx = self.begin_internal().await?;
        embedding_request::ActiveModel {
            profile_id: Set(profile),
            purpose: Set(EmbeddingPurpose::Search),
            answered: Set(answered),
            input_tokens: Set(input_tokens),
            sent_at: Set(tx.now()),
            ..Default::default()
        }
        .insert(tx.db())
        .await?;
        tx.commit().await
    }

    /// The `top_k` chunks most similar to `query`, among the live entries of `spaces` (all
    /// spaces when empty) that are indexed at their current revision.
    pub async fn search_chunks(
        &self,
        profile: EmbeddingProfileId,
        spaces: &[KnowledgeSpaceId],
        query: &[f32],
        top_k: TopK,
    ) -> Result<Vec<SearchHit>, StoreError> {
        let mut entries =
            knowledge_entry::Entity::find().filter(knowledge_entry::Column::DeletedAt.is_null());
        if !spaces.is_empty() {
            entries =
                entries.filter(knowledge_entry::Column::SpaceId.is_in(spaces.iter().copied()));
        }
        let entries: HashMap<KnowledgeEntryId, EntryRow> = entries
            .into_partial_model::<EntryRow>()
            .all(&self.db)
            .await?
            .into_iter()
            .map(|entry| (entry.id, entry))
            .collect();
        let ready: Vec<KnowledgeEntryId> = knowledge_index::Entity::find()
            .filter(knowledge_index::Column::ProfileId.eq(profile))
            .filter(knowledge_index::Column::Status.eq(IndexStatus::Ready))
            .filter(knowledge_index::Column::EntryId.is_in(entries.keys().copied()))
            .all(&self.db)
            .await?
            .into_iter()
            .filter(|index| {
                entries
                    .get(&index.entry_id)
                    .is_some_and(|entry| entry.revision == index.entry_revision)
            })
            .map(|index| index.entry_id)
            .collect();
        let chunks = knowledge_chunk::Entity::find()
            .filter(knowledge_chunk::Column::ProfileId.eq(profile))
            .filter(knowledge_chunk::Column::EntryId.is_in(ready))
            .all(&self.db)
            .await?;
        let mut scored = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            let vector = vector_from_bytes(&chunk.vector).ok_or_else(|| {
                StoreError::Corrupt(format!("chunk {} has a broken vector", chunk.id))
            })?;
            scored.push((cosine(query, &vector), chunk));
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored.truncate(top_k.get());

        let space_names: HashMap<KnowledgeSpaceId, String> = knowledge_space::Entity::find()
            .all(&self.db)
            .await?
            .into_iter()
            .map(|space| (space.id, space.name.as_str().to_owned()))
            .collect();
        scored
            .into_iter()
            .map(|(relevance, chunk)| {
                let entry = entries.get(&chunk.entry_id).ok_or_else(|| {
                    StoreError::Corrupt(format!("chunk {} lost its entry", chunk.id))
                })?;
                let space_name = space_names.get(&entry.space_id).ok_or_else(|| {
                    StoreError::Corrupt(format!("entry {} lost its space", entry.id))
                })?;
                Ok(SearchHit {
                    entry_id: entry.id,
                    entry_revision: entry.revision,
                    chunk_id: chunk.id,
                    title: entry.title.as_str().to_owned(),
                    space_id: entry.space_id,
                    space_name: space_name.clone(),
                    text: chunk.text,
                    relevance,
                })
            })
            .collect()
    }
}
