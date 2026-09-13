//! 系统模型操作的持久化与统计读取。
//!
//! 连接测试这类请求真的会发出一次模型调用，但不属于任何稿本，因此不能写进以 Run
//! 为主键的 `model_requests`。它们单独成表，并带上与对话请求相同的用量口径，
//! 使 Dashboard 能把它们单列而不是让用量凭空消失。

use nekowriter_core::{
    DASHBOARD_SCOPE_GLOBAL, DashboardDailyEmbedding, DashboardEmbeddingKind,
    DashboardEmbeddingUsage, DashboardRange, DashboardSystemOperation, DashboardSystemOperations,
    DashboardUnknownUsage, PersistedModelUsage, UsageUnknownReason,
};
use std::collections::HashMap;

use sea_orm::{ActiveModelTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait};
use uuid::Uuid;

use super::{
    Store, StoreError, db_id, entity::system_model_operation, preparation::insert_app_event,
};

/// 操作类型：模型连接测试。
pub const OPERATION_KIND_CONNECTION_TEST: &str = "connection_test";
/// 操作类型：知识索引任务产生的向量请求。
pub const OPERATION_KIND_EMBEDDING_INDEX: &str = "embedding_index";
/// 操作类型：语义检索产生的查询向量请求。
pub const OPERATION_KIND_EMBEDDING_QUERY: &str = "embedding_query";
/// 操作类型：Embedding 连接测试，包含保存设置前的验证调用。
pub const OPERATION_KIND_EMBEDDING_CONNECTION_TEST: &str = "embedding_connection_test";

/// 这个操作类型是不是 Embedding 请求。
///
/// Embedding 用量与对话用量分开列示，因此聚合时必须能把它们分流；写入时用的种类与
/// 读取时的分类共用这一处判定，两边不会分叉。
#[must_use]
pub fn is_embedding_operation(kind: &str) -> bool {
    matches!(
        kind,
        OPERATION_KIND_EMBEDDING_INDEX
            | OPERATION_KIND_EMBEDDING_QUERY
            | OPERATION_KIND_EMBEDDING_CONNECTION_TEST
    )
}
/// 操作结果：供应商正常应答。
pub const OPERATION_STATUS_SUCCEEDED: &str = "succeeded";
/// 操作结果：供应商明确报错。
pub const OPERATION_STATUS_FAILED: &str = "failed";
/// 操作结果：连接或超时失败，请求可能已经到达供应商。
pub const OPERATION_STATUS_UNKNOWN: &str = "unknown";

/// Dashboard 系统操作列表最多返回的明细条数。
const RECENT_SYSTEM_OPERATIONS: usize = 5;

/// 一次系统模型操作的写入输入。
#[derive(Clone, Debug)]
pub struct NewSystemModelOperation {
    /// 操作类型，取值见本模块的 `OPERATION_KIND_*` 常量。
    pub kind: &'static str,
    pub model_id: String,
    /// 实际请求的服务根地址，不含凭据。
    pub api_base_url: String,
    /// 操作结果，取值见本模块的 `OPERATION_STATUS_*` 常量。
    pub status: &'static str,
    pub provider_request_id: Option<String>,
    /// 本次请求的用量口径；没有报告时必须带类型化原因，不能留空。
    pub usage: PersistedModelUsage,
    /// 失败时的 `{"kind": ..., "message": ...}`。
    pub error: Option<serde_json::Value>,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
}

