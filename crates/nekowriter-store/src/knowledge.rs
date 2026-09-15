//! 知识库的持久化：Space、条目、索引任务、Embedding 配置与向量检索。
//!
//! SQLite 保存知识正文与索引任务状态；LanceDB 只保存可由正文重建的向量，按
//! Embedding profile 隔离成独立表。检索先做向量近邻，再回 SQLite 校验条目存活、
//! 当前修订与索引完成状态，过滤失效块后按需扩大候选继续补取，直到取得足够结果
//! 或该查询范围内的候选耗尽。

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, FixedSizeListArray, Float32Array, Int32Array, Int64Array, RecordBatch,
    RecordBatchIterator, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures_util::TryStreamExt;
use lancedb::Table;
use lancedb::query::{ExecutableQuery, QueryBase};
use nekowriter_core::{
    EmbeddingProfile, EmbeddingSettings, KnowledgeEntry, KnowledgeEntrySummary,
    KnowledgeIndexCoverage, KnowledgeIndexStatus, KnowledgeSearchHit, KnowledgeSearchResult,
    KnowledgeSpace, utc_now_millis,
};
use sea_orm::{
    ActiveModelTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, QueryOrder, Set,
    TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    Command, CredentialIntent, CredentialPatch, Notify, Store, StoreError, db_id,
    entity::{embedding_setting, knowledge_entry, knowledge_index_task, knowledge_space},
    insert_receipt, parse_id, payload_hash,
    preparation::insert_app_event,
    transaction_error, validate_api_base_url,
};

/// 索引任务状态：等待执行。
pub const INDEX_TASK_PENDING: &str = "pending";
/// 索引任务状态：正在执行。
pub const INDEX_TASK_RUNNING: &str = "running";
/// 索引任务状态：当前修订已可检索。
pub const INDEX_TASK_READY: &str = "ready";
/// 索引任务状态：执行失败，可显式重建。
pub const INDEX_TASK_FAILED: &str = "failed";

/// 标题长度上限（Unicode 字符）。
pub const MAX_KNOWLEDGE_TITLE_CHARS: usize = 300;
/// 单条知识正文长度上限（Unicode 字符）。
pub const MAX_KNOWLEDGE_CONTENT_CHARS: usize = 2_000_000;
/// 来源说明长度上限。
const MAX_KNOWLEDGE_SOURCE_CHARS: usize = 20_000;
/// 单条知识的关键词数量上限。
const MAX_KNOWLEDGE_KEYWORDS: usize = 50;
/// 单个关键词长度上限。
const MAX_KNOWLEDGE_KEYWORD_CHARS: usize = 100;
/// 一次检索默认与最大返回块数。
pub const DEFAULT_SEARCH_TOP_K: usize = 5;
/// 一次检索允许请求的最大块数。
pub const MAX_SEARCH_TOP_K: usize = 10;
/// 补取时允许的最大候选数量，防止失控扫描。
const MAX_SEARCH_CANDIDATES: usize = 4096;

/// 切分目标：约 600 个非空白字符。
const CHUNK_TARGET_NON_WHITESPACE_CHARS: usize = 600;
/// 切分硬上限：单个块不超过 1000 个字符。
const CHUNK_MAX_CHARS: usize = 1000;
/// 相邻块之间的重叠字符数。
const CHUNK_OVERLAP_CHARS: usize = 80;

/// 新建 Space 的输入。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSpaceInput {
    pub title: String,
}

/// 新建知识条目的输入。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewKnowledgeEntry {
    pub space_id: Uuid,
    pub title: String,
    pub keywords: Vec<String>,
    pub content: String,
    pub source: Option<String>,
}

/// 修改知识条目的输入；不含 `space_id`，条目不能在 Space 之间移动。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateKnowledgeEntry {
    pub title: String,
    pub keywords: Vec<String>,
    pub content: String,
    pub source: Option<String>,
}

/// 待保存的 Embedding 配置。
///
/// `api_key` 是三态意图（保留 / 清除 / 替换），密钥正文不进入任何持久位置；命令指纹
/// 也只记录意图，不记录密钥本身。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewEmbeddingSettings {
    pub api_base_url: String,
    pub model_id: String,
    pub dimension: i64,
    pub api_key: CredentialPatch,
}

/// 一个可被后台索引器领取的任务。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeIndexTask {
    pub id: Uuid,
    pub entry_id: Uuid,
    pub entry_revision: i64,
    pub embedding_profile_id: String,
    pub status: String,
    pub error: Option<String>,
}

/// 一次语义检索的输入。查询向量由 App 层用当前 Embedding profile 计算一次。
#[derive(Clone, Debug)]
pub struct KnowledgeSearchQuery {
    pub embedding_profile_id: String,
    pub query_vector: Vec<f32>,
    /// 空表示全部 Space。
    pub space_ids: Vec<Uuid>,
    pub top_k: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateSpacePayload<'a> {
    title: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RenameSpacePayload<'a> {
    expected_revision: i64,
    title: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteSpacePayload {
    expected_revision: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateEntryPayload<'a> {
    space_id: Uuid,
    title: &'a str,
    keywords: &'a [String],
    content: &'a str,
    source: Option<&'a str>,
}

/// 知识积累命令的协议路径；用作命令回执的目标。
const ACCUMULATE_TARGET: &str = "/api/knowledge/accumulate";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccumulatePayload<'a> {
    space_id: Uuid,
    clues: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateEntryPayload<'a> {
    expected_revision: i64,
    title: &'a str,
    keywords: &'a [String],
    content: &'a str,
    source: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteEntryPayload {
    expected_revision: i64,
}

/// 保存 Embedding 设置这条命令的指纹输入。
///
/// 只包含密钥意图而不包含密钥正文，理由与系统模型设置相同：以密钥为输入的摘要会留在
/// 库里，比密钥活得久，而且能被离线核对。
#[derive(Serialize)]
struct SaveEmbeddingSettingsPayload<'a> {
    expected_revision: i64,
    api_base_url: &'a str,
    model_id: &'a str,
    dimension: i64,
    credential: CredentialIntent,
}

/// 保存 Embedding 配置这条命令的方法与目标，写入与可重放检查必须一致。
const SAVE_EMBEDDING_SETTINGS_METHOD: &str = "PUT";
const EMBEDDING_SETTINGS_TARGET: &str = "/api/settings/embedding";

/// 一条已归一化的 Embedding 保存请求。
///
/// 命令指纹与真正落库必须用同一份归一化结果，所以这里一次算出、两处使用。
struct NormalizedEmbeddingSettings {
    api_base_url: String,
    model_id: String,
    dimension: i64,
    api_key: CredentialPatch,
}

