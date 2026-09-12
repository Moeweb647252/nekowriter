//! 知识库与 Embedding 的领域类型。
//!
//! 这里只描述跨进程可见的数据形状：Space、条目、索引状态、检索结果与 Embedding
//! 配置。向量计算、LanceDB 读写和 SQLite 查询都属于 Store 与 App，core 不做 I/O。

use serde::{Deserialize, Serialize};

use crate::decimal_i64;

/// 一次 Embedding 调用所固定的模型、服务地址和输出维度。
///
/// 同一 profile 的向量才可以互相比较；模型、地址或维度任一改变都必须建立新的
/// profile 索引。指纹由 Store 计算并用于隔离 `LanceDB` 表。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingProfile {
    pub api_base_url: String,
    pub model_id: String,
    #[serde(with = "decimal_i64")]
    pub dimension: i64,
}

/// Embedding 配置的系统级视图。凭据正文不进入该结构，只暴露是否已配置。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingSettings {
    pub profile: EmbeddingProfile,
    pub credential_configured: bool,
    #[serde(with = "decimal_i64")]
    pub revision: i64,
    #[serde(with = "decimal_i64")]
    pub updated_at_ms: i64,
}

/// 一条知识条目在某个 Embedding profile 下的索引状态。
///
/// `not_configured` 表示尚未保存 Embedding 配置；此时知识照常保存，不做任何网络
/// 请求。其余状态来自该条目当前修订对应的索引任务。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeIndexStatus {
    NotConfigured,
    Pending,
    Running,
    Ready,
    Failed,
}

/// 一个知识 Space 的读取视图。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSpace {
    pub id: uuid::Uuid,
    pub title: String,
    #[serde(with = "decimal_i64")]
    pub revision: i64,
    /// 该 Space 下未删除的条目数；删除 Space 前必须为 0。
    #[serde(with = "decimal_i64")]
    pub entry_count: i64,
    #[serde(with = "decimal_i64")]
    pub created_at_ms: i64,
    #[serde(with = "decimal_i64")]
    pub updated_at_ms: i64,
}

/// 一条已持久化的知识条目及其当前索引状态。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEntry {
    pub id: uuid::Uuid,
    pub space_id: uuid::Uuid,
    pub title: String,
    pub keywords: Vec<String>,
    pub content: String,
    pub source: Option<String>,
    #[serde(with = "decimal_i64")]
    pub revision: i64,
    pub index_status: KnowledgeIndexStatus,
    /// 当前修订已建立 ready 索引任务时为该修订，否则为 `None`。
    #[serde(with = "super::option_decimal_i64")]
    pub indexed_revision: Option<i64>,
    pub index_error: Option<String>,
    #[serde(with = "decimal_i64")]
    pub created_at_ms: i64,
    #[serde(with = "decimal_i64")]
    pub updated_at_ms: i64,
}

/// 知识列表使用的轻量条目，不含正文。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEntrySummary {
    pub id: uuid::Uuid,
    pub space_id: uuid::Uuid,
    pub title: String,
    pub keywords: Vec<String>,
    #[serde(with = "decimal_i64")]
    pub revision: i64,
    pub index_status: KnowledgeIndexStatus,
    #[serde(with = "decimal_i64")]
    pub content_chars: i64,
    #[serde(with = "decimal_i64")]
    pub updated_at_ms: i64,
}

/// 一条语义检索命中的原文块。
///
/// `score` 只是向量距离换算出的相关度排序依据，不代表事实置信度。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSearchHit {
    pub chunk_id: String,
    pub entry_id: uuid::Uuid,
    #[serde(with = "decimal_i64")]
    pub entry_revision: i64,
    pub space_id: uuid::Uuid,
    #[serde(with = "decimal_i64")]
    pub chunk_index: i64,
    pub title: String,
    pub text: String,
    pub score: f64,
}

/// 本次检索范围内各索引状态的条目数量，用于向用户说明检索覆盖度。
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeIndexCoverage {
    #[serde(with = "decimal_i64")]
    pub ready_entries: i64,
    #[serde(with = "decimal_i64")]
    pub pending_entries: i64,
    #[serde(with = "decimal_i64")]
    pub failed_entries: i64,
    #[serde(with = "decimal_i64")]
    pub not_configured_entries: i64,
}

/// 一次语义检索的结果与覆盖度。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSearchResult {
    pub hits: Vec<KnowledgeSearchHit>,
    pub coverage: KnowledgeIndexCoverage,
}