impl Store {
    /// 记录一次系统模型操作，并广播一条不属于任何稿本的事件。
    ///
    /// # Errors
    ///
    /// 数据库写入失败时返回错误。
    pub async fn record_system_model_operation(
        &self,
        operation: NewSystemModelOperation,
    ) -> Result<Uuid, StoreError> {
        let id = Uuid::new_v4();
        let event_payload = serde_json::json!({
            "kind": operation.kind,
            "status": operation.status,
            "modelId": operation.model_id,
        });
        let transaction = self.database.begin().await?;
        // 事件与操作明细在同一事务内写入：前端要么看到这次操作，要么什么都没发生。
        system_model_operation::ActiveModel {
            id: Set(db_id(id)),
            kind: Set(operation.kind.to_owned()),
            model_id: Set(operation.model_id.clone()),
            api_base_url: Set(operation.api_base_url.clone()),
            status: Set(operation.status.to_owned()),
            provider_request_id: Set(operation.provider_request_id.clone()),
            usage_json: Set(Some(serde_json::to_string(&operation.usage)?)),
            error_json: Set(operation
                .error
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?),
            created_at: Set(operation.started_at_ms),
            completed_at: Set(Some(operation.completed_at_ms)),
        }
        .insert(&transaction)
        .await?;
        insert_app_event(
            &transaction,
            "system.model_operation",
            id,
            1,
            event_payload,
            operation.completed_at_ms,
        )
        .await?;
        transaction.commit().await?;
        Ok(id)
    }

    /// 汇总统计区间内的系统操作用量与最近若干条明细。
    ///
    /// # Errors
    ///
    /// 数据库读取失败或记录损坏时返回错误。
    pub async fn dashboard_system_operations(
        &self,
        range: &DashboardRange,
    ) -> Result<DashboardSystemOperations, StoreError> {
        let models = system_model_operation::Entity::find()
            .filter(
                system_model_operation::Entity::COLUMN
                    .created_at
                    .gte(range.start_ms),
            )
            .filter(
                system_model_operation::Entity::COLUMN
                    .created_at
                    .lt(range.end_ms),
            )
            .order_by_desc(system_model_operation::Entity::COLUMN.created_at)
            .order_by_desc(system_model_operation::Entity::COLUMN.id)
            .all(&self.database)
            .await?;

        let mut operations = DashboardSystemOperations {
            request_count: 0,
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            reasoning_tokens: 0,
            unknown_usage: Vec::new(),
            recent: Vec::new(),
        };
        for model in &models {
            // Embedding 用量在 `dashboard_embedding_usage` 里单独汇总；这里再算一遍
            // 会让同一批 token 在概览里出现两次。
            if is_embedding_operation(&model.kind) {
                continue;
            }
            let usage = persisted_usage(model.usage_json.as_deref());
            operations.request_count += 1;
            if usage.is_reported() {
                operations.input_tokens += usage.input_tokens.unwrap_or(0);
                operations.output_tokens += usage.output_tokens.unwrap_or(0);
                operations.cached_input_tokens += usage.cached_input_tokens.unwrap_or(0);
                operations.reasoning_tokens += usage.reasoning_tokens.unwrap_or(0);
            } else {
                add_unknown_usage(
                    &mut operations.unknown_usage,
                    usage.reason.unwrap_or(UsageUnknownReason::ProviderSilent),
                );
            }
            if operations.recent.len() < RECENT_SYSTEM_OPERATIONS {
                operations
                    .recent
                    .push(dashboard_system_operation(model, &usage)?);
            }
        }
        operations
            .unknown_usage
            .sort_by_key(|entry| usage_reason_order(entry.reason));
        Ok(operations)
    }

