//! The runner: `drive(session_id)` — the loop that turns a persisted session into model requests
//! and tool executions (spec/03 §5).
//!
//! ```text
//! loop:
//!   s ← load(session_id); if s.status ≠ running: return
//!   if cancel_token.cancelled: transition(stopped | interrupted); return
//!   if tc ← first pending tool call: step_tool(s, tc); continue
//!   match classify_tail(s):
//!     NeedsModel        → pause check → rollover check → seal_latest_batch → step_model
//!     AssistantTextOnly → handle_text_only
//! ```
//!
//! Invariants this module implements:
//! - the runner is the only writer of a running session (D-034), and every session write is
//!   column-wise and guarded by the expected status;
//! - a call executes and finalizes in **one** transaction (D-033), so a call found in `executing`
//!   after a crash provably had no effect;
//! - `prepare` runs outside any transaction and is cancellable (D-024);
//! - persisted messages and sealed tool results are never modified (spec/03 §1.3).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::agent::tools::research::{
    MAX_ENTRIES as MAX_RESEARCH_ENTRIES, MAX_TOOL_CALLS as MAX_RESEARCH_TOOL_CALLS,
};
use crate::db::entities::{
    agent_inbox, agent_messages, agent_sessions, agent_tool_calls, books, chapters, tracks,
};
use crate::db::json::{
    self, Appendix, AppendixKind, CompletionKind, MessageKind, MessageRole, Phase, RequestMeta,
    SessionError, SessionKind, SessionStatus, StoredBlock, ToolCallStatus, ToolEffects, Usage,
    ViewSnapshot,
};
use crate::error::{AppError, ErrorCode};
use crate::events::{DeltaKind, ServerEvent};
use crate::prompts::PromptSet;
use crate::state::AppState;

use super::manager::{CancelReason, lock};
use super::preview::PreviewExtractor;
use super::seal;
use super::session::{self, TransitionCause};
use super::tools::spec::validate_input;
use super::tools::{
    PrepareCtx, Prepared, ToolCtx, ToolError, ToolErrorKind, ToolOutcome, Visibility,
    allowed_tools, cap_result, is_permitted, phase_label,
};

/// The frozen tool names of a session (spec/03 §1.2).
fn frozen_tool_names(session: &agent_sessions::Model) -> Vec<String> {
    super::context::frozen_tool_names(session)
}

/// How many consecutive tool errors trip the circuit breaker (spec/03 §5.2, §5.7).
pub const TOOL_ERROR_STREAK_LIMIT: i32 = 5;

/// How many unanswered nudges fail the session (spec/03 §5.6).
pub const NUDGE_LIMIT: i32 = 2;

/// Retrieval calls before the writer's retrieval-budget notices start (spec/03 §5.7).
pub const RETRIEVAL_BUDGET: i32 = 30;

/// The maximum length of an `input_text` shown by the API (spec/10 §2.8).
pub const INPUT_TEXT_DISPLAY_LIMIT: usize = 20_000;

/// How often streamed deltas are flushed as `session.delta` events (spec/03 §5.5).
const DELTA_FLUSH: Duration = Duration::from_millis(50);

/// What a cancelled runner must do (spec/03 §5.1, spec/01 §5).
///
/// The cancellation token carries a **reason**: `stop` gives `stopped`, `shutdown` gives
/// `interrupted` (exactly as startup recovery would leave the session), and `abandon` makes the
/// runner exit without a transition because the abandon operation writes `abandoned` itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelledTransition {
    /// Apply this status with this cause: `shutdown` → `interrupted`, `stop` → `stopped`.
    Transition(SessionStatus, TransitionCause),
    /// `abandon`: the operation writes `abandoned`, so the runner just exits.
    Exit,
}

/// Everything one runner needs.
pub struct RunnerCtx {
    pub state: AppState,
    pub session_id: i32,
    pub cancel: CancellationToken,
    /// The reason cell owned by `SessionManager`, **not** its current value: `cancel` writes it
    /// after the task started, so a value captured at spawn time would always be `None` and every
    /// cancellation (shutdown and abandon included) would look like a Stop (spec/03 §5.1).
    pub cancel_reason: Arc<Mutex<Option<CancelReason>>>,
}

impl RunnerCtx {
    /// What this runner must do if it was cancelled, or `None` when it was not (spec/03 §5.1).
    pub fn cancelled_transition(&self) -> Option<CancelledTransition> {
        let reason = (*lock(&self.cancel_reason))?;
        Some(match reason {
            CancelReason::Shutdown => CancelledTransition::Transition(
                SessionStatus::Interrupted,
                TransitionCause::Shutdown,
            ),
            CancelReason::Stop => {
                CancelledTransition::Transition(SessionStatus::Stopped, TransitionCause::Stop)
            }
            CancelReason::Abandon => CancelledTransition::Exit,
        })
    }
}

/// The tail of the conversation (spec/03 §5.1 `classify_tail`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tail {
    NeedsModel,
    AssistantTextOnly,
}

/// Drives one session until it stops running (spec/03 §5.1).
pub async fn drive(ctx: RunnerCtx) -> Result<(), AppError> {
    loop {
        // Re-reading the row at every loop step is what makes a concurrent pause or granularity
        // change impossible to lose (spec/03 §14, test 21).
        let session = session::load(ctx.state.db().conn(), ctx.session_id).await?;
        if session.status != SessionStatus::Running.as_str() {
            return Ok(());
        }
        if ctx.cancel.is_cancelled() {
            let Some(action) = ctx.cancelled_transition() else {
                // Defensive: `is_cancelled()` guarantees a reason, but a token set without one
                // must not strand the session.
                tracing::warn!(
                    session_id = ctx.session_id,
                    "cancel token set without a reason"
                );
                return Ok(());
            };
            match action {
                // Abandon writes `abandoned` itself; the runner just exits (spec/03 §5.1).
                CancelledTransition::Exit => return Ok(()),
                CancelledTransition::Transition(status, cause) => {
                    let events =
                        session::transition(ctx.state.db().conn(), &session, status, cause).await?;
                    ctx.state.events().publish_all(events);
                }
            }
            return Ok(());
        }

        if let Some(call) = first_pending_call(ctx.state.db().conn(), ctx.session_id).await? {
            step_tool(&ctx, &session, call).await?;
            continue;
        }

        match classify_tail(ctx.state.db().conn(), ctx.session_id).await? {
            Tail::NeedsModel => {
                if session.pause_requested != 0 {
                    let events = session::transition(
                        ctx.state.db().conn(),
                        &session,
                        SessionStatus::Paused,
                        TransitionCause::PauseBeforeRequest,
                    )
                    .await?;
                    ctx.state.events().publish_all(events);
                    return Ok(());
                }
                if rollover_needed(&session) {
                    return super::context::rollover(&ctx.state, &session).await;
                }
                seal_latest_batch(&ctx, &session).await?;
                step_model(&ctx, &session).await?;
            }
            Tail::AssistantTextOnly => handle_text_only(&ctx, &session).await?,
        }
    }
}

// ───────────────────────────── conversation reads ─────────────────────────────

/// The messages of a session, oldest first.
pub async fn messages_of(
    conn: &impl ConnectionTrait,
    session_id: i32,
) -> Result<Vec<agent_messages::Model>, AppError> {
    Ok(agent_messages::Entity::find()
        .filter(agent_messages::Column::SessionId.eq(session_id))
        .order_by_asc(agent_messages::Column::Seq)
        .all(conn)
        .await?)
}

/// The tool calls of one message, in `call_index` order.
pub async fn calls_of(
    conn: &impl ConnectionTrait,
    message_id: i32,
) -> Result<Vec<agent_tool_calls::Model>, AppError> {
    Ok(agent_tool_calls::Entity::find()
        .filter(agent_tool_calls::Column::MessageId.eq(message_id))
        .order_by_asc(agent_tool_calls::Column::CallIndex)
        .all(conn)
        .await?)
}

/// The first `pending` tool call of a session (spec/03 §5.1 step A).
async fn first_pending_call(
    conn: &impl ConnectionTrait,
    session_id: i32,
) -> Result<Option<agent_tool_calls::Model>, AppError> {
    Ok(agent_tool_calls::Entity::find()
        .filter(agent_tool_calls::Column::SessionId.eq(session_id))
        .filter(agent_tool_calls::Column::Status.eq(ToolCallStatus::Pending.as_str()))
        .order_by_asc(agent_tool_calls::Column::Id)
        .one(conn)
        .await?)
}

/// `classify_tail` (spec/03 §5.1).
pub async fn classify_tail(conn: &impl ConnectionTrait, session_id: i32) -> Result<Tail, AppError> {
    let Some(last) = messages_of(conn, session_id).await?.pop() else {
        return Ok(Tail::NeedsModel);
    };
    if last.role == MessageRole::User.as_str() {
        return Ok(Tail::NeedsModel);
    }
    let calls = calls_of(conn, last.id).await?;
    if calls.is_empty() {
        return Ok(Tail::AssistantTextOnly);
    }
    Ok(Tail::NeedsModel)
}

/// The next `seq` of a session's messages (contiguous, spec/02 §5 invariant 6).
pub async fn next_seq(conn: &impl ConnectionTrait, session_id: i32) -> Result<i32, AppError> {
    let max = agent_messages::Entity::find()
        .filter(agent_messages::Column::SessionId.eq(session_id))
        .order_by_desc(agent_messages::Column::Seq)
        .one(conn)
        .await?
        .map(|row| row.seq);
    Ok(max.map(|value| value + 1).unwrap_or(0))
}

