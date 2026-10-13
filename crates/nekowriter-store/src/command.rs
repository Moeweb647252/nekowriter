//! The idempotent command envelope: the only way to change stored data on behalf of a user.
//!
//! A command is identified by the `CommandId` the client generated for one user action.
//! [`Store::begin_command`] either replays the recorded outcome of an earlier attempt or
//! opens a write transaction. [`CommandTx::commit`] records the outcome, writes the
//! command's events and commits in one transaction, then wakes event subscribers.
//! Dropping a `CommandTx` without committing rolls everything back and records nothing, so
//! a failed command can be retried with the same ID.

use crate::{Store, StoreError, digest::sha256_hex, entity::command_receipt, events};
use nekowriter_core::{event::Event, ids::CommandId, time::Timestamp};
use sea_orm::{ActiveModelTrait, DatabaseTransaction, EntityTrait, Set, SqlErr};
use serde::{Serialize, de::DeserializeOwned};

/// The result of starting a command.
pub(crate) enum Begun<T> {
    /// The command already succeeded; this is its recorded output.
    Replayed(T),
    /// The command has not run yet; carry it out inside this transaction.
    Fresh(CommandTx),
}

/// An open write transaction for one command.
pub(crate) struct CommandTx {
    store: Store,
    txn: DatabaseTransaction,
    id: CommandId,
    kind: &'static str,
    input_hash: String,
    now: Timestamp,
    events: Vec<Event>,
}

impl Store {
    /// Starts command `kind` (for example `project.create`) with the given input.
    ///
    /// A retry is recognized by its command ID. It must repeat the same kind and input,
    /// otherwise it fails with [`StoreError::IdempotencyConflict`].
    pub(crate) async fn begin_command<T, I>(
        &self,
        id: CommandId,
        kind: &'static str,
        input: &I,
    ) -> Result<Begun<T>, StoreError>
    where
        T: DeserializeOwned,
        I: Serialize,
    {
        let input_hash = input_hash(kind, input)?;
        if let Some(output) = self.replay::<T>(id, kind, &input_hash).await? {
            return Ok(Begun::Replayed(output));
        }
        let txn = self.begin_write().await?;
        Ok(Begun::Fresh(CommandTx {
            store: self.clone(),
            txn,
            id,
            kind,
            input_hash,
            now: Timestamp::now(),
            events: Vec::new(),
        }))
    }

    async fn replay<T: DeserializeOwned>(
        &self,
        id: CommandId,
        kind: &str,
        input_hash: &str,
    ) -> Result<Option<T>, StoreError> {
        let Some(receipt) = command_receipt::Entity::find_by_id(id)
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };
        if receipt.kind != kind || receipt.input_hash != input_hash {
            return Err(StoreError::IdempotencyConflict(id));
        }
        Ok(Some(serde_json::from_str(&receipt.output)?))
    }
}

impl CommandTx {
    /// The transaction to read and write through.
    pub(crate) fn db(&self) -> &DatabaseTransaction {
        &self.txn
    }

    /// The time this command takes effect; use it for every timestamp the command writes.
    pub(crate) fn now(&self) -> Timestamp {
        self.now
    }

    /// Records an event to be written with the command.
    pub(crate) fn emit(&mut self, event: Event) {
        self.events.push(event);
    }

    /// Records the output, writes the events and commits.
    ///
    /// If a concurrent attempt of the same command committed first, this attempt is rolled
    /// back and the other attempt's output is returned.
    pub(crate) async fn commit<T>(self, output: T) -> Result<T, StoreError>
    where
        T: Serialize + DeserializeOwned,
    {
        let receipt = command_receipt::ActiveModel {
            command_id: Set(self.id),
            kind: Set(self.kind.to_owned()),
            input_hash: Set(self.input_hash.clone()),
            output: Set(serde_json::to_string(&output)?),
            committed_at: Set(self.now),
        };
        if let Err(error) = receipt.insert(&self.txn).await {
            if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) {
                self.txn.rollback().await?;
                return self
                    .store
                    .replay(self.id, self.kind, &self.input_hash)
                    .await?
                    .ok_or_else(|| {
                        StoreError::Corrupt(format!(
                            "command {} conflicted with a receipt that does not exist",
                            self.id
                        ))
                    });
            }
            return Err(error.into());
        }
        let last_seq = events::append(&self.txn, &self.events, self.now).await?;
        self.txn.commit().await?;
        if let Some(seq) = last_seq {
            self.store.events.notify(seq);
        }
        Ok(output)
    }
}

fn input_hash<I: Serialize>(kind: &str, input: &I) -> Result<String, StoreError> {
    let input = serde_json::to_vec(input)?;
    Ok(sha256_hex([kind.as_bytes(), &[0], &input]))
}
