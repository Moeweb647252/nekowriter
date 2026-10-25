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
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, Statement};
use serde_json::Value;

use crate::api::dto::sessions::{MessageBlockDto, MessageDto, SessionDto, ToolCallDto};
use crate::db::entities::{
    agent_inbox, agent_messages, agent_sessions, agent_tool_calls, chapters, model_profiles,
    research_tasks, setting_docs,
};
use crate::db::json::{
    self, ConceptionCharacterSnapshot, ConceptionDocSnapshot, ConceptionState,
    ConceptionTopicSnapshot, Granularity, MessageKind, Phase, SessionCounters, SessionError,
    SessionKind, SessionStatus, StoredBlock, ToolCallStatus, Usage, ViewSnapshot,
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
    row.status == SessionStatus::Running.as_str()
}

/// Reads the status column as its enum.
pub fn status_of(row: &agent_sessions::Model) -> SessionStatus {
    SessionStatus::from_str_opt(&row.status).unwrap_or(SessionStatus::Failed)
}

/// Reads the phase column as its enum.
pub fn phase_of(row: &agent_sessions::Model) -> Phase {
    Phase::from_str_opt(&row.phase).unwrap_or(Phase::None)
}

/// Reads the kind column as its enum.
pub fn kind_of(row: &agent_sessions::Model) -> SessionKind {
    SessionKind::from_str_opt(&row.kind).unwrap_or(SessionKind::ChapterWrite)
}

/// Reads the role column as its enum.
pub fn role_of(row: &agent_sessions::Model) -> crate::db::json::AgentRole {
    crate::db::json::AgentRole::from_str_opt(&row.role)
        .unwrap_or(crate::db::json::AgentRole::Writer)
}

/// Reads the granularity column, when set.
pub fn granularity_of(row: &agent_sessions::Model) -> Option<Granularity> {
    row.granularity
        .as_deref()
        .and_then(Granularity::from_str_opt)
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
    json::from_json_or_default(&row.counters)
}

/// The session's cumulative usage.
pub fn usage_of(row: &agent_sessions::Model) -> Usage {
    json::from_json_or_default(&row.usage)
}

/// The session's stored error, when it has one.
pub fn error_of(row: &agent_sessions::Model) -> Option<SessionError> {
    row.error
        .as_deref()
        .and_then(|raw| json::from_json::<SessionError>(raw).ok())
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
    let mut sets: Vec<&'static str> = vec!["status = ?", "updated_at = ?"];
    let mut values: Vec<sea_orm::Value> = vec![to.as_str().into(), now.into()];
    let terminal = is_terminal(to);
    if terminal {
        sets.push("finished_at = ?");
        values.push(now.into());
    } else {
        // `finished_at` describes a terminal status only (spec/03 §3). A re-review re-opens a
        // completed reviewer session (spec/03 §6), so leaving the terminal set must clear the old
        // completion time instead of reporting a running session as finished.
        sets.push("finished_at = NULL");
    }
    if from_running {
        sets.push("pause_requested = 0");
        // The oldest unreported snapshot wins: keep the stored one when there is one.
        if session.view_snapshot.is_none()
            && matches!(
                kind_of(session),
                SessionKind::ChapterWrite
                    | SessionKind::ChapterRevise
                    | SessionKind::ChapterReview
                    | SessionKind::ChapterMaintenance
                    | SessionKind::ChapterResync
                    | SessionKind::CanonSeed
                    | SessionKind::Conception
            )
            && let Some(snapshot) = capture_view_snapshot(conn, session).await?
        {
            sets.push("view_snapshot = ?");
            values.push(snapshot.into());
        }
    }

    let sql = format!(
        "UPDATE agent_sessions SET {} WHERE id = ? AND status = ?",
        sets.join(", ")
    );
    values.push(session.id.into());
    values.push(session.status.clone().into());
    let result = conn
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            &sql,
            values,
        ))
        .await?;
    if result.rows_affected() == 0 {
        tracing::warn!(
            session_id = session.id,
            expected = %session.status,
            to = to.as_str(),
            "session transition lost its expected-status guard"
        );
        return Err(AppError::new(
            ErrorCode::SessionInvalidState,
            format!("session {} is no longer {}", session.id, session.status),
        )
        .with_details(serde_json::json!({
            "status": session.status,
            "operation": "transition",
        })));
    }
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
        conn.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "UPDATE chapters SET canon_status = 'failed', updated_at = ? \
             WHERE id = ? AND canon_status = 'syncing'",
            vec![now.into(), chapter_id.into()],
        ))
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
            crate::services::research::on_session_status(conn, task_id, session.id, to.as_str())
                .await?,
        );
    }
    Ok(events)
}

/// Stores the session's error column (spec/03 §5.5, §5.7) without touching the status.
pub async fn store_error(
    conn: &impl ConnectionTrait,
    session_id: i32,
    error: &SessionError,
) -> Result<(), AppError> {
    let raw = json::to_json(error)
        .map_err(|err| AppError::internal(format!("cannot serialize session error: {err}")))?;
    guard_columns(
        conn,
        session_id,
        SessionStatus::Running.as_str(),
        "error = ?",
        vec![raw.into()],
    )
    .await?;
    Ok(())
}

/// Clears the stored error (spec/03 §6: continue and resume clear it).
pub async fn clear_error(conn: &impl ConnectionTrait, session_id: i32) -> Result<(), AppError> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "UPDATE agent_sessions SET error = NULL WHERE id = ?",
        vec![session_id.into()],
    ))
    .await?;
    Ok(())
}

