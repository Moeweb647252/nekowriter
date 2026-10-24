//! Providers and model profiles (spec/04 §8, spec/10 §2.1).
//!
//! A provider row is a connection (`protocol`, `base_url`, `api_key`, `extra_headers`); a model
//! profile is one model on that connection with its own context window, output cap and
//! `ModelOptions`. Keys may be literals or `env:NAME` indirections (spec/01 §10.3); the API
//! returns them masked and resolves `env:` at call time.

use std::collections::BTreeMap;

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::api::dto::providers::{
    ModelOptionsDto, ModelProfileDto, ModelProfileInput, ProviderDto, ProviderInput,
    ProviderPresetDto, ProviderTestDto, ProviderTestErrorDto, RemoteModelDto,
};
use crate::db::entities::{model_profiles, providers};
use crate::db::json::Protocol;
use crate::error::{AppError, ErrorCode};
use crate::services::app_settings::mask_secret;
use crate::state::AppState;

/// The `mock://demo` base URL of the Demo provider (spec/04 §8).
pub const DEMO_BASE_URL: &str = "mock://demo";

/// The model id the Demo provider advertises (spec/04 §7.2).
pub const DEMO_MODEL_ID: &str = "demo-model";

// ───────────────────────────── presets ─────────────────────────────

/// A provider preset: the UI helper of spec/04 §8. Values are editable defaults, not a closed
/// list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PresetKey {
    Anthropic,
    Openai,
    Deepseek,
    Openrouter,
    Qwen,
    Moonshot,
    Zhipu,
    Ollama,
    LmStudio,
    Custom,
    Demo,
}

struct PresetDefinition {
    key: PresetKey,
    name: &'static str,
    protocol: Protocol,
    base_url: &'static str,
    /// Suggested model ids, in preference order.
    models: &'static [&'static str],
    context_window: i32,
    max_output_tokens: i32,
    options: ModelOptionsDto,
    requires_key: bool,
    demo_only: bool,
}

