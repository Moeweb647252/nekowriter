//! 写命令的幂等信封：一条带 `command_id` 的写入如何预查、提交、重放与通知。
//!
//! `Command` 是幂等规则的唯一实现处：事务外预查既有回执，事务内由调用点把回执与业务
//! 写入放进同一个事务，收尾时把「并发重复命令」还原成既有回执，并按 `Notify` 决定
//! 是否唤醒事件监听者。从 `lib.rs` 拆出后，`lib.rs` 用 `pub(crate) use command::{...}`
//! 让兄弟模块原有的 `use super::{...}` 路径保持不变。

use std::future::Future;
use std::pin::Pin;

use nekowriter_core::ApprovalMode;
use sea_orm::prelude::TextUuid;
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, DatabaseTransaction, Set, TransactionError, TransactionTrait,
};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{Store, StoreError, entity::command_receipt};

#[derive(Clone, Debug)]
pub(crate) struct CommandReceipt {
    pub(crate) method: String,
    pub(crate) target: String,
    pub(crate) payload_hash: String,
    pub(crate) response_json: String,
}

impl Store {
    async fn replay_after_receipt_conflict<T: DeserializeOwned>(
        &self,
        command_id: Uuid,
        method: &str,
        target: &str,
        expected_hash: &str,
        original_error: StoreError,
    ) -> Result<T, StoreError> {
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(&receipt, method, target, expected_hash);
        }
        Err(original_error)
    }
}

pub(crate) fn payload_hash<T: Serialize>(payload: &T) -> Result<String, StoreError> {
    let bytes = serde_json::to_vec(payload)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(crate) fn replay_or_conflict<T: DeserializeOwned>(
    receipt: &CommandReceipt,
    expected_method: &str,
    expected_target: &str,
    expected_hash: &str,
) -> Result<T, StoreError> {
    if receipt.method != expected_method
        || receipt.target != expected_target
        || receipt.payload_hash != expected_hash
    {
        return Err(StoreError::IdempotencyConflict);
    }
    Ok(serde_json::from_str(&receipt.response_json)?)
}

pub(crate) async fn insert_receipt<C: ConnectionTrait>(
    connection: &C,
    command_id: Uuid,
    method: &str,
    target: &str,
    payload_hash: &str,
    response_json: &str,
    response_status: i64,
    created_at: i64,
) -> Result<(), StoreError> {
    command_receipt::ActiveModel {
        command_id: Set(db_id(command_id)),
        method: Set(method.to_owned()),
        target: Set(target.to_owned()),
        payload_hash: Set(payload_hash.to_owned()),
        response_status: Set(response_status),
        response_json: Set(response_json.to_owned()),
        created_at: Set(created_at),
    }
    .insert(connection)
    .await?;
    Ok(())
}

pub(crate) fn db_id(id: Uuid) -> TextUuid {
    id.into()
}

pub(crate) fn parse_id(value: &str, field: &str) -> Result<Uuid, StoreError> {
    Uuid::parse_str(value)
        .map_err(|error| StoreError::CorruptRecord(format!("{field} 不是有效 UUID：{error}")))
}

pub(crate) fn parse_optional_id(
    value: Option<&str>,
    field: &str,
) -> Result<Option<Uuid>, StoreError> {
    value.map(|value| parse_id(value, field)).transpose()
}

pub(crate) fn transaction_error(error: TransactionError<StoreError>) -> StoreError {
    match error {
        TransactionError::Connection(error) => StoreError::Database(error),
        TransactionError::Transaction(error) => error,
    }
}

/// 一条写命令的幂等信封。
///
/// `command_id`、方法、目标与指纹分三处出现：事务外预查、事务内的回执写入、收尾时的
/// 重放还原。前两处由调用点自己拼（回执要与业务写入同事务提交），第三处由信封负责；
/// 让同一个值持有这四样东西，调用点就不必把同一个方法/目标串抄三遍，也不会抄歪。
pub(crate) struct Command<'a> {
    id: Uuid,
    method: &'a str,
    target: &'a str,
    payload_hash: &'a str,
}

