//! 知识库的 HTTP 表面与后台索引器。
//!
//! SQLite 主数据、LanceDB 向量与索引任务都属于 Store；这里只负责把 HTTP 合同
//! 翻译成 Store 调用、解析 Embedding 凭据，并驱动后台索引任务。知识不依赖任何
//! 稿本，因此这些路由是应用级的，而不是稿本级。

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, StatusCode},
};
use nekowriter_agent::{
    embedding::{
        EmbeddingBatch, EmbeddingClient, EmbeddingConfig, EmbeddingError, MAX_EMBEDDING_BATCH,
    },
    engine::{
        ACCUMULATION_UNAVAILABLE_REASON, AccumulationEngine, AccumulationRequest, EngineError,
    },
};
use nekowriter_core::{
    EmbeddingProfile, EmbeddingSettings, KnowledgeEntry, KnowledgeEntrySummary,
    KnowledgeIndexCoverage, KnowledgeSearchResult, KnowledgeSpace, decimal_i64, utc_now_millis,
};
use nekowriter_store::{
    KnowledgeSearchQuery, KnowledgeSpaceInput, NewEmbeddingSettings, NewKnowledgeEntry,
    OPERATION_KIND_EMBEDDING_CONNECTION_TEST, OPERATION_KIND_EMBEDDING_INDEX,
    OPERATION_KIND_EMBEDDING_QUERY, Store, StoreError, UpdateKnowledgeEntry,
    chunk_knowledge_content,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::warn;
use uuid::Uuid;

use crate::{
    ApiError, AppState, ensure_trusted_write_request,
    operations::{SystemOperationResult, record_system_operation},
};

/// 后台索引器在两次空轮询之间的等待时长。
const INDEX_IDLE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(750);
/// 索引失败后的退避，避免持续的数据库或凭据故障造成空转。
const INDEX_ERROR_BACKOFF: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveEmbeddingSettingsRequest {
    pub command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub expected_revision: i64,
    pub api_base_url: String,
    pub model_id: String,
    #[serde(with = "decimal_i64")]
    pub dimension: i64,
    /// 省略表示保留已有凭据。
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingSettingsResponse {
    pub embedding: Option<EmbeddingSettings>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestEmbeddingRequest {
    pub api_base_url: String,
    pub model_id: String,
    #[serde(with = "decimal_i64")]
    pub dimension: i64,
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestEmbeddingResponse {
    pub dimension: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateKnowledgeSpaceRequest {
    pub command_id: Uuid,
    pub title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameKnowledgeSpaceRequest {
    pub command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub expected_revision: i64,
    pub title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteKnowledgeSpaceRequest {
    pub command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub expected_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListKnowledgeEntriesQuery {
    pub space_id: Option<Uuid>,
    pub keyword: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateKnowledgeEntryRequest {
    pub command_id: Uuid,
    pub space_id: Uuid,
    pub title: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    pub content: String,
    pub source: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateKnowledgeEntryRequest {
    pub command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub expected_revision: i64,
    pub title: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    pub content: String,
    pub source: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteKnowledgeEntryRequest {
    pub command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub expected_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchKnowledgeRequest {
    pub query: String,
    #[serde(default)]
    pub space_ids: Vec<Uuid>,
    /// 省略时使用服务端默认值；服务端会把它夹在 1..=10。
    pub top_k: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReindexKnowledgeRequest {
    pub command_id: Uuid,
}

/// 知识积累请求。
///
/// 它是一条普通写入命令：成功时会把引擎返回的候选条目写进知识库，因此携带
/// `command_id`，重放同一条命令不会重复写入，也不会再次调用引擎（细则 §10.2）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccumulateKnowledgeRequest {
    pub command_id: Uuid,
    pub space_id: Uuid,
    /// 用户提供的积累线索。
    pub clues: String,
}

pub async fn get_embedding_settings(
    State(state): State<AppState>,
) -> Result<Json<EmbeddingSettingsResponse>, ApiError> {
    Ok(Json(EmbeddingSettingsResponse {
        embedding: state.store.get_embedding_settings().await?,
    }))
}

/// 保存 Embedding 配置。
///
/// 与写作模型设置一致：密钥正文由 Store 与其余字段在同一事务里落库，保存失败不会
/// 留下与设置不一致的密钥。
///
/// 保存前的真实验证调用必然排在准入检查之后：同一命令已有可重放结果时直接返回，
/// 修订已过期时直接报冲突，两种情况下都不会先花掉一次真实请求。
pub async fn save_embedding_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SaveEmbeddingSettingsRequest>,
) -> Result<Json<EmbeddingSettingsResponse>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let _guard = state.settings_write_lock.lock().await;
    let current = state.store.get_embedding_settings().await?;
    let SaveEmbeddingSettingsRequest {
        command_id,
        expected_revision,
        api_base_url,
        model_id,
        dimension,
        api_key,
    } = request;

    let settings = NewEmbeddingSettings {
        api_base_url,
        model_id,
        dimension,
        api_key: api_key.clone(),
    };

    // 命中已记录的结果时直接返回：既不重复写库，也不重复调用供应商。
    if let Some(replayed) = state
        .store
        .replay_embedding_settings_if_known(command_id, expected_revision, &settings)
        .await?
    {
        return Ok(Json(EmbeddingSettingsResponse {
            embedding: Some(replayed),
        }));
    }

    // 这次保存注定不会写入（修订已过期）时，不该先花掉一次真实请求。
    let actual_revision = current.as_ref().map_or(0, |settings| settings.revision);
    if actual_revision != expected_revision {
        return Err(StoreError::EmbeddingSettingsRevisionConflict {
            expected: expected_revision,
            actual: actual_revision,
        }
        .into());
    }

    // 保存前先用真实配置验证一次地址、维度与模型，避免存入不可用的配置。
    // 未重填密钥时用库里现有的那把，与写入语义保持一致。
    let api_key = match api_key {
        Some(api_key) => Some(api_key),
        None => state.store.embedding_api_key().await?,
    };
    let probe = build_client(
        &settings.api_base_url,
        &settings.model_id,
        settings.dimension,
        api_key,
    )?;
    // 保存前的验证是一次真实的 Embedding 调用，因此和连接测试一样必须落库：
    // 否则这次请求的用量会在统计里凭空消失。
    let started_at_ms = utc_now_millis();
    let outcome = probe.embed(&["连接测试".to_owned()]).await;
    record_embedding_operation(
        &state.store,
        OPERATION_KIND_EMBEDDING_CONNECTION_TEST,
        &settings.model_id,
        &settings.api_base_url,
        started_at_ms,
        utc_now_millis(),
        &outcome,
    )
    .await?;
    outcome.map_err(ApiError::Embedding)?;

    let saved = state
        .store
        .save_embedding_settings(command_id, expected_revision, settings)
        .await?;
    Ok(Json(EmbeddingSettingsResponse {
        embedding: Some(saved),
    }))
}

/// 用未保存的配置做一次真实 Embedding 调用，返回服务端实际输出维度。
pub async fn test_embedding_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<TestEmbeddingRequest>,
) -> Result<Json<TestEmbeddingResponse>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let api_key = match request.api_key {
        Some(api_key) => Some(api_key),
        None => state.store.embedding_api_key().await?,
    };
    let client = build_client(
        &request.api_base_url,
        &request.model_id,
        request.dimension,
        api_key,
    )?;
    let started_at_ms = utc_now_millis();
    let outcome = client.embed(&["连接测试".to_owned()]).await;
    record_embedding_operation(
        &state.store,
        OPERATION_KIND_EMBEDDING_CONNECTION_TEST,
        &request.model_id,
        &request.api_base_url,
        started_at_ms,
        utc_now_millis(),
        &outcome,
    )
    .await?;
    let vectors = outcome.map_err(ApiError::Embedding)?.vectors;
    let actual = vectors.first().map_or(0, Vec::len);
    Ok(Json(TestEmbeddingResponse {
        dimension: actual.to_string(),
    }))
}

pub async fn list_knowledge_spaces(
    State(state): State<AppState>,
) -> Result<Json<Vec<KnowledgeSpace>>, ApiError> {
    Ok(Json(state.store.list_knowledge_spaces().await?))
}

pub async fn create_knowledge_space(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateKnowledgeSpaceRequest>,
) -> Result<(StatusCode, Json<KnowledgeSpace>), ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let space = state
        .store
        .create_knowledge_space(
            request.command_id,
            KnowledgeSpaceInput {
                title: request.title,
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(space)))
}

pub async fn rename_knowledge_space(
    State(state): State<AppState>,
    AxumPath(space_id): AxumPath<Uuid>,
    headers: HeaderMap,
    Json(request): Json<RenameKnowledgeSpaceRequest>,
) -> Result<Json<KnowledgeSpace>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let space = state
        .store
        .rename_knowledge_space(
            space_id,
            request.command_id,
            request.expected_revision,
            KnowledgeSpaceInput {
                title: request.title,
            },
        )
        .await?;
    Ok(Json(space))
}

pub async fn delete_knowledge_space(
    State(state): State<AppState>,
    AxumPath(space_id): AxumPath<Uuid>,
    headers: HeaderMap,
    Json(request): Json<DeleteKnowledgeSpaceRequest>,
) -> Result<Json<KnowledgeSpace>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let space = state
        .store
        .delete_knowledge_space(space_id, request.command_id, request.expected_revision)
        .await?;
    Ok(Json(space))
}

pub async fn list_knowledge_entries(
    State(state): State<AppState>,
    Query(query): Query<ListKnowledgeEntriesQuery>,
) -> Result<Json<Vec<KnowledgeEntrySummary>>, ApiError> {
    Ok(Json(
        state
            .store
            .list_knowledge_entries(query.space_id, query.keyword.as_deref())
            .await?,
    ))
}

pub async fn get_knowledge_entry(
    State(state): State<AppState>,
    AxumPath(entry_id): AxumPath<Uuid>,
) -> Result<Json<KnowledgeEntry>, ApiError> {
    Ok(Json(state.store.get_knowledge_entry(entry_id).await?))
}

pub async fn create_knowledge_entry(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateKnowledgeEntryRequest>,
) -> Result<(StatusCode, Json<KnowledgeEntry>), ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let entry = state
        .store
        .create_knowledge_entry(
            request.command_id,
            NewKnowledgeEntry {
                space_id: request.space_id,
                title: request.title,
                keywords: request.keywords,
                content: request.content,
                source: request.source,
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(entry)))
}

pub async fn update_knowledge_entry(
    State(state): State<AppState>,
    AxumPath(entry_id): AxumPath<Uuid>,
    headers: HeaderMap,
    Json(request): Json<UpdateKnowledgeEntryRequest>,
) -> Result<Json<KnowledgeEntry>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let entry = state
        .store
        .update_knowledge_entry(
            entry_id,
            request.command_id,
            request.expected_revision,
            UpdateKnowledgeEntry {
                title: request.title,
                keywords: request.keywords,
                content: request.content,
                source: request.source,
            },
        )
        .await?;
    Ok(Json(entry))
}

pub async fn delete_knowledge_entry(
    State(state): State<AppState>,
    AxumPath(entry_id): AxumPath<Uuid>,
    headers: HeaderMap,
    Json(request): Json<DeleteKnowledgeEntryRequest>,
) -> Result<Json<KnowledgeEntry>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let entry = state
        .store
        .delete_knowledge_entry(entry_id, request.command_id, request.expected_revision)
        .await?;
    Ok(Json(entry))
}

/// 语义检索：先用当前 Embedding 配置算一次查询向量，再交给 Store 做补取检索。
pub async fn search_knowledge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SearchKnowledgeRequest>,
) -> Result<Json<KnowledgeSearchResult>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let arguments = parse_knowledge_query_arguments(&json!({
        "query": request.query,
        "spaceIds": request.space_ids,
        "topK": request.top_k,
    }))?;
    Ok(Json(run_knowledge_query(&state.store, arguments).await?))
}

