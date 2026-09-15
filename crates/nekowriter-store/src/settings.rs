//! 系统级模型设置的持久化与乐观并发控制。
//!
//! SQLite 保存模型服务地址、能力资料与密钥正文。密钥只经由
//! [`Store::system_model_api_key`] 读出，绝不进入对外视图、事件、日志或回执；回执只落
//! 请求指纹，而指纹不包含密钥正文、也不包含密钥的派生材料。

use nekowriter_core::{ModelCapabilityProfile, decimal_i64, utc_now_millis};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::{
    Command, CredentialIntent, CredentialPatch, Notify, Store, StoreError,
    entity::system_model_setting, insert_receipt, payload_hash, preparation::insert_app_event,
    validate_api_base_url,
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
/// `api_key` 是三态意图（保留 / 清除 / 替换），密钥正文不进入任何持久位置；命令指纹
/// 也只记录意图，不记录密钥本身：以密钥为输入的摘要会留在库里，比密钥活得久，而且能被
/// 离线核对。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewSystemModelSettings {
    pub api_base_url: String,
    pub api_key: CredentialPatch,
    pub model_id: String,
    pub capabilities: ModelCapabilityProfile,
    pub max_context_tokens: i64,
    pub reserved_output_tokens: i64,
}

/// 保存系统模型设置这条命令的指纹输入。
///
/// 只包含密钥意图而不包含密钥正文：摘要里不再有密钥的派生材料（以密钥为输入的摘要会
/// 留在库里、比密钥活得久，而且能被离线核对）。代价是同一个 `command_id` 换一把密钥
/// 重放会命中既有回执（返回原结果、不再写库），而不是报冲突——命令 id 由每次保存重新
/// 生成，所以这只影响「复用 id 改正文」的调用方。
#[derive(Serialize)]
struct SaveSystemModelSettingsPayload<'a> {
    expected_revision: i64,
    api_base_url: &'a str,
    credential: CredentialIntent,
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
            credential: settings.api_key.intent(),
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
    /// 调用方必须直接返回该结果，不得再写库。这是路由层的幂等出口，发生在修订号
    /// 校验之前，不是 `Command::commit` 的事务前预查。
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
        Command::new(
            command_id,
            SAVE_SYSTEM_MODEL_SETTINGS_METHOD,
            SYSTEM_MODEL_SETTINGS_TARGET,
            &payload_hash,
        )
        .replay_if_recorded(self)
        .await
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
        let command = Command::new(
            command_id,
            SAVE_SYSTEM_MODEL_SETTINGS_METHOD,
            SYSTEM_MODEL_SETTINGS_TARGET,
            &payload_hash,
        );

        let settings_for_transaction = settings.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        command
            .commit(self, Notify::Listeners, move |transaction| {
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
                    // 并入既有密钥：`Keep` 不动它（与界面「不重填不覆盖」一致），
                    // `Clear` 把列置回 NULL，`Replace` 换成新正文。密钥只存在于本事务内，
                    // 不进入命令指纹之外的任何产物。
                    let api_key = settings_for_transaction
                        .api_key
                        .clone()
                        .merge(existing.as_ref().and_then(|model| model.api_key.clone()));
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
    }
}