/// Runs one column-wise, expected-status-guarded update and fails with `session.invalid_state`
/// when it matches no row (spec/03 §14 / D-034).
pub async fn guard_columns(
    conn: &impl ConnectionTrait,
    session_id: i32,
    expected_status: &str,
    set_sql: &str,
    mut values: Vec<sea_orm::Value>,
) -> Result<(), AppError> {
    let sql = format!("UPDATE agent_sessions SET {set_sql} WHERE id = ? AND status = ?");
    values.push(session_id.into());
    values.push(expected_status.to_owned().into());
    let result = conn
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            &sql,
            values,
        ))
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::new(
            ErrorCode::SessionInvalidState,
            format!("session {session_id} is no longer {expected_status}"),
        )
        .with_details(serde_json::json!({
            "status": expected_status,
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
) -> Result<Option<String>, AppError> {
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
                content: json::from_json_or_default(&chapter.content),
                outline: chapter.outline,
                reference_notes: chapter.reference_notes,
            }
        }
    };
    let raw = json::to_json(&snapshot)
        .map_err(|err| AppError::internal(format!("cannot serialize view snapshot: {err}")))?;
    Ok(Some(raw))
}

/// The conception view snapshot (spec/02 §4, spec/03 §7).
async fn conception_snapshot(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<ViewSnapshot, AppError> {
    use sea_orm::{ColumnTrait, EntityTrait as _, QueryFilter, QueryOrder};

    let docs = setting_docs::Entity::find()
        .filter(setting_docs::Column::BookId.eq(book_id))
        .all(conn)
        .await?;
    let mut doc_map = std::collections::BTreeMap::new();
    for doc in docs {
        doc_map.insert(
            doc.kind.clone(),
            ConceptionDocSnapshot {
                revision: doc.revision,
                content: doc.content,
            },
        );
    }
    let characters = crate::db::entities::character_designs::Entity::find()
        .filter(crate::db::entities::character_designs::Column::BookId.eq(book_id))
        .order_by_asc(crate::db::entities::character_designs::Column::Rank)
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
        .filter(research_tasks::Column::BookId.eq(book_id))
        .filter(research_tasks::Column::Status.eq("draft"))
        .order_by_asc(research_tasks::Column::Id)
        .all(conn)
        .await?
        .into_iter()
        .map(|row| ConceptionTopicSnapshot {
            topic: row.topic,
            requirements: row.requirements,
            scope: row.scope_name,
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
    let snapshot: json::ModelSnapshot = json::from_json_or_default(&row.model_snapshot);
    let model_label = model_label(conn, row, &snapshot).await?;
    let chapter_number = match row.chapter_id {
        Some(chapter_id) => chapters::Entity::find_by_id(chapter_id)
            .one(conn)
            .await?
            .map(|chapter| chapter.ord),
        None => None,
    };
    let research_topic = match row.research_task_id {
        Some(task_id) => research_tasks::Entity::find_by_id(task_id)
            .one(conn)
            .await?
            .map(|task| task.topic),
        None => None,
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
        model_label,
        status: status_of(row),
        phase: phase_of(row),
        granularity: granularity_of(row),
        auto_pause: row.auto_pause != 0,
        pause_after_outline: row.pause_after_outline != 0,
        pause_requested: row.pause_requested != 0,
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

/// The child session of a superseded session (spec/03 §9), if any.
async fn child_session(
    conn: &impl ConnectionTrait,
    parent_id: i32,
) -> Result<Option<agent_sessions::Model>, AppError> {
    Ok(agent_sessions::Entity::find()
        .filter(agent_sessions::Column::ParentSessionId.eq(parent_id))
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
    Ok(json::from_json_or_default(&book.conception))
}

// ───────────────────────────── message and call DTOs ─────────────────────────────

/// The display cap of a tool call's raw input text (spec/10 §2.8).
pub const INPUT_TEXT_DISPLAY_LIMIT: usize = 20_000;

/// The message DTO of a row (spec/10 §2.8). Tool-call blocks are delivered as `ToolCallDto`s.
pub fn message_dto(row: &agent_messages::Model) -> Result<MessageDto, AppError> {
    let blocks: Vec<StoredBlock> = json::from_json_or_default(&row.content);
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
        role: row.role.clone(),
        kind: MessageKind::from_str_opt(&row.kind).unwrap_or(MessageKind::Assistant),
        blocks,
        stop_reason: row.stop_reason.clone(),
        usage: row
            .usage
            .as_deref()
            .and_then(|raw| json::from_json::<json::Usage>(raw).ok()),
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
        status: ToolCallStatus::from_str_opt(&row.status).unwrap_or(ToolCallStatus::Error),
        is_error: row.is_error != 0,
        result_text: row
            .result_text
            .clone()
            .or_else(|| row.raw_result_text.clone()),
        appendices: json::from_json_or_default(&row.appendices),
        ui: row
            .ui
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok()),
        sealed: row.sealed != 0,
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
    let raw = json::to_json(error)
        .map_err(|err| AppError::internal(format!("cannot serialize session error: {err}")))?;
    guard_columns(
        conn,
        session_id,
        SessionStatus::Running.as_str(),
        "error = ?",
        vec![raw.into()],
    )
    .await
}

/// The number of pending inbox rows of a session (spec/10 §2.7 `inbox_pending`).
pub async fn inbox_pending(conn: &impl ConnectionTrait, session_id: i32) -> Result<i64, AppError> {
    use sea_orm::PaginatorTrait;
    Ok(crate::db::entities::agent_inbox::Entity::find()
        .filter(agent_inbox::Column::SessionId.eq(session_id))
        .filter(agent_inbox::Column::ConsumedAt.is_null())
        .count(conn)
        .await? as i64)
}