/// Inserts one user-side message (kickoff, author message, notice, nudge).
pub async fn insert_user_message(
    tx: &sea_orm::DatabaseTransaction,
    session_id: i32,
    kind: MessageKind,
    blocks: Vec<StoredBlock>,
) -> Result<agent_messages::Model, AppError> {
    let seq = next_seq(tx, session_id).await?;
    let row = agent_messages::ActiveModel {
        session_id: Set(session_id),
        seq: Set(seq),
        role: Set(MessageRole::User.as_str().to_owned()),
        kind: Set(kind.as_str().to_owned()),
        content: Set(
            serde_json::to_string(&blocks).map_err(|err| AppError::internal(err.to_string()))?
        ),
        provider_payload: Set(None),
        request_meta: Set(None),
        stop_reason: Set(None),
        usage: Set(None),
        created_at: Set(Utc::now()),
        ..Default::default()
    }
    .insert(tx)
    .await?;
    Ok(row)
}

/// Inserts the session's kickoff message (`seq 0`, kind `kickoff`, spec/03 §4 step 4).
pub async fn insert_kickoff(
    tx: &sea_orm::DatabaseTransaction,
    session_id: i32,
    blocks: Vec<StoredBlock>,
) -> Result<agent_messages::Model, AppError> {
    insert_user_message(tx, session_id, MessageKind::Kickoff, blocks).await
}

/// The pending inbox rows of a session, in arrival order.
pub async fn pending_inbox_rows(
    conn: &impl ConnectionTrait,
    session_id: i32,
) -> Result<Vec<agent_inbox::Model>, AppError> {
    Ok(agent_inbox::Entity::find()
        .filter(agent_inbox::Column::SessionId.eq(session_id))
        .filter(agent_inbox::Column::ConsumedAt.is_null())
        .order_by_asc(agent_inbox::Column::Id)
        .all(conn)
        .await?)
}

// ───────────────────────────── step_model ─────────────────────────────

/// One tool call being streamed.
#[derive(Debug, Default, Clone)]
pub struct StreamedCall {
    pub id: String,
    pub name: String,
    pub input_text: String,
}

/// The streaming sink of one model request: accumulates the turn and publishes coalesced deltas.
struct StreamSink<'a> {
    state: &'a AppState,
    session_id: i32,
    seq: i32,
    preview_fields: &'a BTreeMap<String, &'static str>,
    text: String,
    reasoning: String,
    calls: Vec<StreamedCall>,
    usage: Usage,
    /// `call_index → (tool name, field, extractor, call id)`.
    previews: BTreeMap<u32, (String, &'static str, PreviewExtractor, String)>,
    pending_text: String,
    pending_reasoning: String,
    last_flush: Instant,
}

impl<'a> StreamSink<'a> {
    fn new(
        state: &'a AppState,
        session_id: i32,
        seq: i32,
        preview_fields: &'a BTreeMap<String, &'static str>,
    ) -> Self {
        Self {
            state,
            session_id,
            seq,
            preview_fields,
            text: String::new(),
            reasoning: String::new(),
            calls: Vec::new(),
            usage: Usage::default(),
            previews: BTreeMap::new(),
            pending_text: String::new(),
            pending_reasoning: String::new(),
            last_flush: Instant::now(),
        }
    }

    /// Handles one stream event (spec/03 §5.5 `on_event`).
    fn accept(&mut self, event: nekowriter_llm::StreamEvent) {
        match event {
            nekowriter_llm::StreamEvent::TextDelta { text, .. } => {
                self.text.push_str(&text);
                self.pending_text.push_str(&text);
            }
            nekowriter_llm::StreamEvent::ReasoningDelta { text, .. } => {
                self.reasoning.push_str(&text);
                self.pending_reasoning.push_str(&text);
            }
            nekowriter_llm::StreamEvent::ToolCallStart {
                call_index,
                id,
                name,
            } => {
                self.flush();
                while self.calls.len() <= call_index as usize {
                    self.calls.push(StreamedCall::default());
                }
                self.calls[call_index as usize] = StreamedCall {
                    id: id.clone(),
                    name: name.clone(),
                    input_text: String::new(),
                };
                if let Some(field) = self.preview_fields.get(&name) {
                    self.previews.insert(
                        call_index,
                        (name, *field, PreviewExtractor::new(*field), id),
                    );
                }
            }
            nekowriter_llm::StreamEvent::ToolCallArgsDelta {
                call_index,
                fragment,
            } => {
                if let Some(call) = self.calls.get_mut(call_index as usize) {
                    call.input_text.push_str(&fragment);
                }
                if let Some((name, field, extractor, _)) = self.previews.get_mut(&call_index)
                    && let Some(delta) = extractor.feed(&fragment)
                {
                    self.state
                        .events()
                        .publish(ServerEvent::SessionToolPreview {
                            session_id: self.session_id,
                            seq: self.seq,
                            call_index,
                            name: name.clone(),
                            field: (*field).to_owned(),
                            delta,
                        });
                }
            }
            nekowriter_llm::StreamEvent::Usage(reported) => {
                self.usage.input_tokens = self.usage.input_tokens.max(reported.input_tokens);
                self.usage.cached_input_tokens = self
                    .usage
                    .cached_input_tokens
                    .max(reported.cached_input_tokens);
                self.usage.cache_write_tokens = self
                    .usage
                    .cache_write_tokens
                    .max(reported.cache_write_tokens);
                self.usage.output_tokens = self.usage.output_tokens.max(reported.output_tokens);
                self.usage.reasoning_tokens =
                    self.usage.reasoning_tokens.max(reported.reasoning_tokens);
                self.usage.estimated |= reported.estimated;
            }
        }
        if self.last_flush.elapsed() >= DELTA_FLUSH {
            self.flush();
        }
    }

    /// Publishes the buffered deltas (coalesced, spec/03 §5.5).
    fn flush(&mut self) {
        if !self.pending_text.is_empty() {
            self.state.events().publish(ServerEvent::SessionDelta {
                session_id: self.session_id,
                seq: self.seq,
                kind: DeltaKind::Text,
                index: 0,
                text: std::mem::take(&mut self.pending_text),
            });
        }
        if !self.pending_reasoning.is_empty() {
            self.state.events().publish(ServerEvent::SessionDelta {
                session_id: self.session_id,
                seq: self.seq,
                kind: DeltaKind::Reasoning,
                index: 0,
                text: std::mem::take(&mut self.pending_reasoning),
            });
        }
        self.last_flush = Instant::now();
    }

    /// Drops the partial output of a failed attempt (spec/03 §5.5).
    fn reset(&mut self) {
        self.text.clear();
        self.reasoning.clear();
        self.calls.clear();
        self.usage = Usage::default();
        self.previews.clear();
        self.pending_text.clear();
        self.pending_reasoning.clear();
    }
}

/// Why a model step stopped early.
enum StepStop {
    Cancelled,
    Failed(Box<SessionError>),
}

/// Builds and runs the model request of one step (spec/03 §5.5).
async fn step_model(ctx: &RunnerCtx, session: &agent_sessions::Model) -> Result<(), AppError> {
    let state = &ctx.state;
    let prompts = resolve_prompts(state, session).await?;
    let request = super::context::build_request(state.db().conn(), session, &prompts).await?;
    let seq = next_seq(state.db().conn(), session.id).await?;
    let preview_fields = preview_fields_of(state);

    let mut sink = StreamSink::new(state, session.id, seq, &preview_fields);
    let client = provider_client(state, session).await?;
    let outcome = run_with_retry(ctx, &client, &request, &mut sink).await;

    match outcome {
        Ok(turn) => {
            let events = persist_assistant(state, session, seq, &turn, &sink, &request).await?;
            state.events().publish_all(events);
            Ok(())
        }
        Err(StepStop::Cancelled) => {
            // Drop the stream, persist nothing (spec/03 §5.5).
            sink.flush();
            let fresh = session::load(state.db().conn(), session.id).await?;
            if fresh.status == SessionStatus::Running.as_str() {
                // The reason decides the status: a shutdown during a stream gives `interrupted`,
                // exactly as startup recovery would (spec/03 §5.1, spec/01 §5). The fallback keeps
                // the Stop behaviour for a defensive token without a reason.
                let (status, cause) = match ctx.cancelled_transition() {
                    Some(CancelledTransition::Transition(status, cause)) => (status, cause),
                    Some(CancelledTransition::Exit) => return Ok(()),
                    None => (SessionStatus::Stopped, TransitionCause::Stop),
                };
                let events = session::transition(state.db().conn(), &fresh, status, cause).await?;
                state.events().publish_all(events);
            }
            Ok(())
        }
        Err(StepStop::Failed(error)) => {
            sink.flush();
            let fresh = session::load(state.db().conn(), session.id).await?;
            // A context overflow rolls over when the session is eligible (spec/03 §9); otherwise
            // it fails with `provider.context_overflow` and suggests a larger model. The runner
            // has not persisted anything, so the rollover would retry the same request.
            if error.code == CONTEXT_OVERFLOW_CODE {
                let mut overflowed = fresh.clone();
                overflowed.error = Some(crate::db::json::to_json(&*error).unwrap_or_default());
                if rollover_needed(&overflowed) {
                    return super::context::rollover(state, &fresh).await;
                }
            }
            let mut events = Vec::new();
            if fresh.status == SessionStatus::Running.as_str() {
                session::set_error_running(state.db().conn(), session.id, &error).await?;
                let reloaded = session::load(state.db().conn(), session.id).await?;
                events = session::transition(
                    state.db().conn(),
                    &reloaded,
                    SessionStatus::Failed,
                    TransitionCause::Failed,
                )
                .await?;
            }
            state.events().publish_all(events);
            Ok(())
        }
    }
}

