//! Session rows: loading, DTOs and [`transition`] — the one function that changes a status.
//!
//! spec/03 §3: every transition goes through one function that, in the same transaction, writes
//! the status (and `finished_at` for terminal statuses), clears `pause_requested` when leaving
//! `running` (a pause is consumed by the hold or pause it caused), and stores `view_snapshot`
//! for writer and conception sessions (the oldest unreported snapshot wins). The event is emitted
//! by the caller **after** the commit.
//!
//! spec/03 §14 / D-034: the runner is the only writer of a running session. Session rows are
//! updated column-wise with an expected-status guard, never by writing back a stale model.

use chrono::Utc;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde_json::Value;

use crate::api::dto::sessions::{MessageBlockDto, MessageDto, SessionDto, ToolCallDto};
use crate::db::entities::{
    agent_inbox, agent_messages, agent_sessions, agent_tool_calls, chapters, model_profiles,
    research_tasks, setting_docs,
};
use crate::db::json::{
    self, CanonSystems, ConceptionCharacterSnapshot, ConceptionDocSnapshot, ConceptionState,
    ConceptionTopicSnapshot, Granularity, Phase, ResearchTaskStatus, SessionCounters, SessionError,
    SessionKind, SessionStatus, StoredBlock, Usage, ViewSnapshot,
};
use crate::error::{AppError, ErrorCode};
use crate::events::ServerEvent;

/// Test-only pause (milliseconds) between `transition`'s status write and its domain consequence
/// (D-113). Zero in production; a test sets it to widen the window and prove the two writes are one
/// transaction.
#[cfg(any(test, feature = "testing"))]
pub static TEST_TRANSITION_PAUSE_MS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Terminal statuses (spec/03 §3): a session is **open** in every other status.
pub fn is_terminal(status: SessionStatus) -> bool {
    matches!(
        status,
        SessionStatus::Completed | SessionStatus::Superseded | SessionStatus::Abandoned
    )
}

/// True when the session is running (the only status a runner may act on).
pub fn is_running(row: &agent_sessions::Model) -> bool {
    row.status == SessionStatus::Running
}

/// Reads the status column as its enum.
pub fn status_of(row: &agent_sessions::Model) -> SessionStatus {
    row.status
}

/// Reads the phase column as its enum.
pub fn phase_of(row: &agent_sessions::Model) -> Phase {
    row.phase
}

/// Reads the kind column as its enum.
pub fn kind_of(row: &agent_sessions::Model) -> SessionKind {
    row.kind
}

/// Reads the role column as its enum.
pub fn role_of(row: &agent_sessions::Model) -> crate::db::json::AgentRole {
    row.role
}

/// Reads the granularity column, when set.
pub fn granularity_of(row: &agent_sessions::Model) -> Option<Granularity> {
    row.granularity
}

/// The canon systems frozen into a session, `None` reading as all five on (spec/02 §2).
pub fn canon_systems_of(row: &agent_sessions::Model) -> CanonSystems {
    row.canon_systems
        .clone()
        .map(|systems| systems.into_inner())
        .unwrap_or_default()
}

/// Loads one session row, or a `not_found` error.
pub async fn load(
    conn: &impl ConnectionTrait,
    session_id: i32,
) -> Result<agent_sessions::Model, AppError> {
    agent_sessions::Entity::find_by_id(session_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("session {session_id}")))
}

/// The session's counters.
pub fn counters_of(row: &agent_sessions::Model) -> SessionCounters {
    row.counters.clone().into_inner()
}

/// The session's cumulative usage.
pub fn usage_of(row: &agent_sessions::Model) -> Usage {
    row.usage.clone().into_inner()
}

/// The session's stored error, when it has one.
pub fn error_of(row: &agent_sessions::Model) -> Option<SessionError> {
    row.error.clone().map(|value| value.into_inner())
}

// ───────────────────────────── transitions ─────────────────────────────