    /// 汇总统计区间内 Embedding 请求的用量。
    ///
    /// 嵌入请求只贡献输入用量（没有输出侧），因此这里只统计输入与请求数；供应商
    /// 没有报告时按类型化原因计入未知，而不是补一个 0。它与 `tokens.daily` 共用
    /// 同一套本地日口径，但自己是独立序列，不会混进对话日柱。
    ///
    /// # Errors
    ///
    /// 数据库读取失败或记录损坏时返回错误。
    pub async fn dashboard_embedding_usage(
        &self,
        range: &DashboardRange,
    ) -> Result<DashboardEmbeddingUsage, StoreError> {
        let models = system_model_operation::Entity::find()
            .filter(
                system_model_operation::Entity::COLUMN
                    .created_at
                    .gte(range.start_ms),
            )
            .filter(
                system_model_operation::Entity::COLUMN
                    .created_at
                    .lt(range.end_ms),
            )
            .order_by_asc(system_model_operation::Entity::COLUMN.created_at)
            .order_by_asc(system_model_operation::Entity::COLUMN.id)
            .all(&self.database)
            .await?;

        let days = range.day_indexes();
        let day_slot: HashMap<i64, usize> = days
            .iter()
            .enumerate()
            .map(|(index, day)| (*day, index))
            .collect();
        let mut embedding = DashboardEmbeddingUsage {
            request_count: 0,
            input_tokens: 0,
            unknown_usage_requests: 0,
            unknown_usage: Vec::new(),
            by_kind: Vec::new(),
            daily: days
                .iter()
                .map(|day| DashboardDailyEmbedding {
                    day_start_ms: range.day_start_ms(*day),
                    input_tokens: 0,
                    request_count: 0,
                })
                .collect(),
            scope: DASHBOARD_SCOPE_GLOBAL.to_owned(),
        };
        for model in &models {
            if !is_embedding_operation(&model.kind) {
                continue;
            }
            let usage = persisted_usage(model.usage_json.as_deref());
            embedding.request_count += 1;
            let bucket = day_slot.get(&range.day_index(model.created_at)).copied();
            if let Some(index) = bucket {
                embedding.daily[index].request_count += 1;
            }
            if !embedding
                .by_kind
                .iter()
                .any(|entry| entry.kind == model.kind)
            {
                embedding.by_kind.push(DashboardEmbeddingKind {
                    kind: model.kind.clone(),
                    request_count: 0,
                    input_tokens: 0,
                });
            }
            let slot = embedding
                .by_kind
                .iter_mut()
                .find(|entry| entry.kind == model.kind)
                .expect("按需创建的操作类型条目");
            slot.request_count += 1;
            if usage.is_reported() {
                let input = usage.input_tokens.unwrap_or(0);
                embedding.input_tokens += input;
                slot.input_tokens += input;
                if let Some(index) = bucket {
                    embedding.daily[index].input_tokens += input;
                }
            } else {
                add_unknown_usage(
                    &mut embedding.unknown_usage,
                    usage.reason.unwrap_or(UsageUnknownReason::ProviderSilent),
                );
                embedding.unknown_usage_requests += 1;
            }
        }
        embedding
            .unknown_usage
            .sort_by_key(|entry| usage_reason_order(entry.reason));
        embedding
            .by_kind
            .sort_by(|left, right| left.kind.cmp(&right.kind));
        Ok(embedding)
    }
}

/// 读取一条系统操作的用量口径；没有记录时按「供应商未报告」处理。
fn persisted_usage(usage_json: Option<&str>) -> PersistedModelUsage {
    let Some(usage_json) = usage_json else {
        return PersistedModelUsage::unknown(UsageUnknownReason::ProviderSilent);
    };
    serde_json::from_str(usage_json)
        .unwrap_or_else(|_| PersistedModelUsage::unknown(UsageUnknownReason::InvalidReport))
}

fn dashboard_system_operation(
    model: &system_model_operation::Model,
    usage: &PersistedModelUsage,
) -> Result<DashboardSystemOperation, StoreError> {
    Ok(DashboardSystemOperation {
        id: model.id.0,
        kind: model.kind.clone(),
        model_id: model.model_id.clone(),
        api_base_url: model.api_base_url.clone(),
        status: model.status.clone(),
        usage_status: usage.usage_status,
        created_at_ms: model.created_at,
        input_tokens: usage.input_tokens.unwrap_or(0),
        output_tokens: usage.output_tokens.unwrap_or(0),
        error_kind: error_kind(model.error_json.as_deref())?,
    })
}