/// 一次知识检索的类型化参数（工作台路由与写作工具共用）。
#[derive(Clone, Debug)]
pub(crate) struct KnowledgeQueryArguments {
    pub(crate) query: String,
    pub(crate) space_ids: Vec<Uuid>,
    pub(crate) top_k: usize,
}

/// 校验知识检索参数。
///
/// 模型输入只作为类型化过滤参数使用，不会被拼成数据库过滤表达式。
///
/// # Errors
///
/// 查询为空、`top_k` 不在 `1..=10`、或 Space ID 不是有效 UUID 时返回错误信息。
pub(crate) fn parse_knowledge_query_arguments(
    arguments: &Value,
) -> Result<KnowledgeQueryArguments, ApiError> {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| StoreError::InvalidKnowledgeSearch("查询不能为空".to_owned()))?
        .to_owned();
    let mut space_ids = Vec::new();
    if let Some(raw) = arguments.get("spaceIds") {
        let list = raw
            .as_array()
            .ok_or_else(|| StoreError::InvalidKnowledgeSearch("spaceIds 必须是数组".to_owned()))?;
        for value in list {
            let raw = value.as_str().ok_or_else(|| {
                StoreError::InvalidKnowledgeSearch("spaceIds 必须由字符串组成".to_owned())
            })?;
            space_ids.push(Uuid::parse_str(raw).map_err(|_| {
                StoreError::InvalidKnowledgeSearch("spaceIds 包含无效 UUID".to_owned())
            })?);
        }
    }
    let top_k = match arguments.get("topK") {
        None | Some(Value::Null) => nekowriter_store::DEFAULT_SEARCH_TOP_K,
        Some(value) => {
            let raw = value
                .as_i64()
                .ok_or_else(|| StoreError::InvalidKnowledgeSearch("topK 必须是整数".to_owned()))?;
            let max_top_k =
                i64::try_from(nekowriter_store::MAX_SEARCH_TOP_K).expect("检索上限必须能装进 i64");
            if !(1..=max_top_k).contains(&raw) {
                return Err(StoreError::InvalidKnowledgeSearch(format!(
                    "topK 必须在 1–{max_top_k} 之间"
                ))
                .into());
            }
            usize::try_from(raw)
                .map_err(|_| StoreError::InvalidKnowledgeSearch("topK 超出平台范围".to_owned()))?
        }
    };
    Ok(KnowledgeQueryArguments {
        query,
        space_ids,
        top_k,
    })
}

