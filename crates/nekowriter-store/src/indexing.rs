//! Indexing knowledge entries for semantic search (`docs/product-spec.md` §12.2).
//!
//! The current profile is the embedding model and vector size in the settings. Saving an
//! entry, or switching to a profile, queues one index task per entry under that profile;
//! the application's indexer works through them and stores each entry's chunks with their
//! vectors. A profile becomes ready, and searchable, once the entries live when it became
//! current are indexed. Vectors of different profiles are never mixed.

use std::collections::HashMap;

use nekowriter_core::{
    conflict::StateConflict,
    embedding::{EmbeddingFailure, EmbeddingModel, vector_bytes},
    event::Event,
    ids::{CommandId, EmbeddingProfileId, KnowledgeEntryId},
    knowledge::IndexState,
    model::ModelService,
    time::Timestamp,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QueryOrder, Set, sea_query::OnConflict,
};

use crate::{
    Store, StoreError,
    command::Begun,
    entity::{
        embedding_profile,
        embedding_request::{self, EmbeddingPurpose},
        embedding_settings::{self, ROW_ID},
        knowledge_chunk, knowledge_entry,
        knowledge_index::{self, IndexStatus},
    },
    knowledge::{EntryView, live_entry},
};

/// An entry to index: its content at a revision, and where to send it.
#[derive(Clone, Debug)]
pub struct IndexTask {
    pub entry_id: KnowledgeEntryId,
    pub profile_id: EmbeddingProfileId,
    pub entry_revision: i64,
    pub content: String,
    pub service: ModelService,
    pub model: EmbeddingModel,
}

/// Chunks with their vectors, in order.
pub type IndexedChunks = Vec<(String, Vec<f32>)>;

impl Store {
    /// Takes the oldest pending task of the current profile and marks it running.
    pub async fn next_index_task(&self) -> Result<Option<IndexTask>, StoreError> {
        let tx = self.begin_internal().await?;
        let Some(settings) = embedding_settings::Entity::find_by_id(ROW_ID)
            .one(tx.db())
            .await?
        else {
            return Ok(None);
        };
        let (Some(base_url), Some(model)) = (settings.api_base_url, settings.model) else {
            return Ok(None);
        };
        let Some(profile) = find_profile(tx.db(), &model).await? else {
            return Ok(None);
        };
        let Some(index) = knowledge_index::Entity::find()
            .filter(knowledge_index::Column::ProfileId.eq(profile.id))
            .filter(knowledge_index::Column::Status.eq(IndexStatus::Pending))
            .order_by_asc(knowledge_index::Column::QueuedAt)
            .one(tx.db())
            .await?
        else {
            return Ok(None);
        };
        let entry = knowledge_entry::Entity::find_by_id(index.entry_id)
            .one(tx.db())
            .await?
            .ok_or_else(|| {
                StoreError::Corrupt(format!("index of a missing entry {}", index.entry_id))
            })?;
        let task = IndexTask {
            entry_id: index.entry_id,
            profile_id: profile.id,
            entry_revision: index.entry_revision,
            content: entry.content,
            service: ModelService {
                base_url,
                api_key: settings.api_key,
            },
            model,
        };
        let mut running = index.into_active_model();
        running.status = Set(IndexStatus::Running);
        running.updated_at = Set(tx.now());
        running.update(tx.db()).await?;
        tx.commit().await?;
        Ok(Some(task))
    }

    /// Stores what came of a task. An outcome for an entry or profile that changed while
    /// it was embedded is dropped; the newer task stands.
    pub async fn finish_index_task(
        &self,
        task: &IndexTask,
        outcome: Result<IndexedChunks, EmbeddingFailure>,
        input_tokens: Option<i64>,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin_internal().await?;
        let now = tx.now();
        embedding_request::ActiveModel {
            profile_id: Set(task.profile_id),
            purpose: Set(EmbeddingPurpose::Index),
            answered: Set(outcome.is_ok()),
            input_tokens: Set(input_tokens),
            sent_at: Set(now),
            ..Default::default()
        }
        .insert(tx.db())
        .await?;
        let current = knowledge_index::Entity::find_by_id((task.entry_id, task.profile_id))
            .one(tx.db())
            .await?
            .filter(|index| {
                index.status == IndexStatus::Running && index.entry_revision == task.entry_revision
            });
        let Some(index) = current else {
            return tx.commit().await;
        };
        let mut finished = index.into_active_model();
        match outcome {
            Ok(chunks) => {
                knowledge_chunk::Entity::delete_many()
                    .filter(knowledge_chunk::Column::EntryId.eq(task.entry_id))
                    .filter(knowledge_chunk::Column::ProfileId.eq(task.profile_id))
                    .exec(tx.db())
                    .await?;
                for (ordinal, (text, vector)) in chunks.into_iter().enumerate() {
                    knowledge_chunk::ActiveModel {
                        entry_id: Set(task.entry_id),
                        profile_id: Set(task.profile_id),
                        entry_revision: Set(task.entry_revision),
                        ordinal: Set(i32::try_from(ordinal).unwrap_or(i32::MAX)),
                        text: Set(text),
                        vector: Set(vector_bytes(&vector)),
                        ..Default::default()
                    }
                    .insert(tx.db())
                    .await?;
                }
                finished.status = Set(IndexStatus::Ready);
                finished.failure = Set(None);
            }
            Err(failure) => {
                finished.status = Set(IndexStatus::Failed);
                finished.failure = Set(Some(failure));
            }
        }
        finished.updated_at = Set(now);
        finished.update(tx.db()).await?;
        mark_ready_when_done(tx.db(), task.profile_id, now).await?;
        tx.emit(Event::KnowledgeChanged);
        tx.commit().await
    }

