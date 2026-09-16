//! 系统面 handler：健康检查、能力查询、写作模型设置，以及模型连接测试。
//!
//! 从 `app/src/main.rs` 整块搬来（`docs/08` 的 P1.6），代码一行未改，只有可见性从私有放宽到
//! `pub(crate)`（`http::router` 要挂这些路由）；`system_settings_response` 与
//! `persist_connection_test` 只被本模块使用，保持私有。

use axum::{Json, extract::State};
use nekowriter_agent::{
    ContextMessageRole, capabilities,
    provider::{
        ChatCompletionsAdapter, ChatCompletionsConfig, ModelCompletion, ModelFailureCode,
        ModelRequest, ModelRequestMessage, ModelTimeouts, ProviderError,
    },
};
use nekowriter_core::utc_now_millis;
use nekowriter_store::{OPERATION_KIND_CONNECTION_TEST, StoreError};

use crate::http::dto::{
    HealthResponse, PublicSystemModelSettings, SaveSystemModelSettingsRequest,
    SystemSettingsResponse, TestModelRequest, TestModelResponse,
};
use crate::http::error::ApiError;
use crate::http::state::AppState;
use crate::model_settings::thinking_probe_level;
use crate::operations::{self, SystemOperationResult};

pub(crate) async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        schema_version: state.store.schema_version(),
        data_dir: state.store.data_dir().display().to_string(),
    })
}

pub(crate) async fn get_capabilities(
    State(state): State<AppState>,
) -> Json<nekowriter_agent::AgentCapabilities> {
    Json(capabilities(state.accumulation_engines))
}

pub(crate) async fn get_system_settings(
    State(state): State<AppState>,
) -> Result<Json<SystemSettingsResponse>, ApiError> {
    Ok(Json(system_settings_response(
        state.store.get_system_model_settings().await?,
    )))
}

pub(crate) async fn save_system_settings(
    State(state): State<AppState>,
    Json(request): Json<SaveSystemModelSettingsRequest>,
) -> Result<Json<SystemSettingsResponse>, ApiError> {
    let _guard = state.settings_write_lock.lock().await;
    let current = state.store.get_system_model_settings().await?;
    let SaveSystemModelSettingsRequest {
        command_id,
        expected_revision,
        api_base_url,
        model_id,
        capabilities,
        max_context_tokens,
        reserved_output_tokens,
        api_key,
    } = request;

    let settings = nekowriter_store::NewSystemModelSettings {
        api_base_url,
        api_key,
        model_id,
        capabilities,
        max_context_tokens,
        reserved_output_tokens,
    };

    if let Some(replayed) = state
        .store
        .replay_system_model_settings_if_known(command_id, expected_revision, settings.clone())
        .await?
    {
        return Ok(Json(system_settings_response(Some(replayed))));
    }

    let actual_revision = current.as_ref().map_or(0, |settings| settings.revision);
    if actual_revision != expected_revision {
        return Err(StoreError::SystemSettingsRevisionConflict {
            expected: expected_revision,
            actual: actual_revision,
        }
        .into());
    }

    let saved = state
        .store
        .save_system_model_settings(command_id, expected_revision, settings)
        .await?;
    Ok(Json(system_settings_response(Some(saved))))
}

fn system_settings_response(
    settings: Option<nekowriter_store::SystemModelSettings>,
) -> SystemSettingsResponse {
    SystemSettingsResponse {
        writing_model: settings.map(|settings| PublicSystemModelSettings {
            api_base_url: settings.api_base_url,
            model_id: settings.model_id,
            capabilities: settings.capabilities,
            max_context_tokens: settings.max_context_tokens.to_string(),
            reserved_output_tokens: settings.reserved_output_tokens.to_string(),
            revision: settings.revision.to_string(),
            credential_configured: settings.credential_configured,
            created_at_ms: settings.created_at_ms.to_string(),
            updated_at_ms: settings.updated_at_ms.to_string(),
        }),
    }
}

