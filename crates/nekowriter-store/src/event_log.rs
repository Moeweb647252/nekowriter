//! 持久事件读取与 SSE 唤醒通知。
//!
//! SQLite events 表是可恢复事件的唯一事实来源。内存广播不携带业务数据，仅用于
//! 唤醒订阅者重新按游标读取该表；即使接收者滞后或广播丢失，也不会遗漏已提交事件。
//!
//! 写入侧也归本模块：`insert_event` 与 `insert_app_event` 是 events 表唯一的写入入口，
//! 读、写、保留期清理与唤醒通知因此共用一个所有者。

use nekowriter_core::{VersionId, utc_now_millis};
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder,
    Set,
};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{Store, StoreError, db_id, entity::event, parse_optional_id};

/// 可恢复 SSE 事件的保留期。
///
/// 业务数据本身不会随事件一起删除；事件只是前端同步的增量提示，因此客户端在
/// 游标落到该窗口之前时必须重新取得业务快照。
pub const EVENT_RETENTION_MILLIS: i64 = 7 * 24 * 60 * 60 * 1_000;

/// 当前保留窗口能够接受的游标范围。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventStreamBounds {
    /// 小于此值的游标已无法由保留事件完整恢复。
    pub minimum_cursor: i64,
    /// 当前可见事件的最高序列号。
    pub watermark: i64,
}

/// 一个可恢复的 SQLite 事件。
///
/// `event_type` 用作 SSE 的 event 名称，因此不重复写入 data 信封。所有跨网络的
/// 64 位整数都编码为十进制字符串。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PersistedEvent {
    pub seq: String,
    #[serde(skip)]
    pub event_type: String,
    pub version_id: Option<VersionId>,
    pub entity_id: Uuid,
    pub revision: String,
    pub payload: Value,
    pub occurred_at: String,
}

impl Store {
    /// 从严格大于 after 的位置按序读取持久事件。
    ///
    /// # Errors
    ///
    /// 当游标为负数或本地事件记录损坏时返回错误。
    pub async fn events_after(&self, after: i64) -> Result<Vec<PersistedEvent>, StoreError> {
        if after < 0 {
            return Err(StoreError::CorruptRecord("事件游标不能小于零".to_owned()));
        }
        event::Entity::find()
            .filter(event::Entity::COLUMN.seq.gt(after))
            .filter(
                event::Entity::COLUMN
                    .occurred_at
                    .gte(retention_started_at()),
            )
            .order_by_asc(event::Entity::COLUMN.seq)
            .all(&self.database)
            .await?
            .into_iter()
            .map(persisted_event_from_model)
            .collect()
    }

    /// 返回当前事件保留窗口的首尾游标。
    ///
    /// 调用方应在订阅广播之后读取边界：即使边界读取结束后恰有新事件提交，后续
    /// 仍会通过 SQLite 的 `seq > cursor` 查询或广播唤醒得到它。
    pub async fn event_stream_bounds(&self) -> Result<EventStreamBounds, StoreError> {
        let retention_started_at = retention_started_at();
        let oldest_retained = event::Entity::find()
            .filter(event::Entity::COLUMN.occurred_at.gte(retention_started_at))
            .order_by_asc(event::Entity::COLUMN.seq)
            .one(&self.database)
            .await?;
        let newest_retained = event::Entity::find()
            .filter(event::Entity::COLUMN.occurred_at.gte(retention_started_at))
            .order_by_desc(event::Entity::COLUMN.seq)
            .one(&self.database)
            .await?;
        let watermark = newest_retained.map_or(0, |event| event.seq);
        let minimum_cursor = oldest_retained.map_or(watermark, |event| event.seq - 1);

        Ok(EventStreamBounds {
            minimum_cursor,
            watermark,
        })
    }

    /// 创建一个仅用于唤醒 SSE 消费者的订阅者。
    #[must_use]
    pub fn subscribe_events(&self) -> broadcast::Receiver<()> {
        self.event_notifier.subscribe()
    }

    pub(crate) fn notify_event_listeners(&self) {
        let _ = self.event_notifier.send(());
    }
}

/// 返回与 SSE 读取完全相同保留窗口内的最高序列号。
///
/// 工作台快照必须使用这个水位，而不是 events 表的历史最大值；否则长期运行但
/// 没有新事件时，客户端会拿着已过期的游标反复收到 reset。
pub(crate) async fn retained_event_watermark<C: ConnectionTrait>(
    connection: &C,
) -> Result<i64, StoreError> {
    Ok(event::Entity::find()
        .filter(
            event::Entity::COLUMN
                .occurred_at
                .gte(retention_started_at()),
        )
        .order_by_desc(event::Entity::COLUMN.seq)
        .one(connection)
        .await?
        .map_or(0, |event| event.seq))
}

/// 启动时移除超过同步保留期的事件记录。
///
/// 该删除只作用于 events 表，不会移除文档、审核、工作流或其他业务事实。
pub(crate) async fn discard_expired_events(store: &Store) -> Result<(), StoreError> {
    event::Entity::delete_many()
        .filter(event::Entity::COLUMN.occurred_at.lt(retention_started_at()))
        .exec(&store.database)
        .await?;
    Ok(())
}

fn retention_started_at() -> i64 {
    utc_now_millis().saturating_sub(EVENT_RETENTION_MILLIS)
}

fn persisted_event_from_model(model: event::Model) -> Result<PersistedEvent, StoreError> {
    let payload = serde_json::from_str(&model.payload_json).map_err(|error| {
        StoreError::CorruptRecord(format!(
            "事件 {} 的 payload_json 无法解析：{error}",
            model.seq
        ))
    })?;
    Ok(PersistedEvent {
        seq: model.seq.to_string(),
        event_type: model.event_type,
        version_id: parse_optional_id(model.version_id.as_deref(), "event.version_id")?,
        entity_id: model.entity_id.into(),
        revision: model.revision,
        payload,
        occurred_at: model.occurred_at.to_string(),
    })
}

pub(crate) async fn insert_event(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    event_type: &str,
    entity_id: Uuid,
    revision: i64,
    payload: serde_json::Value,
    occurred_at: i64,
) -> Result<(), StoreError> {
    insert_event_row(
        transaction,
        Some(version_id),
        event_type,
        entity_id,
        revision,
        payload,
        occurred_at,
    )
    .await
}

/// 写入一条不属于任何稿本的持久事件（如知识库索引状态变化）。
pub(crate) async fn insert_app_event(
    transaction: &DatabaseTransaction,
    event_type: &str,
    entity_id: Uuid,
    revision: i64,
    payload: serde_json::Value,
    occurred_at: i64,
) -> Result<(), StoreError> {
    insert_event_row(
        transaction,
        None,
        event_type,
        entity_id,
        revision,
        payload,
        occurred_at,
    )
    .await
}

async fn insert_event_row(
    transaction: &DatabaseTransaction,
    version_id: Option<VersionId>,
    event_type: &str,
    entity_id: Uuid,
    revision: i64,
    payload: serde_json::Value,
    occurred_at: i64,
) -> Result<(), StoreError> {
    event::ActiveModel {
        version_id: Set(version_id.map(|id| id.to_string())),
        event_type: Set(event_type.to_owned()),
        entity_id: Set(db_id(entity_id)),
        revision: Set(revision.to_string()),
        payload_json: Set(serde_json::to_string(&payload)?),
        occurred_at: Set(occurred_at),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(())
}
