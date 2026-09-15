//! 系统级模型设置的持久化与乐观并发控制。
//!
//! SQLite 保存模型服务地址、能力资料与密钥正文。密钥只经由
//! [`Store::system_model_api_key`] 读出，绝不进入对外视图、回执、事件或请求哈希之外
//! 的任何产物；回执只落 payload 摘要，不落密钥正文。

use nekowriter_core::{ModelCapabilityProfile, decimal_i64, utc_now_millis};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set, TransactionTrait};
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;
use uuid::Uuid;

use super::{
    Store, StoreError, db_id,
    entity::{event, system_model_setting},
    insert_receipt, normalize_api_key, payload_hash, replay_or_conflict, transaction_error,
};

const SAVE_SYSTEM_MODEL_SETTINGS_METHOD: &str = "PUT";
const SYSTEM_MODEL_SETTINGS_TARGET: &str = "/api/settings";
/// 系统写作模型配置的稳定主键。
pub const SYSTEM_WRITING_MODEL_KEY: &str = "writing_model";

/// 已持久化的系统写作模型设置。
///
/// 对外的设置视图：只携带「密钥是否已配置」，从不携带密钥正文。HTTP 层据此向
/// 浏览器公开 `credential_configured`，而不是把本地密钥当成前端可编辑状态。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemModelSettings {
    pub api_base_url: String,
    pub credential_configured: bool,
    pub model_id: String,
    pub capabilities: ModelCapabilityProfile,
    #[serde(with = "decimal_i64")]
    pub max_context_tokens: i64,
    #[serde(with = "decimal_i64")]
    pub reserved_output_tokens: i64,
    #[serde(with = "decimal_i64")]
    pub revision: i64,
    #[serde(with = "decimal_i64")]
    pub created_at_ms: i64,
    #[serde(with = "decimal_i64")]
    pub updated_at_ms: i64,
}

/// 待保存的模型设置。
///
/// `api_key` 是密钥正文：`None` 表示「保留库中现有密钥」，`Some` 表示替换。它参与
/// command receipt 的请求哈希（只落摘要），以阻止同一个 `command_id` 被不同密钥重放。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSystemModelSettings {
    pub api_base_url: String,
    pub api_key: Option<String>,
    pub model_id: String,
    pub capabilities: ModelCapabilityProfile,
    #[serde(with = "decimal_i64")]
    pub max_context_tokens: i64,
    #[serde(with = "decimal_i64")]
    pub reserved_output_tokens: i64,
}

#[derive(Serialize)]
struct SaveSystemModelSettingsPayload<'a> {
    expected_revision: i64,
    api_base_url: &'a str,
    api_key: Option<&'a str>,
    model_id: &'a str,
    capabilities: ModelCapabilityProfile,
    max_context_tokens: i64,
    reserved_output_tokens: i64,
}

impl<'a> SaveSystemModelSettingsPayload<'a> {
    fn new(expected_revision: i64, settings: &'a NewSystemModelSettings) -> Self {
        Self {
            expected_revision,
            api_base_url: &settings.api_base_url,
            api_key: settings.api_key.as_deref(),
            model_id: &settings.model_id,
            capabilities: settings.capabilities,
            max_context_tokens: settings.max_context_tokens,
            reserved_output_tokens: settings.reserved_output_tokens,
        }
    }
}

impl Store {
    /// 返回系统写作模型设置；尚未配置时返回 `None`。
    ///
    /// # Errors
    ///
    /// 设置记录的能力 JSON 损坏或数据库读取失败时返回错误。
    pub async fn get_system_model_settings(
        &self,
    ) -> Result<Option<SystemModelSettings>, StoreError> {
        system_model_setting::Entity::find_by_id(SYSTEM_WRITING_MODEL_KEY.to_owned())
            .one(&self.database)
            .await?
            .map(|model| system_model_settings_from_model(&model))
            .transpose()
    }