fn error_kind(error_json: Option<&str>) -> Result<Option<String>, StoreError> {
    let Some(error_json) = error_json else {
        return Ok(None);
    };
    let error: serde_json::Value = serde_json::from_str(error_json).map_err(|error| {
        StoreError::CorruptRecord(format!("系统操作失败信息不是 JSON：{error}"))
    })?;
    Ok(error
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned))
}

/// 把一条未知用量原因计入分类计数。
pub(crate) fn add_unknown_usage(
    entries: &mut Vec<DashboardUnknownUsage>,
    reason: UsageUnknownReason,
) {
    match entries.iter_mut().find(|entry| entry.reason == reason) {
        Some(entry) => entry.requests += 1,
        None => entries.push(DashboardUnknownUsage {
            reason,
            requests: 1,
        }),
    }
}

/// 稳定的原因展示顺序，保证同一批数据重复查询得到相同结果。
pub(crate) const fn usage_reason_order(reason: UsageUnknownReason) -> u8 {
    match reason {
        UsageUnknownReason::ProviderSilent => 0,
        UsageUnknownReason::RequestFailed => 1,
        UsageUnknownReason::RequestCancelled => 2,
        UsageUnknownReason::Pending => 3,
        UsageUnknownReason::InvalidReport => 4,
    }
}

#[cfg(test)]
mod tests {
    use nekowriter_core::{
        DASHBOARD_ROLE_EMBEDDING, DASHBOARD_SCOPE_GLOBAL, PersistedModelUsage, UsageStatus,
        UsageUnknownReason, utc_now_millis,
    };
    use tempfile::TempDir;

    use super::*;

    async fn open() -> (TempDir, Store) {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        (data_dir, store)
    }

    fn operation(
        status: &'static str,
        usage: PersistedModelUsage,
        started_at_ms: i64,
    ) -> NewSystemModelOperation {
        NewSystemModelOperation {
            kind: OPERATION_KIND_CONNECTION_TEST,
            model_id: "probe-model".to_owned(),
            api_base_url: "https://example.invalid/v1".to_owned(),
            status,
            provider_request_id: Some("req-1".to_owned()),
            usage,
            error: (status != OPERATION_STATUS_SUCCEEDED)
                .then(|| serde_json::json!({ "kind": "provider_error", "message": "拒绝连接" })),
            started_at_ms,
            completed_at_ms: started_at_ms + 500,
        }
    }

    #[tokio::test]
    async fn connection_tests_are_recorded_and_summarized_with_their_own_usage() {
        let (_data_dir, store) = open().await;
        let now = utc_now_millis();
        store
            .record_system_model_operation(operation(
                OPERATION_STATUS_SUCCEEDED,
                PersistedModelUsage::from_provider_stream(12, 3, 15, Some(4), None, None),
                now,
            ))
            .await
            .unwrap();
        store
            .record_system_model_operation(operation(
                OPERATION_STATUS_FAILED,
                PersistedModelUsage::unknown(UsageUnknownReason::RequestFailed),
                now + 1_000,
            ))
            .await
            .unwrap();

        let range = DashboardRange::new(now, 0, 7).unwrap();
        let summary = store.dashboard_system_operations(&range).await.unwrap();
        assert_eq!(summary.request_count, 2);
        assert_eq!(summary.input_tokens, 12);
        assert_eq!(summary.output_tokens, 3);
        assert_eq!(summary.cached_input_tokens, 4);
        // 最近一条在前，失败原因随明细一起可见。
        assert_eq!(summary.recent.len(), 2);
        assert_eq!(summary.recent[0].status, OPERATION_STATUS_FAILED);
        assert_eq!(
            summary.recent[0].error_kind.as_deref(),
            Some("provider_error")
        );
        assert_eq!(summary.recent[1].usage_status, UsageStatus::Reported);
        assert_eq!(summary.unknown_usage.len(), 1);
        assert_eq!(
            summary.unknown_usage[0].reason,
            UsageUnknownReason::RequestFailed
        );

        // 系统操作单列：它们不会混进对话 Token，也不会消失。
        let dashboard = store.compute_dashboard(range, None).await.unwrap();
        assert_eq!(dashboard.tokens.request_count, 0);
        assert_eq!(dashboard.tokens.system_operations.request_count, 2);
        assert_eq!(dashboard.tokens.system_operations.input_tokens, 12);
    }

