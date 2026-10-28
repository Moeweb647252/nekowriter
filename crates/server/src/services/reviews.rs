//! Review services: reports, ask-the-reviewer, re-review and send-back (spec/08 §1.1–§1.2, §3;
//! spec/03 §6; spec/10 §2.2).
//!
//! Re-review has two shapes (spec/03 §6, D-046). When the chapter's `content_version` still
//! equals the report's, the completed session continues with a `reviewer.rereview` user message.
//! When the text changed, a **new** `chapter_review` session starts, whose `reviewer.kickoff`
//! carries the previous report and the comment (`prior`). Both store the comment in
//! `pending_comment`, which `submit_review` copies into the report's `user_comment` and then
//! clears (spec/05 §5).
//!
//! The send-back path stores `chapter_feedback` and the revision/redraft session in one
//! transaction, so a failed session start leaves no feedback row behind.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::json;

use crate::agent::runner;
use crate::agent::session;
use crate::api::dto::reviews::{
    ChapterFeedbackDto, RereviewInput, ReviewReportDto, ReviewSessionResponse, ReviewsResponse,
    SendBackInput,
};
use crate::db::entities::{agent_sessions, chapter_feedback, chapters, review_reports};
use crate::db::json::{
    ChapterStatus, FeedbackMode, MessageKind, ReviewIssue, SessionKind, SessionStatus, StoredBlock,
};
use crate::error::{AppError, ErrorCode};
use crate::events::ServerEvent;
use crate::prompts::context::RereviewContext;
use crate::services::sessions::{self, CreateSession};
use crate::state::AppState;

/// A reviewer session's role string (spec/05 §2).
const REVIEWER_ROLE: &str = "reviewer";

/// `GET /api/chapters/:id/reviews` (spec/10 §2.2).
pub async fn reports(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
) -> Result<ReviewsResponse, AppError> {
    chapters::Entity::find_by_id(chapter_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("chapter {chapter_id}")))?;
    let reports = review_reports::Entity::find()
        .filter(review_reports::Column::ChapterId.eq(chapter_id))
        .order_by_desc(review_reports::Column::Id)
        .all(conn)
        .await?
        .into_iter()
        .map(ReviewReportDto::from)
        .collect();
    let feedback = chapter_feedback::Entity::find()
        .filter(chapter_feedback::Column::ChapterId.eq(chapter_id))
        .order_by_desc(chapter_feedback::Column::Id)
        .all(conn)
        .await?
        .into_iter()
        .map(ChapterFeedbackDto::from)
        .collect();
    Ok(ReviewsResponse { reports, feedback })
}

/// `POST /api/chapters/:id/review` — ask the reviewer (spec/08 §1.1).
pub async fn ask_reviewer(
    state: &AppState,
    chapter_id: i32,
) -> Result<ReviewSessionResponse, AppError> {
    let chapter = chapters::Entity::find_by_id(chapter_id)
        .one(state.db().conn())
        .await?
        .ok_or_else(|| AppError::not_found(format!("chapter {chapter_id}")))?;
    require_in_review(&chapter)?;

    let open = open_reviewer_sessions(state.db().conn(), chapter_id).await?;
    // "Returns the open reviewer session if its base_content_version is the current
    // content_version" (spec/08 §1.1).
    if let Some(session) = open
        .iter()
        .find(|session| session.base_content_version == Some(chapter.content_version))
    {
        return Ok(ReviewSessionResponse {
            session_id: session.id,
        });
    }
    // "otherwise abandons that session (if any) and starts chapter_review".
    for session in open {
        sessions::abandon(state, session.id).await?;
    }
    let (session_id, _warnings) = match sessions::create(
        state,
        CreateSession {
            kind: SessionKind::ChapterReview,
            book_id: Some(chapter.book_id),
            chapter_id: Some(chapter_id),
            chapter_guard: true,
            force_previous: false,
            ..CreateSession::default()
        },
    )
    .await
    {
        Ok(created) => created,
        // A concurrent "ask the reviewer" created the chapter's open reviewer session between the
        // check above and the insert. Session creation refuses the second one with
        // `chapter.session_exists` (spec/02 §5 invariant 5, D-121), and the request the author
        // made — "review this chapter" — is satisfied by the session that won, which is what the
        // first branch above already returns for a matching `base_content_version`. So return it
        // instead of surfacing a 409 for a double click.
        Err(error) if error.code == ErrorCode::ChapterSessionExists => {
            let open = open_reviewer_sessions(state.db().conn(), chapter_id).await?;
            match open
                .iter()
                .find(|session| session.base_content_version == Some(chapter.content_version))
                .or_else(|| open.first())
            {
                Some(session) => (session.id, Vec::new()),
                None => return Err(error),
            }
        }
        Err(error) => return Err(error),
    };
    Ok(ReviewSessionResponse { session_id })
}