    /// 返回系统写作模型的密钥正文；尚未配置时为 `None`。
    ///
    /// 这是密钥的唯一读出口：设置视图、回执、事件与日志都只能拿到
    /// `credential_configured`，不得拿到密钥。
    ///
    /// # Errors
    ///
    /// 数据库读取失败时返回错误。
    pub async fn system_model_api_key(&self) -> Result<Option<String>, StoreError> {
        Ok(
            system_model_setting::Entity::find_by_id(SYSTEM_WRITING_MODEL_KEY.to_owned())
                .one(&self.database)
                .await?
                .and_then(|model| model.api_key),
        )
    }

    /// 在真正写库前检查同一 command 是否已经有可重放的结果。
    ///
    /// App 以此避免网络重试用不同 API key 覆盖库中的既有密钥。若返回 `Some`，
    /// 调用方必须直接返回该结果，不得再写库。
    ///
    /// # Errors
    ///
    /// 设置无效、同一 `command_id` 复用于不同请求，或读取回执失败时返回错误。
    pub async fn replay_system_model_settings_if_known(
        &self,
        command_id: Uuid,
        expected_revision: i64,
        settings: NewSystemModelSettings,
    ) -> Result<Option<SystemModelSettings>, StoreError> {
        let settings = normalize_system_model_settings(settings)?;
        let payload_hash = payload_hash(&SaveSystemModelSettingsPayload::new(
            expected_revision,
            &settings,
        ))?;
        self.find_receipt(command_id)
            .await?
            .map_or(Ok(None), |receipt| {
                replay_or_conflict(
                    receipt,
                    SAVE_SYSTEM_MODEL_SETTINGS_METHOD,
                    SYSTEM_MODEL_SETTINGS_TARGET,
                    &payload_hash,
                )
                .map(Some)
            })
    }

    /// 保存系统写作模型配置并写入持久同步事件。
    ///
    /// 密钥与其余设置在同一事务中提交：保存失败不会留下与设置不一致的密钥。
    /// 设置和 command receipt 也在同一事务中提交。
    ///
    /// # Errors
    ///
    /// `设置格式无效、预期修订冲突、command_id` 冲突或数据库操作失败时返回错误。
    pub async fn save_system_model_settings(
        &self,
        command_id: Uuid,
        expected_revision: i64,
        settings: NewSystemModelSettings,
    ) -> Result<SystemModelSettings, StoreError> {
        let settings = normalize_system_model_settings(settings)?;
        let payload_hash = payload_hash(&SaveSystemModelSettingsPayload::new(
            expected_revision,
            &settings,
        ))?;
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(
                receipt,
                SAVE_SYSTEM_MODEL_SETTINGS_METHOD,
                SYSTEM_MODEL_SETTINGS_TARGET,
                &payload_hash,
            );
        }