/// 校验一次 Embedding 保存请求，并算出这条命令的指纹。
///
/// 应用必须先在真实调用供应商之前做准入检查，而准入检查只有在用同一套字段归一与
/// 指纹时才与写入一致，所以写入和它共用这里。
fn embedding_settings_payload(
    expected_revision: i64,
    settings: &NewEmbeddingSettings,
) -> Result<(NormalizedEmbeddingSettings, String), StoreError> {
    // 地址规则与写作模型设置共用一处判定，不会一侧宽、一侧严。
    let api_base_url = validate_api_base_url(&settings.api_base_url)
        .map_err(StoreError::InvalidEmbeddingSettings)?;
    let model_id = validate_required("Embedding 模型", &settings.model_id, 200)?;
    if settings.dimension <= 0 || settings.dimension > 65_536 {
        return Err(StoreError::InvalidEmbeddingSettings(
            "输出维度必须是 1 到 65536 之间的整数".to_owned(),
        ));
    }
    let dimension = settings.dimension;
    let api_key = settings
        .api_key
        .clone()
        .normalize()
        .map_err(StoreError::InvalidEmbeddingSettings)?;
    let payload_hash = payload_hash(&SaveEmbeddingSettingsPayload {
        expected_revision,
        api_base_url: &api_base_url,
        model_id: &model_id,
        dimension,
        credential: api_key.intent(),
    })?;
    Ok((
        NormalizedEmbeddingSettings {
            api_base_url,
            model_id,
            dimension,
            api_key,
        },
        payload_hash,
    ))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReindexPayload {
    command_id: Uuid,
}

/// 从 `LanceDB` 读回的一条候选块；尚未经过 SQLite 校验。
#[derive(Clone, Debug)]
struct LanceChunk {
    chunk_id: String,
    entry_id: Uuid,
    entry_revision: i64,
    space_id: Uuid,
    chunk_index: i64,
    text: String,
    score: f64,
}

impl Store {
    /// 列出全部未删除的 Space。
    ///
    /// # Errors
    ///
    /// 数据库读取失败或记录损坏时返回错误。
    pub async fn list_knowledge_spaces(&self) -> Result<Vec<KnowledgeSpace>, StoreError> {
        let spaces = knowledge_space::Entity::find()
            .filter(knowledge_space::Entity::COLUMN.deleted_at.is_null())
            .order_by_asc(knowledge_space::Entity::COLUMN.created_at)
            .order_by_asc(knowledge_space::Entity::COLUMN.id)
            .all(&self.database)
            .await?;
        let mut views = Vec::with_capacity(spaces.len());
        for space in spaces {
            views.push(self.knowledge_space_view(space).await?);
        }
        Ok(views)
    }

    /// 创建一个 Space。
    ///
    /// # Errors
    ///
    /// 标题为空或超长、`command_id` 复用于不同请求、数据库操作失败时返回错误。
    pub async fn create_knowledge_space(
        &self,
        command_id: Uuid,
        input: KnowledgeSpaceInput,
    ) -> Result<KnowledgeSpace, StoreError> {
        let title = validate_space_title(&input.title)?;
        let payload_hash = payload_hash(&CreateSpacePayload { title: &title })?;
        let target = "/api/knowledge/spaces".to_owned();
        let command = Command::new(command_id, "POST", &target, &payload_hash);
        let title_for_transaction = title.clone();
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let now = utc_now_millis();
                    let model = knowledge_space::ActiveModel {
                        id: Set(db_id(Uuid::new_v4())),
                        title: Set(title_for_transaction),
                        revision: Set(1),
                        created_at: Set(now),
                        updated_at: Set(now),
                        deleted_at: Set(None),
                    }
                    .insert(transaction)
                    .await?;
                    let response = knowledge_space_from_model(&model, 0);
                    insert_receipt(
                        transaction,
                        command_id,
                        "POST",
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&response)?,
                        201,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 重命名一个 Space。
    ///
    /// # Errors
    ///
    /// Space 不存在、修订冲突、标题无效、`command_id` 复用于不同请求或数据库操作
    /// 失败时返回错误。
    pub async fn rename_knowledge_space(
        &self,
        space_id: Uuid,
        command_id: Uuid,
        expected_revision: i64,
        input: KnowledgeSpaceInput,
    ) -> Result<KnowledgeSpace, StoreError> {
        let title = validate_space_title(&input.title)?;
        let target = format!("/api/knowledge/spaces/{space_id}");
        let payload_hash = payload_hash(&RenameSpacePayload {
            expected_revision,
            title: &title,
        })?;
        let command = Command::new(command_id, "PATCH", &target, &payload_hash);
        let title_for_transaction = title.clone();
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let now = utc_now_millis();
                    let model = active_space(transaction, space_id).await?;
                    if model.revision != expected_revision {
                        return Err(StoreError::KnowledgeSpaceRevisionConflict {
                            expected: expected_revision,
                            actual: model.revision,
                        });
                    }
                    let mut active = model.into_active_model();
                    active.title = Set(title_for_transaction.clone());
                    active.revision = Set(expected_revision + 1);
                    active.updated_at = Set(now);
                    let updated = active.update(transaction).await?;
                    let entry_count = count_entries(transaction, space_id).await?;
                    let response = knowledge_space_from_model(&updated, entry_count);
                    insert_receipt(
                        transaction,
                        command_id,
                        "PATCH",
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&response)?,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 软删除一个空的 Space。非空 Space 会被拒绝，避免隐式批量删除知识。
    ///
    /// # Errors
    ///
    /// Space 不存在、仍有条目、修订冲突、`command_id` 复用于不同请求或数据库操作
    /// 失败时返回错误。
    pub async fn delete_knowledge_space(
        &self,
        space_id: Uuid,
        command_id: Uuid,
        expected_revision: i64,
    ) -> Result<KnowledgeSpace, StoreError> {
        let target = format!("/api/knowledge/spaces/{space_id}");
        let payload_hash = payload_hash(&DeleteSpacePayload { expected_revision })?;
        let command = Command::new(command_id, "DELETE", &target, &payload_hash);
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let now = utc_now_millis();
                    let model = active_space(transaction, space_id).await?;
                    if model.revision != expected_revision {
                        return Err(StoreError::KnowledgeSpaceRevisionConflict {
                            expected: expected_revision,
                            actual: model.revision,
                        });
                    }
                    if count_entries(transaction, space_id).await? > 0 {
                        return Err(StoreError::KnowledgeSpaceNotEmpty);
                    }
                    let mut active = model.into_active_model();
                    active.deleted_at = Set(Some(now));
                    active.revision = Set(expected_revision + 1);
                    active.updated_at = Set(now);
                    let updated = active.update(transaction).await?;
                    let response = knowledge_space_from_model(&updated, 0);
                    insert_app_event(
                        transaction,
                        "recycle.changed",
                        response.id,
                        response.revision,
                        serde_json::json!({ "kind": "knowledge_space", "action": "deleted" }),
                        now,
                    )
                    .await?;
                    insert_receipt(
                        transaction,
                        command_id,
                        "DELETE",
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&response)?,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 列出知识条目摘要；可按 Space 与标题/关键词筛选。
    ///
    /// 这是独立的浏览功能，全部走 SQLite，不作为向量检索失败时的隐式替代。
    ///
    /// # Errors
    ///
    /// 数据库读取失败或记录损坏时返回错误。
    pub async fn list_knowledge_entries(
        &self,
        space_id: Option<Uuid>,
        keyword: Option<&str>,
    ) -> Result<Vec<KnowledgeEntrySummary>, StoreError> {
        let mut query = knowledge_entry::Entity::find()
            .filter(knowledge_entry::Entity::COLUMN.deleted_at.is_null())
            .order_by_desc(knowledge_entry::Entity::COLUMN.updated_at)
            .order_by_asc(knowledge_entry::Entity::COLUMN.id);
        if let Some(space_id) = space_id {
            query = query.filter(knowledge_entry::Entity::COLUMN.space_id.eq(db_id(space_id)));
        }
        let entries = query.all(&self.database).await?;
        let profile = self.embedding_profile_with_id().await?;
        let profile_id = profile.as_ref().map(|(_, id)| id.as_str());
        let tasks = current_tasks_for(&self.database, &entries, profile_id).await?;
        let keyword = keyword.map(str::trim).filter(|value| !value.is_empty());
        let mut summaries = Vec::new();
        for entry in entries {
            let parsed_keywords = parse_keywords(&entry.keywords_json)?;
            if let Some(keyword) = keyword {
                let matches = entry.title.contains(keyword)
                    || parsed_keywords.iter().any(|value| value.contains(keyword));
                if !matches {
                    continue;
                }
            }
            let task = tasks.get(&(entry.id.to_string(), entry.revision));
            let (index_status, _, _) = index_status(profile_id, entry.revision, task);
            summaries.push(KnowledgeEntrySummary {
                id: entry.id.0,
                space_id: entry.space_id.0,
                title: entry.title,
                keywords: parsed_keywords,
                revision: entry.revision,
                index_status,
                content_chars: i64::try_from(non_whitespace_chars(&entry.content))
                    .unwrap_or(i64::MAX),
                updated_at_ms: entry.updated_at,
            });
        }
        Ok(summaries)
    }

    /// 读取单条知识条目及其索引状态。
    ///
    /// # Errors
    ///
    /// 条目不存在或已删除、记录损坏、数据库读取失败时返回错误。
    pub async fn get_knowledge_entry(&self, entry_id: Uuid) -> Result<KnowledgeEntry, StoreError> {
        let entry = active_entry(&self.database, entry_id).await?;
        let profile = self.embedding_profile_with_id().await?;
        let profile_id = profile.as_ref().map(|(_, id)| id.as_str());
        let tasks =
            current_tasks_for(&self.database, std::slice::from_ref(&entry), profile_id).await?;
        knowledge_entry_view(
            &entry,
            profile_id,
            tasks.get(&(entry.id.to_string(), entry.revision)),
        )
    }

    /// 创建一条知识条目并按需建立索引任务。
    ///
    /// 未配置 Embedding 时保存照常成功，索引状态显示 `not_configured`，不发起任何
    /// 网络请求。
    ///
    /// # Errors
    ///
    /// Space 不存在、字段无效、`command_id` 复用于不同请求或数据库操作失败时返回
    /// 错误。
    pub async fn create_knowledge_entry(
        &self,
        command_id: Uuid,
        input: NewKnowledgeEntry,
    ) -> Result<KnowledgeEntry, StoreError> {
        let title = validate_entry_title(&input.title)?;
        let keywords = normalize_keywords(&input.keywords)?;
        let content = validate_entry_content(&input.content)?;
        let source = validate_source(input.source.as_deref())?;
        let target = "/api/knowledge/entries".to_owned();
        let payload_hash = payload_hash(&CreateEntryPayload {
            space_id: input.space_id,
            title: &title,
            keywords: &keywords,
            content: &content,
            source: source.as_deref(),
        })?;
        let command = Command::new(command_id, "POST", &target, &payload_hash);
        // 站点自己保留这次预查：早退省掉的是紧随其后的 `embedding_profile_with_id()`
        // 读库（它要读 `embedding_setting` 并重算 profile id）。这不是副作用，但一次已经
        // 记过账的重放没有理由再付一次读库代价。事务外的预查由此处与 `commit` 分工：
        // 命中在这里返回，未命中由 `commit` 在事务前再确认一次。
        if let Some(replayed) = command.replay_if_recorded(self).await? {
            return Ok(replayed);
        }
        let profile = self.embedding_profile_with_id().await?;
        let space_id = input.space_id;
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                let profile = profile.clone();
                Box::pin(async move {
                    let now = utc_now_millis();
                    let _ = active_space(transaction, space_id).await?;
                    let model = knowledge_entry::ActiveModel {
                        id: Set(db_id(Uuid::new_v4())),
                        space_id: Set(db_id(space_id)),
                        title: Set(title.clone()),
                        keywords_json: Set(serde_json::to_string(&keywords)?),
                        content: Set(content.clone()),
                        source_json: Set(source.clone()),
                        revision: Set(1),
                        created_at: Set(now),
                        updated_at: Set(now),
                        deleted_at: Set(None),
                    }
                    .insert(transaction)
                    .await?;
                    let task = enqueue_index_task(
                        transaction,
                        &model,
                        profile.as_ref().map(|(_, id)| id.as_str()),
                        now,
                    )
                    .await?;
                    let response = knowledge_entry_view(
                        &model,
                        profile.as_ref().map(|(_, id)| id.as_str()),
                        task.as_ref(),
                    )?;
                    insert_receipt(
                        transaction,
                        command_id,
                        "POST",
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&response)?,
                        201,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 查找一次积累命令的回执；命中且请求一致时返回上次写入的条目。
    ///
    /// 应用服务在调用引擎之前先问一次，因此重放命令不会再次执行引擎（细则 §10.2）。
    /// 这是路由层的幂等出口，职责与 `Command::commit` 的事务前预查不同：它在任何真实
    /// 工作开始之前就结束请求。
    ///
    /// # Errors
    ///
    /// `command_id` 复用于不同请求、回执损坏或数据库读取失败时返回错误。
    pub async fn find_accumulation_result(
        &self,
        command_id: Uuid,
        space_id: Uuid,
        clues: &str,
    ) -> Result<Option<Vec<KnowledgeEntry>>, StoreError> {
        let payload_hash = payload_hash(&AccumulatePayload { space_id, clues })?;
        Command::new(command_id, "POST", ACCUMULATE_TARGET, &payload_hash)
            .replay_if_recorded(self)
            .await
    }

    /// 写入一次积累命令的全部候选条目，并为每条建立索引任务。
    ///
    /// 引擎返回的候选先在这里逐条验证，再在同一事务里插入并建立索引任务：任何一条
    /// 无效都不会留下半批条目，也不会写入回执。同一 `command_id` 重放返回原结果。
    ///
    /// # Errors
    ///
    /// 候选字段无效、Space 不存在或已删除、`command_id` 复用于不同请求或数据库操作
    /// 失败时返回错误。
    pub async fn insert_accumulated_entries(
        &self,
        command_id: Uuid,
        space_id: Uuid,
        clues: &str,
        candidates: Vec<NewKnowledgeEntry>,
    ) -> Result<Vec<KnowledgeEntry>, StoreError> {
        let mut validated = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            validated.push((
                validate_entry_title(&candidate.title)?,
                normalize_keywords(&candidate.keywords)?,
                validate_entry_content(&candidate.content)?,
                validate_source(candidate.source.as_deref())?,
            ));
        }
        let payload_hash = payload_hash(&AccumulatePayload { space_id, clues })?;
        let command = Command::new(command_id, "POST", ACCUMULATE_TARGET, &payload_hash);
        // 站点自己保留这次预查：早退省掉紧随其后的 `embedding_profile_with_id()` 读库。
        if let Some(replayed) = command.replay_if_recorded(self).await? {
            return Ok(replayed);
        }
        let profile = self.embedding_profile_with_id().await?;
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                let profile_id = profile.as_ref().map(|(_, id)| id.clone());
                let validated = validated.clone();
                Box::pin(async move {
                    let now = utc_now_millis();
                    let _ = active_space(transaction, space_id).await?;
                    let mut views = Vec::with_capacity(validated.len());
                    for (title, keywords, content, source) in validated {
                        let model = knowledge_entry::ActiveModel {
                            id: Set(db_id(Uuid::new_v4())),
                            space_id: Set(db_id(space_id)),
                            title: Set(title),
                            keywords_json: Set(serde_json::to_string(&keywords)?),
                            content: Set(content),
                            source_json: Set(source),
                            revision: Set(1),
                            created_at: Set(now),
                            updated_at: Set(now),
                            deleted_at: Set(None),
                        }
                        .insert(transaction)
                        .await?;
                        let task =
                            enqueue_index_task(transaction, &model, profile_id.as_deref(), now)
                                .await?;
                        views.push(knowledge_entry_view(
                            &model,
                            profile_id.as_deref(),
                            task.as_ref(),
                        )?);
                    }
                    insert_receipt(
                        transaction,
                        command_id,
                        "POST",
                        ACCUMULATE_TARGET,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&views)?,
                        201,
                        now,
                    )
                    .await?;
                    Ok(views)
                })
            })
            .await
    }

    /// 修改一条知识条目的内容并建立新修订的索引任务。
    ///
    /// # Errors
    ///
    /// 条目不存在、修订冲突、字段无效、`command_id` 复用于不同请求或数据库操作
    /// 失败时返回错误。
    pub async fn update_knowledge_entry(
        &self,
        entry_id: Uuid,
        command_id: Uuid,
        expected_revision: i64,
        input: UpdateKnowledgeEntry,
    ) -> Result<KnowledgeEntry, StoreError> {
        let title = validate_entry_title(&input.title)?;
        let keywords = normalize_keywords(&input.keywords)?;
        let content = validate_entry_content(&input.content)?;
        let source = validate_source(input.source.as_deref())?;
        let target = format!("/api/knowledge/entries/{entry_id}");
        let payload_hash = payload_hash(&UpdateEntryPayload {
            expected_revision,
            title: &title,
            keywords: &keywords,
            content: &content,
            source: source.as_deref(),
        })?;
        let command = Command::new(command_id, "PUT", &target, &payload_hash);
        // 站点自己保留这次预查：早退省掉紧随其后的 `embedding_profile_with_id()` 读库。
        if let Some(replayed) = command.replay_if_recorded(self).await? {
            return Ok(replayed);
        }
        let profile = self.embedding_profile_with_id().await?;
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                let profile = profile.clone();
                Box::pin(async move {
                    let now = utc_now_millis();
                    let model = active_entry(transaction, entry_id).await?;
                    if model.revision != expected_revision {
                        return Err(StoreError::KnowledgeEntryRevisionConflict {
                            expected: expected_revision,
                            actual: model.revision,
                        });
                    }
                    let mut active = model.into_active_model();
                    active.title = Set(title.clone());
                    active.keywords_json = Set(serde_json::to_string(&keywords)?);
                    active.content = Set(content.clone());
                    active.source_json = Set(source.clone());
                    active.revision = Set(expected_revision + 1);
                    active.updated_at = Set(now);
                    let updated = active.update(transaction).await?;
                    let task = enqueue_index_task(
                        transaction,
                        &updated,
                        profile.as_ref().map(|(_, id)| id.as_str()),
                        now,
                    )
                    .await?;
                    let response = knowledge_entry_view(
                        &updated,
                        profile.as_ref().map(|(_, id)| id.as_str()),
                        task.as_ref(),
                    )?;
                    insert_receipt(
                        transaction,
                        command_id,
                        "PUT",
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&response)?,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 软删除一条知识条目。软删除后引用它的检索候选立即被回查过滤。
    ///
    /// # Errors
    ///
    /// 条目不存在、修订冲突、`command_id` 复用于不同请求或数据库操作失败时返回错误。
    pub async fn delete_knowledge_entry(
        &self,
        entry_id: Uuid,
        command_id: Uuid,
        expected_revision: i64,
    ) -> Result<KnowledgeEntry, StoreError> {
        let target = format!("/api/knowledge/entries/{entry_id}");
        let payload_hash = payload_hash(&DeleteEntryPayload { expected_revision })?;
        let command = Command::new(command_id, "DELETE", &target, &payload_hash);
        // 站点自己保留这次预查：早退省掉紧随其后的 `embedding_profile_with_id()` 读库。
        if let Some(replayed) = command.replay_if_recorded(self).await? {
            return Ok(replayed);
        }
        let profile = self.embedding_profile_with_id().await?;
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                let profile = profile.clone();
                Box::pin(async move {
                    let now = utc_now_millis();
                    let model = active_entry(transaction, entry_id).await?;
                    if model.revision != expected_revision {
                        return Err(StoreError::KnowledgeEntryRevisionConflict {
                            expected: expected_revision,
                            actual: model.revision,
                        });
                    }
                    let mut active = model.into_active_model();
                    active.deleted_at = Set(Some(now));
                    active.revision = Set(expected_revision + 1);
                    active.updated_at = Set(now);
                    let updated = active.update(transaction).await?;
                    // 删除后不再需要索引任务；旧向量由检索时的存活校验过滤。
                    knowledge_index_task::Entity::delete_many()
                        .filter(
                            knowledge_index_task::Entity::COLUMN
                                .entry_id
                                .eq(db_id(entry_id)),
                        )
                        .exec(transaction)
                        .await?;
                    let response = knowledge_entry_view(
                        &updated,
                        profile.as_ref().map(|(_, id)| id.as_str()),
                        None,
                    )?;
                    insert_app_event(
                        transaction,
                        "recycle.changed",
                        response.id,
                        response.revision,
                        serde_json::json!({ "kind": "knowledge_entry", "action": "deleted" }),
                        now,
                    )
                    .await?;
                    insert_receipt(
                        transaction,
                        command_id,
                        "DELETE",
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&response)?,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 读取当前 Embedding 配置。
    ///
    /// # Errors
    ///
    /// 配置记录损坏或数据库读取失败时返回错误。
    pub async fn get_embedding_settings(&self) -> Result<Option<EmbeddingSettings>, StoreError> {
        let Some(model) = self.embedding_model().await? else {
            return Ok(None);
        };
        Ok(Some(EmbeddingSettings {
            profile: EmbeddingProfile {
                api_base_url: model.api_base_url,
                model_id: model.model_id,
                dimension: model.dimension,
            },
            credential_configured: model.api_key.is_some(),
            revision: model.revision,
            updated_at_ms: model.updated_at,
        }))
    }

    /// 读取 Embedding 密钥正文；尚未配置时为 `None`。
    ///
    /// 这是 Embedding 密钥的唯一读出口：对外视图只携带
    /// `credential_configured`，不携带密钥。
    ///
    /// # Errors
    ///
    /// 数据库读取失败时返回错误。
    pub async fn embedding_api_key(&self) -> Result<Option<String>, StoreError> {
        Ok(self
            .embedding_model()
            .await?
            .and_then(|model| model.api_key))
    }

    /// 读取 Embedding 配置及其 profile 指纹。
    ///
    /// # Errors
    ///
    /// 配置记录损坏或数据库读取失败时返回错误。
    pub async fn embedding_profile_with_id(
        &self,
    ) -> Result<Option<(EmbeddingProfile, String)>, StoreError> {
        let Some(model) = self.embedding_model().await? else {
            return Ok(None);
        };
        let profile = EmbeddingProfile {
            api_base_url: model.api_base_url,
            model_id: model.model_id,
            dimension: model.dimension,
        };
        let profile_id = embedding_profile_id(&profile);
        Ok(Some((profile, profile_id)))
    }

    /// 保存 Embedding 配置。保存后，新条目会按新 profile 建立索引任务；旧 profile
    /// 的向量不会被混用。
    ///
    /// # Errors
    ///
    /// 字段无效、修订冲突、`command_id` 复用于不同请求或数据库操作失败时返回错误。
    pub async fn save_embedding_settings(
        &self,
        command_id: Uuid,
        expected_revision: i64,
        settings: NewEmbeddingSettings,
    ) -> Result<EmbeddingSettings, StoreError> {
        let (normalized, payload_hash) = embedding_settings_payload(expected_revision, &settings)?;
        let target = EMBEDDING_SETTINGS_TARGET.to_owned();
        let command = Command::new(
            command_id,
            SAVE_EMBEDDING_SETTINGS_METHOD,
            &target,
            &payload_hash,
        );
        let NormalizedEmbeddingSettings {
            api_base_url,
            model_id,
            dimension,
            api_key,
        } = normalized;
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let now = utc_now_millis();
                    let existing =
                        embedding_setting::Entity::find_by_id(EMBEDDING_SETTINGS_KEY.to_owned())
                            .one(transaction)
                            .await?;
                    let actual_revision = existing.as_ref().map_or(0, |model| model.revision);
                    if actual_revision != expected_revision {
                        return Err(StoreError::EmbeddingSettingsRevisionConflict {
                            expected: expected_revision,
                            actual: actual_revision,
                        });
                    }
                    // 并入既有密钥：`Keep` 不动它（与界面「不重填不覆盖」以及保存前那次
                    // 真实探测所用的密钥一致），`Clear` 把列置回 NULL，`Replace` 换新正文。
                    let api_key = api_key
                        .clone()
                        .merge(existing.as_ref().and_then(|model| model.api_key.clone()));
                    let model = match existing {
                        Some(model) => {
                            let mut active = model.into_active_model();
                            active.api_base_url = Set(api_base_url.clone());
                            active.model_id = Set(model_id.clone());
                            active.dimension = Set(dimension);
                            active.api_key = Set(api_key);
                            active.revision = Set(expected_revision + 1);
                            active.updated_at = Set(now);
                            active.update(transaction).await?
                        }
                        None => {
                            embedding_setting::ActiveModel {
                                key: Set(EMBEDDING_SETTINGS_KEY.to_owned()),
                                api_base_url: Set(api_base_url.clone()),
                                model_id: Set(model_id.clone()),
                                dimension: Set(dimension),
                                api_key: Set(api_key),
                                revision: Set(1),
                                created_at: Set(now),
                                updated_at: Set(now),
                            }
                            .insert(transaction)
                            .await?
                        }
                    };
                    let response = EmbeddingSettings {
                        profile: EmbeddingProfile {
                            api_base_url: model.api_base_url.clone(),
                            model_id: model.model_id.clone(),
                            dimension: model.dimension,
                        },
                        credential_configured: model.api_key.is_some(),
                        revision: model.revision,
                        updated_at_ms: model.updated_at,
                    };
                    insert_receipt(
                        transaction,
                        command_id,
                        SAVE_EMBEDDING_SETTINGS_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&response)?,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 在真的调用 Embedding 供应商之前，检查同一命令是否已有可重放的结果。
    ///
    /// 保存前的验证本身是一次真实向量请求，因此命中已记录的结果时调用方必须直接
    /// 返回它，不得再花一次请求；同一 `command_id` 复用于不同配置仍然报冲突。
    /// 这是路由层的幂等出口：它发生在向量请求之前，不是 `Command::commit` 的事务前预查。
    ///
    /// # Errors
    ///
    /// 配置无效、同一 `command_id` 复用于不同请求或读取回执失败时返回错误。
    pub async fn replay_embedding_settings_if_known(
        &self,
        command_id: Uuid,
        expected_revision: i64,
        settings: &NewEmbeddingSettings,
    ) -> Result<Option<EmbeddingSettings>, StoreError> {
        let (_, payload_hash) = embedding_settings_payload(expected_revision, settings)?;
        Command::new(
            command_id,
            SAVE_EMBEDDING_SETTINGS_METHOD,
            EMBEDDING_SETTINGS_TARGET,
            &payload_hash,
        )
        .replay_if_recorded(self)
        .await
    }

    /// 领取一个待执行的索引任务并标记为 running。
    ///
    /// # Errors
    ///
    /// 数据库操作失败时返回错误。
    pub async fn claim_next_index_task(&self) -> Result<Option<KnowledgeIndexTask>, StoreError> {
        let claimed = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    let model = knowledge_index_task::Entity::find()
                        .filter(
                            knowledge_index_task::Entity::COLUMN
                                .status
                                .eq(INDEX_TASK_PENDING),
                        )
                        .order_by_asc(knowledge_index_task::Entity::COLUMN.created_at)
                        .order_by_asc(knowledge_index_task::Entity::COLUMN.id)
                        .one(transaction)
                        .await?;
                    let Some(model) = model else {
                        return Ok(None);
                    };
                    let id = model.id.to_string();
                    let mut active = model.into_active_model();
                    active.status = Set(INDEX_TASK_RUNNING.to_owned());
                    active.updated_at = Set(utc_now_millis());
                    let updated = active.update(transaction).await?;
                    Ok(Some(KnowledgeIndexTask {
                        id: parse_id(&id, "knowledge_index_task.id")?,
                        entry_id: updated.entry_id.0,
                        entry_revision: updated.entry_revision,
                        embedding_profile_id: updated.embedding_profile_id,
                        status: updated.status,
                        error: updated.error,
                    }))
                })
            })
            .await
            .map_err(transaction_error)?;
        Ok(claimed)
    }

    /// 把 `running` 任务重新排回 `pending`，用于进程重启后的恢复。
    ///
    /// # Errors
    ///
    /// 数据库操作失败时返回错误。
    pub async fn requeue_running_index_tasks(&self) -> Result<usize, StoreError> {
        let updated = knowledge_index_task::Entity::update_many()
            .col_expr(
                knowledge_index_task::Column::Status,
                sea_orm::sea_query::Expr::value(INDEX_TASK_PENDING),
            )
            .col_expr(
                knowledge_index_task::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(utc_now_millis()),
            )
            .filter(
                knowledge_index_task::Entity::COLUMN
                    .status
                    .eq(INDEX_TASK_RUNNING),
            )
            .exec(&self.database)
            .await?;
        Ok(usize::try_from(updated.rows_affected).unwrap_or(usize::MAX))
    }

    /// 标记索引任务成功。
    ///
    /// 只有当条目仍是该修订且未被删除时，任务才会成为 `ready`；否则任务以失败
    /// 结束，避免把旧修订当成已索引。
    ///
    /// # Errors
    ///
    /// 任务不存在、数据库操作失败时返回错误。
    pub async fn complete_index_task(&self, task_id: Uuid) -> Result<(), StoreError> {
        self.finish_index_task(task_id, None).await
    }

    /// 标记索引任务失败并保存可读原因。
    ///
    /// # Errors
    ///
    /// 任务不存在、数据库操作失败时返回错误。
    pub async fn fail_index_task(
        &self,
        task_id: Uuid,
        error: impl Into<String>,
    ) -> Result<(), StoreError> {
        let message = error.into();
        self.finish_index_task(task_id, Some(message)).await
    }

    async fn finish_index_task(
        &self,
        task_id: Uuid,
        error: Option<String>,
    ) -> Result<(), StoreError> {
        self.database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let model = knowledge_index_task::Entity::find_by_id(db_id(task_id))
                        .one(transaction)
                        .await?
                        .ok_or_else(|| StoreError::CorruptRecord("索引任务不存在".to_owned()))?;
                    let now = utc_now_millis();
                    let stale = match active_entry(transaction, model.entry_id.0).await {
                        Ok(entry) => entry.revision != model.entry_revision,
                        Err(StoreError::KnowledgeEntryNotFound) => true,
                        Err(other) => return Err(other),
                    };
                    let (status, message) = if stale {
                        (
                            INDEX_TASK_FAILED,
                            Some("条目已更新或删除，本次索引结果不再适用于当前修订".to_owned()),
                        )
                    } else {
                        match error {
                            Some(message) => (INDEX_TASK_FAILED, Some(message)),
                            None => (INDEX_TASK_READY, None),
                        }
                    };
                    let entry_id = model.entry_id.0;
                    let entry_revision = model.entry_revision;
                    let mut active = model.into_active_model();
                    active.status = Set(status.to_owned());
                    active.error = Set(message);
                    active.updated_at = Set(now);
                    active.update(transaction).await?;
                    insert_app_event(
                        transaction,
                        "knowledge.index_changed",
                        entry_id,
                        entry_revision,
                        json!({
                            "entryId": entry_id.to_string(),
                            "indexStatus": status,
                        }),
                        now,
                    )
                    .await?;
                    Ok(())
                })
            })
            .await
            .map_err(transaction_error)?;
        // 索引状态变化不经过命令写入路径，必须显式唤醒 SSE 消费者，否则新状态
        // 要等到下一次无关写入才会被推送。
        self.notify_event_listeners();
        Ok(())
    }

    /// 为所有未删除条目按当前 Embedding profile 重建索引任务，返回重建后的覆盖度。
    ///
    /// # Errors
    ///
    /// 尚未配置 Embedding、`command_id` 复用于不同请求或数据库操作失败时返回错误。
    pub async fn reindex_all_knowledge(
        &self,
        command_id: Uuid,
    ) -> Result<KnowledgeIndexCoverage, StoreError> {
        let target = "/api/knowledge/reindex".to_owned();
        let payload_hash = payload_hash(&ReindexPayload { command_id })?;
        let command = Command::new(command_id, "POST", &target, &payload_hash);
        // 站点必须自己保留这次预查：早退省掉的是随后的 `embedding_profile_with_id()` 读库
        // 与整张 `knowledge_entry` 表的扫描（并把未配置嵌入的部署拦在 `EmbeddingNotConfigured`
        // 之前）。这两步在重放时是纯浪费，而且是这五处里最贵的一处。
        if let Some(replayed) = command.replay_if_recorded(self).await? {
            return Ok(replayed);
        }
        let Some((_, profile_id)) = self.embedding_profile_with_id().await? else {
            return Err(StoreError::EmbeddingNotConfigured);
        };
        let entries = knowledge_entry::Entity::find()
            .filter(knowledge_entry::Entity::COLUMN.deleted_at.is_null())
            .all(&self.database)
            .await?;
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                let profile_id = profile_id.clone();
                let entries = entries.clone();
                Box::pin(async move {
                    let now = utc_now_millis();
                    for entry in &entries {
                        reset_index_task(transaction, entry, &profile_id, now).await?;
                    }
                    let coverage = index_coverage(transaction, &[], Some(&profile_id)).await?;
                    insert_receipt(
                        transaction,
                        command_id,
                        "POST",
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&coverage)?,
                        200,
                        now,
                    )
                    .await?;
                    Ok(coverage)
                })
            })
            .await
    }

    /// 计算可检索范围内的索引覆盖度。
    ///
    /// # Errors
    ///
    /// 数据库读取失败或记录损坏时返回错误。
    pub async fn knowledge_index_coverage(
        &self,
        space_ids: &[Uuid],
    ) -> Result<KnowledgeIndexCoverage, StoreError> {
        let profile = self.embedding_profile_with_id().await?;
        index_coverage(
            &self.database,
            space_ids,
            profile.as_ref().map(|(_, id)| id.as_str()),
        )
        .await
    }

    /// 用给定的查询向量做一次语义检索。
    ///
    /// 查询向量只计算一次；多轮扩大候选属于同一次检索的全部行为。
    ///
    /// # Errors
    ///
    /// 查询向量为空（尚未配置 Embedding）、向量维度不符、向量或数据库操作失败、
    /// 或表版本固定失败时返回错误。
    pub async fn search_knowledge(
        &self,
        query: KnowledgeSearchQuery,
    ) -> Result<KnowledgeSearchResult, StoreError> {
        let top_k = query.top_k.clamp(1, MAX_SEARCH_TOP_K);
        let coverage = self.knowledge_index_coverage(&query.space_ids).await?;
        if query.query_vector.is_empty() {
            return Err(StoreError::EmbeddingNotConfigured);
        }
        let candidates = self.search_lance_candidates(&query, top_k).await?;
        let hits = self
            .validate_candidates(
                candidates,
                &query.embedding_profile_id,
                &query.space_ids,
                top_k,
            )
            .await?;
        Ok(KnowledgeSearchResult { hits, coverage })
    }

    /// 写入一个条目的向量块，并清理该条目旧修订的向量。
    ///
    /// # Errors
    ///
    /// 向量数量与块不一致、维度不符或 `LanceDB` 操作失败时返回错误。
    #[allow(clippy::too_many_arguments)] // 索引写入的全部字段都属于同一次 upsert。
    pub async fn write_knowledge_index(
        &self,
        profile: &EmbeddingProfile,
        profile_id: &str,
        entry_id: Uuid,
        entry_revision: i64,
        space_id: Uuid,
        chunks: &[String],
        vectors: &[Vec<f32>],
    ) -> Result<(), StoreError> {
        if chunks.is_empty() {
            return Ok(());
        }
        if chunks.len() != vectors.len() {
            return Err(StoreError::InvalidKnowledgeEntry(
                "向量数量与切分块数量不一致".to_owned(),
            ));
        }
        let dimension = profile.dimension;
        let expected = usize::try_from(dimension)
            .map_err(|_| StoreError::InvalidEmbeddingSettings("维度超出平台范围".to_owned()))?;
        let list_size = i32::try_from(dimension).map_err(|_| {
            StoreError::InvalidEmbeddingSettings("维度超出向量索引可表示范围".to_owned())
        })?;
        for vector in vectors {
            if vector.len() != expected {
                return Err(StoreError::EmbeddingDimensionMismatch {
                    expected: dimension,
                    actual: i64::try_from(vector.len()).unwrap_or(i64::MAX),
                });
            }
        }
        let batch = knowledge_chunks_batch(
            profile_id,
            list_size,
            entry_id,
            entry_revision,
            space_id,
            chunks,
            vectors,
        )?;
        let name = knowledge_table_name(profile_id);
        let exists = self
            .lance
            .table_names()
            .execute()
            .await?
            .iter()
            .any(|candidate| candidate == &name);
        if exists {
            let table = self.lance.open_table(&name).execute().await?;
            let reader = RecordBatchIterator::new(vec![Ok(batch)], table.schema().await?);
            let mut builder = table.merge_insert(&["chunk_id"]);
            builder
                .when_matched_update_all(None)
                .when_not_matched_insert_all();
            builder.execute(Box::new(reader)).await?;
            // 条目更新后旧修订的块不再被引用，随本次写入一并清理。
            table
                .delete(&format!(
                    "entry_id = '{entry_id}' AND entry_revision != {entry_revision}"
                ))
                .await?;
        } else {
            self.lance.create_table(&name, batch).execute().await?;
        }
        Ok(())
    }

    async fn search_lance_candidates(
        &self,
        query: &KnowledgeSearchQuery,
        top_k: usize,
    ) -> Result<Vec<LanceChunk>, StoreError> {
        let name = knowledge_table_name(&query.embedding_profile_id);
        let exists = self
            .lance
            .table_names()
            .execute()
            .await?
            .iter()
            .any(|candidate| candidate == &name);
        if !exists {
            return Ok(Vec::new());
        }
        let table = self.lance.open_table(&name).execute().await?;
        // 固定本次只读实例的表版本：补取的所有轮次看到同一批候选。
        let version = table.version().await?;
        table.checkout(version).await?;
        let mut limit = top_k.max(1);
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut candidates: Vec<LanceChunk> = Vec::new();
        loop {
            let batch = lance_search(&table, &query.query_vector, limit).await?;
            let mut returned = 0_usize;
            for chunk in batch {
                returned += 1;
                if seen.insert(chunk.chunk_id.clone()) {
                    candidates.push(chunk);
                }
            }
            if returned < limit || limit >= MAX_SEARCH_CANDIDATES {
                break;
            }
            limit = (limit * 2).min(MAX_SEARCH_CANDIDATES);
        }
        Ok(candidates)
    }

    async fn validate_candidates(
        &self,
        candidates: Vec<LanceChunk>,
        profile_id: &str,
        space_ids: &[Uuid],
        top_k: usize,
    ) -> Result<Vec<KnowledgeSearchHit>, StoreError> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let entry_ids: Vec<sea_orm::prelude::TextUuid> = candidates
            .iter()
            .map(|chunk| chunk.entry_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(db_id)
            .collect();
        let entries = knowledge_entry::Entity::find()
            .filter(knowledge_entry::Entity::COLUMN.id.is_in(entry_ids.clone()))
            .filter(knowledge_entry::Entity::COLUMN.deleted_at.is_null())
            .all(&self.database)
            .await?;
        let mut live: BTreeMap<String, (i64, String)> = BTreeMap::new();
        for entry in entries {
            live.insert(entry.id.to_string(), (entry.revision, entry.title));
        }
        // 一次查出这批条目在当前 profile 下的 ready 修订，避免逐块回查。
        let ready_tasks: BTreeSet<(String, i64)> = knowledge_index_task::Entity::find()
            .filter(
                knowledge_index_task::Entity::COLUMN
                    .entry_id
                    .is_in(entry_ids),
            )
            .filter(
                knowledge_index_task::Entity::COLUMN
                    .embedding_profile_id
                    .eq(profile_id.to_owned()),
            )
            .filter(
                knowledge_index_task::Entity::COLUMN
                    .status
                    .eq(INDEX_TASK_READY),
            )
            .all(&self.database)
            .await?
            .into_iter()
            .map(|task| (task.entry_id.to_string(), task.entry_revision))
            .collect();
        let space_filter: BTreeSet<String> = space_ids.iter().map(ToString::to_string).collect();
        let mut hits = Vec::new();
        for chunk in candidates {
            let Some((revision, title)) = live.get(&chunk.entry_id.to_string()) else {
                continue;
            };
            if *revision != chunk.entry_revision {
                continue;
            }
            if !space_filter.is_empty() && !space_filter.contains(&chunk.space_id.to_string()) {
                continue;
            }
            if !ready_tasks.contains(&(chunk.entry_id.to_string(), chunk.entry_revision)) {
                continue;
            }
            hits.push(KnowledgeSearchHit {
                chunk_id: chunk.chunk_id,
                entry_id: chunk.entry_id,
                entry_revision: chunk.entry_revision,
                space_id: chunk.space_id,
                chunk_index: chunk.chunk_index,
                title: title.clone(),
                text: chunk.text,
                score: chunk.score,
            });
        }
        hits.sort_by(compare_hits);
        hits.truncate(top_k);
        Ok(hits)
    }

    async fn embedding_model(&self) -> Result<Option<embedding_setting::Model>, StoreError> {
        Ok(
            embedding_setting::Entity::find_by_id(EMBEDDING_SETTINGS_KEY.to_owned())
                .one(&self.database)
                .await?,
        )
    }

    async fn knowledge_space_view(
        &self,
        space: knowledge_space::Model,
    ) -> Result<KnowledgeSpace, StoreError> {
        let entry_count = count_entries(&self.database, space.id.0).await?;
        Ok(knowledge_space_from_model(&space, entry_count))
    }
}