/// 执行 `query_knowledge` 前的前置校验：没有可用的 Embedding 就是「工具不可用」，
/// 调用方不应当扣辅助工具预算。
///
/// # Errors
///
/// 未配置 Embedding 时返回 [`StoreError::EmbeddingNotConfigured`]。
pub(crate) async fn require_knowledge_query(store: &Store) -> Result<(), ApiError> {
    match store.embedding_profile_with_id().await? {
        Some(_) => Ok(()),
        None => Err(StoreError::EmbeddingNotConfigured.into()),
    }
}

/// 用当前 Embedding 配置执行一次语义检索。
///
/// 调用方负责辅助工具预算记账：这里只做检索，不扣费。
///
/// # Errors
///
/// 未配置 Embedding、Embedding 请求失败或检索失败时返回错误。
pub(crate) async fn run_knowledge_query(
    store: &Store,
    arguments: KnowledgeQueryArguments,
) -> Result<KnowledgeSearchResult, ApiError> {
    let Some((profile, profile_id)) = store.embedding_profile_with_id().await? else {
        return Err(StoreError::EmbeddingNotConfigured.into());
    };
    let client = client_for_profile(store, &profile).await?;
    let started_at_ms = utc_now_millis();
    let outcome = client.embed(std::slice::from_ref(&arguments.query)).await;
    record_embedding_operation(
        store,
        OPERATION_KIND_EMBEDDING_QUERY,
        &profile.model_id,
        &profile.api_base_url,
        started_at_ms,
        utc_now_millis(),
        &outcome,
    )
    .await?;
    let query_vector = outcome
        .map_err(ApiError::Embedding)?
        .vectors
        .into_iter()
        .next()
        .unwrap_or_default();
    store
        .search_knowledge(KnowledgeSearchQuery {
            embedding_profile_id: profile_id,
            query_vector,
            space_ids: arguments.space_ids,
            top_k: arguments.top_k,
        })
        .await
        .map_err(ApiError::Store)
}