fn preset_definitions() -> Vec<PresetDefinition> {
    let anthropic_options = ModelOptionsDto {
        effort: Some("high".to_owned()),
        ..ModelOptionsDto::default()
    };
    let replay_options = || ModelOptionsDto {
        reasoning_replay: Some("always".to_owned()),
        ..ModelOptionsDto::default()
    };
    vec![
        PresetDefinition {
            key: PresetKey::Anthropic,
            name: "Anthropic",
            protocol: Protocol::Anthropic,
            base_url: "https://api.anthropic.com",
            models: &["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"],
            context_window: 1_000_000,
            max_output_tokens: 128_000,
            options: anthropic_options,
            requires_key: true,
            demo_only: false,
        },
        PresetDefinition {
            key: PresetKey::Openai,
            name: "OpenAI",
            protocol: Protocol::Openai,
            base_url: "https://api.openai.com/v1",
            models: &["gpt-5.2", "gpt-5.2-mini"],
            context_window: 400_000,
            max_output_tokens: 128_000,
            options: ModelOptionsDto {
                max_tokens_field: Some("max_completion_tokens".to_owned()),
                ..ModelOptionsDto::default()
            },
            requires_key: true,
            demo_only: false,
        },
        PresetDefinition {
            key: PresetKey::Deepseek,
            name: "DeepSeek",
            protocol: Protocol::Openai,
            base_url: "https://api.deepseek.com",
            models: &["deepseek-reasoner", "deepseek-chat"],
            context_window: 128_000,
            max_output_tokens: 64_000,
            options: replay_options(),
            requires_key: true,
            demo_only: false,
        },
        PresetDefinition {
            key: PresetKey::Openrouter,
            name: "OpenRouter",
            protocol: Protocol::Openai,
            base_url: "https://openrouter.ai/api/v1",
            models: &["anthropic/claude-sonnet-5.5"],
            context_window: 1_000_000,
            max_output_tokens: 64_000,
            options: replay_options(),
            requires_key: true,
            demo_only: false,
        },
        PresetDefinition {
            key: PresetKey::Qwen,
            name: "Qwen (DashScope)",
            protocol: Protocol::Openai,
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
            models: &["qwen3-max", "qwen3-coder-plus"],
            context_window: 262_144,
            max_output_tokens: 32_768,
            options: ModelOptionsDto::default(),
            requires_key: true,
            demo_only: false,
        },
        PresetDefinition {
            key: PresetKey::Moonshot,
            name: "Moonshot (Kimi)",
            protocol: Protocol::Openai,
            base_url: "https://api.moonshot.cn/v1",
            models: &["kimi-k2-turbo", "kimi-k2"],
            context_window: 262_144,
            max_output_tokens: 32_768,
            options: replay_options(),
            requires_key: true,
            demo_only: false,
        },
        PresetDefinition {
            key: PresetKey::Zhipu,
            name: "Zhipu (GLM)",
            protocol: Protocol::Openai,
            base_url: "https://open.bigmodel.cn/api/paas/v4",
            models: &["glm-5", "glm-5-air"],
            context_window: 200_000,
            max_output_tokens: 32_768,
            options: ModelOptionsDto::default(),
            requires_key: true,
            demo_only: false,
        },
        PresetDefinition {
            key: PresetKey::Ollama,
            name: "Ollama",
            protocol: Protocol::Openai,
            base_url: "http://127.0.0.1:11434/v1",
            models: &["qwen3:8b", "llama3.3:8b"],
            context_window: 32_768,
            max_output_tokens: 8_192,
            options: ModelOptionsDto {
                send_parallel_tool_calls: Some(false),
                ..ModelOptionsDto::default()
            },
            requires_key: false,
            demo_only: false,
        },
        PresetDefinition {
            key: PresetKey::LmStudio,
            name: "LM Studio",
            protocol: Protocol::Openai,
            base_url: "http://127.0.0.1:1234/v1",
            models: &[],
            context_window: 32_768,
            max_output_tokens: 8_192,
            options: ModelOptionsDto::default(),
            requires_key: false,
            demo_only: false,
        },
        PresetDefinition {
            key: PresetKey::Custom,
            name: "Custom",
            protocol: Protocol::Openai,
            base_url: "",
            models: &[],
            context_window: 128_000,
            max_output_tokens: 16_384,
            options: ModelOptionsDto::default(),
            requires_key: false,
            demo_only: false,
        },
        PresetDefinition {
            key: PresetKey::Demo,
            name: "Demo (dev only)",
            protocol: Protocol::Mock,
            base_url: DEMO_BASE_URL,
            models: &[DEMO_MODEL_ID],
            context_window: 200_000,
            max_output_tokens: 16_384,
            options: ModelOptionsDto::default(),
            requires_key: false,
            demo_only: true,
        },
    ]
}

/// `GET /api/provider-presets` (spec/04 §8). The Demo preset appears only when the mock protocol
/// is allowed (spec/01 §6).
pub fn presets(demo_enabled: bool) -> Vec<ProviderPresetDto> {
    preset_definitions()
        .into_iter()
        .filter(|preset| !preset.demo_only || demo_enabled)
        .map(|preset| ProviderPresetDto {
            key: preset.key,
            name: preset.name.to_owned(),
            protocol: preset.protocol,
            base_url: preset.base_url.to_owned(),
            suggested_models: preset.models.iter().map(|id| (*id).to_owned()).collect(),
            context_window: preset.context_window,
            max_output_tokens: preset.max_output_tokens,
            options: preset.options,
            requires_key: preset.requires_key,
        })
        .collect()
}

// ───────────────────────────── providers ─────────────────────────────

/// Rejects the `mock` protocol unless the Demo provider is allowed (spec/01 §6, D-019).
pub fn check_protocol_allowed(state: &AppState, protocol: Protocol) -> Result<(), AppError> {
    if protocol == Protocol::Mock && !state.config().demo_provider_enabled() {
        return Err(AppError::validation(
            "the demo provider is disabled; start the server with --enable-demo-provider",
        ));
    }
    Ok(())
}