const EMBEDDING_SETTINGS_KEY: &str = "embedding";

/// 读取这批条目在当前 profile 下的索引任务，按 `(entry_id, revision)` 索引。
///
/// 只发一条 SQL，供列表、单条读取与覆盖度统计共用。
async fn current_tasks_for<C: sea_orm::ConnectionTrait>(
    connection: &C,
    entries: &[knowledge_entry::Model],
    profile_id: Option<&str>,
) -> Result<BTreeMap<(String, i64), knowledge_index_task::Model>, StoreError> {
    let Some(profile_id) = profile_id else {
        return Ok(BTreeMap::new());
    };
    if entries.is_empty() {
        return Ok(BTreeMap::new());
    }
    let entry_ids: Vec<sea_orm::prelude::TextUuid> = entries.iter().map(|entry| entry.id).collect();
    Ok(knowledge_index_task::Entity::find()
        .filter(
            knowledge_index_task::Entity::COLUMN
                .entry_id
                .is_in(entry_ids),
        )
        .filter(
            knowledge_index_task::Entity::COLUMN
                .embedding_profile_id
                .eq(profile_id.to_owned()),
        )
        .all(connection)
        .await?
        .into_iter()
        .map(|task| ((task.entry_id.to_string(), task.entry_revision), task))
        .collect())
}