/// What triggered a transition. Only used for logs: behaviour is decided by the target status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionCause {
    /// The loop reached its next boundary.
    Boundary,
    /// The author asked to pause before the next model request.
    PauseBeforeRequest,
    /// A tool call held for author review.
    Hold,
    /// A tool call needs the author's input.
    AwaitInput,
    /// The tool-error circuit breaker tripped (spec/03 §5.7).
    NeedsAttention,
    /// A completing call finished the session.
    Completed,
    /// An unrecoverable provider or runtime error (spec/03 §5.7).
    Failed,
    /// The author pressed Stop (spec/03 §6, D-023).
    Stop,
    /// The server is shutting down (spec/01 §5: like a restart).
    Shutdown,
    /// A conception turn ended with text (spec/03 §5.6).
    Idle,
    /// A rollover superseded this session (spec/03 §9).
    Rollover,
    /// The author replied to a completed reviewer session (spec/03 §6).
    Rereview,
}

impl TransitionCause {
    fn as_str(self) -> &'static str {
        match self {
            TransitionCause::Boundary => "boundary",
            TransitionCause::PauseBeforeRequest => "pause_before_request",
            TransitionCause::Hold => "hold",
            TransitionCause::AwaitInput => "await_input",
            TransitionCause::NeedsAttention => "needs_attention",
            TransitionCause::Completed => "completed",
            TransitionCause::Failed => "failed",
            TransitionCause::Stop => "stop",
            TransitionCause::Shutdown => "shutdown",
            TransitionCause::Idle => "idle",
            TransitionCause::Rollover => "rollover",
            TransitionCause::Rereview => "rereview",
        }
    }
}

/// Changes the session's status and returns the `session.updated` event to publish after the
/// commit (spec/03 §3).
///
/// The update is guarded by the status the caller loaded, so a session another writer moved on
/// cannot be clobbered (spec/03 §14). Leaving `running` clears `pause_requested` and captures the
/// view snapshot for writer and conception sessions.
pub async fn transition(
    conn: &impl ConnectionTrait,
    session: &agent_sessions::Model,
    to: SessionStatus,
    cause: TransitionCause,
) -> Result<Vec<ServerEvent>, AppError> {
    let now = Utc::now();
    let from_running = is_running(session);
    // The oldest unreported snapshot wins: keep the stored one when there is one.
    let snapshot = if from_running
        && session.view_snapshot.is_none()
        && matches!(
            kind_of(session),
            SessionKind::ChapterOutline
                | SessionKind::ChapterWrite
                | SessionKind::ChapterRevise
                | SessionKind::ChapterReview
                | SessionKind::ChapterMaintenance
                | SessionKind::ChapterResync
                | SessionKind::CanonSeed
                | SessionKind::Conception
        ) {
        capture_view_snapshot(conn, session).await?
    } else {
        None
    };
    let changes = transition_changes(to, now, from_running, snapshot);
    guard_update(conn, session.id, session.status, changes).await?;
    tracing::debug!(
        session_id = session.id,
        from = %session.status,
        to = to.as_str(),
        cause = cause.as_str(),
        "session transition"
    );

    // Test-only seam (D-113): pause between the status write and the domain consequence so a test
    // can prove they commit atomically. A plain pool connection would auto-commit the status here
    // and expose the inconsistent pair to readers; inside `Db::write` the pause holds the
    // transaction and readers keep seeing the previous state. Production builds have no pause.
    #[cfg(any(test, feature = "testing"))]
    {
        let millis = TEST_TRANSITION_PAUSE_MS.load(std::sync::atomic::Ordering::Relaxed);
        if millis > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(millis)).await;
        }
    }

    // An archivist session that fails or is abandoned leaves the chapter's canon `failed`, so the
    // chapter never stays `syncing` without a session and Retry works (spec/06 §6, D-048). This
    // runs in the transition's own transaction, so the status and the chapter flag commit
    // together.
    if matches!(to, SessionStatus::Failed | SessionStatus::Abandoned)
        && matches!(
            kind_of(session),
            SessionKind::ChapterMaintenance | SessionKind::ChapterResync
        )
        && let Some(chapter_id) = session.chapter_id
    {
        chapters::Entity::update_many()
            .set(canon_status_failed_changes(now))
            .filter(chapters::COLUMN.id.eq(chapter_id))
            .filter(
                chapters::COLUMN
                    .canon_status
                    .eq(crate::db::json::CanonStatus::Syncing),
            )
            .exec(conn)
            .await?;
    }

    let row = load(conn, session.id).await?;
    let mut events = vec![ServerEvent::SessionUpdated {
        session: to_dto(conn, &row, Vec::new()).await?,
        warnings: Vec::new(),
    }];

    // A research session owns its task's status (spec/07 §8 step 1, spec/03 §6): an abandoned or
    // cancelled session leaves the task `cancelled`, a failed one leaves it `failed`, and a
    // resumed session makes it `running` again. The same transaction commits both statuses.
    if kind_of(session) == SessionKind::Research
        && let Some(task_id) = session.research_task_id
    {
        events.extend(
            crate::services::research::on_session_status(conn, task_id, session.id, to).await?,
        );
    }
    Ok(events)
}