/// Runs one request with the retry ladder of spec/04 §6, raced against the stop token.
async fn run_with_retry(
    ctx: &RunnerCtx,
    client: &nekowriter_llm::ProviderClient,
    request: &nekowriter_llm::ChatRequest,
    sink: &mut StreamSink<'_>,
) -> Result<nekowriter_llm::AssistantTurn, StepStop> {
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let mut on_event = |event: nekowriter_llm::StreamEvent| sink.accept(event);
        let raced = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => None,
            result = client.run(request, &mut on_event) => Some(result),
        };
        let Some(result) = raced else {
            return Err(StepStop::Cancelled);
        };
        match result {
            Ok(turn) => {
                sink.flush();
                return Ok(turn);
            }
            Err(error) => {
                if error.retryable() && attempt < nekowriter_llm::retry::MAX_ATTEMPTS {
                    let retry_after = match &error {
                        nekowriter_llm::LlmError::RateLimited { retry_after } => *retry_after,
                        _ => None,
                    };
                    let wait = nekowriter_llm::retry::backoff_delay(
                        attempt,
                        retry_after,
                        u64::from(attempt),
                    )
                    .unwrap_or(Duration::from_secs(2));
                    tracing::warn!(
                        session_id = ctx.session_id,
                        attempt,
                        wait_seconds = wait.as_secs_f64(),
                        error = %error,
                        "retrying the model request"
                    );
                    // spec/04 §6: `session.updated` carries `retrying: {attempt, wait_seconds,
                    // reason}` so the UI can show "Provider overloaded — retrying in 8 s". The
                    // state is per-attempt and in flight, so it is published here rather than
                    // stored: `SessionDto.retrying` is `None` everywhere else (spec/10 §2.8).
                    publish_retrying(ctx, attempt, wait, &error).await;
                    tokio::select! {
                        biased;
                        _ = ctx.cancel.cancelled() => return Err(StepStop::Cancelled),
                        _ = tokio::time::sleep(wait) => {}
                    }
                    // Discard the partial output of the failed attempt.
                    sink.reset();
                    continue;
                }
                return Err(StepStop::Failed(Box::new(classify_llm_error(&error))));
            }
        }
    }
}

/// Maps a provider failure to the session error of spec/03 §5.7.
fn classify_llm_error(error: &nekowriter_llm::LlmError) -> SessionError {
    use nekowriter_llm::LlmError;
    let (code, retryable) = match error {
        LlmError::Network(_) | LlmError::Timeout(_) => ("provider.unavailable", true),
        LlmError::RateLimited { .. } | LlmError::Overloaded | LlmError::Server(_, _) => {
            ("provider.unavailable", true)
        }
        LlmError::Auth(_) => ("provider.auth", false),
        LlmError::ContextOverflow(_) => (CONTEXT_OVERFLOW_CODE, false),
        LlmError::BadRequest(_) | LlmError::NotFound(_) => ("provider.bad_request", false),
        LlmError::Provider(_) | LlmError::Protocol(_) => ("provider.bad_request", false),
        LlmError::Cancelled => ("internal", false),
    };
    SessionError {
        code: code.to_owned(),
        message: error.to_string(),
        retryable,
        at: Utc::now(),
    }
}

/// Persists one assistant turn, its tool calls and the session counters in one transaction
/// (spec/03 §5.5), then handles the stop reason.
async fn persist_assistant(
    state: &AppState,
    session: &agent_sessions::Model,
    seq: i32,
    turn: &nekowriter_llm::AssistantTurn,
    sink: &StreamSink<'_>,
    _request: &nekowriter_llm::ChatRequest,
) -> Result<Vec<ServerEvent>, AppError> {
    use nekowriter_llm::StopReason;

    let prompts = resolve_prompts(state, session).await?;
    let meta = RequestMeta {
        phase: session::phase_of(session),
        granularity: session::granularity_of(session),
    };

    // A max_tokens truncation finishes the calls whose arguments are not valid JSON as
    // `error.output_truncated`; the valid ones run normally (spec/03 §5.5).
    let mut truncated: Vec<usize> = Vec::new();
    if matches!(turn.stop_reason, StopReason::MaxTokens) {
        for (index, call) in turn.tool_calls.iter().enumerate() {
            if serde_json::from_str::<Value>(&call.input_text).is_err() {
                truncated.push(index);
            }
        }
    }
    let truncation_texts: Vec<String> = truncated
        .iter()
        .map(|_| ToolOutcome::error(&prompts, &ToolError::new(ToolErrorKind::OutputTruncated)).text)
        .collect();

    let turn_owned = turn.clone();
    let session_owned = session.clone();
    let (message, rows) = crate::services::write(state.db(), move |tx| {
        let turn = turn_owned;
        let session = session_owned;
        let truncated = truncated.clone();
        let truncation_texts = truncation_texts.clone();
        Box::pin(async move {
            let now = Utc::now();
            let mut blocks = Vec::new();
            if !turn.reasoning.is_empty() {
                blocks.push(StoredBlock::Reasoning {
                    text: turn.reasoning.clone(),
                });
            }
            if !turn.text.is_empty() {
                blocks.push(StoredBlock::Text {
                    text: turn.text.clone(),
                    cache: None,
                });
            }
            for call in &turn.tool_calls {
                blocks.push(StoredBlock::ToolCall {
                    call_id: call.id.clone(),
                    name: call.name.clone(),
                    input_text: call.input_text.clone(),
                });
            }
            let payload = json::to_json(&to_stored_payload(&turn.payload))
                .map_err(|err| AppError::internal(err.to_string()))?;
            let message = agent_messages::ActiveModel {
                session_id: Set(session.id),
                seq: Set(seq),
                role: Set(MessageRole::Assistant.as_str().to_owned()),
                kind: Set(MessageKind::Assistant.as_str().to_owned()),
                content: Set(
                    json::to_json(&blocks).map_err(|err| AppError::internal(err.to_string()))?
                ),
                provider_payload: Set(Some(payload)),
                request_meta: Set(Some(
                    json::to_json(&meta).map_err(|err| AppError::internal(err.to_string()))?,
                )),
                stop_reason: Set(Some(turn.stop_reason.as_str().to_owned())),
                usage: Set(Some(
                    json::to_json(&turn.usage)
                        .map_err(|err| AppError::internal(err.to_string()))?,
                )),
                created_at: Set(now),
                ..Default::default()
            }
            .insert(tx)
            .await?;

            let mut rows = Vec::new();
            for (index, call) in turn.tool_calls.iter().enumerate() {
                let truncated_index = truncated.iter().position(|value| *value == index);
                let is_truncated = truncated_index.is_some();
                let mut appendices: Vec<Appendix> = Vec::new();
                if is_truncated {
                    // A truncated call never runs again: seal it immediately with its error.
                    appendices.push(Appendix {
                        kind: AppendixKind::EffectNotice,
                        text: String::new(),
                        inbox_id: None,
                    });
                }
                let raw_result_text =
                    truncated_index.and_then(|position| truncation_texts.get(position).cloned());
                let status = if is_truncated {
                    ToolCallStatus::Error
                } else {
                    ToolCallStatus::Pending
                };
                let row = agent_tool_calls::ActiveModel {
                    session_id: Set(session.id),
                    message_id: Set(message.id),
                    call_index: Set(index as i32),
                    provider_call_id: Set(call.id.clone()),
                    name: Set(call.name.clone()),
                    input: Set(call.input_text.clone()),
                    status: Set(status.as_str().to_owned()),
                    raw_result_text: Set(raw_result_text),
                    appendices: Set(json::to_json(&appendices)
                        .map_err(|err| AppError::internal(err.to_string()))?),
                    result_text: Set(None),
                    is_error: Set(i32::from(is_truncated)),
                    ui: Set(None),
                    effects: Set(None),
                    sealed: Set(0),
                    created_at: Set(now),
                    started_at: Set(None),
                    finished_at: Set(if is_truncated { Some(now) } else { None }),
                    ..Default::default()
                }
                .insert(tx)
                .await?;
                rows.push(row);
            }

            // Counters and cumulative usage (spec/03 §5.5, §13).
            let mut counters = session::counters_of(&session);
            counters.steps += 1;
            if !turn.tool_calls.is_empty() {
                counters.consecutive_nudges = 0;
            }
            let mut usage = session::usage_of(&session);
            let request_usage = json::Usage {
                requests: 1,
                input_tokens: turn.usage.input_tokens,
                cached_input_tokens: turn.usage.cached_input_tokens,
                cache_write_tokens: turn.usage.cache_write_tokens,
                output_tokens: turn.usage.output_tokens,
                reasoning_tokens: turn.usage.reasoning_tokens,
                last_prompt_tokens: turn.usage.input_tokens
                    + turn.usage.cached_input_tokens
                    + turn.usage.cache_write_tokens,
                estimated: turn.usage.estimated,
            };
            usage.accumulate(&request_usage);
            session::guard_columns(
                tx,
                session.id,
                SessionStatus::Running.as_str(),
                "counters = ?, usage = ?, updated_at = ?",
                vec![
                    json::to_json(&counters)
                        .map_err(|err| AppError::internal(err.to_string()))?
                        .into(),
                    json::to_json(&usage)
                        .map_err(|err| AppError::internal(err.to_string()))?
                        .into(),
                    now.into(),
                ],
            )
            .await?;
            Ok((message, rows))
        })
    })
    .await?;

    let _ = sink;
    let mut events = vec![ServerEvent::SessionMessage {
        session_id: session.id,
        message: session::message_dto(&message)?,
    }];
    for row in &rows {
        events.push(ServerEvent::SessionToolCall {
            session_id: session.id,
            tool_call: session::tool_call_dto(row)?,
        });
    }
    let reloaded = session::load(state.db().conn(), session.id).await?;
    events.push(ServerEvent::SessionUpdated {
        session: session::to_dto(state.db().conn(), &reloaded, Vec::new()).await?,
        warnings: Vec::new(),
    });

    // `refusal` finishes every call as an error (not executed) and fails the session
    // (spec/03 §5.5). Like `max_tokens`, a stop reason that ends the turn must resolve its batch:
    // leaving a call `pending` would make the next step execute it, the opposite of "not
    // executed".
    if let StopReason::Refusal {
        category,
        explanation,
    } = &turn.stop_reason
    {
        let message = explanation
            .clone()
            .or_else(|| category.clone())
            .unwrap_or_default();
        let error = SessionError {
            code: "provider.refusal".to_owned(),
            message,
            retryable: true,
            at: Utc::now(),
        };
        let fresh = session::load(state.db().conn(), session.id).await?;
        if fresh.status == SessionStatus::Running.as_str() {
            let mut resolved = refuse_batch(state, &fresh, &prompts).await?;
            events.append(&mut resolved);
            session::set_error_running(state.db().conn(), session.id, &error).await?;
            let reloaded = session::load(state.db().conn(), session.id).await?;
            let mut transition = session::transition(
                state.db().conn(),
                &reloaded,
                SessionStatus::Failed,
                TransitionCause::Failed,
            )
            .await?;
            events.append(&mut transition);
        }
    }
    Ok(events)
}