fn normalize_system_model_settings(
    mut settings: NewSystemModelSettings,
) -> Result<NewSystemModelSettings, StoreError> {
    // 地址规则与 Embedding 设置共用一处判定：不会一侧宽、一侧严。
    settings.api_base_url =
        validate_api_base_url(&settings.api_base_url).map_err(invalid_settings)?;
    settings.model_id = settings.model_id.trim().to_owned();
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
    settings.api_key = settings.api_key.normalize().map_err(invalid_settings)?;
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
        // 读路径不修改密钥，但仍然校验库中那一行的正文（空正文应被当成损坏记录）。
        api_key: model
            .api_key
            .clone()
            .map_or(CredentialPatch::Keep, CredentialPatch::Replace),
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
    insert_app_event(
        transaction,
        "settings.changed",
        Uuid::new_v4(),
        settings.revision,
        json!({
            "scope": "system",
            "model_id": settings.model_id,
            "revision": settings.revision.to_string(),
        }),
        now,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with(api_key: CredentialPatch) -> NewSystemModelSettings {
        NewSystemModelSettings {
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
        }
    }

    #[test]
    fn normalizes_and_rejects_invalid_model_settings() {
        let settings = normalize_system_model_settings(NewSystemModelSettings {
            api_base_url: " https://model.example/v1/ ".to_owned(),
            model_id: " writing-model ".to_owned(),
            ..settings_with(CredentialPatch::Replace("  secret-key-1  ".to_owned()))
        })
        .unwrap();
        assert_eq!(settings.api_base_url, "https://model.example/v1");
        assert_eq!(settings.model_id, "writing-model");
        assert_eq!(
            settings.api_key,
            CredentialPatch::Replace("secret-key-1".to_owned())
        );
        assert!(matches!(
            normalize_system_model_settings(NewSystemModelSettings {
                api_base_url: "https://model.example/v1/chat/completions".to_owned(),
                ..settings.clone()
            }),
            Err(StoreError::InvalidSystemModelSettings(_))
        ));
    }

    /// 三种意图各自成立：保留与清除不做正文校验，替换必须是单行非空正文。
    #[test]
    fn normalizes_and_rejects_invalid_api_keys() {
        assert_eq!(
            normalize_system_model_settings(settings_with(CredentialPatch::Keep))
                .unwrap()
                .api_key,
            CredentialPatch::Keep
        );
        assert_eq!(
            normalize_system_model_settings(settings_with(CredentialPatch::Clear))
                .unwrap()
                .api_key,
            CredentialPatch::Clear
        );
        assert_eq!(
            normalize_system_model_settings(settings_with(CredentialPatch::Replace(
                " key ".to_owned()
            )))
            .unwrap()
            .api_key,
            CredentialPatch::Replace("key".to_owned())
        );
        for api_key in [
            String::new(),
            "   ".to_owned(),
            "line\nbreak".to_owned(),
            "k".repeat(crate::MAX_API_KEY_CHARS + 1),
        ] {
            assert!(matches!(
                normalize_system_model_settings(settings_with(CredentialPatch::Replace(api_key))),
                Err(StoreError::InvalidSystemModelSettings(_))
            ));
        }
    }

    /// 回执里的指纹不以密钥为输入：换一把密钥不会改变它（库里因此没有能被离线核对的
    /// 密钥派生材料），但「保留」「替换」「清除」三种意图以及不同的请求版本仍分得开。
    #[test]
    fn request_fingerprint_ignores_the_credential_body() {
        let fingerprint = |expected_revision: i64, api_key: CredentialPatch| {
            let settings = normalize_system_model_settings(settings_with(api_key)).unwrap();
            payload_hash(&SaveSystemModelSettingsPayload::new(
                expected_revision,
                &settings,
            ))
            .unwrap()
        };
        let replace = |key: &str| CredentialPatch::Replace(key.to_owned());

        assert_eq!(
            fingerprint(7, replace("first-key")),
            fingerprint(7, replace("second-key"))
        );
        assert_ne!(
            fingerprint(7, replace("first-key")),
            fingerprint(7, CredentialPatch::Keep)
        );
        assert_ne!(
            fingerprint(7, replace("first-key")),
            fingerprint(7, CredentialPatch::Clear)
        );
        assert_ne!(
            fingerprint(7, replace("first-key")),
            fingerprint(8, replace("first-key"))
        );
    }

    /// 三态来自 JSON：字段缺席是保留，`null` 是清除，字符串是替换。
    #[test]
    fn credential_patch_reads_the_three_json_shapes() {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Patch {
            #[serde(default)]
            api_key: CredentialPatch,
        }

        let keep: Patch = serde_json::from_str("{}").unwrap();
        let clear: Patch = serde_json::from_str(r#"{"apiKey": null}"#).unwrap();
        let replace: Patch = serde_json::from_str(r#"{"apiKey": "k-1"}"#).unwrap();
        assert_eq!(keep.api_key, CredentialPatch::Keep);
        assert_eq!(clear.api_key, CredentialPatch::Clear);
        assert_eq!(replace.api_key, CredentialPatch::Replace("k-1".to_owned()));
    }
}