/// The partial session update of [`transition`]. Shared with the SQL-equality test so an extra
/// column cannot be added to the production path without changing the tested statement.
pub(crate) fn transition_changes(
    to: SessionStatus,
    now: chrono::DateTime<chrono::Utc>,
    from_running: bool,
    view_snapshot: Option<ViewSnapshot>,
) -> agent_sessions::ActiveModel {
    let mut changes = agent_sessions::ActiveModel {
        status: sea_orm::Set(to),
        updated_at: sea_orm::Set(now),
        ..Default::default()
    };
    if is_terminal(to) {
        changes.finished_at = sea_orm::Set(Some(now));
    } else {
        // `finished_at` describes a terminal status only (spec/03 §3). A re-review re-opens a
        // completed reviewer session (spec/03 §6), so leaving the terminal set must clear the old
        // completion time instead of reporting a running session as finished.
        changes.finished_at = sea_orm::Set(None);
    }
    if from_running {
        changes.pause_requested = sea_orm::Set(false);
        if let Some(snapshot) = view_snapshot {
            changes.view_snapshot = sea_orm::Set(Some(json::JsonText(snapshot)));
        }
    }
    changes
}

/// The chapter consequence of a failed/abandoned archivist session.
pub(crate) fn canon_status_failed_changes(
    now: chrono::DateTime<chrono::Utc>,
) -> chapters::ActiveModel {
    chapters::ActiveModel {
        canon_status: sea_orm::Set(crate::db::json::CanonStatus::Failed),
        updated_at: sea_orm::Set(now),
        ..Default::default()
    }
}

/// The partial session update for a stored error.
pub(crate) fn error_changes(error: &SessionError) -> agent_sessions::ActiveModel {
    agent_sessions::ActiveModel {
        error: sea_orm::Set(Some(json::JsonText(error.clone()))),
        ..Default::default()
    }
}

/// The partial session update that clears the stored error (the SQL equality test only).
#[cfg(test)]
pub(crate) fn clear_error_changes() -> agent_sessions::ActiveModel {
    agent_sessions::ActiveModel {
        error: sea_orm::Set(None),
        ..Default::default()
    }
}

/// The partial session update of a stored view snapshot.
pub(crate) fn view_snapshot_changes(snapshot: ViewSnapshot) -> agent_sessions::ActiveModel {
    agent_sessions::ActiveModel {
        view_snapshot: sea_orm::Set(Some(json::JsonText(snapshot))),
        ..Default::default()
    }
}

/// Runs one expected-status-guarded `agent_sessions` update and fails with `session.invalid_state`
/// when it matches no row (spec/03 §14 / D-034). `NotSet` fields are not written; `Set(None)` is
/// SQL NULL.
pub(crate) fn guard_update_query(
    session_id: i32,
    expected: SessionStatus,
    changes: agent_sessions::ActiveModel,
) -> sea_orm::UpdateMany<agent_sessions::Entity> {
    agent_sessions::Entity::update_many()
        .set(changes)
        .filter(agent_sessions::COLUMN.id.eq(session_id))
        .filter(agent_sessions::COLUMN.status.eq(expected))
}

