//! Canon maintenance and re-sync services (spec/06 §6, §8; spec/08 §1.1; spec/03 §14).
//!
//! Maintenance runs on official text only (D-011). Approval starts it when `auto_maintenance` is
//! on; `maintain` retries a failed chapter and `resync` re-syncs canon after an author edit of an
//! official chapter. The completion handler (which decides `synced` / `stale`) lives in
//! [`crate::agent::completion`], because it runs inside the transaction that finalizes the call
//! (spec/03 §5.4, D-033).

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::json;

use crate::agent::session;
use crate::api::dto::reviews::ReviewSessionResponse;
use crate::db::entities::{agent_sessions, chapter_versions, chapters};
use crate::db::json::{CanonStatus, ChapterContent, ChapterStatus, SessionKind, SessionStatus};
use crate::error::{AppError, ErrorCode};
use crate::services::sessions::{self, CreateSession};
use crate::services::versions::{self, NewVersion};
use crate::state::AppState;

/// The archivist session kinds that maintain a chapter's canon.
pub const ARCHIVIST_KINDS: [&str; 2] = ["chapter_maintenance", "chapter_resync"];

/// `POST /api/chapters/:id/maintain` (spec/08 §1.1): continue the chapter's open archivist session
/// when there is one (failed), otherwise start the kind that failed last.
pub async fn maintain(
    state: &AppState,
    chapter_id: i32,
) -> Result<ReviewSessionResponse, AppError> {
    let chapter = chapters::Entity::find_by_id(chapter_id)
        .one(state.db().conn())
        .await?
        .ok_or_else(|| AppError::not_found(format!("chapter {chapter_id}")))?;
    require_official(&chapter)?;
    let canon = canon_status_of(&chapter);
    if !matches!(canon, CanonStatus::Pending | CanonStatus::Failed) {
        return Err(canon_state_error(&chapter, "maintain"));
    }

    if let Some(open) = open_archivist_session(state.db().conn(), chapter_id).await? {
        // Continuing a failed session puts the chapter back to `syncing` (spec/06 §6). The status
        // update and the session recheck/continue are one transaction (M5-04): an abandon cannot
        // leave a `syncing` chapter with no recoverable open session.
        let status = session::status_of(&open);
        if matches!(
            status,
            SessionStatus::Stopped | SessionStatus::Failed | SessionStatus::Interrupted
        ) {
            sessions::continue_session_and_mark_syncing(state, open.id, chapter_id).await?;
        }
        return Ok(ReviewSessionResponse {
            session_id: open.id,
        });
    }

    // "starts a new session of the kind that failed last (`chapter_resync` after an abandoned
    // re-sync, so canon is not maintained twice)" (spec/08 §1.1).
    let kind = last_archivist_kind(state.db().conn(), chapter_id)
        .await?
        .unwrap_or(SessionKind::ChapterMaintenance);
    let (session_id, _warnings) = sessions::create(
        state,
        CreateSession {
            kind,
            book_id: Some(chapter.book_id),
            chapter_id: Some(chapter_id),
            ..CreateSession::default()
        },
    )
    .await?;
    Ok(ReviewSessionResponse { session_id })
}

/// `POST /api/chapters/:id/resync` (spec/08 §1.1, spec/06 §8): save the current text as a new
/// `official` version (reusing an identical one, D-046), then start `chapter_resync`.
pub async fn resync(state: &AppState, chapter_id: i32) -> Result<ReviewSessionResponse, AppError> {
    let state_owned = state.clone();
    let (session_id, events) = crate::services::write(state.db(), move |tx| {
        Box::pin(async move {
            let chapter = chapters::Entity::find_by_id(chapter_id)
                .one(tx)
                .await?
                .ok_or_else(|| AppError::not_found(format!("chapter {chapter_id}")))?;
            require_official(&chapter)?;
            if canon_status_of(&chapter) != CanonStatus::Stale {
                return Err(canon_state_error(&chapter, "resync"));
            }
            if let Some(open) = open_archivist_session(tx, chapter_id).await? {
                return Err(AppError::new(
                    ErrorCode::ChapterSessionExists,
                    "the chapter already has an open archivist session",
                )
                .with_details(json!({ "session_id": open.id })));
            }
            let content: ChapterContent = crate::db::json::from_json(&chapter.content)
                .map_err(|err| AppError::internal(format!("corrupt chapter content: {err}")))?;
            // "an existing official version with identical content is reused" (spec/06 §8,
            // D-046): the diff then shows only the author's real edits.
            let official_id = official_version_with_content(tx, chapter_id, &content).await?;
            let version_id = match official_id {
                Some(id) => id,
                None => {
                    let word_count = crate::text::content_word_count(&content);
                    versions::create_version(
                        tx,
                        NewVersion {
                            chapter_id,
                            kind: crate::db::json::VersionKind::Official,
                            title: &chapter.title,
                            content: &content,
                            word_count,
                            note: "",
                            session_id: None,
                        },
                    )
                    .await?
                    .id
                }
            };
            chapters::ActiveModel {
                id: Set(chapter_id),
                official_version_id: Set(Some(version_id)),
                updated_at: Set(crate::services::now()),
                ..Default::default()
            }
            .update(tx)
            .await?;
            let (session_id, events, _warnings) = sessions::create_inner(
                &state_owned,
                tx,
                CreateSession {
                    kind: SessionKind::ChapterResync,
                    book_id: Some(chapter.book_id),
                    chapter_id: Some(chapter_id),
                    ..CreateSession::default()
                },
            )
            .await?;
            Ok((session_id, events))
        })
    })
    .await?;
    state.events().publish_all(events);
    state.manager().spawn(state.clone(), session_id);
    Ok(ReviewSessionResponse { session_id })
}