pub async fn reindex_knowledge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ReindexKnowledgeRequest>,
) -> Result<Json<KnowledgeIndexCoverage>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    Ok(Json(
        state
            .store
            .reindex_all_knowledge(request.command_id)
            .await?,
    ))
}

/// 知识积累入口（细则 §7.4）。
///
/// 先用应用服务校验线索与目标 Space，再取本次进程实际拥有的引擎。MVP 生产注册表
/// 严格为 0 项，因此这里总是 `ENGINE_NOT_AVAILABLE`：不创建运行任务、不扣辅助工具
/// 预算，界面据此展示「暂无可用引擎」而不是模拟进度。
pub async fn accumulate_knowledge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AccumulateKnowledgeRequest>,
) -> Result<Json<Vec<KnowledgeEntry>>, ApiError> {
    ensure_trusted_write_request(&headers, &state)?;
    let clues = request.clues.trim();
    if clues.is_empty() {
        return Err(StoreError::InvalidKnowledgeEntry("积累线索不能为空".to_owned()).into());
    }
    if !state
        .store
        .list_knowledge_spaces()
        .await?
        .iter()
        .any(|space| space.id == request.space_id)
    {
        return Err(StoreError::KnowledgeSpaceNotFound.into());
    }
    let Some(engine) = state.accumulation_engines.first() else {
        return Err(EngineError::NotAvailable {
            reason: ACCUMULATION_UNAVAILABLE_REASON.to_owned(),
        }
        .into());
    };
    Ok(Json(
        run_accumulation(
            &state.store,
            *engine,
            request.command_id,
            request.space_id,
            clues,
        )
        .await?,
    ))
}