/// Resolves every unresolved call of the latest assistant message as `error.refusal` and seals the
/// batch, so a refused response leaves no executable work behind (spec/03 §5.5).
async fn refuse_batch(
    state: &AppState,
    session: &agent_sessions::Model,
    prompts: &std::sync::Arc<PromptSet>,
) -> Result<Vec<ServerEvent>, AppError> {
    let session_owned = session.clone();
    let prompts = prompts.clone();
    let events = crate::services::write(state.db(), move |tx| {
        Box::pin(async move {
            let mut events = Vec::new();
            let messages = messages_of(tx, session_owned.id).await?;
            let Some(last) = messages
                .iter()
                .rev()
                .find(|message| message.role == MessageRole::Assistant.as_str())
            else {
                return Ok(events);
            };
            // The refusal text is the model-facing explanation of *why* the call was not run.
            let text = ToolOutcome::error(&prompts, &ToolError::new(ToolErrorKind::Refusal)).text;
            for call in calls_of(tx, last.id).await? {
                if !matches!(
                    ToolCallStatus::from_str_opt(&call.status),
                    Some(ToolCallStatus::Pending)
                        | Some(ToolCallStatus::Executing)
                        | Some(ToolCallStatus::Held)
                        | Some(ToolCallStatus::AwaitingInput)
                ) {
                    continue;
                }
                agent_tool_calls::ActiveModel {
                    id: Set(call.id),
                    status: Set(ToolCallStatus::Error.as_str().to_owned()),
                    raw_result_text: Set(Some(text.clone())),
                    is_error: Set(1),
                    finished_at: Set(Some(Utc::now())),
                    effects: Set(Some(
                        json::to_json(&ToolEffects::default())
                            .map_err(|err| AppError::internal(err.to_string()))?,
                    )),
                    ..Default::default()
                }
                .update(tx)
                .await?;
                let row = load_call(tx, call.id).await?;
                events.push(ServerEvent::SessionToolCall {
                    session_id: session_owned.id,
                    tool_call: session::tool_call_dto(&row)?,
                });
            }
            // Seal the whole batch: a failed session keeps a valid history for Continue.
            let mut sealed = seal_inner(tx, &session_owned, &prompts).await?;
            events.append(&mut sealed);
            Ok(events)
        })
    })
    .await?;
    Ok(events)
}

fn to_stored_payload(payload: &nekowriter_llm::ProviderPayload) -> json::ProviderPayload {
    match payload {
        nekowriter_llm::ProviderPayload::Openai {
            content,
            reasoning_content,
            tool_calls,
        } => json::ProviderPayload::Openai {
            content: content.clone(),
            reasoning_content: reasoning_content.clone(),
            tool_calls: tool_calls.clone(),
        },
        nekowriter_llm::ProviderPayload::Anthropic { content } => {
            json::ProviderPayload::Anthropic {
                content: content.clone(),
            }
        }
        nekowriter_llm::ProviderPayload::Mock { content } => json::ProviderPayload::Mock {
            content: content.clone(),
        },
    }
}

// ───────────────────────────── step_tool ─────────────────────────────

/// Executes and finalizes one tool call (spec/03 §5.2).
async fn step_tool(
    ctx: &RunnerCtx,
    session: &agent_sessions::Model,
    call: agent_tool_calls::Model,
) -> Result<(), AppError> {
    let state = &ctx.state;
    let prompts = resolve_prompts(state, session).await?;
    let meta = request_meta_of(state.db().conn(), &call).await?;
    let kind = session::kind_of(session);
    let phase = meta.phase;

    // `mark tc executing (started_at); emit session.tool_call`.
    let call_id = call.id;
    crate::services::write(state.db(), move |tx| {
        Box::pin(async move {
            agent_tool_calls::ActiveModel {
                id: Set(call_id),
                status: Set(ToolCallStatus::Executing.as_str().to_owned()),
                started_at: Set(Some(Utc::now())),
                ..Default::default()
            }
            .update(tx)
            .await?;
            Ok(())
        })
    })
    .await?;
    let executing = load_call(state.db().conn(), call.id).await?;
    state.events().publish(ServerEvent::SessionToolCall {
        session_id: session.id,
        tool_call: session::tool_call_dto(&executing)?,
    });

    // Phase gating and argument handling (spec/03 §1.2, spec/05 §1.4).
    let frozen = frozen_tool_names(session);
    let mut outcome: Option<ToolOutcome> = None;
    if !is_permitted(kind, phase, &call.name, &frozen) {
        let error = ToolError::tool_not_allowed(
            &call.name,
            phase_label(&prompts, phase),
            allowed_tools(kind, phase, &frozen),
        );
        outcome = Some(ToolOutcome::error(&prompts, &error));
    }

    let mut prepared = Prepared::None;
    let mut input: Option<Value> = None;
    if outcome.is_none() {
        let specs = state.tool_specs();
        match validate_input(specs, &call.name, &call.input) {
            Err(error) => outcome = Some(ToolOutcome::error(&prompts, &error)),
            Ok(parsed) => input = Some(parsed),
        }
    }
    if outcome.is_none() {
        let Some(parsed) = input.clone() else {
            return Err(AppError::internal("tool input vanished"));
        };
        match state.tool_registry().get(&call.name).cloned() {
            None => {
                tracing::error!(tool = %call.name, "no handler registered for a permitted tool");
                outcome = Some(ToolOutcome::error(
                    &prompts,
                    &ToolError::new(ToolErrorKind::Internal),
                ));
            }
            Some(handler) => {
                let prepare_ctx = PrepareCtx {
                    db: state.db().conn(),
                    session,
                    state,
                };
                // `prepare` runs OUTSIDE any transaction and is cancellable (D-024).
                let prepared_result = tokio::select! {
                    biased;
                    _ = ctx.cancel.cancelled() => None,
                    result = handler.prepare(&prepare_ctx, &parsed) => Some(result),
                };
                match prepared_result {
                    None => {
                        // Stop during `prepare`: the call returns to `pending`, nothing was
                        // written (spec/03 §5.2, test 17).
                        let events =
                            reset_call_to_pending_and_stop(ctx, state, session, &call).await?;
                        state.events().publish_all(events);
                        return Ok(());
                    }
                    Some(Err(error)) => outcome = Some(ToolOutcome::error(&prompts, &error)),
                    Some(Ok(value)) => prepared = value,
                }
            }
        }
    }

    // Execute and finalize in ONE transaction (D-033).
    let prompts_tx = prompts.clone();
    let session_tx = session.clone();
    let call_tx = call.clone();
    let state_tx = state.clone();
    let result = crate::services::write(state.db(), move |tx| {
        let prompts = prompts_tx.clone();
        let session = session_tx.clone();
        let call = call_tx.clone();
        let state = state_tx.clone();
        let prepared = prepared.clone();
        let outcome_before = outcome.clone();
        let input = input.clone();
        Box::pin(async move {
            let mut domain_events: Vec<ServerEvent> = Vec::new();
            let mut outcome = match outcome_before {
                Some(outcome) => outcome,
                None => {
                    let handler = state
                        .tool_registry()
                        .get(&call.name)
                        .cloned()
                        .ok_or_else(|| AppError::internal("tool handler vanished"))?;
                    let book = lookup_book(tx, &session).await?;
                    let chapter = lookup_chapter(tx, &session).await?;
                    let track = lookup_track(tx, &session).await?;
                    let mut tool_ctx = ToolCtx {
                        tx,
                        session: &session,
                        book: book.as_ref(),
                        chapter: chapter.as_ref(),
                        track: track.as_ref(),
                        // The session's track decides which canon rows it sees and writes
                        // (spec/06 §2, D-014).
                        visibility: Visibility::of(&session, track.as_ref()),
                        request_meta: meta,
                        prompts: &prompts,
                        state: &state,
                        events: Vec::new(),
                    };
                    let handled = handler
                        .execute(&mut tool_ctx, input.unwrap_or(Value::Null), prepared)
                        .await;
                    domain_events = std::mem::take(&mut tool_ctx.events);
                    match handled {
                        Ok(outcome) => outcome,
                        Err(error) => ToolOutcome::error(&prompts, &error),
                    }
                }
            };
            let finalization = finalize_inner(tx, &session, &call, &mut outcome, &prompts).await?;
            domain_events.extend(finalization.events);
            // Canon writes by tools keep `search_docs`/`fts_book` in step inside this same
            // transaction (spec/07 §2); the agent path does not go through `canon::write`.
            crate::search::maintenance::sync_events(tx, &domain_events).await?;
            crate::search::index::wake();
            Ok(domain_events)
        })
    })
    .await;

    match result {
        Ok(events) => {
            state.events().publish_all(events);
            Ok(())
        }
        Err(error) => {
            // A database failure rolls the transaction back; a second, minimal transaction then
            // records the call as `error.internal` and the session continues (spec/05 §1.2).
            tracing::error!(
                session_id = session.id,
                call_id = call.id,
                error = %error,
                "tool execution failed; recording error.internal"
            );
            record_internal_failure(state, session, call.id, &prompts).await?;
            Ok(())
        }
    }
}

