//! 系统写作模型设置与密钥：版本化、幂等、密钥保留与 API 根地址规则。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3），只移动：函数体逐字不变。

use super::*;

/// 地址规则只有一份：写作模型与 Embedding 两条设置路径拒绝同一批形状。
#[test]
fn api_base_url_rule_is_one_rule_for_both_settings() {
    assert_eq!(
        validate_api_base_url(" https://host/v1/ ").unwrap(),
        "https://host/v1"
    );
    for invalid in [
        "",
        "host/v1",
        "ftp://host/v1",
        "https://user:pw@host/v1",
        "https://host/v1?x=1",
        "https://host/v1#frag",
        "https://host",
        "https://host/v1/chat/completions",
    ] {
        assert!(
            validate_api_base_url(invalid).is_err(),
            "应被拒绝：{invalid}"
        );
    }
}

#[tokio::test]
async fn system_model_settings_are_revisioned_idempotent_and_emit_sync_events() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let command_id = Uuid::new_v4();
    let input = NewSystemModelSettings {
        api_base_url: "https://model.example/v1".to_owned(),
        api_key: CredentialPatch::Replace("secret-key-1".to_owned()),
        model_id: "writer-1".to_owned(),
        capabilities: ModelCapabilityProfile {
            streaming: true,
            tools: true,
            stream_usage: true,
            thinking: true,
        },
        max_context_tokens: 32_768,
        reserved_output_tokens: 4_096,
    };

    let saved = store
        .save_system_model_settings(command_id, 0, input.clone())
        .await
        .unwrap();
    assert_eq!(saved.revision, 1);
    // 对外视图只公布「已配置」，密钥正文只能经由专属读出口取得。
    let view = serde_json::to_value(&saved).unwrap();
    assert_eq!(view["credentialConfigured"], serde_json::Value::Bool(true));
    assert!(!view.to_string().contains("secret-key-1"));
    assert_eq!(
        store.system_model_api_key().await.unwrap().as_deref(),
        Some("secret-key-1")
    );
    assert_eq!(
        store
            .save_system_model_settings(command_id, 0, input.clone())
            .await
            .unwrap(),
        saved
    );
    assert_eq!(
        store.get_system_model_settings().await.unwrap(),
        Some(saved)
    );
    assert!(matches!(
        store
            .save_system_model_settings(Uuid::new_v4(), 0, input)
            .await,
        Err(StoreError::SystemSettingsRevisionConflict {
            expected: 0,
            actual: 1,
        })
    ));
    assert!(
        store
            .events_after(0)
            .await
            .unwrap()
            .iter()
            .any(|event| event.event_type == "settings.changed")
    );
}

/// 省略密钥（`Keep`）保留库中现有密钥；换密钥重放同一命令命中既有回执。
#[tokio::test]
async fn system_model_api_key_is_kept_when_omitted_and_guarded_when_replayed() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let settings = |api_key| NewSystemModelSettings {
        api_base_url: "https://model.example/v1".to_owned(),
        api_key,
        model_id: "writer-1".to_owned(),
        capabilities: ModelCapabilityProfile {
            streaming: true,
            tools: true,
            stream_usage: true,
            thinking: true,
        },
        max_context_tokens: 32_768,
        reserved_output_tokens: 4_096,
    };

    let first = store
        .save_system_model_settings(
            Uuid::new_v4(),
            0,
            settings(CredentialPatch::Replace("secret-key-1".to_owned())),
        )
        .await
        .unwrap();
    assert!(first.credential_configured);

    // 省略密钥的保存把既有密钥原样带过：修订前进，密钥不变。
    let second = store
        .save_system_model_settings(Uuid::new_v4(), 1, settings(CredentialPatch::Keep))
        .await
        .unwrap();
    assert_eq!(second.revision, 2);
    assert_eq!(second.api_base_url, first.api_base_url);
    assert_eq!(
        store.system_model_api_key().await.unwrap().as_deref(),
        Some("secret-key-1")
    );
    assert!(
        store
            .save_system_model_settings(
                Uuid::new_v4(),
                2,
                settings(CredentialPatch::Replace(String::new()))
            )
            .await
            .is_err()
    );

    // 指纹不记录密钥正文：同一 command 换一把密钥重放是幂等命中——返回既有回执、
    // 不再写库，库里的密钥保持第一次落下的那一把。
    let command_id = Uuid::new_v4();
    let recorded = store
        .save_system_model_settings(
            command_id,
            2,
            settings(CredentialPatch::Replace("secret-key-2".to_owned())),
        )
        .await
        .unwrap();
    let replayed = store
        .save_system_model_settings(
            command_id,
            2,
            settings(CredentialPatch::Replace("secret-key-3".to_owned())),
        )
        .await
        .unwrap();
    assert_eq!(replayed, recorded);
    assert_eq!(replayed.revision, 3);
    assert_eq!(
        store.system_model_api_key().await.unwrap().as_deref(),
        Some("secret-key-2")
    );

    // 「是否替换密钥」留在指纹里：同一 command 一次带密钥、一次省略是真冲突。
    let mixed = Uuid::new_v4();
    store
        .save_system_model_settings(
            mixed,
            3,
            settings(CredentialPatch::Replace("secret-key-4".to_owned())),
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .save_system_model_settings(mixed, 3, settings(CredentialPatch::Keep))
            .await,
        Err(StoreError::IdempotencyConflict)
    ));

    // 清除密钥把列置回 NULL：`credential_configured` 随之变回 false。
    let cleared = store
        .save_system_model_settings(Uuid::new_v4(), 4, settings(CredentialPatch::Clear))
        .await
        .unwrap();
    assert_eq!(cleared.revision, 5);
    assert!(!cleared.credential_configured);
    assert_eq!(store.system_model_api_key().await.unwrap(), None);
}