pub async fn guard_update(
    conn: &impl ConnectionTrait,
    session_id: i32,
    expected: SessionStatus,
    changes: agent_sessions::ActiveModel,
) -> Result<(), AppError> {
    let result = guard_update_query(session_id, expected, changes)
        .exec(conn)
        .await?;
    if result.rows_affected == 0 {
        return Err(AppError::new(
            ErrorCode::SessionInvalidState,
            format!("session {session_id} is no longer {}", expected.as_str()),
        )
        .with_details(serde_json::json!({
            "status": expected.as_str(),
            "operation": "update",
        })));
    }
    Ok(())
}

// ───────────────────────────── view snapshots ─────────────────────────────

/// What the agent last saw when the session stopped running (spec/03 §7): for a writer, the
/// chapter text, outline and reference notes; for conception, the three documents, the character
/// designs and the draft research topics.
pub async fn capture_view_snapshot(
    conn: &impl ConnectionTrait,
    session: &agent_sessions::Model,
) -> Result<Option<ViewSnapshot>, AppError> {
    let snapshot = match kind_of(session) {
        SessionKind::Conception => {
            let Some(book_id) = session.book_id else {
                return Ok(None);
            };
            conception_snapshot(conn, book_id).await?
        }
        _ => {
            let Some(chapter_id) = session.chapter_id else {
                return Ok(None);
            };
            let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(conn).await? else {
                return Ok(None);
            };
            ViewSnapshot::Chapter {
                content: chapter.content.clone().into_inner(),
                outline: chapter.outline,
                reference_notes: chapter.reference_notes,
            }
        }
    };
    Ok(Some(snapshot))
}

