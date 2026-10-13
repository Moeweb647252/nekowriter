//! The event log: committed changes, in commit order, for clients to follow.
//!
//! Events are appended inside the transaction that makes the change, so a client that
//! reads data and then follows events from an earlier cursor never misses a change.
//! Subscribers are woken through a watch channel that holds the highest committed sequence
//! number; they always read the events themselves from the database.

use nekowriter_core::{event::Event, time::Timestamp};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait, sea_query::Expr,
};
use tokio::sync::watch;

use crate::{
    Store, StoreError,
    entity::{event, schema_info},
};

/// How long events are kept. A client that falls further behind has to reload.
pub const RETENTION_MILLIS: i64 = 7 * 24 * 60 * 60 * 1000;

/// One event with its position in the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventRecord {
    pub seq: i64,
    pub event: Event,
}

/// What a client gets when it asks for events after a cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventBatch {
    /// The events after the cursor, oldest first (possibly none).
    Events(Vec<EventRecord>),
    /// The cursor is older than the retained log or newer than the newest event. The
    /// client must reload its data and continue from a fresh cursor.
    ResetRequired,
}

/// Wakes event subscribers after commits.
#[derive(Clone, Debug)]
pub(crate) struct Notifier {
    latest: watch::Sender<i64>,
}

impl Notifier {
    pub(crate) fn new(latest_seq: i64) -> Self {
        Self {
            latest: watch::Sender::new(latest_seq),
        }
    }

    /// Records that events up to `seq` are committed. Commits may report out of order, so
    /// the value only ever grows.
    pub(crate) fn notify(&self, seq: i64) {
        self.latest.send_if_modified(|latest| {
            let grew = seq > *latest;
            if grew {
                *latest = seq;
            }
            grew
        });
    }
}

/// Appends `events` and returns the sequence number of the last one, if any.
pub(crate) async fn append<C: ConnectionTrait>(
    db: &C,
    events: &[Event],
    occurred_at: Timestamp,
) -> Result<Option<i64>, StoreError> {
    let mut last = None;
    for event in events {
        let row = event::ActiveModel {
            payload: Set(serde_json::to_string(event)?),
            occurred_at: Set(occurred_at),
            ..Default::default()
        }
        .insert(db)
        .await?;
        last = Some(row.seq);
    }
    Ok(last)
}

impl Store {
    /// The sequence number of the newest event, or 0 when the log is empty. A client reads
    /// this before loading data and then follows events after it.
    pub async fn event_cursor(&self) -> Result<i64, StoreError> {
        Ok(max_seq(&self.db).await?)
    }

    /// Up to `limit` events after `cursor`.
    pub async fn events_after(&self, cursor: i64, limit: u64) -> Result<EventBatch, StoreError> {
        let txn = self.db.begin().await?;
        let pruned_through = schema_info::Entity::find_by_id(schema_info::ROW_ID)
            .one(&txn)
            .await?
            .ok_or_else(|| StoreError::Corrupt("the schema_info row is missing".to_owned()))?
            .events_pruned_through;
        if cursor < pruned_through || cursor > max_seq(&txn).await? {
            return Ok(EventBatch::ResetRequired);
        }
        let rows = event::Entity::find()
            .filter(event::Column::Seq.gt(cursor))
            .order_by_asc(event::Column::Seq)
            .limit(limit)
            .all(&txn)
            .await?;
        txn.commit().await?;
        let records = rows
            .into_iter()
            .map(|row| {
                Ok(EventRecord {
                    seq: row.seq,
                    event: serde_json::from_str(&row.payload)?,
                })
            })
            .collect::<Result<_, StoreError>>()?;
        Ok(EventBatch::Events(records))
    }

    /// A receiver that changes whenever new events are committed.
    #[must_use]
    pub fn subscribe_events(&self) -> watch::Receiver<i64> {
        self.events.latest.subscribe()
    }

    /// Deletes events older than the retention period and remembers how far it deleted.
    pub(crate) async fn prune_events(&self, now: Timestamp) -> Result<(), StoreError> {
        let cutoff = Timestamp::from_millis(now.as_millis() - RETENTION_MILLIS);
        let txn = self.begin_write().await?;
        let newest_expired = event::Entity::find()
            .select_only()
            .column_as(event::Column::Seq.max(), "seq")
            .filter(event::Column::OccurredAt.lt(cutoff))
            .into_tuple::<Option<i64>>()
            .one(&txn)
            .await?
            .flatten();
        if let Some(seq) = newest_expired {
            event::Entity::delete_many()
                .filter(event::Column::Seq.lte(seq))
                .exec(&txn)
                .await?;
            schema_info::Entity::update_many()
                .col_expr(schema_info::Column::EventsPrunedThrough, Expr::value(seq))
                .filter(schema_info::Column::Id.eq(schema_info::ROW_ID))
                .exec(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }
}

async fn max_seq<C: ConnectionTrait>(db: &C) -> Result<i64, sea_orm::DbErr> {
    let max = event::Entity::find()
        .select_only()
        .column_as(event::Column::Seq.max(), "seq")
        .into_tuple::<Option<i64>>()
        .one(db)
        .await?
        .flatten();
    // After pruning the table can be empty while events existed; the cursor then stays at
    // the pruned position.
    match max {
        Some(seq) => Ok(seq),
        None => Ok(schema_info::Entity::find_by_id(schema_info::ROW_ID)
            .one(db)
            .await?
            .map_or(0, |info| info.events_pruned_through)),
    }
}