    /// Queues an entry for indexing again, for example after a failure.
    pub async fn reindex_entry(
        &self,
        command: CommandId,
        id: KnowledgeEntryId,
    ) -> Result<EntryView, StoreError> {
        let mut tx = match self.begin_command(command, "entry.reindex", &id).await? {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let entry = live_entry(tx.db(), id).await?;
        let now = tx.now();
        if !queue_entry(tx.db(), entry.id, entry.revision, now).await? {
            return Err(StoreError::StateConflict(
                StateConflict::EmbeddingNotConfigured,
            ));
        }
        tx.emit(Event::KnowledgeChanged);
        tx.commit(EntryView::new(entry, IndexState::Pending)).await
    }

    /// Returns tasks that were running when the application stopped to the queue.
    pub async fn resume_indexing(&self) -> Result<u64, StoreError> {
        let tx = self.begin_internal().await?;
        let resumed = knowledge_index::Entity::update_many()
            .col_expr(
                knowledge_index::Column::Status,
                sea_orm::sea_query::Expr::value(IndexStatus::Pending),
            )
            .filter(knowledge_index::Column::Status.eq(IndexStatus::Running))
            .exec(tx.db())
            .await?
            .rows_affected;
        tx.commit().await?;
        Ok(resumed)
    }
}

async fn find_profile<C: ConnectionTrait>(
    db: &C,
    model: &EmbeddingModel,
) -> Result<Option<embedding_profile::Model>, StoreError> {
    Ok(embedding_profile::Entity::find()
        .filter(embedding_profile::Column::ModelId.eq(model.model_id.clone()))
        .filter(
            embedding_profile::Column::Dimensions
                .eq(i32::try_from(model.dimensions).unwrap_or(i32::MAX)),
        )
        .one(db)
        .await?)
}

/// The profile of the model in the settings, if one is set.
pub(crate) async fn current_profile<C: ConnectionTrait>(
    db: &C,
) -> Result<Option<embedding_profile::Model>, StoreError> {
    let model = embedding_settings::Entity::find_by_id(ROW_ID)
        .one(db)
        .await?
        .and_then(|settings| settings.model);
    match model {
        Some(model) => find_profile(db, &model).await,
        None => Ok(None),
    }
}

/// Makes `model` the current profile: creates it if needed and queues every live entry
/// that has no up-to-date index under it.
pub(crate) async fn activate_profile<C: ConnectionTrait>(
    db: &C,
    model: &EmbeddingModel,
    now: Timestamp,
) -> Result<(), StoreError> {
    let profile = match find_profile(db, model).await? {
        Some(profile) => profile,
        None => {
            embedding_profile::ActiveModel {
                id: Set(EmbeddingProfileId::generate()),
                model_id: Set(model.model_id.clone()),
                dimensions: Set(i32::try_from(model.dimensions).unwrap_or(i32::MAX)),
                ready_at: Set(None),
                created_at: Set(now),
            }
            .insert(db)
            .await?
        }
    };
    let indexes: HashMap<KnowledgeEntryId, knowledge_index::Model> =
        knowledge_index::Entity::find()
            .filter(knowledge_index::Column::ProfileId.eq(profile.id))
            .all(db)
            .await?
            .into_iter()
            .map(|index| (index.entry_id, index))
            .collect();
    let entries = knowledge_entry::Entity::find()
        .filter(knowledge_entry::Column::DeletedAt.is_null())
        .all(db)
        .await?;
    for entry in entries {
        let current = indexes.get(&entry.id).is_some_and(|index| {
            index.entry_revision == entry.revision && index.status != IndexStatus::Failed
        });
        if !current {
            upsert_pending(db, entry.id, profile.id, entry.revision, now).await?;
        }
    }
    mark_ready_when_done(db, profile.id, now).await
}

/// Queues an entry revision under the current profile; `false` when no model is set.
pub(crate) async fn queue_entry<C: ConnectionTrait>(
    db: &C,
    entry: KnowledgeEntryId,
    revision: i64,
    now: Timestamp,
) -> Result<bool, StoreError> {
    let Some(profile) = current_profile(db).await? else {
        return Ok(false);
    };
    upsert_pending(db, entry, profile.id, revision, now).await?;
    Ok(true)
}

async fn upsert_pending<C: ConnectionTrait>(
    db: &C,
    entry: KnowledgeEntryId,
    profile: EmbeddingProfileId,
    revision: i64,
    now: Timestamp,
) -> Result<(), StoreError> {
    knowledge_index::Entity::insert(knowledge_index::ActiveModel {
        entry_id: Set(entry),
        profile_id: Set(profile),
        entry_revision: Set(revision),
        status: Set(IndexStatus::Pending),
        failure: Set(None),
        queued_at: Set(now),
        updated_at: Set(now),
    })
    .on_conflict(
        OnConflict::columns([
            knowledge_index::Column::EntryId,
            knowledge_index::Column::ProfileId,
        ])
        .update_columns([
            knowledge_index::Column::EntryRevision,
            knowledge_index::Column::Status,
            knowledge_index::Column::Failure,
            knowledge_index::Column::QueuedAt,
            knowledge_index::Column::UpdatedAt,
        ])
        .to_owned(),
    )
    .exec(db)
    .await?;
    Ok(())
}

/// Removes a deleted entry's indexes and chunks under every profile.
pub(crate) async fn forget_entry<C: ConnectionTrait>(
    db: &C,
    entry: KnowledgeEntryId,
) -> Result<(), StoreError> {
    knowledge_chunk::Entity::delete_many()
        .filter(knowledge_chunk::Column::EntryId.eq(entry))
        .exec(db)
        .await?;
    knowledge_index::Entity::delete_many()
        .filter(knowledge_index::Column::EntryId.eq(entry))
        .exec(db)
        .await?;
    Ok(())
}

/// Marks the profile ready once none of its tasks is pending or running.
async fn mark_ready_when_done<C: ConnectionTrait>(
    db: &C,
    profile: EmbeddingProfileId,
    now: Timestamp,
) -> Result<(), StoreError> {
    let Some(profile) = embedding_profile::Entity::find_by_id(profile)
        .one(db)
        .await?
    else {
        return Ok(());
    };
    if profile.ready_at.is_some() {
        return Ok(());
    }
    let open = knowledge_index::Entity::find()
        .filter(knowledge_index::Column::ProfileId.eq(profile.id))
        .filter(knowledge_index::Column::Status.is_in([IndexStatus::Pending, IndexStatus::Running]))
        .count(db)
        .await?;
    if open == 0 {
        let mut ready = profile.into_active_model();
        ready.ready_at = Set(Some(now));
        ready.update(db).await?;
    }
    Ok(())
}

/// The index state of each entry under the current profile, in the order given.
pub(crate) async fn index_states<C: ConnectionTrait>(
    db: &C,
    entries: &[KnowledgeEntryId],
) -> Result<Vec<IndexState>, StoreError> {
    let Some(profile) = current_profile(db).await? else {
        return Ok(vec![IndexState::NotConfigured; entries.len()]);
    };
    let indexes: HashMap<KnowledgeEntryId, knowledge_index::Model> =
        knowledge_index::Entity::find()
            .filter(knowledge_index::Column::ProfileId.eq(profile.id))
            .filter(knowledge_index::Column::EntryId.is_in(entries.iter().copied()))
            .all(db)
            .await?
            .into_iter()
            .map(|index| (index.entry_id, index))
            .collect();
    entries
        .iter()
        .map(|entry| state_of(*entry, indexes.get(entry)))
        .collect()
}

/// The index state of one entry under the current profile.
pub(crate) async fn index_state<C: ConnectionTrait>(
    db: &C,
    entry: KnowledgeEntryId,
) -> Result<IndexState, StoreError> {
    let Some(profile) = current_profile(db).await? else {
        return Ok(IndexState::NotConfigured);
    };
    let index = knowledge_index::Entity::find_by_id((entry, profile.id))
        .one(db)
        .await?;
    state_of(entry, index.as_ref())
}

fn state_of(
    entry: KnowledgeEntryId,
    index: Option<&knowledge_index::Model>,
) -> Result<IndexState, StoreError> {
    // An entry without a task is about to get one: profiles queue every entry.
    let Some(index) = index else {
        return Ok(IndexState::Pending);
    };
    Ok(match (index.status, &index.failure) {
        (IndexStatus::Pending, _) => IndexState::Pending,
        (IndexStatus::Running, _) => IndexState::Running,
        (IndexStatus::Ready, _) => IndexState::Ready,
        (IndexStatus::Failed, Some(failure)) => IndexState::Failed {
            failure: failure.clone(),
        },
        (IndexStatus::Failed, None) => {
            return Err(StoreError::Corrupt(format!(
                "the failed index of entry {entry} has no failure"
            )));
        }
    })
}
