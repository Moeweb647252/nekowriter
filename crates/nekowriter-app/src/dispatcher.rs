//! The dispatcher: sends queued runs' requests to the model service, one request at a time
//! (the global model permit, `docs/product-spec.md` §8.2).
//!
//! A round reads the run's input from the store, has the agent build and check the
//! request, seals it in the store, streams the answer (publishing previews), has the agent
//! classify it, and records the outcome in the store. Every store step re-checks the run, so
//! a run that changed in between is never advanced by a stale round.

use std::time::Instant;

use nekowriter_agent::{
    client::{ChatClient, Completion, request_body},
    context,
    prompts::builtin,
    render,
    round::{self, RoundOutcome},
    stage::{self, Contract, PromptTexts},
};
use nekowriter_core::{
    call::CallRequest,
    ids::{ModelRequestId, RunId},
    model::EffectiveModel,
    prompt::PromptRole,
    run::{RunHalt, RunStatus},
    run_message::TranscriptMessage,
};
use nekowriter_store::{
    DispatchInput, DispatchOutcome, RecordedAnswer, RoundRecord, SealedRequest, Store, StoreError,
    TranscriptEntry,
};
use serde_json::Value;
use tokio::task::JoinHandle;

use crate::{lifecycle::Shutdown, preview::Previews};

pub(crate) struct Dispatcher {
    pub(crate) store: Store,
    pub(crate) client: ChatClient,
    pub(crate) previews: Previews,
    pub(crate) shutdown: Shutdown,
}

/// A request that is ready to seal and send.
struct Prepared {
    model: EffectiveModel,
    contract: Contract,
    body: Value,
    sealed: SealedRequest,
}

impl Dispatcher {
    /// Runs until shutdown. A request in flight at shutdown is abandoned; the shutdown
    /// marks its run interrupted.
    pub(crate) fn spawn(self) -> JoinHandle<()> {
        tokio::spawn(self.run())
    }