/// 实际验证一个模型是否可用，并在声明支持思考时真的带上 `reasoning_effort`。
///
/// 服务地址与凭据始终取自系统设置：连接测试测的是「同一服务上的某个模型」，
/// 不允许绕过已配置的服务。
pub(crate) async fn test_model_connection(
    State(state): State<AppState>,
    Json(request): Json<TestModelRequest>,
) -> Result<Json<TestModelResponse>, ApiError> {
    let model_id = request.model_id.trim().to_owned();
    if model_id.is_empty() {
        return Err(StoreError::InvalidSystemModelSettings("模型 ID 不能为空".to_owned()).into());
    }
    let settings_guard = state.settings_write_lock.lock().await;
    let settings = state
        .store
        .get_system_model_settings()
        .await?
        .ok_or_else(|| StoreError::InvalidSystemModelSettings("请先保存写作模型设置".to_owned()))?;
    let api_key = state.store.system_model_api_key().await?;
    let thinking_probed = request.capabilities.thinking;
    let config = ChatCompletionsConfig::new(
        &settings.api_base_url,
        model_id.clone(),
        api_key,
        request.capabilities,
        ModelTimeouts::default(),
    )?;
    let provider = ChatCompletionsAdapter::new(config)?;
    drop(settings_guard);
    let request_payload = ModelRequest {
        messages: vec![
            ModelRequestMessage::Text {
                role: ContextMessageRole::System,
                content: "你正在执行 NekoWriter 的模型连接测试。".to_owned(),
            },
            ModelRequestMessage::Text {
                role: ContextMessageRole::User,
                content: "请简短回复“连接测试成功”。不要调用工具。".to_owned(),
            },
        ],
        tools: Vec::new(),
        // 声明支持思考时用一个代表性等级真的发一次，使这个声明可被验证；
        // 否则探针保持 Off，不做超出声明的断言。
        thinking_level: thinking_probe_level(thinking_probed),
    };

    // 系统设置测试与写作 Agent 共用同一个全局许可，避免用户在测试连接时绕开
    // “全局最多一个模型网络请求”的约束。
    let _permit = state
        .model_request_permit
        .clone()
        .acquire_owned()
        .await
        .expect("AppState 持有模型调度 Semaphore，许可不会永久关闭");
    let started_at_ms = utc_now_millis();
    let outcome = provider.execute_with(&request_payload, |_| {}).await;
    let completed_at_ms = utc_now_millis();
    // 连接测试真的发出了一次模型请求，因此它和对话请求共用同一份用量口径：
    // 成功时留下供应商报告的用量，失败时留下类型化原因，两者都必须落库，
    // 否则这次请求的用量会在统计里凭空消失。
    persist_connection_test(
        &state,
        &model_id,
        &settings.api_base_url,
        started_at_ms,
        completed_at_ms,
        &outcome,
    )
    .await?;
    let completion = match outcome {
        Ok(completion) => completion,
        Err(ProviderError::HttpStatus { status, .. }) if thinking_probed && status == 400 => {
            return Err(StoreError::InvalidSystemModelSettings(format!(
                "模型 {model_id} 拒绝了 reasoning_effort（400）；该模型可能不支持思考参数"
            ))
            .into());
        }
        Err(error) => return Err(error.into()),
    };
    Ok(Json(TestModelResponse {
        provider_request_id: completion.provider_request_id,
        content_received: completion
            .content
            .as_deref()
            .is_some_and(|content| !content.trim().is_empty()),
        tool_call_count: completion.tool_calls.len(),
        usage: completion.usage,
        thinking_probed,
    }))
}