/// Reads the session's book inside the transaction.
async fn lookup_book(
    tx: &impl ConnectionTrait,
    session: &agent_sessions::Model,
) -> Result<Option<books::Model>, AppError> {
    match session.book_id {
        Some(book_id) => Ok(books::Entity::find_by_id(book_id).one(tx).await?),
        None => Ok(None),
    }
}

/// Reads the session's chapter inside the transaction.
async fn lookup_chapter(
    tx: &impl ConnectionTrait,
    session: &agent_sessions::Model,
) -> Result<Option<chapters::Model>, AppError> {
    match session.chapter_id {
        Some(chapter_id) => Ok(chapters::Entity::find_by_id(chapter_id).one(tx).await?),
        None => Ok(None),
    }
}

/// Reads the session's track inside the transaction.
async fn lookup_track(
    tx: &impl ConnectionTrait,
    session: &agent_sessions::Model,
) -> Result<Option<tracks::Model>, AppError> {
    match session.track_id {
        Some(track_id) => Ok(tracks::Entity::find_by_id(track_id).one(tx).await?),
        None => Ok(None),
    }
}

/// The result of finalizing a call.
struct Finalization {
    events: Vec<ServerEvent>,
}

/// Applies the call's deferred effects and writes its final status (spec/03 §5.4).
async fn finalize_inner(
    tx: &sea_orm::DatabaseTransaction,
    session: &agent_sessions::Model,
    call: &agent_tool_calls::Model,
    outcome: &mut ToolOutcome,
    prompts: &PromptSet,
) -> Result<Finalization, AppError> {
    let mut events = Vec::new();
    let mut appendices: Vec<Appendix> = json::from_json_or_default(&call.appendices);

    // The field is public API for later milestones (M4 appends its reference notes notice).
    let _ = &prompts;

    let hold = hold_condition(session, call, outcome);
    let await_input = outcome.await_input;
    let deferred = hold || await_input.is_some();

    // Effect notices are appended by the transaction that finalizes the call (spec/03 §5.3):
    // while the call is held its effects are not applied yet, so its acceptance notice waits for
    // the resume that applies them.
    if !deferred
        && let Some(notice_key) = phase_notice(&call.name)
        && outcome.effects.phase_to.is_some()
    {
        let granularity = granularity_value(tx, session, outcome, prompts).await?;
        if let Ok(rendered) = prompts.render(notice_key, &json!({ "granularity": granularity })) {
            appendices.push(Appendix {
                kind: AppendixKind::EffectNotice,
                text: rendered.text,
                inbox_id: None,
            });
        }
    }

    // Counters (spec/03 §5.2).
    let mut counters = session::counters_of(session);
    counters.tool_calls += 1;
    if super::tools::registry::RETRIEVAL_TOOLS.contains(&call.name.as_str()) {
        counters.retrieval_calls += 1;
        if counters.retrieval_calls > RETRIEVAL_BUDGET
            && !outcome.is_error
            && let Ok(rendered) = prompts.render(
                "writer.notice.retrieval_budget",
                &json!({ "used": counters.retrieval_calls }),
            )
        {
            appendices.push(Appendix {
                kind: AppendixKind::EffectNotice,
                text: rendered.text,
                inbox_id: None,
            });
        }
    }
    // The researcher's budget notice (spec/05 §8): past 40 tool calls or 25 entries the model is
    // told to wrap up — a soft limit, never a refusal. Entries are counted from the task's rows,
    // which accumulate across runs, so a resumed session does not start a second budget from
    // zero (spec/07 §8).
    if super::session::kind_of(session) == SessionKind::Research
        && !outcome.is_error
        && let Some(task_id) = session.research_task_id
    {
        let entries = crate::services::research::entry_count(tx, task_id).await?;
        if (counters.tool_calls > MAX_RESEARCH_TOOL_CALLS || entries > MAX_RESEARCH_ENTRIES)
            && let Ok(rendered) = prompts.render(
                "notice.research_budget",
                &json!({
                    "used_calls": counters.tool_calls,
                    "max_calls": MAX_RESEARCH_TOOL_CALLS,
                    "entries": entries,
                    "max_entries": MAX_RESEARCH_ENTRIES,
                }),
            )
        {
            appendices.push(Appendix {
                kind: AppendixKind::EffectNotice,
                text: rendered.text,
                inbox_id: None,
            });
        }
    }
    if outcome.is_error {
        counters.consecutive_tool_errors += 1;
    } else {
        counters.consecutive_tool_errors = 0;
    }
    // A tool can consume a one-shot counter flag (e.g. the withheld short completion of
    // spec/03 §8); it is applied here because this transaction writes the counters.
    counters.finish_warned |= outcome.finish_warned;

    let status = if await_input.is_some() {
        ToolCallStatus::AwaitingInput
    } else if hold {
        ToolCallStatus::Held
    } else if outcome.is_error {
        ToolCallStatus::Error
    } else {
        ToolCallStatus::Done
    };
    let stored_effects = if deferred {
        json::to_json(&outcome.effects).map_err(|err| AppError::internal(err.to_string()))?
    } else {
        json::to_json(&ToolEffects::default()).map_err(|err| AppError::internal(err.to_string()))?
    };
    let now = Utc::now();

    // Every result goes through the default cap here, tool-agnostically (spec/05 §1.5): tools
    // that own a `max_chars` cap are exempt inside `cap_result`.
    let raw_result_text = cap_result(prompts, &outcome.text, &call.name);
    agent_tool_calls::ActiveModel {
        id: Set(call.id),
        status: Set(status.as_str().to_owned()),
        raw_result_text: Set(Some(raw_result_text)),
        appendices: Set(
            json::to_json(&appendices).map_err(|err| AppError::internal(err.to_string()))?
        ),
        ui: Set(if outcome.ui.is_null() {
            None
        } else {
            Some(json::to_json(&outcome.ui).map_err(|err| AppError::internal(err.to_string()))?)
        }),
        effects: Set(Some(stored_effects)),
        is_error: Set(i32::from(outcome.is_error)),
        finished_at: Set(if deferred { None } else { Some(now) }),
        ..Default::default()
    }
    .update(tx)
    .await?;

    // The session row: counters, held call, phase (column-wise and guarded).
    let mut sets = vec!["counters = ?".to_owned(), "updated_at = ?".to_owned()];
    let mut values: Vec<sea_orm::Value> = vec![
        json::to_json(&counters)
            .map_err(|err| AppError::internal(err.to_string()))?
            .into(),
        now.into(),
    ];
    if deferred {
        sets.push("held_tool_call_id = ?".to_owned());
        values.push(Some(call.id).into());
    }
    // Deferred effects are applied by the transaction that *finalizes* the call (spec/03 §5.4):
    // a held or awaiting call only stores them, so its phase change must not happen yet.
    if !deferred && let Some(phase) = outcome.effects.phase_to {
        sets.push("phase = ?".to_owned());
        values.push(phase.as_str().to_owned().into());
    }
    session::guard_columns(
        tx,
        session.id,
        SessionStatus::Running.as_str(),
        &sets.join(", "),
        values,
    )
    .await?;

    let refreshed = load_call(tx, call.id).await?;
    events.push(ServerEvent::SessionToolCall {
        session_id: session.id,
        tool_call: session::tool_call_dto(&refreshed)?,
    });

    match status {
        ToolCallStatus::Held => {
            let reloaded = session::load(tx, session.id).await?;
            let mut transition =
                session::transition(tx, &reloaded, SessionStatus::Held, TransitionCause::Hold)
                    .await?;
            events.append(&mut transition);
        }
        ToolCallStatus::AwaitingInput => {
            let reloaded = session::load(tx, session.id).await?;
            let mut transition = session::transition(
                tx,
                &reloaded,
                SessionStatus::AwaitingInput,
                TransitionCause::AwaitInput,
            )
            .await?;
            events.append(&mut transition);
        }
        _ => {
            if outcome.effects.complete.is_some() {
                // The completion handler runs in this same transaction (spec/03 §5.4).
                let completion = outcome.effects.complete;
                let mut completion_events =
                    run_completion_handler(tx, session, call, completion, prompts).await?;
                events.append(&mut completion_events);
            } else if counters.consecutive_tool_errors >= TOOL_ERROR_STREAK_LIMIT {
                // Circuit breaker: five consecutive tool errors pause for the author (spec/03 §5.7).
                let error = SessionError {
                    code: "agent.tool_error_streak".to_owned(),
                    message: format!("{TOOL_ERROR_STREAK_LIMIT} consecutive tool errors"),
                    retryable: true,
                    at: now,
                };
                session::guard_columns(
                    tx,
                    session.id,
                    SessionStatus::Running.as_str(),
                    "error = ?",
                    vec![
                        json::to_json(&error)
                            .map_err(|err| AppError::internal(err.to_string()))?
                            .into(),
                    ],
                )
                .await?;
                let reloaded = session::load(tx, session.id).await?;
                let mut transition = session::transition(
                    tx,
                    &reloaded,
                    SessionStatus::Paused,
                    TransitionCause::NeedsAttention,
                )
                .await?;
                events.append(&mut transition);
            }
        }
    }

    let reloaded = session::load(tx, session.id).await?;
    events.push(ServerEvent::SessionUpdated {
        session: session::to_dto(tx, &reloaded, Vec::new()).await?,
        warnings: Vec::new(),
    });
    Ok(Finalization { events })
}