/// 统计给定 Space 范围内的条目各处于哪种索引状态。
async fn index_coverage<C: sea_orm::ConnectionTrait>(
    connection: &C,
    space_ids: &[Uuid],
    profile_id: Option<&str>,
) -> Result<KnowledgeIndexCoverage, StoreError> {
    let mut query = knowledge_entry::Entity::find()
        .filter(knowledge_entry::Entity::COLUMN.deleted_at.is_null());
    if !space_ids.is_empty() {
        query = query.filter(
            knowledge_entry::Entity::COLUMN
                .space_id
                .is_in(space_ids.iter().map(|id| db_id(*id)).collect::<Vec<_>>()),
        );
    }
    let entries = query.all(connection).await?;
    let mut coverage = KnowledgeIndexCoverage::default();
    let Some(profile_id) = profile_id else {
        coverage.not_configured_entries = i64::try_from(entries.len()).unwrap_or(i64::MAX);
        return Ok(coverage);
    };
    let tasks = current_tasks_for(connection, &entries, Some(profile_id)).await?;
    for entry in &entries {
        let task = tasks.get(&(entry.id.to_string(), entry.revision));
        match index_status(Some(profile_id), entry.revision, task).0 {
            KnowledgeIndexStatus::NotConfigured => coverage.not_configured_entries += 1,
            KnowledgeIndexStatus::Pending | KnowledgeIndexStatus::Running => {
                coverage.pending_entries += 1;
            }
            KnowledgeIndexStatus::Ready => coverage.ready_entries += 1,
            KnowledgeIndexStatus::Failed => coverage.failed_entries += 1,
        }
    }
    Ok(coverage)
}

