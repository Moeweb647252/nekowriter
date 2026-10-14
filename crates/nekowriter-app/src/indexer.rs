//! The indexer: embeds knowledge entries in the background (`docs/product-spec.md` §12.2).
//!
//! It takes one index task at a time from the store, splits the entry into chunks, embeds
//! them and hands the outcome back. The store drops outcomes for entries or settings that
//! changed meanwhile, so a slow task never overwrites a newer one.

use nekowriter_agent::client::ChatClient;
use nekowriter_core::knowledge::chunks;
use nekowriter_store::{IndexTask, Store};
use tokio::task::JoinHandle;

use crate::lifecycle::Shutdown;

pub(crate) struct Indexer {
    pub(crate) store: Store,
    pub(crate) client: ChatClient,
    pub(crate) shutdown: Shutdown,
}

impl Indexer {
    /// Runs until shutdown. A task in flight at shutdown is queued again at the next start.
    pub(crate) fn spawn(self) -> JoinHandle<()> {
        tokio::spawn(self.run())
    }

    async fn run(self) {
        let mut notifications = self.store.subscribe_events();
        loop {
            if self.shutdown.is_triggered() {
                return;
            }
            // Mark the current notification as seen before looking, so a task queued after
            // the look still wakes the wait below.
            notifications.borrow_and_update();
            match self.store.next_index_task().await {
                Ok(Some(task)) => {
                    self.index(task).await;
                    continue;
                }
                Ok(None) => {}
                Err(error) => tracing::error!(%error, "the indexer could not look for tasks"),
            }
            tokio::select! {
                changed = notifications.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                () = self.shutdown.triggered() => return,
            }
        }
    }

    async fn index(&self, task: IndexTask) {
        let texts = chunks(&task.content);
        let embedded = tokio::select! {
            embedded = self.client.embed(&task.service, &task.model, &texts) => embedded,
            () = self.shutdown.triggered() => return,
        };
        let (outcome, input_tokens) = match embedded {
            Ok(embeddings) => (
                Ok(texts.into_iter().zip(embeddings.vectors).collect()),
                embeddings.input_tokens,
            ),
            Err(error) => {
                tracing::warn!(entry = %task.entry_id, %error, "an entry could not be indexed");
                (Err(error.failure()), None)
            }
        };
        if let Err(error) = self
            .store
            .finish_index_task(&task, outcome, input_tokens)
            .await
        {
            tracing::error!(entry = %task.entry_id, %error, "an index could not be stored");
        }
    }
}