/// 执行一次知识积累：重放检查 → 调用引擎 → 校验并写入候选条目。
///
/// 引擎不接触数据库（细则 §7.4）；候选条目在 Store 边界逐条验证，再在同一事务里
/// 插入并建立索引任务。重放同一个 `command_id` 时不再次调用引擎，直接返回上次
/// 写入的条目。
///
/// # Errors
///
/// 引擎不可用或执行失败、候选字段无效、目标 Space 不存在或数据库操作失败时返回错误。
pub(crate) async fn run_accumulation(
    store: &Store,
    engine: &dyn AccumulationEngine,
    command_id: Uuid,
    space_id: Uuid,
    clues: &str,
) -> Result<Vec<KnowledgeEntry>, ApiError> {
    if let Some(entries) = store
        .find_accumulation_result(command_id, space_id, clues)
        .await?
    {
        return Ok(entries);
    }
    let outcome = engine
        .accumulate(&AccumulationRequest {
            clues: clues.to_owned(),
            space_id,
        })
        .await
        .map_err(ApiError::Engine)?;
    let candidates = outcome
        .entries
        .into_iter()
        .map(|entry| NewKnowledgeEntry {
            space_id,
            title: entry.title,
            keywords: entry.keywords,
            content: entry.content,
            source: entry.source,
        })
        .collect();
    store
        .insert_accumulated_entries(command_id, space_id, clues, candidates)
        .await
        .map_err(ApiError::Store)
}

/// 把一次 Embedding 请求落库成系统操作。
///
/// 索引、检索与连接测试的向量请求不属于任何稿本，也不是对话请求，因此与模型连接
/// 测试共用同一条写入路径：用量带上来源与状态，没有报告时留下类型化原因，概览
/// 据此把 Embedding 输入用量单独列示。每次真实 HTTP 调用只写一行，不重复计数。
///
/// # Errors
///
/// 数据库写入失败时返回错误。
async fn record_embedding_operation(
    store: &Store,
    kind: &'static str,
    model_id: &str,
    api_base_url: &str,
    started_at_ms: i64,
    completed_at_ms: i64,
    outcome: &Result<EmbeddingBatch, EmbeddingError>,
) -> Result<(), StoreError> {
    let result = match outcome {
        Ok(batch) => SystemOperationResult::Succeeded {
            usage: batch.usage.clone(),
            provider_request_id: None,
        },
        Err(error) => SystemOperationResult::Failed {
            // 连接与超时失败无法断定请求是否已经到达供应商，因此标为未知而非失败。
            reached_provider_unknown: matches!(error, EmbeddingError::Transport { .. }),
            kind: "embedding_error",
            message: error.to_string(),
        },
    };
    record_system_operation(
        store,
        kind,
        model_id,
        api_base_url,
        started_at_ms,
        completed_at_ms,
        result,
    )
    .await?;
    Ok(())
}

/// 按当前配置解析凭据并创建 Embedding 客户端。
async fn client_for_profile(
    store: &Store,
    profile: &EmbeddingProfile,
) -> Result<EmbeddingClient, ApiError> {
    let api_key = store.embedding_api_key().await?;
    build_client(
        &profile.api_base_url,
        &profile.model_id,
        profile.dimension,
        api_key,
    )
}