/// The conception view snapshot (spec/02 §4, spec/03 §7).
async fn conception_snapshot(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<ViewSnapshot, AppError> {
    use sea_orm::{ColumnTrait, EntityTrait as _, QueryFilter, QueryOrder};

    let docs = setting_docs::Entity::find()
        .filter(setting_docs::COLUMN.book_id.eq(book_id))
        .all(conn)
        .await?;
    let mut doc_map = std::collections::BTreeMap::new();
    for doc in docs {
        doc_map.insert(
            doc.kind.as_str().to_owned(),
            ConceptionDocSnapshot {
                revision: doc.revision,
                content: doc.content,
            },
        );
    }
    let characters = crate::db::entities::character_designs::Entity::find()
        .filter(
            crate::db::entities::character_designs::COLUMN
                .book_id
                .eq(book_id),
        )
        .order_by_asc(crate::db::entities::character_designs::COLUMN.rank)
        .all(conn)
        .await?
        .into_iter()
        .map(|row| ConceptionCharacterSnapshot {
            id: row.id,
            rank: row.rank,
            name: row.name,
            role_label: row.role_label,
            summary: row.summary,
            profile: row.profile,
        })
        .collect();
    let topics = research_tasks::Entity::find()
        .filter(research_tasks::COLUMN.book_id.eq(book_id))
        .filter(
            research_tasks::COLUMN
                .kind
                .eq(crate::db::json::ResearchTaskKind::Topic),
        )
        .filter(research_tasks::COLUMN.status.eq(ResearchTaskStatus::Draft))
        .order_by_asc(research_tasks::COLUMN.id)
        .all(conn)
        .await?
        .into_iter()
        .map(|row| ConceptionTopicSnapshot {
            id: Some(row.id),
            topic: row.topic,
            requirements: row.requirements,
            scope: row.scope_name,
            origin: row.origin,
        })
        .collect();
    Ok(ViewSnapshot::Conception {
        docs: doc_map,
        characters,
        topics,
    })
}

// ───────────────────────────── DTO ─────────────────────────────

/// Builds the session DTO (spec/10 §2.8).
pub async fn to_dto(
    conn: &impl ConnectionTrait,
    row: &agent_sessions::Model,
    warnings: Vec<String>,
) -> Result<SessionDto, AppError> {
    let snapshot: json::ModelSnapshot = row.model_snapshot.clone().into_inner();
    let model_label = model_label(conn, row, &snapshot).await?;
    let effort = effort_of(&snapshot);
    let chapter_number = match row.chapter_id {
        Some(chapter_id) => chapters::Entity::find_by_id(chapter_id)
            .one(conn)
            .await?
            .map(|chapter| chapter.ord),
        None => None,
    };
    let (research_topic, research_task_kind) = match row.research_task_id {
        Some(task_id) => match research_tasks::Entity::find_by_id(task_id)
            .one(conn)
            .await?
        {
            Some(task) => (Some(task.topic), Some(task.kind)),
            None => (None, None),
        },
        None => (None, None),
    };
    let child_session_id = child_session(conn, row.id).await?.map(|child| child.id);

    Ok(SessionDto {
        id: row.id,
        book_id: row.book_id,
        role: role_of(row),
        kind: kind_of(row),
        chapter_id: row.chapter_id,
        track_id: row.track_id,
        research_task_id: row.research_task_id,
        chapter_number,
        research_topic,
        research_task_kind,
        model_label,
        effort,
        status: status_of(row),
        phase: phase_of(row),
        granularity: granularity_of(row),
        auto_pause: row.auto_pause,
        pause_after_outline: row.pause_after_outline,
        pause_requested: row.pause_requested,
        held_tool_call_id: row.held_tool_call_id,
        base_content_version: row.base_content_version,
        error: error_of(row),
        usage: usage_of(row),
        counters: counters_of(row),
        parent_session_id: row.parent_session_id,
        child_session_id,
        retrying: None,
        warnings,
        created_at: row.created_at,
        updated_at: row.updated_at,
        finished_at: row.finished_at,
    })
}

/// The model's display label: the profile's `display_name` while the row exists, else the frozen
/// snapshot's model id (D-069: a deleted provider leaves the session readable).
async fn model_label(
    conn: &impl ConnectionTrait,
    row: &agent_sessions::Model,
    snapshot: &json::ModelSnapshot,
) -> Result<String, AppError> {
    if let Some(profile_id) = row.model_profile_id
        && let Some(profile) = model_profiles::Entity::find_by_id(profile_id)
            .one(conn)
            .await?
    {
        return Ok(profile.display_name);
    }
    Ok(snapshot.model.clone())
}

/// The frozen thinking effort of a session (spec/04 §5.1, D-162).
///
/// Read from the snapshot's protocol wire field, so the header shows exactly what the adapter
/// sends: `reasoning_effort` on OpenAI-compatible providers, `effort` on Anthropic, nothing on
/// the Demo provider.
fn effort_of(snapshot: &json::ModelSnapshot) -> Option<String> {
    let key = match snapshot.protocol {
        json::Protocol::Openai => "reasoning_effort",
        json::Protocol::Anthropic => "effort",
        json::Protocol::Mock => return None,
    };
    snapshot
        .options
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The child session of a superseded session (spec/03 §9), if any.
async fn child_session(
    conn: &impl ConnectionTrait,
    parent_id: i32,
) -> Result<Option<agent_sessions::Model>, AppError> {
    Ok(agent_sessions::Entity::find()
        .filter(agent_sessions::COLUMN.parent_session_id.eq(parent_id))
        .all(conn)
        .await?
        .into_iter()
        .next())
}

/// The book's conception state (used by the conception flows).
pub async fn conception_state(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<ConceptionState, AppError> {
    let book = crate::db::entities::books::Entity::find_by_id(book_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?;
    Ok(book.conception.clone().into_inner())
}

// ───────────────────────────── message and call DTOs ─────────────────────────────

/// The display cap of a tool call's raw input text (spec/10 §2.8).
pub const INPUT_TEXT_DISPLAY_LIMIT: usize = 20_000;

/// The message DTO of a row (spec/10 §2.8). Tool-call blocks are delivered as `ToolCallDto`s.
pub fn message_dto(row: &agent_messages::Model) -> Result<MessageDto, AppError> {
    let blocks: Vec<StoredBlock> = row.content.clone().into_inner();
    let blocks = blocks
        .into_iter()
        .filter_map(|block| match block {
            StoredBlock::Text { text, .. } => Some(MessageBlockDto::Text { text }),
            StoredBlock::Reasoning { text } => Some(MessageBlockDto::Reasoning { text }),
            StoredBlock::ToolCall { .. } => None,
        })
        .collect();
    Ok(MessageDto {
        id: row.id,
        seq: row.seq,
        role: row.role.as_str().to_owned(),
        kind: row.kind,
        blocks,
        stop_reason: row.stop_reason.clone(),
        usage: row.usage.clone().map(|value| value.into_inner()),
        created_at: row.created_at,
    })
}

/// The tool-call DTO of a row (spec/10 §2.8). `result_text` is `raw_result_text` before sealing
/// and the composed text after it (spec/03 §1.3).
pub fn tool_call_dto(row: &agent_tool_calls::Model) -> Result<ToolCallDto, AppError> {
    let input_text = if row.input.chars().count() > INPUT_TEXT_DISPLAY_LIMIT {
        row.input
            .chars()
            .take(INPUT_TEXT_DISPLAY_LIMIT)
            .collect::<String>()
    } else {
        row.input.clone()
    };
    Ok(ToolCallDto {
        id: row.id,
        message_id: row.message_id,
        seq: row.call_index,
        call_index: row.call_index,
        name: row.name.clone(),
        input: serde_json::from_str::<Value>(&row.input).ok(),
        input_text,
        status: row.status,
        is_error: row.is_error,
        result_text: row
            .result_text
            .clone()
            .or_else(|| row.raw_result_text.clone()),
        appendices: row.appendices.clone().into_inner(),
        ui: row.ui.clone().map(|value| value.into_inner()),
        sealed: row.sealed,
        // A deferred call stores its staged write in `effects`; a finalized call has already applied
        // and cleared it, so this is `None` exactly when the text is in the chapter (D-139).
        pending_content: row
            .effects
            .clone()
            .map(|value| value.into_inner())
            .unwrap_or_default()
            .pending_content,
        pending_outline: row
            .effects
            .clone()
            .map(|value| value.into_inner())
            .unwrap_or_default()
            .pending_outline,
        created_at: row.created_at,
        finished_at: row.finished_at,
    })
}

/// Stores the session's error column while it is still running (spec/03 §5.5, §5.7).
pub async fn set_error_running(
    conn: &impl ConnectionTrait,
    session_id: i32,
    error: &SessionError,
) -> Result<(), AppError> {
    guard_update(
        conn,
        session_id,
        SessionStatus::Running,
        error_changes(error),
    )
    .await
}

/// The number of pending inbox rows of a session (spec/10 §2.7 `inbox_pending`).
pub async fn inbox_pending(conn: &impl ConnectionTrait, session_id: i32) -> Result<i64, AppError> {
    use sea_orm::PaginatorTrait;
    Ok(crate::db::entities::agent_inbox::Entity::find()
        .filter(agent_inbox::COLUMN.session_id.eq(session_id))
        .filter(agent_inbox::COLUMN.consumed_at.is_null())
        .count(conn)
        .await? as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::manager::recovery_changes;
    use crate::agent::runner::{counters_changes, finalize_changes, persist_assistant_changes};
    use crate::api::dto::sessions::SessionPatch;
    use crate::db::entities::{agent_tool_calls, knowledge_chunks, search_docs};
    use crate::db::json::{
        ChapterContent, EmbedState, SessionCounters, ToolCallStatus, Usage, ViewSnapshot,
    };
    use crate::services::indexer::{chunk_embed_changes, doc_embed_changes};
    use crate::services::sessions::patch_changes;
    use sea_orm::{DbBackend, QueryTrait};

    fn canonical(sql: &str) -> String {
        let mut without_strings = String::new();
        let mut in_string = false;
        for character in sql.chars() {
            if in_string {
                if character == '\'' {
                    in_string = false;
                    without_strings.push('?');
                }
                continue;
            }
            if character == '\'' {
                in_string = true;
                continue;
            }
            if character != '"' {
                without_strings.push(character);
            }
        }
        without_strings
            .replace("agent_sessions.", "")
            .replace("agent_tool_calls.", "")
            .replace("chapters.", "")
            .replace("search_docs.", "")
            .replace("knowledge_chunks.", "")
            .split_whitespace()
            .map(|token| {
                let core = token.trim_end_matches(',');
                if core.parse::<i64>().is_ok() {
                    if token.ends_with(',') { "?," } else { "?" }
                } else {
                    token
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn session_sql(changes: agent_sessions::ActiveModel, expected: SessionStatus) -> String {
        super::guard_update_query(7, expected, changes)
            .build(DbBackend::Sqlite)
            .to_string()
    }

    #[test]
    fn guard_update_sql_matches_the_production_transition_builder() {
        let sql = session_sql(
            transition_changes(
                SessionStatus::Completed,
                chrono::Utc::now(),
                true,
                Some(ViewSnapshot::Chapter {
                    content: ChapterContent::default(),
                    outline: String::new(),
                    reference_notes: String::new(),
                }),
            ),
            SessionStatus::Running,
        );
        assert_eq!(
            canonical(&sql),
            "UPDATE agent_sessions SET status = ?, pause_requested = FALSE, view_snapshot = ?, updated_at = ?, finished_at = ? WHERE id = ? AND status = ?"
        );
    }

    #[test]
    fn transition_sql_matches_the_production_builder() {
        let sql = session_sql(
            transition_changes(SessionStatus::Completed, chrono::Utc::now(), false, None),
            SessionStatus::Running,
        );
        assert_eq!(
            canonical(&sql),
            "UPDATE agent_sessions SET status = ?, updated_at = ?, finished_at = ? WHERE id = ? AND status = ?"
        );
    }

    #[test]
    fn error_set_sql_matches_the_production_builder() {
        let sql = session_sql(
            error_changes(&SessionError {
                code: "agent.test".to_owned(),
                message: "test".to_owned(),
                retryable: true,
                at: chrono::Utc::now(),
            }),
            SessionStatus::Running,
        );
        assert_eq!(
            canonical(&sql),
            "UPDATE agent_sessions SET error = ? WHERE id = ? AND status = ?"
        );
    }

    #[test]
    fn error_clear_sql_matches_the_production_builder() {
        let sql = agent_sessions::Entity::update_many()
            .set(clear_error_changes())
            .filter(agent_sessions::COLUMN.id.eq(7))
            .build(DbBackend::Sqlite)
            .to_string();
        assert_eq!(
            canonical(&sql),
            "UPDATE agent_sessions SET error = NULL WHERE id = ?"
        );
    }

    #[test]
    fn canon_status_cas_sql_matches_the_production_builder() {
        let sql = chapters::Entity::update_many()
            .set(canon_status_failed_changes(chrono::Utc::now()))
            .filter(chapters::COLUMN.id.eq(9))
            .filter(
                chapters::COLUMN
                    .canon_status
                    .eq(crate::db::json::CanonStatus::Syncing),
            )
            .build(DbBackend::Sqlite)
            .to_string();
        assert_eq!(
            canonical(&sql),
            "UPDATE chapters SET canon_status = ?, updated_at = ? WHERE id = ? AND canon_status = ?"
        );
    }

    #[test]
    fn manager_recovery_sql_matches_the_production_builder() {
        let sql = agent_tool_calls::Entity::update_many()
            .set(recovery_changes())
            .filter(
                agent_tool_calls::COLUMN
                    .status
                    .eq(ToolCallStatus::Executing),
            )
            .build(DbBackend::Sqlite)
            .to_string();
        assert_eq!(
            canonical(&sql),
            "UPDATE agent_tool_calls SET status = ?, started_at = NULL WHERE status = ?"
        );
    }

    #[test]
    fn indexer_done_sql_matches_the_production_builder() {
        let sql = search_docs::Entity::update_many()
            .set(doc_embed_changes(EmbedState::Done))
            .filter(search_docs::COLUMN.id.eq(1))
            .filter(search_docs::COLUMN.content_hash.eq("hash"))
            .build(DbBackend::Sqlite)
            .to_string();
        assert_eq!(
            canonical(&sql),
            "UPDATE search_docs SET embed_state = ? WHERE id = ? AND content_hash = ?"
        );
    }

    #[test]
    fn indexer_pending_sql_matches_the_production_builder() {
        let sql = knowledge_chunks::Entity::update_many()
            .set(chunk_embed_changes(EmbedState::Pending))
            .filter(knowledge_chunks::COLUMN.embed_state.ne(EmbedState::Pending))
            .build(DbBackend::Sqlite)
            .to_string();
        assert_eq!(
            canonical(&sql),
            "UPDATE knowledge_chunks SET embed_state = ? WHERE embed_state <> ?"
        );
    }

    #[test]
    fn counters_sql_matches_the_production_builder() {
        let sql = session_sql(
            counters_changes(SessionCounters::default()),
            SessionStatus::Running,
        );
        assert_eq!(
            canonical(&sql),
            "UPDATE agent_sessions SET counters = ? WHERE id = ? AND status = ?"
        );
    }

    #[test]
    fn persist_assistant_sql_matches_the_production_builder() {
        let sql = session_sql(
            persist_assistant_changes(
                SessionCounters::default(),
                Usage::default(),
                chrono::Utc::now(),
            ),
            SessionStatus::Running,
        );
        assert_eq!(
            canonical(&sql),
            "UPDATE agent_sessions SET counters = ?, usage = ?, updated_at = ? WHERE id = ? AND status = ?"
        );
    }

    #[test]
    fn finalize_sql_matches_the_production_builder() {
        let sql = session_sql(
            finalize_changes(
                SessionCounters::default(),
                chrono::Utc::now(),
                Some(7),
                Some(Phase::Drafting),
            ),
            SessionStatus::Running,
        );
        assert_eq!(
            canonical(&sql),
            "UPDATE agent_sessions SET phase = ?, held_tool_call_id = ?, counters = ?, updated_at = ? WHERE id = ? AND status = ?"
        );
    }

    #[test]
    fn view_snapshot_sql_matches_the_production_builder() {
        let sql = session_sql(
            view_snapshot_changes(ViewSnapshot::Chapter {
                content: ChapterContent::default(),
                outline: String::new(),
                reference_notes: String::new(),
            }),
            SessionStatus::Running,
        );
        assert_eq!(
            canonical(&sql),
            "UPDATE agent_sessions SET view_snapshot = ? WHERE id = ? AND status = ?"
        );
    }

    #[test]
    fn patch_sql_matches_the_production_builder() {
        let changes = patch_changes(
            &SessionPatch {
                granularity: Some(Granularity::Paragraph),
                auto_pause: Some(true),
                pause_after_outline: None,
            },
            chrono::Utc::now(),
        )
        .expect("a non-empty patch");
        let sql = session_sql(changes, SessionStatus::Held);
        assert_eq!(
            canonical(&sql),
            "UPDATE agent_sessions SET granularity = ?, auto_pause = TRUE, updated_at = ? WHERE id = ? AND status = ?"
        );
    }

    /// `SessionDto.effort` reads exactly the protocol's wire field from the frozen snapshot
    /// (spec/04 §5.1, D-162).
    #[test]
    fn effort_reads_the_protocol_wire_field() {
        let mut snapshot = json::ModelSnapshot {
            protocol: json::Protocol::Openai,
            options: serde_json::json!({ "reasoning_effort": "max" }),
            ..json::ModelSnapshot::default()
        };
        assert_eq!(effort_of(&snapshot), Some("max".to_owned()));
        snapshot.protocol = json::Protocol::Anthropic;
        snapshot.options = serde_json::json!({ "effort": "low" });
        assert_eq!(effort_of(&snapshot), Some("low".to_owned()));
        snapshot.protocol = json::Protocol::Mock;
        assert_eq!(effort_of(&snapshot), None);
        snapshot.protocol = json::Protocol::Openai;
        snapshot.options = serde_json::json!({});
        assert_eq!(effort_of(&snapshot), None);
    }
}