/// Starts `chapter_maintenance` inside an existing transaction (approval with `auto_maintenance`).
///
/// Returns `Ok(None)` and appends `model.missing_role` when no archivist model is configured: the
/// chapter still becomes official and its canon stays `pending` (spec/08 §1.1, D-046).
pub(crate) async fn start_after_approval(
    state: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    chapter: &chapters::Model,
) -> Result<(Option<i32>, Vec<crate::events::ServerEvent>, Vec<String>), AppError> {
    match sessions::create_inner(
        state,
        tx,
        CreateSession {
            kind: SessionKind::ChapterMaintenance,
            book_id: Some(chapter.book_id),
            chapter_id: Some(chapter.id),
            ..CreateSession::default()
        },
    )
    .await
    {
        Ok((session_id, events, warnings)) => Ok((Some(session_id), events, warnings)),
        Err(error) if error.code == ErrorCode::ModelMissingRole => {
            Ok((None, Vec::new(), vec!["model.missing_role".to_owned()]))
        }
        Err(error) => Err(error),
    }
}

/// Reads `books.settings.auto_maintenance` (spec/02 §4, default on).
pub async fn auto_maintenance_enabled(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<bool, AppError> {
    let book = crate::db::entities::books::Entity::find_by_id(book_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?;
    let settings: crate::db::json::BookSettings =
        crate::db::json::from_json_or_default(&book.settings);
    Ok(settings.auto_maintenance)
}

/// The chapter's open maintenance or re-sync session, oldest first.
pub async fn open_archivist_session(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
) -> Result<Option<agent_sessions::Model>, AppError> {
    Ok(agent_sessions::Entity::find()
        .filter(agent_sessions::Column::ChapterId.eq(chapter_id))
        .filter(agent_sessions::Column::Kind.is_in(ARCHIVIST_KINDS))
        .filter(agent_sessions::Column::Status.is_not_in([
            SessionStatus::Completed.as_str(),
            SessionStatus::Superseded.as_str(),
            SessionStatus::Abandoned.as_str(),
        ]))
        .order_by_asc(agent_sessions::Column::Id)
        .one(conn)
        .await?)
}

/// The newest archivist session of the chapter, whatever its status.
pub async fn last_archivist_kind(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
) -> Result<Option<SessionKind>, AppError> {
    let row = agent_sessions::Entity::find()
        .filter(agent_sessions::Column::ChapterId.eq(chapter_id))
        .filter(agent_sessions::Column::Kind.is_in(ARCHIVIST_KINDS))
        .order_by_desc(agent_sessions::Column::Id)
        .one(conn)
        .await?;
    Ok(row.and_then(|row| SessionKind::from_str_opt(&row.kind)))
}

/// An `official` version whose text equals `content`, newest first (spec/06 §8: "an existing
/// official version with identical content is reused").
///
/// The comparison is over the **text**, not the whole `ChapterContent`: `next_pid` is a monotonically
/// growing allocation counter (spec/02 §5 invariant 3), not part of the prose, so restoring text
/// that is byte-identical paragraph for paragraph must reuse its official version even though the
/// chapter's counter has moved on since (D-123). Comparing the counter too made every re-sync after
/// a restore create a second official version with the same text.
async fn official_version_with_content(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
    content: &ChapterContent,
) -> Result<Option<i32>, AppError> {
    let rows = chapter_versions::Entity::find()
        .filter(chapter_versions::Column::ChapterId.eq(chapter_id))
        .filter(chapter_versions::Column::Kind.eq(crate::db::json::VersionKind::Official.as_str()))
        .order_by_desc(chapter_versions::Column::Id)
        .all(conn)
        .await?;
    for row in rows {
        if let Ok(stored) = crate::db::json::from_json::<ChapterContent>(&row.content)
            && stored.paragraphs == content.paragraphs
            && stored.tail_open == content.tail_open
        {
            return Ok(Some(row.id));
        }
    }
    Ok(None)
}

/// Reads the canon status column.
pub fn canon_status_of(chapter: &chapters::Model) -> CanonStatus {
    CanonStatus::from_str_opt(&chapter.canon_status).unwrap_or(CanonStatus::None)
}

/// Rejects an operation on a chapter that is not official.
fn require_official(chapter: &chapters::Model) -> Result<(), AppError> {
    let status = ChapterStatus::from_str_opt(&chapter.status).unwrap_or(ChapterStatus::Planned);
    if status == ChapterStatus::Official {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::ChapterInvalidState,
        "only an official chapter has canon to maintain",
    )
    .with_details(json!({ "status": status.as_str() })))
}

/// The `409 chapter.invalid_state` of a maintain/re-sync in the wrong canon status.
fn canon_state_error(chapter: &chapters::Model, operation: &str) -> AppError {
    AppError::new(
        ErrorCode::ChapterInvalidState,
        format!("canon cannot be {operation}ed in this state"),
    )
    .with_details(json!({
        "canon_status": canon_status_of(chapter).as_str(),
        "operation": operation,
    }))
}