/// `POST /api/review-reports/:id/rereview` (spec/03 §6).
pub async fn rereview(
    state: &AppState,
    report_id: i32,
    input: RereviewInput,
) -> Result<ReviewSessionResponse, AppError> {
    let comment = input.comment.trim().to_owned();
    if comment.is_empty() {
        return Err(AppError::validation("the comment is empty"));
    }
    let conn = state.db().conn();
    let report = review_reports::Entity::find_by_id(report_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("review report {report_id}")))?;
    let chapter = chapters::Entity::find_by_id(report.chapter_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("chapter {}", report.chapter_id)))?;
    require_in_review(&chapter)?;

    let Some(session_id) = report.session_id else {
        return Err(AppError::new(
            ErrorCode::SessionInvalidState,
            "the report's session no longer exists",
        )
        .with_details(json!({ "status": "missing", "operation": "rereview" })));
    };
    let reviewer = session::load(conn, session_id).await?;
    if session::status_of(&reviewer) != SessionStatus::Completed {
        return Err(AppError::new(
            ErrorCode::SessionInvalidState,
            "the reviewer session is still working",
        )
        .with_details(json!({
            "status": reviewer.status,
            "operation": "rereview",
        })));
    }

    if chapter.content_version != report.content_version {
        // The text changed since that report: a new session carries the previous report and the
        // comment in its kickoff (spec/03 §6, D-046).
        let (session_id, _warnings) = sessions::create(
            state,
            CreateSession {
                kind: SessionKind::ChapterReview,
                book_id: Some(chapter.book_id),
                chapter_id: Some(chapter.id),
                pending_comment: Some(comment),
                kickoff_extra: Some(json!({ "prior_report_id": report.id })),
                chapter_guard: true,
                force_previous: false,
                ..CreateSession::default()
            },
        )
        .await?;
        return Ok(ReviewSessionResponse { session_id });
    }

    // Same text: continue the completed session with the rendered re-review request (spec/03 §6).
    let prompts = state
        .prompts()
        .for_book(state.db(), Some(chapter.book_id))
        .await?;
    let rendered = prompts
        .render_value(
            "reviewer.rereview",
            crate::prompts::template_value(&json!({
                "review": RereviewContext {
                    comment: comment.clone(),
                    attempt: report.attempt,
                },
            })),
        )
        .map_err(|err| err.into_app_error())?;
    let comment_for_store = comment.clone();
    let (events, message) = crate::services::write(state.db(), move |tx| {
        Box::pin(async move {
            let current_chapter = chapters::Entity::find_by_id(chapter.id)
                .one(tx)
                .await?
                .ok_or_else(|| AppError::not_found(format!("chapter {}", chapter.id)))?;
            require_in_review(&current_chapter)?;
            let current = session::load(tx, session_id).await?;
            if session::status_of(&current) != SessionStatus::Completed {
                return Err(AppError::new(
                    ErrorCode::SessionInvalidState,
                    "the reviewer session is still working",
                )
                .with_details(json!({
                    "status": current.status,
                    "operation": "rereview",
                })));
            }
            // The next report copies this into `user_comment` and clears it (spec/05 §5).
            agent_sessions::ActiveModel {
                id: Set(session_id),
                pending_comment: Set(Some(comment_for_store)),
                ..Default::default()
            }
            .update(tx)
            .await?;
            let message = runner::insert_user_message(
                tx,
                session_id,
                MessageKind::User,
                vec![StoredBlock::Text {
                    text: rendered.text,
                    cache: None,
                }],
            )
            .await?;
            let events = session::transition(
                tx,
                &current,
                SessionStatus::Running,
                session::TransitionCause::Rereview,
            )
            .await?;
            Ok((events, message))
        })
    })
    .await?;
    let mut events = events;
    events.push(ServerEvent::SessionMessage {
        session_id,
        message: session::message_dto(&message)?,
    });
    state.events().publish_all(events);
    state.manager().spawn(state.clone(), session_id);
    Ok(ReviewSessionResponse { session_id })
}