    /// 一次 Embedding 向量请求的持久化输入。
    fn embedding_operation(
        kind: &'static str,
        usage: PersistedModelUsage,
        at: i64,
    ) -> NewSystemModelOperation {
        NewSystemModelOperation {
            kind,
            model_id: "probe-embed".to_owned(),
            api_base_url: "https://embed.example/v1".to_owned(),
            status: OPERATION_STATUS_SUCCEEDED,
            provider_request_id: None,
            usage,
            error: None,
            started_at_ms: at,
            completed_at_ms: at + 100,
        }
    }

    #[tokio::test]
    async fn embedding_usage_is_listed_separately_and_never_double_counted() {
        let (_data_dir, store) = open().await;
        let now = utc_now_millis();
        store
            .record_system_model_operation(embedding_operation(
                OPERATION_KIND_EMBEDDING_INDEX,
                PersistedModelUsage::from_embeddings_response(128, 128, None),
                now,
            ))
            .await
            .unwrap();
        store
            .record_system_model_operation(embedding_operation(
                OPERATION_KIND_EMBEDDING_QUERY,
                PersistedModelUsage::from_embeddings_response(12, 12, None),
                now + 1,
            ))
            .await
            .unwrap();
        store
            .record_system_model_operation(embedding_operation(
                OPERATION_KIND_EMBEDDING_QUERY,
                PersistedModelUsage::unknown(UsageUnknownReason::RequestFailed),
                now + 2,
            ))
            .await
            .unwrap();
        // 一次模型连接测试仍然留在系统操作里，不会被当成 Embedding。
        store
            .record_system_model_operation(operation(
                OPERATION_STATUS_SUCCEEDED,
                PersistedModelUsage::from_provider_stream(12, 3, 15, None, None, None),
                now + 3,
            ))
            .await
            .unwrap();

        let range = DashboardRange::new(now, 0, 7).unwrap();
        let dashboard = store.compute_dashboard(range, None).await.unwrap();
        let embedding = &dashboard.tokens.embedding;
        assert_eq!(embedding.request_count, 3);
        assert_eq!(embedding.input_tokens, 140);
        assert_eq!(embedding.unknown_usage_requests, 1);
        assert_eq!(
            embedding.unknown_usage[0].reason,
            UsageUnknownReason::RequestFailed
        );
        assert_eq!(embedding.by_kind.len(), 2);
        assert_eq!(embedding.by_kind[0].kind, OPERATION_KIND_EMBEDDING_INDEX);
        assert_eq!(embedding.by_kind[0].input_tokens, 128);
        assert_eq!(embedding.by_kind[1].request_count, 2);
        assert_eq!(embedding.by_kind[1].input_tokens, 12);

        // 对话总量不受影响；系统操作只统计那次模型连接测试。
        assert_eq!(dashboard.tokens.request_count, 0);
        assert_eq!(dashboard.tokens.input_tokens, 0);
        assert_eq!(dashboard.tokens.system_operations.request_count, 1);
        assert_eq!(dashboard.tokens.system_operations.input_tokens, 12);

        // 角色镜头里 Embedding 与写作/审核并列，而且它就是那份独立列示的数字。
        let role = dashboard
            .tokens
            .by_role
            .iter()
            .find(|entry| entry.role == DASHBOARD_ROLE_EMBEDDING)
            .expect("角色镜头里应当有 Embedding 角色");
        assert_eq!(role.input_tokens, embedding.input_tokens);
        assert_eq!(role.output_tokens, 0);
        assert_eq!(role.request_count, embedding.request_count);
        assert_eq!(
            role.unknown_usage_requests,
            embedding.unknown_usage_requests
        );

        // 日序列与区间总量一致，且它自己说明归属。
        assert_eq!(embedding.scope, DASHBOARD_SCOPE_GLOBAL);
        assert_eq!(embedding.daily.len(), 7);
        assert_eq!(
            embedding
                .daily
                .iter()
                .map(|day| day.request_count)
                .sum::<i64>(),
            embedding.request_count
        );
        assert_eq!(
            embedding
                .daily
                .iter()
                .map(|day| day.input_tokens)
                .sum::<i64>(),
            embedding.input_tokens
        );
    }