/// 把一次连接测试落库成系统模型操作。
///
/// 这类请求不属于任何稿本，因此不能写进以 Run 为主键的 `model_requests`；它们
/// 单独成表并在 Dashboard 里单列，使「刚才那次测试花了多少、成功没有」有据可查。
async fn persist_connection_test(
    state: &AppState,
    model_id: &str,
    api_base_url: &str,
    started_at_ms: i64,
    completed_at_ms: i64,
    outcome: &Result<ModelCompletion, ProviderError>,
) -> Result<(), ApiError> {
    let result = match outcome {
        Ok(completion) => SystemOperationResult::Succeeded {
            usage: completion.persisted_usage(),
            provider_request_id: completion.provider_request_id.clone(),
        },
        Err(error) => SystemOperationResult::Failed {
            // 连接与超时失败无法断定请求是否已经到达供应商，因此标为未知而非失败。
            reached_provider_unknown: matches!(
                error.failure_code(),
                ModelFailureCode::Connection | ModelFailureCode::Timeout
            ),
            kind: "provider_error",
            message: error.to_string(),
        },
    };
    operations::record_system_operation(
        &state.store,
        OPERATION_KIND_CONNECTION_TEST,
        model_id,
        api_base_url,
        started_at_ms,
        completed_at_ms,
        result,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use nekowriter_agent::engine::ACCUMULATION_ENGINES;
    use serde_json::{Value, json};
    use uuid::Uuid;

    use crate::http::tests::{accumulation_app, spawn_app};
    use crate::knowledge;

    use super::*;

    /// 密钥三态在 HTTP 上真的成立：保存 → 省略 → 清除，且响应体从不带回密钥正文。
    ///
    /// 「未配置」是被降级逻辑依赖的合法状态，因此它必须能被写回，而不只是能读出。
    #[tokio::test]
    async fn system_model_credential_can_be_cleared_over_http() {
        let (_dir, state) = accumulation_app(ACCUMULATION_ENGINES).await;
        let base = spawn_app(state.clone());
        let client = reqwest::Client::new();
        let save = |revision: &str, api_key: Option<Value>| {
            let mut body = json!({
                "commandId": Uuid::new_v4(),
                "expectedRevision": revision,
                "apiBaseUrl": "https://model.example/v1",
                "modelId": "writer-1",
                "capabilities": {
                    "streaming": true,
                    "tools": true,
                    "streamUsage": true,
                    "thinking": true,
                },
                "maxContextTokens": "32768",
                "reservedOutputTokens": "4096",
            });
            if let Some(api_key) = api_key {
                body["apiKey"] = api_key;
            }
            body
        };
        let put = |body: Value| {
            client
                .put(format!("{base}/api/settings"))
                .json(&body)
                .send()
        };

        let created = put(save("0", Some(json!("secret-key-1")))).await.unwrap();
        assert_eq!(created.status(), StatusCode::OK);
        let created_body = created.text().await.unwrap();
        assert!(created_body.contains("\"credentialConfigured\":true"));
        // 响应体只公开「是否已配置」，密钥正文绝不出现在任何对外视图里。
        assert!(!created_body.contains("secret-key-1"));

        // 省略密钥：保留既有凭据。
        let kept = put(save("1", None)).await.unwrap();
        assert_eq!(kept.status(), StatusCode::OK);
        assert!(
            kept.text()
                .await
                .unwrap()
                .contains("\"credentialConfigured\":true")
        );
        assert_eq!(
            state.store.system_model_api_key().await.unwrap().as_deref(),
            Some("secret-key-1")
        );

        // 显式 `null`：清回未配置。
        let cleared = put(save("2", Some(Value::Null))).await.unwrap();
        assert_eq!(cleared.status(), StatusCode::OK);
        assert!(
            cleared
                .text()
                .await
                .unwrap()
                .contains("\"credentialConfigured\":false")
        );
        assert_eq!(state.store.system_model_api_key().await.unwrap(), None);
        assert_eq!(
            state
                .store
                .get_system_model_settings()
                .await
                .unwrap()
                .unwrap()
                .revision,
            3
        );
    }

    /// Embedding 设置响应必须与界面 `EmbeddingSettings` 接口同形。
    #[test]
    fn embedding_settings_response_matches_the_interface_shape() {
        let empty =
            serde_json::to_value(knowledge::EmbeddingSettingsResponse { embedding: None }).unwrap();
        assert_eq!(empty, json!({ "embedding": null }));

        let response = knowledge::EmbeddingSettingsResponse {
            embedding: Some(nekowriter_core::EmbeddingSettings {
                profile: nekowriter_core::EmbeddingProfile {
                    api_base_url: "http://127.0.0.1:8899/v1".to_owned(),
                    model_id: "stub-embed".to_owned(),
                    dimension: 8,
                },
                credential_configured: true,
                revision: 2,
                updated_at_ms: 1_789_399_707_544,
            }),
        };
        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "embedding": {
                    "profile": {
                        "apiBaseUrl": "http://127.0.0.1:8899/v1",
                        "modelId": "stub-embed",
                        "dimension": "8",
                    },
                    "credentialConfigured": true,
                    "revision": "2",
                    "updatedAtMs": "1789399707544",
                },
            })
        );
    }

    /// Embedding 失败必须点名到具体原因：界面只展示 `message`，一句「请检查地址、
    /// 模型、维度与密钥」会让用户在四个字段之间猜。适配器已经知道原因（维不符、
    /// HTTP 状态、连不上），这里要求它原样透出。
    #[tokio::test]
    async fn embedding_failures_name_the_concrete_reason() {
        let response = ApiError::Embedding(
            nekowriter_agent::embedding::EmbeddingError::DimensionMismatch {
                expected: 4,
                actual: 8,
            },
        )
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], "EMBEDDING_SERVICE_FAILED");
        assert_eq!(
            value["error"]["message"],
            "Embedding 维度不符：期望 4，实际 8"
        );
        assert_eq!(
            value["error"]["details"]["reason"],
            value["error"]["message"]
        );

        // 地址与字段本身不合规仍然是客户端错误，而且同样点名是哪一项。
        let response =
            ApiError::Embedding(nekowriter_agent::embedding::EmbeddingError::EmptyModelId)
                .into_response();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], "INVALID_EMBEDDING_SETTINGS");
        assert_eq!(value["error"]["message"], "Embedding 模型 ID 不能为空");
    }
}