/// The completion handler of a finalizing call (spec/03 §5.4, spec/08).
///
/// Public inside the crate because the control operations that finalize a deferred call
/// (`services::sessions::answer_inner` / `resume_inner`) apply the same full sequence: domain
/// effects, `error.after_completion` on the siblings, sealing, and the transition to
/// `completed`. Applying only the domain half would leave a completed session `running`.
pub(crate) async fn run_completion_handler(
    tx: &sea_orm::DatabaseTransaction,
    session: &agent_sessions::Model,
    call: &agent_tool_calls::Model,
    completion: Option<CompletionKind>,
    prompts: &PromptSet,
) -> Result<Vec<ServerEvent>, AppError> {
    let mut events = Vec::new();

    // 1. Domain effects of the completed session kind.
    if let Some(completion) = completion {
        let mut domain = super::context::apply_completion(tx, session, call, completion).await?;
        events.append(&mut domain);
    }

    // 2. Every remaining pending call of the same assistant message ends as
    //    `error.after_completion` (spec/03 §5.4, D-025).
    let siblings = agent_tool_calls::Entity::find()
        .filter(agent_tool_calls::Column::MessageId.eq(call.message_id))
        .filter(agent_tool_calls::Column::Status.eq(ToolCallStatus::Pending.as_str()))
        .order_by_asc(agent_tool_calls::Column::CallIndex)
        .all(tx)
        .await?;
    for sibling in siblings {
        let rendered = ToolOutcome::error(prompts, &ToolError::new(ToolErrorKind::AfterCompletion));
        agent_tool_calls::ActiveModel {
            id: Set(sibling.id),
            status: Set(ToolCallStatus::Error.as_str().to_owned()),
            raw_result_text: Set(Some(rendered.text)),
            is_error: Set(1),
            finished_at: Set(Some(Utc::now())),
            ..Default::default()
        }
        .update(tx)
        .await?;
        let row = load_call(tx, sibling.id).await?;
        events.push(ServerEvent::SessionToolCall {
            session_id: session.id,
            tool_call: session::tool_call_dto(&row)?,
        });
    }

    // 3. Seal the whole batch, so a completed session keeps a valid history (spec/03 §5.4).
    let mut sealed = seal_inner(tx, session, prompts).await?;
    events.append(&mut sealed);

    // 4. Transition to completed.
    let reloaded = session::load(tx, session.id).await?;
    let mut transition = session::transition(
        tx,
        &reloaded,
        SessionStatus::Completed,
        TransitionCause::Completed,
    )
    .await?;
    events.append(&mut transition);
    Ok(events)
}

/// The notice key of a phase-changing effect (spec/03 §5.3 slot 1).
///
/// Public inside the crate because the resume path appends the same notice when it applies a held
/// call's kept effects (spec/03 §5.4).
pub fn phase_notice(tool: &str) -> Option<&'static str> {
    match tool {
        "submit_outline" => Some("writer.notice.outline_accepted"),
        "submit_revision_plan" => Some("writer.notice.revision_plan_accepted"),
        _ => None,
    }
}

/// The `granularity` template context of the session entering its new phase (spec/03 §8).
async fn granularity_value(
    tx: &impl ConnectionTrait,
    session: &agent_sessions::Model,
    outcome: &ToolOutcome,
    prompts: &PromptSet,
) -> Result<Value, AppError> {
    let phase = outcome
        .effects
        .phase_to
        .unwrap_or_else(|| session::phase_of(session));
    let (tail_open, last_pid) = tail_state(tx, session).await?;
    let key = session::granularity_of(session)
        .map(|value| value.as_str())
        .unwrap_or(json::Granularity::Paragraph.as_str())
        .to_owned();
    Ok(json!({
        "key": key.clone(),
        "label": prompts.label("granularity", &key),
        "revising": matches!(phase, Phase::RevisionPlanning | Phase::Revising),
        "tail_open": tail_open,
        "last_pid": last_pid,
    }))
}

/// The tail state and last pid of the session's chapter (spec/03 §8 "open tail").
///
/// The chapter's current content is authoritative. `view_snapshot` is only written when a session
/// *leaves* `running` (spec/03 §3), so reading it here — while the notice is rendered inside the
/// transaction that finalizes a call — would always see `NULL` and silently drop the open-tail hint
/// from every notice on the running path. The snapshot stays as the fallback for a session whose
/// chapter row is gone.
async fn tail_state(
    tx: &impl ConnectionTrait,
    session: &agent_sessions::Model,
) -> Result<(bool, String), AppError> {
    if let Some(chapter) = lookup_chapter(tx, session).await? {
        let content: json::ChapterContent = json::from_json_or_default(&chapter.content);
        return Ok((
            content.tail_open,
            content
                .paragraphs
                .last()
                .map(|paragraph| paragraph.pid.clone())
                .unwrap_or_default(),
        ));
    }
    Ok(snapshot_tail_state(session))
}

/// The tail state recorded in the session's view snapshot, when it has one.
fn snapshot_tail_state(session: &agent_sessions::Model) -> (bool, String) {
    let Some(raw) = session.view_snapshot.as_deref() else {
        return (false, String::new());
    };
    let Ok(ViewSnapshot::Chapter { content, .. }) = json::from_json::<ViewSnapshot>(raw) else {
        return (false, String::new());
    };
    (
        content.tail_open,
        content
            .paragraphs
            .last()
            .map(|paragraph| paragraph.pid.clone())
            .unwrap_or_default(),
    )
}

/// `hold_condition(s, tc, outcome)` (spec/03 §5.2).
fn hold_condition(
    session: &agent_sessions::Model,
    call: &agent_tool_calls::Model,
    outcome: &ToolOutcome,
) -> bool {
    if session.pause_requested != 0 {
        return true;
    }
    if session.auto_pause != 0
        && super::tools::registry::is_write_tool(&call.name)
        && !outcome.is_error
    {
        return true;
    }
    if call.name == "submit_outline" && session.pause_after_outline != 0 && !outcome.is_error {
        return true;
    }
    false
}

/// Records a rolled-back call as `error.internal` in a minimal transaction (spec/05 §1.2).
async fn record_internal_failure(
    state: &AppState,
    session: &agent_sessions::Model,
    call_id: i32,
    prompts: &PromptSet,
) -> Result<(), AppError> {
    let text = ToolOutcome::error(prompts, &ToolError::new(ToolErrorKind::Internal)).text;
    let session_id = session.id;
    let events = crate::services::write(state.db(), move |tx| {
        Box::pin(async move {
            agent_tool_calls::ActiveModel {
                id: Set(call_id),
                status: Set(ToolCallStatus::Error.as_str().to_owned()),
                raw_result_text: Set(Some(text)),
                is_error: Set(1),
                finished_at: Set(Some(Utc::now())),
                ..Default::default()
            }
            .update(tx)
            .await?;
            let row = load_call(tx, call_id).await?;
            Ok(vec![ServerEvent::SessionToolCall {
                session_id,
                tool_call: session::tool_call_dto(&row)?,
            }])
        })
    })
    .await?;
    state.events().publish_all(events);
    Ok(())
}

/// Returns a call in its prepare phase to `pending` and stops the session (spec/03 §5.2).
async fn reset_call_to_pending_and_stop(
    ctx: &RunnerCtx,
    state: &AppState,
    session: &agent_sessions::Model,
    call: &agent_tool_calls::Model,
) -> Result<Vec<ServerEvent>, AppError> {
    // Resolve the transition BEFORE the transaction: the reason cell is read once, and the call
    // reset and the transition stay one atomic unit (spec/03 §5.2, D-033). A shutdown during a
    // tool's prepare gives `interrupted`, exactly as startup recovery would (spec/03 §5.1).
    let (status, cause) = match ctx.cancelled_transition() {
        Some(CancelledTransition::Transition(status, cause)) => (status, cause),
        // Abandon writes `abandoned` itself; the call still goes back to `pending` so it is
        // re-runnable, and the defensive no-reason case keeps the Stop behaviour.
        Some(CancelledTransition::Exit) | None => (SessionStatus::Stopped, TransitionCause::Stop),
    };
    let call_id = call.id;
    let session_owned = session.clone();
    let mut events = crate::services::write(state.db(), move |tx| {
        Box::pin(async move {
            agent_tool_calls::ActiveModel {
                id: Set(call_id),
                status: Set(ToolCallStatus::Pending.as_str().to_owned()),
                started_at: Set(None),
                ..Default::default()
            }
            .update(tx)
            .await?;
            session::transition(tx, &session_owned, status, cause).await
        })
    })
    .await?;
    // The reset itself is announced so the UI stops showing a spinner.
    let row = load_call(state.db().conn(), call.id).await?;
    events.insert(
        0,
        ServerEvent::SessionToolCall {
            session_id: session.id,
            tool_call: session::tool_call_dto(&row)?,
        },
    );
    Ok(events)
}