        let settings_for_transaction = settings.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let existing = system_model_setting::Entity::find_by_id(
                        SYSTEM_WRITING_MODEL_KEY.to_owned(),
                    )
                    .one(transaction)
                    .await?;
                    let actual_revision = existing.as_ref().map_or(0, |model| model.revision);
                    if actual_revision != expected_revision {
                        return Err(StoreError::SystemSettingsRevisionConflict {
                            expected: expected_revision,
                            actual: actual_revision,
                        });
                    }
                    let revision = actual_revision.checked_add(1).ok_or_else(|| {
                        StoreError::InvalidSystemModelSettings(
                            "settings revision 已达到 i64 上限".to_owned(),
                        )
                    })?;
                    let capabilities_json =
                        serde_json::to_string(&settings_for_transaction.capabilities)?;
                    // 并入既有密钥：`None` 表示这次保存不动它，与界面「不重填不覆盖」
                    // 一致。密钥只存在于本事务内，不进入命令指纹之外的任何产物。
                    let api_key = settings_for_transaction
                        .api_key
                        .clone()
                        .or_else(|| existing.as_ref().and_then(|model| model.api_key.clone()));
                    let model = match existing {
                        Some(model) => {
                            let mut active = model.into_active_model();
                            active.api_base_url =
                                Set(settings_for_transaction.api_base_url.clone());
                            active.api_key = Set(api_key);
                            active.model_id = Set(settings_for_transaction.model_id.clone());
                            active.capabilities_json = Set(capabilities_json);
                            active.max_context_tokens =
                                Set(settings_for_transaction.max_context_tokens);
                            active.reserved_output_tokens =
                                Set(settings_for_transaction.reserved_output_tokens);
                            active.revision = Set(revision);
                            active.updated_at = Set(now);
                            active.update(transaction).await?
                        }
                        None => system_model_setting::ActiveModel {
                            key: Set(SYSTEM_WRITING_MODEL_KEY.to_owned()),
                            api_base_url: Set(settings_for_transaction.api_base_url.clone()),
                            api_key: Set(api_key),
                            model_id: Set(settings_for_transaction.model_id.clone()),
                            capabilities_json: Set(capabilities_json),
                            max_context_tokens: Set(settings_for_transaction.max_context_tokens),
                            reserved_output_tokens: Set(
                                settings_for_transaction.reserved_output_tokens
                            ),
                            revision: Set(revision),
                            created_at: Set(now),
                            updated_at: Set(now),
                            ..Default::default()
                        }
                        .insert(transaction)
                        .await?,
                    };
                    let response = system_model_settings_from_model(&model)?;
                    insert_system_settings_event(transaction, &response, now).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SAVE_SYSTEM_MODEL_SETTINGS_METHOD,
                        SYSTEM_MODEL_SETTINGS_TARGET,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
            .map_err(transaction_error);

        match result {
            Ok(settings) => {
                self.notify_event_listeners();
                Ok(settings)
            }
            Err(StoreError::Database(error)) => {
                self.replay_after_receipt_conflict(
                    command_id,
                    SAVE_SYSTEM_MODEL_SETTINGS_METHOD,
                    SYSTEM_MODEL_SETTINGS_TARGET,
                    &payload_hash,
                    StoreError::Database(error),
                )
                .await
            }
            Err(error) => Err(error),
        }
    }
}