fn embedding_profile_id(profile: &EmbeddingProfile) -> String {
    let mut hasher = Sha256::new();
    hasher.update(profile.api_base_url.as_bytes());
    hasher.update(b"\n");
    hasher.update(profile.model_id.as_bytes());
    hasher.update(b"\n");
    hasher.update(profile.dimension.to_string().as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    digest[..32].to_owned()
}

fn knowledge_table_name(profile_id: &str) -> String {
    format!("knowledge_{profile_id}")
}

fn knowledge_arrow_schema(list_size: i32) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("chunk_id", DataType::Utf8, false),
        Field::new("entry_id", DataType::Utf8, false),
        Field::new("entry_revision", DataType::Int64, false),
        Field::new("space_id", DataType::Utf8, false),
        Field::new("chunk_index", DataType::Int32, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("embedding_profile_id", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                list_size,
            ),
            false,
        ),
    ]))
}

fn knowledge_chunks_batch(
    profile_id: &str,
    list_size: i32,
    entry_id: Uuid,
    entry_revision: i64,
    space_id: Uuid,
    chunks: &[String],
    vectors: &[Vec<f32>],
) -> Result<RecordBatch, StoreError> {
    let schema = knowledge_arrow_schema(list_size);
    let mut flat: Vec<f32> = Vec::with_capacity(chunks.len() * vectors[0].len());
    for vector in vectors {
        flat.extend_from_slice(vector);
    }
    let values: ArrayRef = Arc::new(Float32Array::from(flat));
    let list_field = Arc::new(Field::new("item", DataType::Float32, true));
    let list = FixedSizeListArray::try_new(list_field, list_size, values, None)
        .map_err(|error| StoreError::InvalidEmbeddingSettings(error.to_string()))?;
    let chunk_ids: Vec<String> = (0..chunks.len())
        .map(|index| format!("{entry_id}:{entry_revision}:{index}"))
        .collect();
    let chunk_indexes: Vec<i32> = (0..chunks.len())
        .map(|index| i32::try_from(index).unwrap_or(i32::MAX))
        .collect();
    let entry_id_text = entry_id.to_string();
    let space_id_text = space_id.to_string();
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(chunk_ids)),
            Arc::new(StringArray::from(vec![entry_id_text; chunks.len()])),
            Arc::new(Int64Array::from(vec![entry_revision; chunks.len()])),
            Arc::new(StringArray::from(vec![space_id_text; chunks.len()])),
            Arc::new(Int32Array::from(chunk_indexes)),
            Arc::new(StringArray::from(chunks.to_vec())),
            Arc::new(StringArray::from(vec![profile_id.to_owned(); chunks.len()])),
            Arc::new(list),
        ],
    )
    .map_err(|error| StoreError::CorruptRecord(error.to_string()))?;
    Ok(batch)
}