fn build_client(
    api_base_url: &str,
    model_id: &str,
    dimension: i64,
    api_key: Option<String>,
) -> Result<EmbeddingClient, ApiError> {
    let config = EmbeddingConfig::new(api_base_url, model_id, dimension, api_key)
        .map_err(ApiError::Embedding)?;
    EmbeddingClient::new(config).map_err(ApiError::Embedding)
}

/// 后台索引器：领取索引任务、切分正文、计算向量并写入 `LanceDB`。
///
/// 任务状态与向量写入都属于 Store；这里只负责把「当前配置算出的向量」交给
/// Store，因此进程重启时未完成的任务可以安全地重新排队。
pub struct KnowledgeIndexer {
    store: Store,
}

/// 一次索引任务的失败原因；都会作为任务失败原因写回 SQLite。
#[derive(Debug, thiserror::Error)]
enum IndexError {
    #[error("尚未配置 Embedding，索引任务无法执行")]
    NotConfigured,
    #[error("Embedding 配置已更换，旧 profile 的索引任务不再执行")]
    ProfileChanged,
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Embedding(#[from] nekowriter_agent::embedding::EmbeddingError),
}

impl KnowledgeIndexer {
    #[must_use]
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// 持续执行索引任务，直到进程结束。
    pub async fn run(self) {
        loop {
            match self.tick().await {
                Ok(true) => {}
                Ok(false) => tokio::time::sleep(INDEX_IDLE_INTERVAL).await,
                Err(error) => {
                    warn!(error = %error, "知识索引轮询失败");
                    tokio::time::sleep(INDEX_ERROR_BACKOFF).await;
                }
            }
        }
    }

    /// 领取并执行一个任务；返回是否真的处理了一个任务。
    async fn tick(&self) -> Result<bool, StoreError> {
        let Some(task) = self.store.claim_next_index_task().await? else {
            return Ok(false);
        };
        match self.index(&task).await {
            Ok(()) => self.store.complete_index_task(task.id).await?,
            Err(error) => {
                warn!(
                    entry_id = %task.entry_id,
                    revision = task.entry_revision,
                    error = %error,
                    "知识索引任务失败"
                );
                self.store
                    .fail_index_task(task.id, error.to_string())
                    .await?;
            }
        }
        Ok(true)
    }

    async fn index(&self, task: &nekowriter_store::KnowledgeIndexTask) -> Result<(), IndexError> {
        let Some((profile, profile_id)) = self.store.embedding_profile_with_id().await? else {
            return Err(IndexError::NotConfigured);
        };
        if profile_id != task.embedding_profile_id {
            return Err(IndexError::ProfileChanged);
        }
        let entry = self.store.get_knowledge_entry(task.entry_id).await?;
        if entry.revision != task.entry_revision {
            // 领取后条目已被更新：不写入任何旧修订向量，交给 complete 判定为过期。
            return Ok(());
        }
        let client = self.embedding_client(&profile).await?;
        let chunks = chunk_knowledge_content(&entry.content);
        let mut vectors = Vec::with_capacity(chunks.len());
        for batch in chunks.chunks(MAX_EMBEDDING_BATCH) {
            // 每一批向量请求都对应一次真实 HTTP 调用，因此各自落一行用量；
            // 超长条目会被拆成多批，这里不会把它们合成一个估算值。
            let started_at_ms = utc_now_millis();
            let outcome = client.embed(batch).await;
            record_embedding_operation(
                &self.store,
                OPERATION_KIND_EMBEDDING_INDEX,
                &profile.model_id,
                &profile.api_base_url,
                started_at_ms,
                utc_now_millis(),
                &outcome,
            )
            .await
            .map_err(IndexError::Store)?;
            vectors.extend(outcome?.vectors);
        }
        self.store
            .write_knowledge_index(
                &profile,
                &profile_id,
                entry.id,
                entry.revision,
                entry.space_id,
                &chunks,
                &vectors,
            )
            .await?;
        Ok(())
    }

    async fn embedding_client(
        &self,
        profile: &EmbeddingProfile,
    ) -> Result<EmbeddingClient, IndexError> {
        let api_key = self.store.embedding_api_key().await?;
        let config = EmbeddingConfig::new(
            &profile.api_base_url,
            &profile.model_id,
            profile.dimension,
            api_key,
        )?;
        Ok(EmbeddingClient::new(config)?)
    }
}