/// `GET /api/providers`.
pub async fn list(conn: &DatabaseConnection) -> Result<Vec<ProviderDto>, AppError> {
    let rows = providers::Entity::find()
        .order_by_asc(providers::Column::Id)
        .all(conn)
        .await?;
    let mut dtos = Vec::with_capacity(rows.len());
    for row in rows {
        dtos.push(provider_dto(conn, row).await?);
    }
    Ok(dtos)
}

/// One provider with its model profiles.
async fn provider_dto(
    conn: &DatabaseConnection,
    row: providers::Model,
) -> Result<ProviderDto, AppError> {
    let models = model_profiles::Entity::find()
        .filter(model_profiles::Column::ProviderId.eq(row.id))
        .order_by_asc(model_profiles::Column::Id)
        .all(conn)
        .await?;
    let extra_headers: BTreeMap<String, String> =
        serde_json::from_str(&row.extra_headers).unwrap_or_default();
    Ok(ProviderDto {
        id: row.id,
        name: row.name,
        protocol: parse_protocol(&row.protocol)?,
        base_url: row.base_url,
        api_key_masked: row.api_key.as_deref().map(mask_secret),
        has_api_key: row.api_key.is_some(),
        extra_headers,
        models: models
            .into_iter()
            .map(|model| model_profile_dto(model).unwrap_or_else(|_| unreachable_dto()))
            .collect(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// A placeholder profile used when a stored `options` blob cannot be parsed; validation prevents
/// this from ever being written. It keeps the list endpoint usable.
fn unreachable_dto() -> ModelProfileDto {
    ModelProfileDto {
        id: 0,
        provider_id: 0,
        display_name: String::new(),
        model: String::new(),
        context_window: 0,
        max_output_tokens: 0,
        options: ModelOptionsDto::default(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

fn parse_protocol(value: &str) -> Result<Protocol, AppError> {
    Protocol::from_str_opt(value)
        .ok_or_else(|| AppError::internal(format!("unknown provider protocol {value}")))
}

/// `GET /api/providers/:id`.
pub async fn get(conn: &DatabaseConnection, id: i32) -> Result<ProviderDto, AppError> {
    let row = find_provider(conn, id).await?;
    provider_dto(conn, row).await
}

async fn find_provider(conn: &impl ConnectionTrait, id: i32) -> Result<providers::Model, AppError> {
    providers::Entity::find_by_id(id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("provider {id}")))
}

/// `POST /api/providers`. Validates and stores a provider row.
pub async fn create(state: &AppState, input: &ProviderInput) -> Result<ProviderDto, AppError> {
    let validated = validate_provider_input(input, true)?;
    check_protocol_allowed(state, validated.protocol)?;
    let id = crate::services::write(state.db(), {
        let validated = validated.clone();
        move |tx| {
            Box::pin(async move {
                let existing = providers::Entity::find()
                    .filter(providers::Column::Name.eq(validated.name.clone()))
                    .one(tx)
                    .await?;
                if let Some(existing) = existing {
                    return Err(duplicate_provider_error(&validated.name, existing.id));
                }
                let now = chrono::Utc::now();
                let active = providers::ActiveModel {
                    name: Set(validated.name.clone()),
                    protocol: Set(validated.protocol.as_str().to_owned()),
                    base_url: Set(validated.base_url.clone()),
                    api_key: Set(validated.api_key.clone()),
                    extra_headers: Set(serde_json::to_string(&validated.extra_headers)
                        .unwrap_or_else(|_| "{}".to_owned())),
                    created_at: Set(now),
                    updated_at: Set(now),
                    ..Default::default()
                };
                Ok(active.insert(tx).await?.id)
            })
        }
    })
    .await?;
    get(state.db().conn(), id).await
}

/// `PATCH /api/providers/:id`. An omitted `api_key` keeps the stored key; a masked value is
/// ignored so the UI can round-trip what it received (spec/10 §2.1).
pub async fn update(
    state: &AppState,
    id: i32,
    input: &ProviderInput,
) -> Result<ProviderDto, AppError> {
    let validated = validate_provider_input(input, false)?;
    check_protocol_allowed(state, validated.protocol)?;
    crate::services::write(state.db(), {
        let validated = validated.clone();
        move |tx| {
            Box::pin(async move {
                let row = find_provider(tx, id).await?;
                let duplicate = providers::Entity::find()
                    .filter(providers::Column::Name.eq(validated.name.clone()))
                    .filter(providers::Column::Id.ne(id))
                    .one(tx)
                    .await?;
                if let Some(duplicate) = duplicate {
                    return Err(duplicate_provider_error(&validated.name, duplicate.id));
                }
                let mut active: providers::ActiveModel = row.clone().into();
                active.name = Set(validated.name.clone());
                active.protocol = Set(validated.protocol.as_str().to_owned());
                active.base_url = Set(validated.base_url.clone());
                active.extra_headers = Set(serde_json::to_string(&validated.extra_headers)
                    .unwrap_or_else(|_| "{}".to_owned()));
                active.updated_at = Set(chrono::Utc::now());
                if let Some(key) = &validated.api_key {
                    active.api_key = Set(if key.is_empty() {
                        None
                    } else {
                        Some(key.clone())
                    });
                }
                active.update(tx).await?;
                Ok(())
            })
        }
    })
    .await?;
    get(state.db().conn(), id).await
}

/// `409 conflict.exists` for a duplicate provider name (spec/10 §4: the code carries
/// `details.id` of the row that holds the name, so the UI can point at it).
fn duplicate_provider_error(name: &str, id: i32) -> AppError {
    AppError::new(
        ErrorCode::ConflictExists,
        format!("a provider named {name} already exists"),
    )
    .with_details(serde_json::json!({ "id": id }))
}

/// `DELETE /api/providers/:id`. Model profiles cascade with the provider row (spec/02 §2).
///
/// A role default that pointed at one of the deleted profiles keeps its id; starting a session
/// then fails with `model.missing_role`, which is the documented code for that state. Deleting a
/// provider is allowed because an existing session must degrade to `model.missing_provider`
/// rather than block the operation (DECISIONS D-048).
pub async fn delete(state: &AppState, id: i32) -> Result<(), AppError> {
    crate::services::write(state.db(), move |tx| {
        Box::pin(async move {
            let row = find_provider(tx, id).await?;
            providers::Entity::delete_by_id(row.id).exec(tx).await?;
            Ok(())
        })
    })
    .await
}

/// A validated provider input: the API shape after the checks the service applies.
#[derive(Debug, Clone)]
struct ValidatedProvider {
    name: String,
    protocol: Protocol,
    base_url: String,
    /// `None` means "unchanged" on PATCH; `Some("")` clears the key on create and update.
    api_key: Option<String>,
    extra_headers: BTreeMap<String, String>,
}

fn validate_provider_input(
    input: &ProviderInput,
    creating: bool,
) -> Result<ValidatedProvider, AppError> {
    let name = input.name.trim().to_owned();
    if name.is_empty() {
        return Err(field_error("name", "a provider needs a name"));
    }
    let base_url = input.base_url.trim().to_owned();
    if base_url.is_empty() {
        return Err(field_error("base_url", "a provider needs a base URL"));
    }
    let is_mock = base_url.starts_with("mock://");
    if !is_mock && !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return Err(field_error(
            "base_url",
            "the base URL must start with http://, https:// or mock://",
        ));
    }
    if input.protocol == Protocol::Mock && base_url != DEMO_BASE_URL {
        return Err(field_error(
            "base_url",
            "the demo provider's base URL is mock://demo",
        ));
    }
    if is_mock && input.protocol != Protocol::Mock {
        return Err(field_error(
            "base_url",
            "mock:// is only valid for the demo (mock) protocol",
        ));
    }
    // An `env:` key must name a variable that exists: a typo would fail at call time instead of
    // at save time, which is exactly the confusing case the API should catch (spec/04 §1).
    let api_key = match input.api_key.as_deref() {
        Some(key) if crate::services::app_settings::is_masked(key) => None,
        Some("") => {
            if creating {
                None
            } else {
                Some(String::new())
            }
        }
        Some(key) => {
            if let Some(env_name) = key.strip_prefix("env:") {
                if env_name.is_empty() {
                    return Err(field_error("api_key", "env: needs a variable name"));
                }
                if std::env::var(env_name).is_err() {
                    return Err(field_error(
                        "api_key",
                        format!("the environment variable {env_name} is not set"),
                    ));
                }
            }
            Some(key.to_owned())
        }
        None => None,
    };
    Ok(ValidatedProvider {
        name,
        protocol: input.protocol,
        base_url,
        api_key,
        extra_headers: input.extra_headers.clone(),
    })
}

fn field_error(field: &str, message: impl Into<String>) -> AppError {
    AppError::invalid_fields(serde_json::Map::from_iter([(
        field.to_owned(),
        Value::String(message.into()),
    )]))
}

// ───────────────────────────── model profiles ─────────────────────────────

/// `GET /api/model-profiles`.
pub async fn list_models(conn: &DatabaseConnection) -> Result<Vec<ModelProfileDto>, AppError> {
    let rows = model_profiles::Entity::find()
        .order_by_asc(model_profiles::Column::Id)
        .all(conn)
        .await?;
    rows.into_iter().map(model_profile_dto).collect()
}

/// The DTO of one model profile row.
fn model_profile_dto(row: model_profiles::Model) -> Result<ModelProfileDto, AppError> {
    let options: ModelOptionsDto = serde_json::from_str(&row.options).unwrap_or_default();
    Ok(ModelProfileDto {
        id: row.id,
        provider_id: row.provider_id,
        display_name: row.display_name,
        model: row.model,
        context_window: row.context_window,
        max_output_tokens: row.max_output_tokens,
        options,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// `GET /api/model-profiles/:id`.
pub async fn get_model(conn: &DatabaseConnection, id: i32) -> Result<ModelProfileDto, AppError> {
    let row = model_profiles::Entity::find_by_id(id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("model profile {id}")))?;
    model_profile_dto(row)
}

/// `POST /api/model-profiles`.
pub async fn create_model(
    state: &AppState,
    input: &ModelProfileInput,
) -> Result<ModelProfileDto, AppError> {
    validate_model_input(input)?;
    let demo_enabled = state.config().demo_provider_enabled();
    let id = crate::services::write(state.db(), {
        let input = input.clone();
        move |tx| {
            Box::pin(async move {
                let provider = find_provider(tx, input.provider_id).await?;
                check_protocol_row_allowed(&provider, demo_enabled)?;
                ensure_unique_name(tx, input.provider_id, &input.display_name, None).await?;
                let now = chrono::Utc::now();
                let active = model_profiles::ActiveModel {
                    provider_id: Set(input.provider_id),
                    display_name: Set(input.display_name.trim().to_owned()),
                    model: Set(input.model.trim().to_owned()),
                    context_window: Set(input.context_window),
                    max_output_tokens: Set(input.max_output_tokens),
                    options: Set(
                        serde_json::to_string(&input.options).unwrap_or_else(|_| "{}".to_owned())
                    ),
                    created_at: Set(now),
                    updated_at: Set(now),
                    ..Default::default()
                };
                Ok(active.insert(tx).await?.id)
            })
        }
    })
    .await?;
    get_model(state.db().conn(), id).await
}

/// The demo gate for a stored provider row (spec/01 §6). Used where the row's protocol, not the
/// request, decides: creating a model on a provider needs the same rule as creating the provider.
fn check_protocol_row_allowed(
    provider: &providers::Model,
    demo_enabled: bool,
) -> Result<(), AppError> {
    if provider.protocol == Protocol::Mock.as_str() && !demo_enabled {
        return Err(AppError::validation(
            "the demo provider is disabled; start the server with --enable-demo-provider",
        ));
    }
    Ok(())
}

/// `PATCH /api/model-profiles/:id`.
pub async fn update_model(
    state: &AppState,
    id: i32,
    input: &ModelProfileInput,
) -> Result<ModelProfileDto, AppError> {
    validate_model_input(input)?;
    let demo_enabled = state.config().demo_provider_enabled();
    crate::services::write(state.db(), {
        let input = input.clone();
        move |tx| {
            Box::pin(async move {
                let provider = find_provider(tx, input.provider_id).await?;
                check_protocol_row_allowed(&provider, demo_enabled)?;
                ensure_unique_name(tx, input.provider_id, &input.display_name, Some(id)).await?;
                let row = model_profiles::Entity::find_by_id(id)
                    .one(tx)
                    .await?
                    .ok_or_else(|| AppError::not_found(format!("model profile {id}")))?;
                let mut active: model_profiles::ActiveModel = row.into();
                active.provider_id = Set(input.provider_id);
                active.display_name = Set(input.display_name.trim().to_owned());
                active.model = Set(input.model.trim().to_owned());
                active.context_window = Set(input.context_window);
                active.max_output_tokens = Set(input.max_output_tokens);
                active.options =
                    Set(serde_json::to_string(&input.options).unwrap_or_else(|_| "{}".to_owned()));
                active.updated_at = Set(chrono::Utc::now());
                active.update(tx).await?;
                Ok(())
            })
        }
    })
    .await?;
    get_model(state.db().conn(), id).await
}

/// `DELETE /api/model-profiles/:id`.
pub async fn delete_model(state: &AppState, id: i32) -> Result<(), AppError> {
    crate::services::write(state.db(), move |tx| {
        Box::pin(async move {
            let row = model_profiles::Entity::find_by_id(id)
                .one(tx)
                .await?
                .ok_or_else(|| AppError::not_found(format!("model profile {id}")))?;
            model_profiles::Entity::delete_by_id(row.id)
                .exec(tx)
                .await?;
            Ok(())
        })
    })
    .await
}

fn validate_model_input(input: &ModelProfileInput) -> Result<(), AppError> {
    if input.display_name.trim().is_empty() {
        return Err(field_error(
            "display_name",
            "a model profile needs a display name",
        ));
    }
    if input.model.trim().is_empty() {
        return Err(field_error("model", "a model profile needs a model id"));
    }
    if input.context_window <= 0 {
        return Err(field_error(
            "context_window",
            "the context window must be positive",
        ));
    }
    if input.max_output_tokens <= 0 {
        return Err(field_error(
            "max_output_tokens",
            "the maximum output must be positive",
        ));
    }
    if input.max_output_tokens > input.context_window {
        return Err(field_error(
            "max_output_tokens",
            "the maximum output cannot exceed the context window",
        ));
    }
    // The options blob has to survive a round trip through `ModelOptions` (spec/04 §5): the type
    // is the closed set the adapters switch on, so an unknown value such as `effort: "ultra"`
    // must be rejected here. Storing it would make the session-side deserialization fail and
    // silently reset *every* option of the profile, losing `reasoning_replay` and with it the
    // DeepSeek/Kimi thinking-mode replay (spec/04 §2).
    let options = serde_json::to_value(&input.options)
        .map_err(|err| field_error("options", format!("the model options are not valid: {err}")))?;
    serde_json::from_value::<nekowriter_llm::ModelOptions>(options)
        .map_err(|err| field_error("options", format!("the model options are not valid: {err}")))?;
    Ok(())
}

async fn ensure_unique_name(
    tx: &impl ConnectionTrait,
    provider_id: i32,
    display_name: &str,
    exclude: Option<i32>,
) -> Result<(), AppError> {
    let mut query = model_profiles::Entity::find()
        .filter(model_profiles::Column::ProviderId.eq(provider_id))
        .filter(model_profiles::Column::DisplayName.eq(display_name.trim()));
    if let Some(exclude) = exclude {
        query = query.filter(model_profiles::Column::Id.ne(exclude));
    }
    if let Some(existing) = query.one(tx).await? {
        return Err(AppError::new(
            ErrorCode::ConflictExists,
            format!(
                "a model named {} already exists on this provider",
                display_name.trim()
            ),
        )
        .with_details(serde_json::json!({ "id": existing.id })));
    }
    Ok(())
}

/// Counts the models of a provider (settings UI).
pub async fn model_count(conn: &DatabaseConnection, provider_id: i32) -> Result<u64, AppError> {
    let count = model_profiles::Entity::find()
        .filter(model_profiles::Column::ProviderId.eq(provider_id))
        .count(conn)
        .await?;
    Ok(count)
}

// ───────────────────────────── connection and models ─────────────────────────────

/// `POST /api/providers/:id/test` (spec/04 §8): one tiny streamed request that reports success
/// with the latency and response text, or the classified error.
pub async fn test_connection(
    state: &AppState,
    id: i32,
    model: Option<String>,
) -> Result<ProviderTestDto, AppError> {
    let provider = find_provider(state.db().conn(), id).await?;
    // The test reports on the *saved* configuration, so it uses the profile's model id and its
    // frozen `ModelOptions` (spec/04 §8, §5). Sending the defaults instead would report a false
    // failure for a correct profile — the OpenAI preset, for one, needs
    // `max_tokens_field: max_completion_tokens`, and Anthropic would get the default effort.
    let requested = model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned);
    let (model_id, options) = test_request_config(state, id, requested, &provider).await?;
    let client = client_for(&provider, state.config().demo_provider_enabled())?;
    let mut request = nekowriter_llm::ChatRequest::ping(model_id, "");
    request.options = options;
    let started = std::time::Instant::now();
    let mut text = String::new();
    let result = client
        .run(&request, &mut |event| {
            if let nekowriter_llm::StreamEvent::TextDelta { text: delta, .. } = event {
                text.push_str(&delta);
            }
        })
        .await;
    let latency_ms = started.elapsed().as_millis() as i64;
    match result {
        Ok(turn) => {
            let text = if text.is_empty() { turn.text } else { text };
            Ok(ProviderTestDto {
                ok: true,
                latency_ms,
                text: Some(text),
                error: None,
            })
        }
        Err(error) => Ok(ProviderTestDto {
            ok: false,
            latency_ms,
            text: None,
            error: Some(provider_error(error)),
        }),
    }
}

/// The model id and the `ModelOptions` a connection test uses (spec/04 §8).
///
/// The test reports on the *saved* configuration, so a named model borrows its profile's options
/// and an unnamed one borrows the provider's first profile. Sending the defaults instead would
/// report a false failure for a correct profile: the OpenAI preset needs
/// `max_tokens_field: max_completion_tokens`, and Anthropic would silently get the default effort.
/// A model without a profile is still testable — the caller may name any id — but then there are
/// no options to borrow.
async fn test_request_config(
    state: &AppState,
    provider_id: i32,
    requested: Option<String>,
    provider: &providers::Model,
) -> Result<(String, nekowriter_llm::ModelOptions), AppError> {
    let rows = model_profiles::Entity::find()
        .filter(model_profiles::Column::ProviderId.eq(provider_id))
        .order_by_asc(model_profiles::Column::Id)
        .all(state.db().conn())
        .await?;
    let borrowed = match requested.as_deref() {
        Some(model_id) => rows.iter().find(|row| row.model == model_id),
        None => rows.first(),
    };
    let options = borrowed
        .and_then(|row| serde_json::from_str(&row.options).ok())
        .unwrap_or_default();
    if let Some(model_id) = requested {
        return Ok((model_id, options));
    }
    if let Some(row) = borrowed {
        return Ok((row.model.clone(), options));
    }
    if provider.protocol == Protocol::Mock.as_str() {
        return Ok((DEMO_MODEL_ID.to_owned(), options));
    }
    Err(AppError::validation(
        "test the connection with a model id: this provider has no saved models",
    ))
}

/// `GET /api/providers/:id/remote-models` (spec/04 §8).
pub async fn remote_models(state: &AppState, id: i32) -> Result<Vec<RemoteModelDto>, AppError> {
    let provider = find_provider(state.db().conn(), id).await?;
    let client = client_for(&provider, state.config().demo_provider_enabled())?;
    let models = client.list_models().await.map_err(|error| {
        // The API status mapping of spec/10 §4: a rejected key is `provider.auth`, other
        // failures are `provider.unavailable`.
        match error {
            nekowriter_llm::LlmError::Auth(message) => {
                AppError::new(ErrorCode::ProviderAuth, message)
            }
            other => AppError::new(ErrorCode::ProviderUnavailable, other.to_string()),
        }
    })?;
    Ok(models
        .into_iter()
        .map(|model| RemoteModelDto {
            id: model.id,
            display_name: model.display_name,
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
        })
        .collect())
}

/// Classifies an LLM failure for the connection test (spec/10 §4): a rejected key is
/// `provider.auth`, a rejected request is `provider.bad_request`, everything else is
/// `provider.unavailable`. The UI localizes by `code`.
fn provider_error(error: nekowriter_llm::LlmError) -> ProviderTestErrorDto {
    use nekowriter_llm::LlmError;
    let code = match &error {
        LlmError::Auth(_) => ErrorCode::ProviderAuth,
        LlmError::BadRequest(_) | LlmError::ContextOverflow(_) => ErrorCode::ProviderBadRequest,
        _ => ErrorCode::ProviderUnavailable,
    };
    ProviderTestErrorDto {
        code,
        message: error.to_string(),
    }
}

/// Builds the provider client from a row, resolving an `env:NAME` key (spec/04 §1).
///
/// This is the funnel every *use* of a stored connection goes through (connection test, remote
/// models), so the demo gate lives here: with `--enable-demo-provider` off, a stored `mock` row
/// must not be usable either, exactly like creating one (spec/01 §6). Reading and deleting the row
/// stay allowed, so the flag never locks the author out of data a debug build created.
pub fn client_for(
    provider: &providers::Model,
    demo_enabled: bool,
) -> Result<nekowriter_llm::ProviderClient, AppError> {
    check_protocol_row_allowed(provider, demo_enabled)?;
    let config = provider_config(provider)?;
    nekowriter_llm::ProviderClient::from_config(config)
        .map_err(|error| AppError::internal(error.to_string()))
}

/// Builds the neutral provider configuration of a row (spec/04 §1). A missing environment
/// variable is a clear error instead of a request without a key.
pub fn provider_config(
    provider: &providers::Model,
) -> Result<nekowriter_llm::ProviderConfig, AppError> {
    let protocol = parse_protocol(&provider.protocol)?;
    let api_key = match provider.api_key.as_deref() {
        Some(key) if key.starts_with("env:") => {
            let name = &key["env:".len()..];
            match std::env::var(name) {
                Ok(value) if !value.is_empty() => Some(value),
                _ => {
                    return Err(AppError::new(
                        ErrorCode::ProviderAuth,
                        format!("environment variable {name} is not set"),
                    )
                    .with_details(serde_json::json!({ "env": name })));
                }
            }
        }
        Some(key) if !key.is_empty() => Some(key.to_owned()),
        _ => None,
    };
    let extra_headers: BTreeMap<String, String> =
        serde_json::from_str(&provider.extra_headers).unwrap_or_default();
    Ok(nekowriter_llm::ProviderConfig {
        protocol: match protocol {
            Protocol::Openai => nekowriter_llm::Protocol::Openai,
            Protocol::Anthropic => nekowriter_llm::Protocol::Anthropic,
            Protocol::Mock => nekowriter_llm::Protocol::Mock,
        },
        base_url: provider.base_url.clone(),
        api_key,
        extra_headers,
    })
}