async fn lance_search(
    table: &Table,
    vector: &[f32],
    limit: usize,
) -> Result<Vec<LanceChunk>, StoreError> {
    let batches = table
        .query()
        .nearest_to(vector.to_vec())?
        .limit(limit)
        .execute()
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    let mut chunks = Vec::new();
    for batch in batches {
        let chunk_ids = string_column(&batch, "chunk_id")?;
        let entry_ids = string_column(&batch, "entry_id")?;
        let space_ids = string_column(&batch, "space_id")?;
        let texts = string_column(&batch, "text")?;
        let revisions = int64_column(&batch, "entry_revision")?;
        let indexes = int32_column(&batch, "chunk_index")?;
        let distances = float32_column(&batch, "_distance");
        for row in 0..batch.num_rows() {
            let distance = distances
                .and_then(|values| (!values.is_null(row)).then(|| f64::from(values.value(row))));
            chunks.push(LanceChunk {
                chunk_id: chunk_ids.value(row).to_owned(),
                entry_id: parse_id(entry_ids.value(row), "lance.entry_id")?,
                entry_revision: revisions.value(row),
                space_id: parse_id(space_ids.value(row), "lance.space_id")?,
                chunk_index: i64::from(indexes.value(row)),
                text: texts.value(row).to_owned(),
                score: distance.map_or(0.0, |value| 1.0 / (1.0 + value.max(0.0))),
            });
        }
    }
    Ok(chunks)
}

fn string_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a StringArray, StoreError> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref::<StringArray>())
        .ok_or_else(|| StoreError::CorruptRecord(format!("LanceDB 结果缺少文本列 {name}")))
}

fn int64_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a Int64Array, StoreError> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref::<Int64Array>())
        .ok_or_else(|| StoreError::CorruptRecord(format!("LanceDB 结果缺少整数列 {name}")))
}

fn int32_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a Int32Array, StoreError> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref::<Int32Array>())
        .ok_or_else(|| StoreError::CorruptRecord(format!("LanceDB 结果缺少整数列 {name}")))
}

fn float32_column<'a>(batch: &'a RecordBatch, name: &str) -> Option<&'a Float32Array> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref::<Float32Array>())
}

fn compare_hits(left: &KnowledgeSearchHit, right: &KnowledgeSearchHit) -> Ordering {
    right
        .score
        .partial_cmp(&left.score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| left.chunk_id.cmp(&right.chunk_id))
}

async fn reset_index_task(
    transaction: &sea_orm::DatabaseTransaction,
    entry: &knowledge_entry::Model,
    profile_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    let existing = knowledge_index_task::Entity::find()
        .filter(knowledge_index_task::Entity::COLUMN.entry_id.eq(entry.id))
        .filter(
            knowledge_index_task::Entity::COLUMN
                .entry_revision
                .eq(entry.revision),
        )
        .filter(
            knowledge_index_task::Entity::COLUMN
                .embedding_profile_id
                .eq(profile_id.to_owned()),
        )
        .one(transaction)
        .await?;
    match existing {
        Some(model) => {
            let mut active = model.into_active_model();
            active.status = Set(INDEX_TASK_PENDING.to_owned());
            active.error = Set(None);
            active.updated_at = Set(now);
            active.update(transaction).await?;
        }
        None => {
            knowledge_index_task::ActiveModel {
                id: Set(db_id(Uuid::new_v4())),
                entry_id: Set(entry.id),
                entry_revision: Set(entry.revision),
                embedding_profile_id: Set(profile_id.to_owned()),
                operation: Set("upsert".to_owned()),
                status: Set(INDEX_TASK_PENDING.to_owned()),
                error: Set(None),
                checkpoint_json: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(transaction)
            .await?;
        }
    }
    insert_app_event(
        transaction,
        "knowledge.index_changed",
        entry.id.0,
        entry.revision,
        json!({
            "entryId": entry.id.0.to_string(),
            "indexStatus": INDEX_TASK_PENDING,
        }),
        now,
    )
    .await?;
    Ok(())
}

async fn enqueue_index_task(
    transaction: &sea_orm::DatabaseTransaction,
    entry: &knowledge_entry::Model,
    profile_id: Option<&str>,
    now: i64,
) -> Result<Option<knowledge_index_task::Model>, StoreError> {
    let Some(profile_id) = profile_id else {
        return Ok(None);
    };
    reset_index_task(transaction, entry, profile_id, now).await?;
    Ok(knowledge_index_task::Entity::find()
        .filter(knowledge_index_task::Entity::COLUMN.entry_id.eq(entry.id))
        .filter(
            knowledge_index_task::Entity::COLUMN
                .entry_revision
                .eq(entry.revision),
        )
        .filter(
            knowledge_index_task::Entity::COLUMN
                .embedding_profile_id
                .eq(profile_id.to_owned()),
        )
        .one(transaction)
        .await?)
}

fn index_status(
    profile_id: Option<&str>,
    current_revision: i64,
    task: Option<&knowledge_index_task::Model>,
) -> (KnowledgeIndexStatus, Option<i64>, Option<String>) {
    if profile_id.is_none() {
        return (KnowledgeIndexStatus::NotConfigured, None, None);
    }
    let Some(task) = task else {
        return (KnowledgeIndexStatus::Pending, None, None);
    };
    match task.status.as_str() {
        INDEX_TASK_READY => (KnowledgeIndexStatus::Ready, Some(current_revision), None),
        INDEX_TASK_RUNNING => (KnowledgeIndexStatus::Running, None, None),
        INDEX_TASK_FAILED => (KnowledgeIndexStatus::Failed, None, task.error.clone()),
        _ => (KnowledgeIndexStatus::Pending, None, None),
    }
}

fn knowledge_entry_view(
    model: &knowledge_entry::Model,
    profile_id: Option<&str>,
    task: Option<&knowledge_index_task::Model>,
) -> Result<KnowledgeEntry, StoreError> {
    let (index_status, indexed_revision, index_error) =
        index_status(profile_id, model.revision, task);
    Ok(KnowledgeEntry {
        id: model.id.0,
        space_id: model.space_id.0,
        title: model.title.clone(),
        keywords: parse_keywords(&model.keywords_json)?,
        content: model.content.clone(),
        source: model.source_json.clone(),
        revision: model.revision,
        index_status,
        indexed_revision,
        index_error,
        created_at_ms: model.created_at,
        updated_at_ms: model.updated_at,
    })
}

fn knowledge_space_from_model(model: &knowledge_space::Model, entry_count: i64) -> KnowledgeSpace {
    KnowledgeSpace {
        id: model.id.0,
        title: model.title.clone(),
        revision: model.revision,
        entry_count,
        created_at_ms: model.created_at,
        updated_at_ms: model.updated_at,
    }
}

async fn active_space<C: sea_orm::ConnectionTrait>(
    connection: &C,
    space_id: Uuid,
) -> Result<knowledge_space::Model, StoreError> {
    knowledge_space::Entity::find_by_id(db_id(space_id))
        .filter(knowledge_space::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .ok_or(StoreError::KnowledgeSpaceNotFound)
}

async fn active_entry<C: sea_orm::ConnectionTrait>(
    connection: &C,
    entry_id: Uuid,
) -> Result<knowledge_entry::Model, StoreError> {
    knowledge_entry::Entity::find_by_id(db_id(entry_id))
        .filter(knowledge_entry::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .ok_or(StoreError::KnowledgeEntryNotFound)
}

async fn count_entries<C: sea_orm::ConnectionTrait>(
    connection: &C,
    space_id: Uuid,
) -> Result<i64, StoreError> {
    let count = knowledge_entry::Entity::find()
        .filter(knowledge_entry::Entity::COLUMN.space_id.eq(db_id(space_id)))
        .filter(knowledge_entry::Entity::COLUMN.deleted_at.is_null())
        .count(connection)
        .await?;
    Ok(i64::try_from(count).unwrap_or(i64::MAX))
}

fn parse_keywords(value: &str) -> Result<Vec<String>, StoreError> {
    serde_json::from_str(value)
        .map_err(|error| StoreError::CorruptRecord(format!("知识关键词无法解析：{error}")))
}

fn non_whitespace_chars(value: &str) -> usize {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .count()
}

fn validate_space_title(value: &str) -> Result<String, StoreError> {
    validate_required("Space 标题", value, MAX_KNOWLEDGE_TITLE_CHARS)
}

fn validate_entry_title(value: &str) -> Result<String, StoreError> {
    validate_required("知识标题", value, MAX_KNOWLEDGE_TITLE_CHARS)
}

fn validate_entry_content(value: &str) -> Result<String, StoreError> {
    validate_required("知识正文", value, MAX_KNOWLEDGE_CONTENT_CHARS)
}

fn validate_source(value: Option<&str>) -> Result<Option<String>, StoreError> {
    match value {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => Ok(Some(validate_required(
            "来源说明",
            text,
            MAX_KNOWLEDGE_SOURCE_CHARS,
        )?)),
    }
}

fn validate_required(field: &str, value: &str, max: usize) -> Result<String, StoreError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(StoreError::InvalidKnowledgeEntry(format!(
            "{field}不能为空"
        )));
    }
    if value.chars().count() > max {
        return Err(StoreError::InvalidKnowledgeEntry(format!(
            "{field}不能超过 {max} 个字符"
        )));
    }
    Ok(value.to_owned())
}

fn normalize_keywords(values: &[String]) -> Result<Vec<String>, StoreError> {
    let mut normalized = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().count() > MAX_KNOWLEDGE_KEYWORD_CHARS {
            return Err(StoreError::InvalidKnowledgeEntry(format!(
                "关键词不能超过 {MAX_KNOWLEDGE_KEYWORD_CHARS} 个字符"
            )));
        }
        if !normalized.iter().any(|existing| existing == trimmed) {
            normalized.push(trimmed.to_owned());
        }
    }
    if normalized.len() > MAX_KNOWLEDGE_KEYWORDS {
        return Err(StoreError::InvalidKnowledgeEntry(format!(
            "关键词不能超过 {MAX_KNOWLEDGE_KEYWORDS} 个"
        )));
    }
    Ok(normalized)
}