    async fn run(self) {
        let mut notifications = self.store.subscribe_events();
        loop {
            if self.shutdown.is_triggered() {
                return;
            }
            // Mark the current notification as seen before looking, so a run queued after
            // the look still wakes the wait below.
            notifications.borrow_and_update();
            match self.store.next_queued_run().await {
                Ok(Some(run_id)) => {
                    if self.round_or_fail(run_id).await {
                        continue;
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::error!(%error, "the dispatcher could not look for queued runs");
                }
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

    /// Runs one round. When the store fails, the run fails with an internal error so the
    /// user sees it, instead of staying queued. Returns whether the dispatcher can go on
    /// without waiting for a change.
    async fn round_or_fail(&self, run_id: RunId) -> bool {
        let Err(error) = self.round(run_id).await else {
            return true;
        };
        tracing::error!(run = %run_id, %error, "a run's round failed");
        match self.store.fail_run(run_id, RunHalt::Internal).await {
            Ok(()) => true,
            Err(error) => {
                tracing::error!(run = %run_id, %error, "the run could not be marked failed");
                false
            }
        }
    }

    async fn round(&self, run_id: RunId) -> Result<(), StoreError> {
        let input = self.store.dispatch_input(run_id).await?;
        let prepared = match prepare(&input) {
            Ok(prepared) => prepared,
            Err(halt) => {
                tracing::info!(run = %run_id, ?halt, "the run cannot make its next request");
                return self.store.fail_run(run_id, halt).await;
            }
        };
        let Prepared {
            model,
            contract,
            body,
            sealed,
        } = prepared;
        let request_id = match self.store.dispatch(&input, sealed).await? {
            DispatchOutcome::Dispatched { request_id } => request_id,
            DispatchOutcome::Changed => return Ok(()),
        };
        let started = Instant::now();
        self.previews.start(run_id, input.version_id, request_id);
        // A stop or the shutdown abandons the request; the store has recorded why.
        let result = tokio::select! {
            result = self.client.complete(
                &model.service,
                &body,
                |delta| self.previews.add(delta),
            ) => Some(result),
            () = self.run_leaves_running(run_id) => None,
            () = self.shutdown.triggered() => None,
        };
        self.previews.clear();
        let elapsed_ms = started.elapsed().as_millis();
        match result {
            None => Ok(()),
            Some(Ok(completion)) => {
                self.record(run_id, request_id, &contract, completion, elapsed_ms)
                    .await
            }
            Some(Err(error)) => {
                tracing::warn!(
                    run = %run_id,
                    request = %request_id,
                    elapsed_ms,
                    %error,
                    "the model request failed"
                );
                self.store
                    .record_request_failure(request_id, error.failure())
                    .await
            }
        }
    }

    /// Completes once the run is no longer running, for example because the user stopped
    /// it.
    async fn run_leaves_running(&self, run_id: RunId) {
        let mut notifications = self.store.subscribe_events();
        loop {
            match self.store.run_status(run_id).await {
                Ok(RunStatus::Running) => {}
                Ok(_) => return,
                Err(error) => {
                    tracing::error!(run = %run_id, %error, "the run's status could not be read");
                }
            }
            if notifications.changed().await.is_err() {
                return std::future::pending().await;
            }
        }
    }

    async fn record(
        &self,
        run_id: RunId,
        request_id: ModelRequestId,
        contract: &Contract,
        completion: Completion,
        elapsed_ms: u128,
    ) -> Result<(), StoreError> {
        let round = match round::classify(contract, &completion.turn) {
            RoundOutcome::Call { call, request } => RoundRecord::Call { call, request },
            RoundOutcome::ProtocolError { calls, correction } => {
                RoundRecord::ProtocolError { calls, correction }
            }
        };
        tracing::info!(
            run = %run_id,
            request = %request_id,
            stage = ?contract.stage,
            elapsed_ms,
            outcome = match &round {
                RoundRecord::Call {
                    request: CallRequest::Edit { .. },
                    ..
                } => "edit",
                RoundRecord::Call {
                    request: CallRequest::Submit { .. },
                    ..
                } => "submission",
                RoundRecord::ProtocolError { .. } => "protocol_error",
            },
            "the model answered"
        );
        let answer = RecordedAnswer {
            content: completion.turn.content,
            tool_calls: completion.turn.tool_calls,
            finish_reason: completion.finish_reason,
            usage: completion.usage,
            raw_usage: completion.raw_usage.map(|usage| usage.to_string()),
            provider_request_id: completion.provider_request_id,
        };
        self.store.record_answer(request_id, answer, round).await
    }
}

/// Builds the run's next request from its input, or says why it cannot be made. Pending
/// messages are rendered here, once; sealing stores the rendering.
fn prepare(input: &DispatchInput) -> Result<Prepared, RunHalt> {
    let model = input
        .model
        .clone()
        .resolve()
        .map_err(|problem| RunHalt::ModelNotReady { problem })?;
    let key = stage::prompt_key(&input.stage);
    let prompts = PromptTexts {
        system: builtin(key, PromptRole::System),
        user: builtin(key, PromptRole::User),
    };
    let stage = stage::build(&input.stage, input.writing_language, prompts).map_err(|error| {
        tracing::error!(run = %input.run_id, ?key, %error, "a built-in prompt does not render");
        RunHalt::Internal
    })?;
    let mut rendered = Vec::new();
    let transcript: Vec<TranscriptMessage> = input
        .messages
        .iter()
        .map(|entry| match entry {
            TranscriptEntry::Rendered(message) => message.clone(),
            TranscriptEntry::Pending { id, message } => {
                let message = render::unsealed(message, stage.contract.submission.name);
                rendered.push((*id, message.clone()));
                message
            }
        })
        .collect();
    let folds = render::folds(&input.folds);
    let budget = model.profile.input_budget();
    let planned = context::plan(&stage, folds, &transcript, budget).map_err(|too_large| {
        RunHalt::ContextLimitExceeded {
            estimated_tokens: too_large.estimated_tokens,
            limit_tokens: too_large.limit_tokens,
        }
    })?;
    let body = request_body(&model, &planned);
    Ok(Prepared {
        sealed: SealedRequest {
            rendered,
            body: body.to_string(),
            estimated_input_tokens: planned.estimated_input_tokens,
        },
        model,
        contract: stage.contract,
        body,
    })
}
