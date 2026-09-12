//! 一次系统模型调用（模型连接测试、向量请求）的落库。
//!
//! 这类调用真的会发出一次请求，但不属于任何稿本，因此不能写进以 Run 为主键的
//! `model_requests`；它们写进 `system_model_operations`，并在概览里单列。
//!
//! 「结果怎么翻译成状态、用量与错误」对对话与 Embedding 两种适配器是一样的，所以
//! 这条策略只写一次；各适配器只负责把自己类型的结果翻译成 [`SystemOperationResult`]。

use nekowriter_core::{PersistedModelUsage, UsageUnknownReason};
use nekowriter_store::{
    NewSystemModelOperation, OPERATION_STATUS_FAILED, OPERATION_STATUS_SUCCEEDED,
    OPERATION_STATUS_UNKNOWN, Store, StoreError,
};
use serde_json::json;
use uuid::Uuid;

/// 一次系统模型调用的结果。
pub(crate) enum SystemOperationResult {
    /// 供应商正常应答。
    Succeeded {
        usage: PersistedModelUsage,
        provider_request_id: Option<String>,
    },
    /// 请求失败。
    ///
    /// `reached_provider_unknown` 表示无法断定请求是否已经到达供应商（连接与超时类
    /// 失败），这类情况标为结果未知而不是失败；用量一律记为请求失败，因为没有报告
    /// 就是未知，不补 0。
    Failed {
        reached_provider_unknown: bool,
        kind: &'static str,
        message: String,
    },
}

/// 把一次系统模型调用落库，并广播一条不属于任何稿本的事件。
///
/// # Errors
///
/// 数据库写入失败时返回错误。
pub(crate) async fn record_system_operation(
    store: &Store,
    kind: &'static str,
    model_id: &str,
    api_base_url: &str,
    started_at_ms: i64,
    completed_at_ms: i64,
    result: SystemOperationResult,
) -> Result<Uuid, StoreError> {
    let (status, usage, error, provider_request_id) = match result {
        SystemOperationResult::Succeeded {
            usage,
            provider_request_id,
        } => (OPERATION_STATUS_SUCCEEDED, usage, None, provider_request_id),
        SystemOperationResult::Failed {
            reached_provider_unknown,
            kind,
            message,
        } => (
            if reached_provider_unknown {
                OPERATION_STATUS_UNKNOWN
            } else {
                OPERATION_STATUS_FAILED
            },
            PersistedModelUsage::unknown(UsageUnknownReason::RequestFailed),
            Some(json!({ "kind": kind, "message": message })),
            None,
        ),
    };
    store
        .record_system_model_operation(NewSystemModelOperation {
            kind,
            model_id: model_id.to_owned(),
            api_base_url: api_base_url.to_owned(),
            status,
            provider_request_id,
            usage,
            error,
            started_at_ms,
            completed_at_ms,
        })
        .await
}