/// 把正文切分为可检索块：段落优先，目标约 600 个非空白字符，硬上限 1000 字符，
/// 相邻块约 80 字符重叠。超长段按 Unicode 字符边界拆分，不切坏多字节字符。
#[must_use]
pub fn chunk_knowledge_content(content: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0_usize;
    for line in content.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        let line_chars = line.chars().count();
        if line_chars > CHUNK_MAX_CHARS {
            flush_chunk(&mut chunks, &mut current, &mut current_chars, false);
            let chars: Vec<char> = line.chars().collect();
            let mut start = 0_usize;
            while start < chars.len() {
                let end = (start + CHUNK_MAX_CHARS).min(chars.len());
                chunks.push(chars[start..end].iter().collect());
                if end == chars.len() {
                    break;
                }
                start = end.saturating_sub(CHUNK_OVERLAP_CHARS);
            }
            continue;
        }
        let separator = usize::from(!current.is_empty());
        let projected = current_chars + separator + line_chars;
        if projected > CHUNK_MAX_CHARS
            || non_whitespace_chars(&current) >= CHUNK_TARGET_NON_WHITESPACE_CHARS
        {
            flush_chunk(&mut chunks, &mut current, &mut current_chars, true);
        }
        if !current.is_empty() {
            current.push('\n');
            current_chars += 1;
        }
        current.push_str(line);
        current_chars += line_chars;
    }
    flush_chunk(&mut chunks, &mut current, &mut current_chars, false);
    if chunks.is_empty() && !content.trim().is_empty() {
        chunks.push(content.to_owned());
    }
    chunks
}

