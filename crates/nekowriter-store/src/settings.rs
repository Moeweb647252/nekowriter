//! System settings: the model service and the default model (`docs/product-spec.md` §13.3).
//!
//! The API key is write-only: views only say whether one is set, and only
//! [`Store::model_service`] reads it, to authenticate requests (§4 invariant 17).

use nekowriter_core::{
    conflict::Resource,
    event::Event,
    ids::CommandId,
    model::{ApiBaseUrl, ApiKey, ModelProfile, ModelService},
};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Store, StoreError,
    command::Begun,
    entity::system_settings::{self, ROW_ID},
    projects::check_revision,
};

/// The system settings as clients see them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SystemSettingsView {
    pub api_base_url: Option<ApiBaseUrl>,
    /// Whether an API key is stored. The key itself is never sent.
    pub api_key_set: bool,
    pub default_model: Option<ModelProfile>,
    /// 0 until the settings are first saved.
    pub revision: i64,
}

/// What a save does with the stored API key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiKeyChange {
    Keep,
    Clear,
    Replace(ApiKey),
}

/// The input of `settings.save`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemSettingsInput {
    pub api_base_url: Option<ApiBaseUrl>,
    pub api_key: ApiKeyChange,
    pub default_model: Option<ModelProfile>,
}

impl Store {
    pub async fn system_settings(&self) -> Result<SystemSettingsView, StoreError> {
        let row = system_settings::Entity::find_by_id(ROW_ID)
            .one(&self.db)
            .await?;
        Ok(settings_view(row.as_ref()))
    }

    /// Where requests go, with the API key; `None` until a service address is saved.
    pub async fn model_service(&self) -> Result<Option<ModelService>, StoreError> {
        let row = system_settings::Entity::find_by_id(ROW_ID)
            .one(&self.db)
            .await?;
        Ok(row.and_then(|row| {
            row.api_base_url.map(|base_url| ModelService {
                base_url,
                api_key: row.api_key,
            })
        }))
    }

    pub async fn save_system_settings(
        &self,
        command: CommandId,
        expected_revision: i64,
        input: SystemSettingsInput,
    ) -> Result<SystemSettingsView, StoreError> {
        let mut tx = match self
            .begin_command(
                command,
                "settings.save",
                &hash_input(expected_revision, &input),
            )
            .await?
        {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let existing = system_settings::Entity::find_by_id(ROW_ID)
            .one(tx.db())
            .await?;
        let current_revision = existing.as_ref().map_or(0, |row| row.revision);
        check_revision(Resource::Settings, expected_revision, current_revision)?;
        let api_key = match input.api_key {
            ApiKeyChange::Keep => existing.as_ref().and_then(|row| row.api_key.clone()),
            ApiKeyChange::Clear => None,
            ApiKeyChange::Replace(key) => Some(key),
        };
        let now = tx.now();
        let row = match existing {
            Some(row) => {
                let mut model = row.into_active_model();
                model.api_base_url = Set(input.api_base_url);
                model.api_key = Set(api_key);
                model.default_model = Set(input.default_model);
                model.revision = Set(current_revision + 1);
                model.updated_at = Set(now);
                model.update(tx.db()).await?
            }
            None => {
                system_settings::ActiveModel {
                    id: Set(ROW_ID),
                    api_base_url: Set(input.api_base_url),
                    api_key: Set(api_key),
                    default_model: Set(input.default_model),
                    revision: Set(1),
                    updated_at: Set(now),
                }
                .insert(tx.db())
                .await?
            }
        };
        tx.emit(Event::SettingsChanged);
        tx.commit(settings_view(Some(&row))).await
    }
}

fn settings_view(row: Option<&system_settings::Model>) -> SystemSettingsView {
    SystemSettingsView {
        api_base_url: row.and_then(|row| row.api_base_url.clone()),
        api_key_set: row.is_some_and(|row| row.api_key.is_some()),
        default_model: row.and_then(|row| row.default_model.clone()),
        revision: row.map_or(0, |row| row.revision),
    }
}

/// The command input as hashed for idempotency. A key enters only as its SHA-256 digest
/// inside the command hash; receipts never contain it.
#[derive(Serialize)]
struct HashedInput<'a> {
    expected_revision: i64,
    api_base_url: Option<&'a str>,
    api_key: (&'static str, Option<&'a str>),
    default_model: Option<&'a ModelProfile>,
}

fn hash_input(expected_revision: i64, input: &SystemSettingsInput) -> HashedInput<'_> {
    HashedInput {
        expected_revision,
        api_base_url: input.api_base_url.as_ref().map(ApiBaseUrl::as_str),
        api_key: match &input.api_key {
            ApiKeyChange::Keep => ("keep", None),
            ApiKeyChange::Clear => ("clear", None),
            ApiKeyChange::Replace(key) => ("replace", Some(key.expose())),
        },
        default_model: input.default_model.as_ref(),
    }
}