// ───────────────────────────── sealing ─────────────────────────────

/// `seal_latest_batch(s)` (spec/03 §5.3), in its own transaction.
async fn seal_latest_batch(
    ctx: &RunnerCtx,
    session: &agent_sessions::Model,
) -> Result<(), AppError> {
    let state = &ctx.state;
    let prompts = resolve_prompts(state, session).await?;
    let session_owned = session.clone();
    let events = crate::services::write(state.db(), move |tx| {
        let prompts = prompts.clone();
        Box::pin(async move { seal_inner(tx, &session_owned, &prompts).await })
    })
    .await?;
    state.events().publish_all(events);
    Ok(())
}

/// The in-transaction sealing step (spec/03 §5.3).
pub async fn seal_inner(
    tx: &sea_orm::DatabaseTransaction,
    session: &agent_sessions::Model,
    prompts: &PromptSet,
) -> Result<Vec<ServerEvent>, AppError> {
    let mut events = Vec::new();
    let report = edit_report_text(tx, session, prompts).await?;
    let items = pending_inbox_rows(tx, session.id).await?;

    // The unsealed calls of the latest assistant message are the batch.
    let messages = messages_of(tx, session.id).await?;
    let batch = match messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::Assistant.as_str())
    {
        Some(message) => calls_of(tx, message.id)
            .await?
            .into_iter()
            .filter(|call| call.sealed == 0)
            .collect::<Vec<_>>(),
        None => Vec::new(),
    };

    if !batch.is_empty() {
        let last_id = batch.last().map(|call| call.id);
        for call in &batch {
            let mut appendices: Vec<Appendix> = json::from_json_or_default(&call.appendices);
            if Some(call.id) == last_id {
                if let Some(report) = &report {
                    appendices.push(Appendix {
                        kind: AppendixKind::EditReport,
                        text: report.clone(),
                        inbox_id: None,
                    });
                }
                for item in &items {
                    appendices.push(seal::inbox_appendix(item.content.clone(), item.id));
                }
            }
            let composed = seal::compose(call.raw_result_text.as_deref(), &appendices);
            agent_tool_calls::ActiveModel {
                id: Set(call.id),
                appendices: Set(json::to_json(&appendices)
                    .map_err(|err| AppError::internal(err.to_string()))?),
                result_text: Set(Some(composed)),
                sealed: Set(1),
                ..Default::default()
            }
            .update(tx)
            .await?;
            let row = load_call(tx, call.id).await?;
            events.push(ServerEvent::SessionToolCall {
                session_id: session.id,
                tool_call: session::tool_call_dto(&row)?,
            });
        }
        if let Some(last_id) = last_id {
            mark_inbox_consumed(tx, &items, Some(last_id), None).await?;
        }
    } else if report.is_some() || !items.is_empty() {
        // The batch was sealed by a request that never finished: the report and the inbox
        // become one new user message (spec/03 §5.3, D-035).
        let mut sections = Vec::new();
        if let Some(report) = &report {
            sections.push(report.clone());
        }
        for item in &items {
            sections.push(item.content.clone());
        }
        let author = items
            .iter()
            .any(|item| item.kind == json::InboxKind::UserMessage.as_str());
        let kind = if author {
            MessageKind::User
        } else {
            MessageKind::Notice
        };
        let blocks = text_blocks(&sections);
        let message = insert_user_message(tx, session.id, kind, blocks).await?;
        mark_inbox_consumed(tx, &items, None, Some(message.id)).await?;
        events.push(ServerEvent::SessionMessage {
            session_id: session.id,
            message: session::message_dto(&message)?,
        });
    }

    if report.is_some() {
        // The snapshot is cleared after it is reported (spec/03 §7).
        agent_sessions::ActiveModel {
            id: Set(session.id),
            view_snapshot: Set(None),
            ..Default::default()
        }
        .update(tx)
        .await?;
    }
    Ok(events)
}

/// Turns rendered sections into text blocks, dropping empty ones.
pub fn text_blocks(sections: &[String]) -> Vec<StoredBlock> {
    sections
        .iter()
        .filter(|section| !section.trim().is_empty())
        .map(|section| StoredBlock::Text {
            text: section.clone(),
            cache: None,
        })
        .collect()
}

/// Marks inbox rows consumed by a tool call or by a message (spec/02 §2).
async fn mark_inbox_consumed(
    tx: &sea_orm::DatabaseTransaction,
    items: &[agent_inbox::Model],
    tool_call_id: Option<i32>,
    message_id: Option<i32>,
) -> Result<(), AppError> {
    let now = Utc::now();
    for item in items {
        agent_inbox::ActiveModel {
            id: Set(item.id),
            consumed_at: Set(Some(now)),
            consumed_by_tool_call_id: Set(tool_call_id),
            consumed_by_message_id: Set(message_id),
            ..Default::default()
        }
        .update(tx)
        .await?;
    }
    Ok(())
}

/// The rendered edit report of a session's stored view snapshot (spec/03 §7).
async fn edit_report_text(
    tx: &sea_orm::DatabaseTransaction,
    session: &agent_sessions::Model,
    prompts: &PromptSet,
) -> Result<Option<String>, AppError> {
    let Some(raw) = session.view_snapshot.as_deref() else {
        return Ok(None);
    };
    let Ok(snapshot) = json::from_json::<ViewSnapshot>(raw) else {
        return Ok(None);
    };
    match snapshot {
        ViewSnapshot::Chapter { .. } => {
            let Some(chapter_id) = session.chapter_id else {
                return Ok(None);
            };
            let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(tx).await? else {
                return Ok(None);
            };
            let current = super::report::ChapterState {
                content: json::from_json_or_default(&chapter.content),
                outline: chapter.outline,
                reference_notes: chapter.reference_notes,
            };
            let report = super::report::chapter_report(&snapshot, &current);
            if report.is_empty() {
                return Ok(None);
            }
            Ok(Some(report.render(prompts)?))
        }
        snapshot @ ViewSnapshot::Conception { .. } => {
            let Some(book_id) = session.book_id else {
                return Ok(None);
            };
            let current = super::report::current_conception_state(tx, book_id).await?;
            let report = super::report::conception_report(&snapshot, &current, prompts);
            if report.is_empty() {
                return Ok(None);
            }
            Ok(Some(report.render(prompts)?))
        }
    }
}

// ───────────────────────────── retry state ─────────────────────────────

/// Publishes the `retrying` state of an in-flight retry as a `session.updated` event
/// (spec/04 §6, spec/10 §2.8).
///
/// `SessionDto.retrying` is transport state, not a column: the model request is not persisted
/// until it succeeds, so the only way the UI can show "Provider overloaded — retrying in 8 s" is
/// an event published when the wait starts. A failed publish must not abort the retry.
async fn publish_retrying(
    ctx: &RunnerCtx,
    attempt: u32,
    wait: Duration,
    error: &nekowriter_llm::retry::LlmError,
) {
    let Ok(row) = session::load(ctx.state.db().conn(), ctx.session_id).await else {
        return;
    };
    let Ok(mut dto) = session::to_dto(ctx.state.db().conn(), &row, Vec::new()).await else {
        return;
    };
    dto.retrying = Some(crate::api::dto::sessions::RetryingDto {
        attempt: i32::try_from(attempt).unwrap_or(i32::MAX),
        // Round up, so "retrying in 1 s" never hides a 1.4 s wait behind a 1.
        wait_seconds: i32::try_from(wait.as_secs().max(1)).unwrap_or(i32::MAX),
        // The provider's own wording, so the code stays English-only (AGENTS.md §2): it is a
        // classification label, not a user-visible sentence (the UI localizes by this string).
        reason: error.to_string(),
    });
    ctx.state.events().publish(ServerEvent::SessionUpdated {
        session: dto,
        warnings: Vec::new(),
    });
}

// ───────────────────────────── text-only replies ─────────────────────────────

/// Handles an assistant message without tool calls (spec/03 §5.6).
async fn handle_text_only(
    ctx: &RunnerCtx,
    session: &agent_sessions::Model,
) -> Result<(), AppError> {
    let state = &ctx.state;
    let prompts = resolve_prompts(state, session).await?;
    let session_owned = session.clone();
    let prompts_tx = prompts.clone();
    let events = crate::services::write(state.db(), move |tx| {
        let prompts = prompts_tx.clone();
        Box::pin(async move { text_only_inner(tx, &session_owned, &prompts).await })
    })
    .await?;
    state.events().publish_all(events);
    Ok(())
}