/// 收尾是否把这次写入广播给事件监听者。
///
/// 用类型而不是 `bool`：`true` / `false` 在调用点读不出含义，而「no-op 命令不该打扰
/// 界面重读」正是最容易写反、评审时也最难看出来的一处。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Notify {
    /// 命令确实改了东西：界面需要重读。
    Listeners,
    /// 命令是 no-op：不打扰界面。
    Quiet,
}

impl Notify {
    /// no-op 命令不通知：`changed` 为真才代表这次命令真的写入了。
    pub(crate) const fn on_change(changed: bool) -> Self {
        if changed {
            Self::Listeners
        } else {
            Self::Quiet
        }
    }
}

impl<'a> Command<'a> {
    pub(crate) const fn new(
        id: Uuid,
        method: &'a str,
        target: &'a str,
        payload_hash: &'a str,
    ) -> Self {
        Self {
            id,
            method,
            target,
            payload_hash,
        }
    }

    /// 事务外的幂等预查：已有同 `command_id` 的回执就直接还原（指纹不符是冲突）。
    pub(crate) async fn replay_if_recorded<T: DeserializeOwned>(
        &self,
        store: &Store,
    ) -> Result<Option<T>, StoreError> {
        match store.find_receipt(self.id).await? {
            Some(receipt) => {
                replay_or_conflict(&receipt, self.method, self.target, self.payload_hash).map(Some)
            }
            None => Ok(None),
        }
    }

    /// 一条写命令的完整幂等流程：预查 → `body`（事务体）→ 收尾。
    ///
    /// 事务外的预查由这里独占，调用点不再自己查一遍。例外只有两种，都在调用点写明
    /// 了理由：早退能省掉真实前置计算的站点（例如 `embedding_profile_with_id()`，
    /// 或预查之后的拒绝分支本身会改写重放结果），以及只看事务结果决定通知、收尾走
    /// `finish` 的两段式站点。
    ///
    /// `body` 里仍自己写 `insert_receipt`：回执必须与业务写入在同一个事务里提交，
    /// 把这一步也收进来只会让事务边界消失在两次调用之间。并发重复命令靠
    /// `command_receipts.command_id` 主键冲突挡住，错误在 `finish` 里换成既有回执。
    pub(crate) async fn commit<T, F>(
        &self,
        store: &Store,
        notify: Notify,
        body: F,
    ) -> Result<T, StoreError>
    where
        T: DeserializeOwned + Serialize + Send,
        F: for<'c> FnOnce(
                &'c DatabaseTransaction,
            )
                -> Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'c>>
            + Send,
    {
        if let Some(replayed) = self.replay_if_recorded(store).await? {
            return Ok(replayed);
        }
        let result = store
            .database
            .transaction::<_, _, StoreError>(body)
            .await
            .map_err(transaction_error);
        self.finish(store, notify, result).await
    }

    /// 收尾：成功时按 `notify` 通知事件监听者，失败时把「并发重复命令」还原成既有回执。
    ///
    /// 所有带 `command_id` 的写入共用它，因此幂等重放规则只有一处。事务外的预查挡住了
    /// 绝大多数重放，而两次预查之间可能已经有同 `command_id` 的命令提交了回执——这时唯一
    /// 正确的结果是那条既有回执，不是报错；因此失败分支不去区分错误类型：有回执就按回执
    /// 处理，没有就原样返回原错误。
    pub(crate) async fn finish<T: DeserializeOwned + Serialize>(
        &self,
        store: &Store,
        notify: Notify,
        result: Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        match result {
            Ok(response) => {
                if notify == Notify::Listeners {
                    store.notify_event_listeners();
                }
                Ok(response)
            }
            Err(error) => {
                store
                    .replay_after_receipt_conflict(
                        self.id,
                        self.method,
                        self.target,
                        self.payload_hash,
                        error,
                    )
                    .await
            }
        }
    }
}

pub(crate) const fn approval_mode_name(mode: ApprovalMode) -> &'static str {
    match mode {
        ApprovalMode::AutoApprove => "auto_approve",
        ApprovalMode::ApproveRequired => "approve_required",
    }
}