/// `POST /api/chapters/:id/send-back` (spec/08 §1.1): store the feedback, then start the revision
/// or redraft session. Everything happens in one transaction, so a failed session start leaves no
/// feedback row behind.
pub async fn send_back(
    state: &AppState,
    chapter_id: i32,
    input: SendBackInput,
) -> Result<ReviewSessionResponse, AppError> {
    let feedback_text = input.feedback.trim().to_owned();
    if feedback_text.is_empty() {
        return Err(AppError::invalid_fields(serde_json::Map::from_iter([(
            "feedback".to_owned(),
            json!("the feedback is empty"),
        )])));
    }
    let conn = state.db().conn();
    let chapter = chapters::Entity::find_by_id(chapter_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("chapter {chapter_id}")))?;
    require_in_review(&chapter)?;
    // A redraft drafts from the current accepted outline; without one the dialog offers Revise in
    // place or writing an outline instead (spec/08 §1.1, D-146).
    if input.mode == FeedbackMode::Redraft && chapter.outline.trim().is_empty() {
        return Err(AppError::new(
            ErrorCode::ChapterOutlineMissing,
            "the chapter has no outline to redraft from",
        ));
    }

    // The ticked issues come from the report the dialog was built from (spec/08 §3). The report
    // is validated whenever it is named — not only when issues are ticked: a dangling or foreign
    // report id would otherwise be stored (and an unknown id even reached the foreign-key
    // constraint as an internal error).
    let mut issues: Vec<ReviewIssue> = Vec::new();
    match input.review_report_id {
        Some(report_id) => {
            let report = review_reports::Entity::find_by_id(report_id)
                .one(conn)
                .await?
                .ok_or_else(|| AppError::not_found(format!("review report {report_id}")))?;
            if report.chapter_id != chapter_id {
                return Err(AppError::validation(
                    "the review report belongs to another chapter",
                ));
            }
            let all: Vec<ReviewIssue> = crate::db::json::from_json_or_default(&report.issues);
            for index in &input.issue_indexes {
                let issue = all.get(*index).ok_or_else(|| {
                    AppError::invalid_fields(serde_json::Map::from_iter([(
                        "issue_indexes".to_owned(),
                        json!(format!("no issue at index {index}")),
                    )]))
                })?;
                issues.push(issue.clone());
            }
        }
        None if !input.issue_indexes.is_empty() => {
            return Err(AppError::invalid_fields(serde_json::Map::from_iter([(
                "issue_indexes".to_owned(),
                json!("issues require the report they came from"),
            )])));
        }
        None => {}
    }

    // "abandons an open reviewer session" (spec/08 §1.1).
    for reviewer in open_reviewer_sessions(conn, chapter_id).await? {
        sessions::abandon(state, reviewer.id).await?;
    }

    let mode = input.mode;
    let state_owned = state.clone();
    let feedback_for_row = feedback_text.clone();
    let (session_id, events) = crate::services::write(state.db(), move |tx| {
        Box::pin(async move {
            // The `require_in_review` above is a read on a different connection; an approve can
            // commit `official` before this transaction. Re-read and revalidate here, before the
            // feedback row or any creation side effect (R4-01). This is intentionally not the
            // generic `chapter_guard`: an in-review redraft is legal and must stay legal.
            let current = chapters::Entity::find_by_id(chapter_id)
                .one(tx)
                .await?
                .ok_or_else(|| AppError::not_found(format!("chapter {chapter_id}")))?;
            require_in_review(&current)?;
            if mode == FeedbackMode::Redraft && current.outline.trim().is_empty() {
                return Err(AppError::new(
                    ErrorCode::ChapterOutlineMissing,
                    "the chapter has no outline to redraft from",
                ));
            }
            let row = chapter_feedback::ActiveModel {
                chapter_id: Set(chapter_id),
                content_version: Set(current.content_version),
                review_report_id: Set(input.review_report_id),
                text: Set(feedback_for_row),
                issues: Set(crate::db::json::to_json(&issues)
                    .map_err(|err| AppError::internal(err.to_string()))?),
                mode: Set(mode.as_str().to_owned()),
                created_at: Set(chrono::Utc::now()),
                ..Default::default()
            }
            .insert(tx)
            .await?;
            // The kind and its creation side effects: `revise` saves a backup and moves the
            // chapter to `revising`; `redraft` saves a backup, clears the text and moves it to
            // `drafting` (spec/08 §1.1, spec/03 §4).
            let input = match mode {
                FeedbackMode::Revise => CreateSession {
                    kind: SessionKind::ChapterRevise,
                    book_id: Some(current.book_id),
                    chapter_id: Some(chapter_id),
                    ..CreateSession::default()
                },
                FeedbackMode::Redraft => CreateSession {
                    kind: SessionKind::ChapterWrite,
                    book_id: Some(current.book_id),
                    chapter_id: Some(chapter_id),
                    redraft: true,
                    // The feedback row is linked to the session only after `create_inner`
                    // returns, so the kickoff finds it by id (D-146).
                    kickoff_extra: Some(json!({ "feedback_id": row.id })),
                    ..CreateSession::default()
                },
            };
            let (session_id, events, _warnings) =
                sessions::create_inner(&state_owned, tx, input).await?;
            chapter_feedback::ActiveModel {
                id: Set(row.id),
                session_id: Set(Some(session_id)),
                ..Default::default()
            }
            .update(tx)
            .await?;
            Ok((session_id, events))
        })
    })
    .await?;
    state.events().publish_all(events);
    // A reviewer session can appear between the pre-loop above and this write transaction
    // (M5-07); the commit must not leave it open after the chapter leaves `in_review`.
    for reviewer in open_reviewer_sessions(state.db().conn(), chapter_id).await? {
        sessions::abandon(state, reviewer.id).await?;
    }
    state.manager().spawn(state.clone(), session_id);
    Ok(ReviewSessionResponse { session_id })
}

/// The chapter's open reviewer sessions, oldest first (spec/03 §3: open = not terminal).
pub async fn open_reviewer_sessions(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
) -> Result<Vec<agent_sessions::Model>, AppError> {
    Ok(agent_sessions::Entity::find()
        .filter(agent_sessions::Column::ChapterId.eq(chapter_id))
        .filter(agent_sessions::Column::Role.eq(REVIEWER_ROLE))
        .filter(agent_sessions::Column::Status.is_not_in([
            SessionStatus::Completed.as_str(),
            SessionStatus::Superseded.as_str(),
            SessionStatus::Abandoned.as_str(),
        ]))
        .order_by_asc(agent_sessions::Column::Id)
        .all(conn)
        .await?)
}

/// Rejects a review operation on a chapter that is not `in_review`.
fn require_in_review(chapter: &chapters::Model) -> Result<(), AppError> {
    let status = ChapterStatus::from_str_opt(&chapter.status).unwrap_or(ChapterStatus::Planned);
    if status == ChapterStatus::InReview {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::ChapterInvalidState,
        "the chapter is not in review",
    )
    .with_details(json!({ "status": status.as_str() })))
}