fn flush_chunk(
    chunks: &mut Vec<String>,
    current: &mut String,
    current_chars: &mut usize,
    overlap: bool,
) {
    if current.trim().is_empty() {
        current.clear();
        *current_chars = 0;
        return;
    }
    let finished = std::mem::take(current);
    chunks.push(finished.clone());
    *current_chars = 0;
    if overlap {
        let chars: Vec<char> = finished.chars().collect();
        let start = chars.len().saturating_sub(CHUNK_OVERLAP_CHARS);
        *current = chars[start..].iter().collect();
        *current_chars = current.chars().count();
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    async fn open() -> (TempDir, Store) {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        (data_dir, store)
    }

    async fn configure_embedding(store: &Store, dimension: i64) {
        store
            .save_embedding_settings(
                Uuid::new_v4(),
                0,
                NewEmbeddingSettings {
                    api_base_url: "https://embed.example/v1".to_owned(),
                    model_id: "test-embed".to_owned(),
                    dimension,
                    api_key: CredentialPatch::Keep,
                },
            )
            .await
            .unwrap();
    }

    async fn new_space(store: &Store, title: &str) -> KnowledgeSpace {
        store
            .create_knowledge_space(
                Uuid::new_v4(),
                KnowledgeSpaceInput {
                    title: title.into(),
                },
            )
            .await
            .unwrap()
    }

    async fn new_entry(
        store: &Store,
        space_id: Uuid,
        title: &str,
        content: &str,
    ) -> KnowledgeEntry {
        store
            .create_knowledge_entry(
                Uuid::new_v4(),
                NewKnowledgeEntry {
                    space_id,
                    title: title.into(),
                    keywords: vec!["设定".into()],
                    content: content.into(),
                    source: None,
                },
            )
            .await
            .unwrap()
    }

    /// 把一条知识写成 ready 索引，模拟后台索引器完成的全部副作用。
    async fn index_entry(store: &Store, entry: &KnowledgeEntry, vector: &[f32]) {
        let Some((profile, profile_id)) = store.embedding_profile_with_id().await.unwrap() else {
            panic!("测试必须先配置 Embedding");
        };
        let chunks = chunk_knowledge_content(&entry.content);
        let vectors: Vec<Vec<f32>> = chunks.iter().map(|_| vector.to_vec()).collect();
        store
            .write_knowledge_index(
                &profile,
                &profile_id,
                entry.id,
                entry.revision,
                entry.space_id,
                &chunks,
                &vectors,
            )
            .await
            .unwrap();
        let task = store
            .claim_next_index_task()
            .await
            .unwrap()
            .expect("有待领取的索引任务");
        assert_eq!(task.entry_id, entry.id);
        store.complete_index_task(task.id).await.unwrap();
    }

    #[tokio::test]
    async fn space_and_entry_crud_enforces_revisions_and_non_empty_delete() {
        let (_dir, store) = open().await;
        let space = new_space(&store, "人物设定").await;
        assert_eq!(store.list_knowledge_spaces().await.unwrap().len(), 1);

        let conflict = store
            .rename_knowledge_space(
                space.id,
                Uuid::new_v4(),
                space.revision + 5,
                KnowledgeSpaceInput {
                    title: "新标题".into(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            conflict,
            StoreError::KnowledgeSpaceRevisionConflict { .. }
        ));

        let renamed = store
            .rename_knowledge_space(
                space.id,
                Uuid::new_v4(),
                space.revision,
                KnowledgeSpaceInput {
                    title: "新标题".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(renamed.title, "新标题");
        assert_eq!(renamed.revision, space.revision + 1);

        let entry = new_entry(&store, space.id, "林舟", "林舟是报纸记者。").await;
        assert_eq!(entry.index_status, KnowledgeIndexStatus::NotConfigured);
        assert_eq!(entry.keywords, vec!["设定".to_owned()]);

        let blocked = store
            .delete_knowledge_space(space.id, Uuid::new_v4(), renamed.revision)
            .await
            .unwrap_err();
        assert!(matches!(blocked, StoreError::KnowledgeSpaceNotEmpty));

        let listed = store
            .list_knowledge_entries(Some(space.id), Some("林舟"))
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].content_chars > 0);
        assert_eq!(
            store
                .list_knowledge_entries(Some(space.id), Some("不存在"))
                .await
                .unwrap()
                .len(),
            0
        );

        store
            .delete_knowledge_entry(entry.id, Uuid::new_v4(), entry.revision)
            .await
            .unwrap();
        assert!(matches!(
            store.get_knowledge_entry(entry.id).await.unwrap_err(),
            StoreError::KnowledgeEntryNotFound
        ));
        store
            .delete_knowledge_space(space.id, Uuid::new_v4(), renamed.revision)
            .await
            .unwrap();
        assert_eq!(store.list_knowledge_spaces().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn stale_index_task_cannot_mark_a_newer_revision_ready() {
        let (_dir, store) = open().await;
        configure_embedding(&store, 2).await;
        let space = new_space(&store, "时间线").await;
        let entry = new_entry(&store, space.id, "雨夜", "雨夜的第一版内容。").await;
        assert_eq!(entry.index_status, KnowledgeIndexStatus::Pending);

        let first = store.claim_next_index_task().await.unwrap().unwrap();
        assert_eq!(first.entry_revision, entry.revision);

        let updated = store
            .update_knowledge_entry(
                entry.id,
                Uuid::new_v4(),
                entry.revision,
                UpdateKnowledgeEntry {
                    title: entry.title.clone(),
                    keywords: entry.keywords.clone(),
                    content: "雨夜的第二版内容，已经改写。".into(),
                    source: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.revision, entry.revision + 1);
        assert_eq!(updated.index_status, KnowledgeIndexStatus::Pending);

        // 迟到的旧任务完成信号必须失败，而不是把新修订标成已索引。
        store.complete_index_task(first.id).await.unwrap();
        let reloaded = store.get_knowledge_entry(entry.id).await.unwrap();
        assert_eq!(reloaded.index_status, KnowledgeIndexStatus::Pending);
        assert!(reloaded.indexed_revision.is_none());

        let second = store.claim_next_index_task().await.unwrap().unwrap();
        assert_eq!(second.entry_revision, updated.revision);
        store
            .fail_index_task(second.id, "模拟 Embedding 失败")
            .await
            .unwrap();
        let failed = store.get_knowledge_entry(entry.id).await.unwrap();
        assert_eq!(failed.index_status, KnowledgeIndexStatus::Failed);
        assert!(failed.index_error.is_some());
    }

    #[tokio::test]
    async fn embedding_settings_replay_is_known_before_any_real_call() {
        let (_dir, store) = open().await;
        let settings = NewEmbeddingSettings {
            api_base_url: "https://embed.example/v1".to_owned(),
            model_id: "test-embed".to_owned(),
            dimension: 4,
            api_key: CredentialPatch::Keep,
        };
        let command_id = Uuid::new_v4();
        // 尚未提交过：没有可重放的结果，调用方因此照常做真实验证。
        assert!(
            store
                .replay_embedding_settings_if_known(command_id, 0, &settings)
                .await
                .unwrap()
                .is_none()
        );

        let saved = store
            .save_embedding_settings(command_id, 0, settings.clone())
            .await
            .unwrap();
        // 同一命令重放：拿到与上次逐字相同的记录，应用据此跳过真实调用。
        let replayed = store
            .replay_embedding_settings_if_known(command_id, 0, &settings)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(replayed, saved);

        // 同一命令换配置：冲突，且不写出任何东西。
        let mut other = settings.clone();
        other.model_id = "other-embed".to_owned();
        assert!(matches!(
            store
                .replay_embedding_settings_if_known(command_id, 0, &other)
                .await
                .unwrap_err(),
            StoreError::IdempotencyConflict
        ));
        assert_eq!(
            store.get_embedding_settings().await.unwrap().unwrap(),
            saved
        );
    }

    /// 省略密钥（`Keep`）保留库中现有密钥；密钥只能经由专属读出口取得。
    #[tokio::test]
    async fn embedding_api_key_is_kept_when_omitted() {
        let (_dir, store) = open().await;
        let settings = |api_key| NewEmbeddingSettings {
            api_base_url: "https://embed.example/v1".to_owned(),
            model_id: "test-embed".to_owned(),
            dimension: 4,
            api_key,
        };

        let first = store
            .save_embedding_settings(
                Uuid::new_v4(),
                0,
                settings(CredentialPatch::Replace("secret-key-1".to_owned())),
            )
            .await
            .unwrap();
        assert!(first.credential_configured);
        assert_eq!(
            store.embedding_api_key().await.unwrap().as_deref(),
            Some("secret-key-1")
        );

        let second = store
            .save_embedding_settings(Uuid::new_v4(), 1, settings(CredentialPatch::Keep))
            .await
            .unwrap();
        assert_eq!(second.revision, 2);
        assert!(second.credential_configured);
        assert_eq!(
            store.embedding_api_key().await.unwrap().as_deref(),
            Some("secret-key-1")
        );

        // 清除密钥：设置行仍在，但凭据回到「未配置」。
        let cleared = store
            .save_embedding_settings(Uuid::new_v4(), 2, settings(CredentialPatch::Clear))
            .await
            .unwrap();
        assert_eq!(cleared.revision, 3);
        assert!(!cleared.credential_configured);
        assert_eq!(store.embedding_api_key().await.unwrap(), None);
        assert_eq!(
            store.get_embedding_settings().await.unwrap().unwrap(),
            cleared
        );
    }

    #[tokio::test]
    async fn embedding_settings_reject_invalid_input_and_guard_revisions() {
        let (_dir, store) = open().await;
        assert!(store.get_embedding_settings().await.unwrap().is_none());

        let invalid = store
            .save_embedding_settings(
                Uuid::new_v4(),
                0,
                NewEmbeddingSettings {
                    api_base_url: "ftp://embed.example/v1".to_owned(),
                    model_id: "test-embed".to_owned(),
                    dimension: 8,
                    api_key: CredentialPatch::Keep,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(invalid, StoreError::InvalidEmbeddingSettings(_)));

        configure_embedding(&store, 4).await;
        let saved = store.get_embedding_settings().await.unwrap().unwrap();
        assert_eq!(saved.profile.dimension, 4);
        assert_eq!(saved.revision, 1);

        let conflict = store
            .save_embedding_settings(
                Uuid::new_v4(),
                7,
                NewEmbeddingSettings {
                    api_base_url: "https://embed.example/v1".to_owned(),
                    model_id: "other-embed".to_owned(),
                    dimension: 4,
                    api_key: CredentialPatch::Keep,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            conflict,
            StoreError::EmbeddingSettingsRevisionConflict { .. }
        ));
        assert_eq!(
            store
                .get_embedding_settings()
                .await
                .unwrap()
                .unwrap()
                .revision,
            1
        );
    }

    #[tokio::test]
    async fn semantic_search_filters_stale_revisions_and_refills_candidates() {
        let (_dir, store) = open().await;
        configure_embedding(&store, 2).await;
        let space = new_space(&store, "地点").await;
        let alpha = new_entry(&store, space.id, "甲", "甲号地点的完整设定。").await;
        let beta = new_entry(&store, space.id, "乙", "乙号地点的完整设定。").await;
        let gamma = new_entry(&store, space.id, "丙", "丙号地点的完整设定。").await;
        index_entry(&store, &alpha, &[1.0, 0.0]).await;
        index_entry(&store, &beta, &[0.0, 1.0]).await;
        index_entry(&store, &gamma, &[0.7, 0.7]).await;

        let profile_id = store.embedding_profile_with_id().await.unwrap().unwrap().1;
        let query = |space_ids: Vec<Uuid>| KnowledgeSearchQuery {
            embedding_profile_id: profile_id.clone(),
            query_vector: vec![1.0, 0.0],
            space_ids,
            top_k: 2,
        };
        let first = store.search_knowledge(query(Vec::new())).await.unwrap();
        assert_eq!(first.hits.len(), 2);
        assert_eq!(first.hits[0].title, "甲");
        assert_eq!(first.coverage.ready_entries, 3);

        // 更新甲后，旧修订的向量必须在回查里被剔除，并由补取补回其它条目。
        store
            .update_knowledge_entry(
                alpha.id,
                Uuid::new_v4(),
                alpha.revision,
                UpdateKnowledgeEntry {
                    title: alpha.title.clone(),
                    keywords: alpha.keywords.clone(),
                    content: "甲号地点已经改名并改写设定。".into(),
                    source: None,
                },
            )
            .await
            .unwrap();
        let refilled = store.search_knowledge(query(Vec::new())).await.unwrap();
        assert_eq!(refilled.hits.len(), 2);
        assert!(refilled.hits.iter().all(|hit| hit.title != "甲"));
        assert_eq!(refilled.coverage.ready_entries, 2);
        assert_eq!(refilled.coverage.pending_entries, 1);

        // Space 过滤是类型化参数，不匹配的 Space 不会返回任何候选。
        let other = new_space(&store, "其它").await;
        assert_eq!(
            store
                .search_knowledge(query(vec![other.id]))
                .await
                .unwrap()
                .hits
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn search_requires_an_embedding_profile() {
        let (_dir, store) = open().await;
        let error = store
            .search_knowledge(KnowledgeSearchQuery {
                embedding_profile_id: "unknown".to_owned(),
                query_vector: Vec::new(),
                space_ids: Vec::new(),
                top_k: DEFAULT_SEARCH_TOP_K,
            })
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::EmbeddingNotConfigured));
    }

    #[tokio::test]
    async fn reindex_records_a_receipt_instead_of_recomputing() {
        let (_dir, store) = open().await;
        let space = new_space(&store, "索引").await;
        let entry = new_entry(&store, space.id, "条目", "需要重建索引的内容。").await;
        assert!(matches!(
            store
                .reindex_all_knowledge(Uuid::new_v4())
                .await
                .unwrap_err(),
            StoreError::EmbeddingNotConfigured
        ));

        configure_embedding(&store, 2).await;
        let command_id = Uuid::new_v4();
        let first = store.reindex_all_knowledge(command_id).await.unwrap();
        assert_eq!(first.pending_entries, 1);
        assert_eq!(first.ready_entries, 0);

        let task = store.claim_next_index_task().await.unwrap().unwrap();
        assert_eq!(task.entry_id, entry.id);
        store.complete_index_task(task.id).await.unwrap();
        assert_eq!(
            store
                .knowledge_index_coverage(&[])
                .await
                .unwrap()
                .ready_entries,
            1
        );

        // 同一 command_id 返回当时的结果，而不是重新计算当前覆盖度。
        let replayed = store.reindex_all_knowledge(command_id).await.unwrap();
        assert_eq!(replayed.pending_entries, 1);
        assert_eq!(replayed.ready_entries, 0);
    }

    fn candidate(space_id: Uuid, title: &str, content: &str) -> NewKnowledgeEntry {
        NewKnowledgeEntry {
            space_id,
            title: title.to_owned(),
            keywords: vec!["积累".to_owned()],
            content: content.to_owned(),
            source: Some("引擎".to_owned()),
        }
    }

    #[tokio::test]
    async fn accumulation_writes_every_candidate_in_one_command_and_replays() {
        let (_dir, store) = open().await;
        configure_embedding(&store, 2).await;
        let space = new_space(&store, "积累").await;
        let command_id = Uuid::new_v4();
        assert!(
            store
                .find_accumulation_result(command_id, space.id, "雨夜灯塔")
                .await
                .unwrap()
                .is_none()
        );

        let created = store
            .insert_accumulated_entries(
                command_id,
                space.id,
                "雨夜灯塔",
                vec![
                    candidate(space.id, "灯塔线索", "灯塔在雨夜里反复出现。"),
                    candidate(space.id, "码头线索", "码头在第三章被烧毁。"),
                ],
            )
            .await
            .unwrap();
        assert_eq!(created.len(), 2);
        assert!(
            created
                .iter()
                .all(|entry| entry.index_status == KnowledgeIndexStatus::Pending)
        );
        assert_eq!(
            store
                .list_knowledge_entries(Some(space.id), None)
                .await
                .unwrap()
                .len(),
            2
        );
        // 每条候选都有自己的索引任务，且指向刚写入的那一修订。
        let mut indexed: Vec<(Uuid, i64)> = Vec::new();
        while let Some(task) = store.claim_next_index_task().await.unwrap() {
            indexed.push((task.entry_id, task.entry_revision));
        }
        indexed.sort_unstable();
        let mut expected: Vec<(Uuid, i64)> = created
            .iter()
            .map(|entry| (entry.id, entry.revision))
            .collect();
        expected.sort_unstable();
        assert_eq!(indexed, expected);

        // 同一命令重放返回原结果，不再次写入。
        let replayed = store
            .find_accumulation_result(command_id, space.id, "雨夜灯塔")
            .await
            .unwrap()
            .expect("命令回执已写入");
        assert_eq!(replayed, created);
        assert_eq!(
            store
                .list_knowledge_entries(Some(space.id), None)
                .await
                .unwrap()
                .len(),
            2
        );

        // 同一个 command_id 用另一组线索是幂等冲突，不会写入。
        assert!(matches!(
            store
                .find_accumulation_result(command_id, space.id, "另一次积累")
                .await
                .unwrap_err(),
            StoreError::IdempotencyConflict
        ));
    }

    #[tokio::test]
    async fn accumulation_rejects_invalid_candidates_and_missing_spaces_atomically() {
        let (_dir, store) = open().await;
        let space = new_space(&store, "积累").await;

        let invalid = store
            .insert_accumulated_entries(
                Uuid::new_v4(),
                space.id,
                "线索",
                vec![
                    candidate(space.id, "有效候选", "这一条本身是合法的。"),
                    candidate(space.id, "   ", "标题为空的那一条。"),
                ],
            )
            .await
            .unwrap_err();
        assert!(matches!(invalid, StoreError::InvalidKnowledgeEntry(_)));
        // 整批原子：一条候选无效就不留下任何条目，也没有回执。
        assert_eq!(
            store
                .list_knowledge_entries(Some(space.id), None)
                .await
                .unwrap(),
            [] as [nekowriter_core::KnowledgeEntrySummary; 0]
        );

        let command_id = Uuid::new_v4();
        let missing_space = Uuid::new_v4();
        let missing = store
            .insert_accumulated_entries(
                command_id,
                missing_space,
                "线索",
                vec![candidate(
                    space.id,
                    "目标 Space 不存在",
                    "没有可写入的地方。",
                )],
            )
            .await
            .unwrap_err();
        assert!(matches!(missing, StoreError::KnowledgeSpaceNotFound));
        // 失败的命令不留下回执，重试会重新执行而不是返回上次的失败。
        assert!(
            store
                .find_accumulation_result(command_id, missing_space, "线索")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn chunking_keeps_unicode_boundaries_and_bounds_size() {
        let long_line = "雨".repeat(2_500);
        let chunks = chunk_knowledge_content(&format!("引言。\n{long_line}\n结尾。"));
        assert!(chunks.len() >= 3);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 1_000);
        }
        let rejoined: String = chunks.concat();
        assert!(rejoined.contains("引言。"));
        assert!(rejoined.contains("结尾。"));

        let emoji = chunk_knowledge_content("🙂🙂🙂 段落一\n\n段落二 🙂");
        assert_eq!(emoji.len(), 1);
        assert!(emoji[0].contains('🙂'));
        assert_eq!(chunk_knowledge_content("   ").len(), 0);
    }
}