fn normalize_system_model_settings(
    mut settings: NewSystemModelSettings,
) -> Result<NewSystemModelSettings, StoreError> {
    settings.api_base_url = settings.api_base_url.trim().to_owned();
    settings.model_id = settings.model_id.trim().to_owned();
    if settings.api_base_url.is_empty() {
        return Err(invalid_settings("模型服务地址不能为空"));
    }
    let url = Url::parse(&settings.api_base_url)
        .map_err(|error| invalid_settings(format!("模型服务地址无效：{error}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(invalid_settings("模型服务地址必须是带主机的 HTTP(S) URL"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid_settings("模型服务地址不能包含 query 或 fragment"));
    }
    let path = url.path().trim_matches('/');
    if path.is_empty() || path.ends_with("chat/completions") {
        return Err(invalid_settings(
            "模型服务地址必须是 API 根路径（例如 https://host/v1），不能是 chat/completions 完整端点",
        ));
    }
    if settings.model_id.is_empty() || settings.model_id.chars().count() > 256 {
        return Err(invalid_settings("模型 ID 必须为 1–256 个字符"));
    }
    if !settings.capabilities.streaming {
        return Err(invalid_settings("写作模型必须支持流式输出"));
    }
    if settings.max_context_tokens <= 0
        || settings.reserved_output_tokens <= 0
        || settings.reserved_output_tokens >= settings.max_context_tokens
    {
        return Err(invalid_settings(
            "上下文窗口必须为正数，且预留输出必须小于上下文窗口",
        ));
    }
    settings.api_key = normalize_api_key(settings.api_key).map_err(invalid_settings)?;
    Ok(settings)
}

fn invalid_settings(reason: impl Into<String>) -> StoreError {
    StoreError::InvalidSystemModelSettings(reason.into())
}

fn system_model_settings_from_model(
    model: &system_model_setting::Model,
) -> Result<SystemModelSettings, StoreError> {
    let capabilities = serde_json::from_str(&model.capabilities_json).map_err(|error| {
        StoreError::CorruptRecord(format!("系统模型能力配置 JSON 损坏：{error}"))
    })?;
    normalize_system_model_settings(NewSystemModelSettings {
        api_base_url: model.api_base_url.clone(),
        api_key: model.api_key.clone(),
        model_id: model.model_id.clone(),
        capabilities,
        max_context_tokens: model.max_context_tokens,
        reserved_output_tokens: model.reserved_output_tokens,
    })?;
    if model.revision <= 0 || model.updated_at < model.created_at {
        return Err(StoreError::CorruptRecord(
            "系统模型设置的修订或时间戳无效".to_owned(),
        ));
    }
    Ok(SystemModelSettings {
        api_base_url: model.api_base_url.clone(),
        credential_configured: model.api_key.is_some(),
        model_id: model.model_id.clone(),
        capabilities,
        max_context_tokens: model.max_context_tokens,
        reserved_output_tokens: model.reserved_output_tokens,
        revision: model.revision,
        created_at_ms: model.created_at,
        updated_at_ms: model.updated_at,
    })
}

async fn insert_system_settings_event(
    transaction: &sea_orm::DatabaseTransaction,
    settings: &SystemModelSettings,
    now: i64,
) -> Result<(), StoreError> {
    event::ActiveModel {
        version_id: Set(None),
        event_type: Set("settings.changed".to_owned()),
        entity_id: Set(db_id(Uuid::new_v4())),
        revision: Set(settings.revision.to_string()),
        payload_json: Set(serde_json::to_string(&json!({
            "scope": "system",
            "model_id": settings.model_id,
            "revision": settings.revision.to_string(),
        }))?),
        occurred_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_rejects_invalid_model_settings() {
        let settings = normalize_system_model_settings(NewSystemModelSettings {
            api_base_url: " https://model.example/v1/ ".to_owned(),
            api_key: Some("  secret-key-1  ".to_owned()),
            model_id: " writing-model ".to_owned(),
            capabilities: ModelCapabilityProfile {
                streaming: true,
                tools: true,
                stream_usage: true,
                thinking: true,
            },
            max_context_tokens: 32_768,
            reserved_output_tokens: 4_096,
        })
        .unwrap();
        assert_eq!(settings.api_base_url, "https://model.example/v1/");
        assert_eq!(settings.model_id, "writing-model");
        assert_eq!(settings.api_key.as_deref(), Some("secret-key-1"));
        assert!(matches!(
            normalize_system_model_settings(NewSystemModelSettings {
                api_base_url: "https://model.example/v1/chat/completions".to_owned(),
                ..settings.clone()
            }),
            Err(StoreError::InvalidSystemModelSettings(_))
        ));
    }

    /// `None` 表示保留既有密钥；给出时必须是单行非空正文。
    #[test]
    fn normalizes_and_rejects_invalid_api_keys() {
        let base = |api_key| NewSystemModelSettings {
            api_base_url: "https://model.example/v1".to_owned(),
            api_key,
            model_id: "writing-model".to_owned(),
            capabilities: ModelCapabilityProfile {
                streaming: true,
                tools: true,
                stream_usage: true,
                thinking: true,
            },
            max_context_tokens: 32_768,
            reserved_output_tokens: 4_096,
        };

        assert_eq!(
            normalize_system_model_settings(base(None)).unwrap().api_key,
            None
        );
        assert_eq!(
            normalize_system_model_settings(base(Some(" key ".to_owned())))
                .unwrap()
                .api_key
                .as_deref(),
            Some("key")
        );
        for api_key in [
            Some(String::new()),
            Some("   ".to_owned()),
            Some("line\nbreak".to_owned()),
            Some("k".repeat(crate::MAX_API_KEY_CHARS + 1)),
        ] {
            assert!(matches!(
                normalize_system_model_settings(base(api_key)),
                Err(StoreError::InvalidSystemModelSettings(_))
            ));
        }
    }
}