    /// 向量用量按本地日分桶，与对话用量同一套时间口径，但是独立序列。
    #[tokio::test]
    async fn embedding_usage_is_bucketed_by_local_day_like_conversation_usage() {
        let (_data_dir, store) = open().await;
        // UTC+8：今天写入两行（一次报告、一次未知），昨天写入一行。
        let now = utc_now_millis();
        let range = DashboardRange::new(now, 480, 7).unwrap();
        let today = range.day_start_ms(range.day_index(now));
        let yesterday = range.day_start_ms(range.day_index(now) - 1);
        store
            .record_system_model_operation(embedding_operation(
                OPERATION_KIND_EMBEDDING_INDEX,
                PersistedModelUsage::from_embeddings_response(30, 30, None),
                today + 1_000,
            ))
            .await
            .unwrap();
        store
            .record_system_model_operation(embedding_operation(
                OPERATION_KIND_EMBEDDING_QUERY,
                PersistedModelUsage::unknown(UsageUnknownReason::ProviderSilent),
                today + 2_000,
            ))
            .await
            .unwrap();
        store
            .record_system_model_operation(embedding_operation(
                OPERATION_KIND_EMBEDDING_QUERY,
                PersistedModelUsage::from_embeddings_response(7, 7, None),
                yesterday + 500,
            ))
            .await
            .unwrap();

        let dashboard = store.compute_dashboard(range, None).await.unwrap();
        let daily = &dashboard.tokens.embedding.daily;
        let last = daily.last().expect("区间至少一天");
        let previous = daily[daily.len() - 2].clone();
        assert_eq!(last.day_start_ms, today);
        assert_eq!(last.request_count, 2);
        // 未知用量的请求计入请求数，但不贡献 token，也不会被算成 0。
        assert_eq!(last.input_tokens, 30);
        assert_eq!(previous.day_start_ms, yesterday);
        assert_eq!(previous.request_count, 1);
        assert_eq!(previous.input_tokens, 7);
        assert_eq!(dashboard.tokens.embedding.input_tokens, 37);
        assert_eq!(dashboard.tokens.embedding.request_count, 3);
        // 对话日序列不受影响：向量用量不进入对话日柱。
        assert!(
            dashboard
                .tokens
                .daily
                .iter()
                .all(|day| day.request_count == 0 && day.input_tokens == 0)
        );
    }

    #[tokio::test]
    async fn system_operations_outside_the_range_are_excluded() {
        let (_data_dir, store) = open().await;
        let now = utc_now_millis();
        let long_ago = now - 40 * nekowriter_core::DASHBOARD_DAY_MS;
        store
            .record_system_model_operation(operation(
                OPERATION_STATUS_SUCCEEDED,
                PersistedModelUsage::from_provider_stream(5, 5, 10, None, None, None),
                long_ago,
            ))
            .await
            .unwrap();

        let range = DashboardRange::new(now, 0, 7).unwrap();
        let summary = store.dashboard_system_operations(&range).await.unwrap();
        assert_eq!(summary.request_count, 0);
        assert_eq!(
            summary.recent,
            [] as [nekowriter_core::DashboardSystemOperation; 0]
        );
        assert_eq!(
            summary.unknown_usage,
            [] as [nekowriter_core::DashboardUnknownUsage; 0]
        );
    }
}