/// The in-transaction text-only handling (spec/03 §5.6).
pub async fn text_only_inner(
    tx: &sea_orm::DatabaseTransaction,
    session: &agent_sessions::Model,
    prompts: &PromptSet,
) -> Result<Vec<ServerEvent>, AppError> {
    let mut events = Vec::new();
    // Pending edits or messages become a user message first (spec/03 §5.6).
    let report = edit_report_text(tx, session, prompts).await?;
    let items = pending_inbox_rows(tx, session.id).await?;
    if report.is_some() || !items.is_empty() {
        let mut sections = Vec::new();
        if let Some(report) = &report {
            sections.push(report.clone());
        }
        for item in &items {
            sections.push(item.content.clone());
        }
        let author = items
            .iter()
            .any(|item| item.kind == json::InboxKind::UserMessage.as_str());
        let kind = if author {
            MessageKind::User
        } else {
            MessageKind::Notice
        };
        let message = insert_user_message(tx, session.id, kind, text_blocks(&sections)).await?;
        mark_inbox_consumed(tx, &items, None, Some(message.id)).await?;
        events.push(ServerEvent::SessionMessage {
            session_id: session.id,
            message: session::message_dto(&message)?,
        });
        if report.is_some() {
            agent_sessions::ActiveModel {
                id: Set(session.id),
                view_snapshot: Set(None),
                ..Default::default()
            }
            .update(tx)
            .await?;
        }
        return Ok(events);
    }

    match session::kind_of(session) {
        SessionKind::Conception => {
            // The one kind where a text-only reply is not a failure (spec/03 §5.6).
            let mut transition =
                session::transition(tx, session, SessionStatus::Idle, TransitionCause::Idle)
                    .await?;
            events.append(&mut transition);
        }
        kind => {
            let mut counters = session::counters_of(session);
            if counters.consecutive_nudges < NUDGE_LIMIT {
                counters.consecutive_nudges += 1;
                let key = nudge_key(kind, session::phase_of(session));
                let (tail_open, last_pid) = tail_state(tx, session).await?;
                let granularity_key = session::granularity_of(session)
                    .map(|value| value.as_str())
                    .unwrap_or(json::Granularity::Paragraph.as_str())
                    .to_owned();
                let context = json!({
                    "granularity": {
                        "key": granularity_key.clone(),
                        "label": prompts.label("granularity", &granularity_key),
                        "revising": matches!(
                            session::phase_of(session),
                            Phase::RevisionPlanning | Phase::Revising
                        ),
                        "tail_open": tail_open,
                        "last_pid": last_pid,
                    },
                });
                let rendered = prompts
                    .render_value(key, crate::prompts::template_value(&context))
                    .map_err(|err| err.into_app_error())?;
                let message = insert_user_message(
                    tx,
                    session.id,
                    MessageKind::Nudge,
                    vec![StoredBlock::Text {
                        text: rendered.text,
                        cache: None,
                    }],
                )
                .await?;
                session::guard_columns(
                    tx,
                    session.id,
                    SessionStatus::Running.as_str(),
                    "counters = ?",
                    vec![
                        json::to_json(&counters)
                            .map_err(|err| AppError::internal(err.to_string()))?
                            .into(),
                    ],
                )
                .await?;
                events.push(ServerEvent::SessionMessage {
                    session_id: session.id,
                    message: session::message_dto(&message)?,
                });
            } else {
                // Two unanswered nudges: the session fails (spec/03 §5.7).
                let error = SessionError {
                    code: "agent.no_tool_call".to_owned(),
                    message: "the model replied without calling a tool".to_owned(),
                    retryable: true,
                    at: Utc::now(),
                };
                session::guard_columns(
                    tx,
                    session.id,
                    SessionStatus::Running.as_str(),
                    "error = ?",
                    vec![
                        json::to_json(&error)
                            .map_err(|err| AppError::internal(err.to_string()))?
                            .into(),
                    ],
                )
                .await?;
                let reloaded = session::load(tx, session.id).await?;
                let mut transition = session::transition(
                    tx,
                    &reloaded,
                    SessionStatus::Failed,
                    TransitionCause::Failed,
                )
                .await?;
                events.append(&mut transition);
            }
        }
    }
    Ok(events)
}

/// The nudge key of a session kind and phase (spec/03 §5.6, D-057).
fn nudge_key(kind: SessionKind, phase: Phase) -> &'static str {
    match (kind, phase) {
        (SessionKind::ChapterWrite, Phase::Planning) => "writer.notice.nudge_planning",
        (SessionKind::ChapterWrite | SessionKind::ChapterRevise, Phase::Drafting) => {
            "writer.notice.nudge_drafting"
        }
        (SessionKind::ChapterRevise, Phase::RevisionPlanning) => {
            "writer.notice.nudge_revision_planning"
        }
        (SessionKind::ChapterWrite | SessionKind::ChapterRevise, Phase::Revising) => {
            "writer.notice.nudge_revising"
        }
        _ => "notice.nudge_generic",
    }
}

// ───────────────────────────── rollover check ─────────────────────────────

/// True when the next request would overflow the context window (spec/03 §9).
pub fn rollover_needed(session: &agent_sessions::Model) -> bool {
    // Eligibility first: only a writer that is drafting or revising rolls over (spec/03 §9).
    let eligible = matches!(
        (session::kind_of(session), session::phase_of(session)),
        (SessionKind::ChapterWrite, Phase::Drafting)
            | (SessionKind::ChapterRevise, Phase::Revising)
    );
    if !eligible {
        return false;
    }
    // The previous request failed with a context overflow: the next step rolls over even when the
    // token estimate says otherwise (spec/03 §9 "or the previous request failed with context
    // overflow").
    if session::error_of(session).is_some_and(|error| error.code == CONTEXT_OVERFLOW_CODE) {
        return true;
    }
    let Ok(snapshot) = json::from_json::<json::ModelSnapshot>(&session.model_snapshot) else {
        return false;
    };
    let usage = session::usage_of(session);
    let window = i64::from(snapshot.context_window);
    if window <= 0 {
        return false;
    }
    let projected = usage.last_prompt_tokens + i64::from(snapshot.max_output_tokens);
    projected > (window as f64 * 0.85) as i64
}

/// The session error code of a context overflow (spec/03 §5.7, §9).
pub const CONTEXT_OVERFLOW_CODE: &str = "provider.context_overflow";

// ───────────────────────────── shared helpers ─────────────────────────────

/// Resolves the prompt set of a session's book.
async fn resolve_prompts(
    state: &AppState,
    session: &agent_sessions::Model,
) -> Result<std::sync::Arc<PromptSet>, AppError> {
    state.prompts().for_book(state.db(), session.book_id).await
}

/// Loads one call row.
pub async fn load_call(
    conn: &impl ConnectionTrait,
    call_id: i32,
) -> Result<agent_tool_calls::Model, AppError> {
    agent_tool_calls::Entity::find_by_id(call_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::internal(format!("tool call {call_id} vanished")))
}

/// The provider client of a session's frozen snapshot; the key is resolved at call time so key
/// rotation works (spec/03 §4 step 1).
async fn provider_client(
    state: &AppState,
    session: &agent_sessions::Model,
) -> Result<nekowriter_llm::ProviderClient, AppError> {
    // Tests inject a scripted provider so no network is touched (spec/04 §7.1).
    #[cfg(any(test, feature = "testing"))]
    if let Some(provider) = state.provider_override() {
        return Ok(provider.clone());
    }
    let snapshot: json::ModelSnapshot = json::from_json_or_default(&session.model_snapshot);
    let provider = crate::db::entities::providers::Entity::find_by_id(snapshot.provider_id)
        .one(state.db().conn())
        .await?;
    let Some(provider) = provider else {
        // A deleted provider row is a session error, not an HTTP error (D-048).
        let error = SessionError {
            code: "model.missing_provider".to_owned(),
            message: format!("provider {} no longer exists", snapshot.provider_id),
            retryable: false,
            at: Utc::now(),
        };
        let fresh = session::load(state.db().conn(), session.id).await?;
        if fresh.status == SessionStatus::Running.as_str() {
            session::set_error_running(state.db().conn(), session.id, &error).await?;
            let reloaded = session::load(state.db().conn(), session.id).await?;
            let events = session::transition(
                state.db().conn(),
                &reloaded,
                SessionStatus::Failed,
                TransitionCause::Failed,
            )
            .await?;
            state.events().publish_all(events);
        }
        return Err(AppError::new(
            ErrorCode::ProviderUnavailable,
            "model.missing_provider",
        ));
    };
    let mut config = crate::services::providers::provider_config(&provider)?;
    // The snapshot wins over the edited provider row (spec/03 §1.3).
    config.base_url = snapshot.base_url.clone();
    config.extra_headers = snapshot.extra_headers.clone();
    nekowriter_llm::ProviderClient::from_config(config)
        .map_err(|error| AppError::internal(error.to_string()))
}

/// The `request_meta` of the assistant message a call belongs to (spec/03 §8).
async fn request_meta_of(
    conn: &impl ConnectionTrait,
    call: &agent_tool_calls::Model,
) -> Result<RequestMeta, AppError> {
    let message = agent_messages::Entity::find_by_id(call.message_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::internal("tool call has no message"))?;
    Ok(message
        .request_meta
        .as_deref()
        .and_then(|raw| json::from_json::<RequestMeta>(raw).ok())
        .unwrap_or(RequestMeta {
            phase: Phase::None,
            granularity: None,
        }))
}

/// The preview field of every registered tool (spec/03 §12).
fn preview_fields_of(state: &AppState) -> BTreeMap<String, &'static str> {
    let registry = state.tool_registry();
    registry
        .names()
        .filter_map(|name| {
            registry
                .get(name)
                .and_then(|handler| handler.preview_field())
                .map(|field| (name.to_owned(), field))
        })
        .collect()
}
