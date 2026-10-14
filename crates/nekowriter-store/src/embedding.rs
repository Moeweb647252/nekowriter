//! The embedding settings (`docs/product-spec.md` §12.2, §13.3, D10). Like the chat
//! model's, the API key is write-only.

use nekowriter_core::{
    conflict::Resource,
    embedding::EmbeddingModel,
    event::Event,
    ids::CommandId,
    model::{ApiBaseUrl, ModelService},
};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    ApiKeyChange, Store, StoreError,
    command::Begun,
    entity::embedding_settings::{self, ROW_ID},
    projects::check_revision,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EmbeddingSettingsView {
    pub api_base_url: Option<ApiBaseUrl>,
    /// Whether an API key is stored. The key itself is never sent.
    pub api_key_set: bool,
    pub model: Option<EmbeddingModel>,
    /// 0 until the settings are first saved.
    pub revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddingSettingsInput {
    pub api_base_url: Option<ApiBaseUrl>,
    pub api_key: ApiKeyChange,
    pub model: Option<EmbeddingModel>,
}

/// Where embeddings come from, once both a service and a model are set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddingService {
    pub service: ModelService,
    pub model: EmbeddingModel,
}

impl Store {
    pub async fn embedding_settings(&self) -> Result<EmbeddingSettingsView, StoreError> {
        let row = embedding_settings::Entity::find_by_id(ROW_ID)
            .one(&self.db)
            .await?;
        Ok(view(row.as_ref()))
    }

    /// The service with its API key, whether or not a model is set.
    pub async fn embedding_endpoint(&self) -> Result<Option<ModelService>, StoreError> {
        let row = embedding_settings::Entity::find_by_id(ROW_ID)
            .one(&self.db)
            .await?;
        Ok(row.and_then(|row| {
            Some(ModelService {
                base_url: row.api_base_url?,
                api_key: row.api_key,
            })
        }))
    }

    /// The service and model with the API key, to send requests with.
    pub async fn embedding_service(&self) -> Result<Option<EmbeddingService>, StoreError> {
        let row = embedding_settings::Entity::find_by_id(ROW_ID)
            .one(&self.db)
            .await?;
        Ok(row.and_then(|row| {
            Some(EmbeddingService {
                service: ModelService {
                    base_url: row.api_base_url?,
                    api_key: row.api_key,
                },
                model: row.model?,
            })
        }))
    }

    pub async fn save_embedding_settings(
        &self,
        command: CommandId,
        expected_revision: i64,
        input: EmbeddingSettingsInput,
    ) -> Result<EmbeddingSettingsView, StoreError> {
        let hashed = (
            expected_revision,
            input.api_base_url.as_ref().map(ApiBaseUrl::as_str),
            input.api_key.hashed(),
            input.model.as_ref(),
        );
        let mut tx = match self
            .begin_command(command, "embedding.save", &hashed)
            .await?
        {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let existing = embedding_settings::Entity::find_by_id(ROW_ID)
            .one(tx.db())
            .await?;
        let revision = existing.as_ref().map_or(0, |row| row.revision);
        check_revision(Resource::Settings, expected_revision, revision)?;
        let api_key = match input.api_key {
            ApiKeyChange::Keep => existing.as_ref().and_then(|row| row.api_key.clone()),
            ApiKeyChange::Clear => None,
            ApiKeyChange::Replace(key) => Some(key),
        };
        let now = tx.now();
        let row = match existing {
            Some(row) => {
                let mut row = row.into_active_model();
                row.api_base_url = Set(input.api_base_url);
                row.api_key = Set(api_key);
                row.model = Set(input.model);
                row.revision = Set(revision + 1);
                row.updated_at = Set(now);
                row.update(tx.db()).await?
            }
            None => {
                embedding_settings::ActiveModel {
                    id: Set(ROW_ID),
                    api_base_url: Set(input.api_base_url),
                    api_key: Set(api_key),
                    model: Set(input.model),
                    revision: Set(1),
                    updated_at: Set(now),
                }
                .insert(tx.db())
                .await?
            }
        };
        tx.emit(Event::SettingsChanged);
        tx.commit(view(Some(&row))).await
    }
}

fn view(row: Option<&embedding_settings::Model>) -> EmbeddingSettingsView {
    EmbeddingSettingsView {
        api_base_url: row.and_then(|row| row.api_base_url.clone()),
        api_key_set: row.is_some_and(|row| row.api_key.is_some()),
        model: row.and_then(|row| row.model.clone()),
        revision: row.map_or(0, |row| row.revision),
    }
}
