//! Conception service (spec/08 §5): the book's conception lifecycle.
//!
//! Creating a book with `start_conception` makes it `conceiving` and starts the session;
//! `conception/start` returns the open session when there is one (D-047) and otherwise starts
//! one; `conception/finalize` completes the session, activates the book, readies the draft
//! research tasks, creates and attaches their scopes, and starts the `canon_seed` session when
//! an archivist model exists; `canon/seed` is the idempotent entry point the overview offers
//! later.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::json;

use crate::api::dto::books::BookDto;
use crate::db::entities::{
    agent_sessions, books, knowledge_scopes, research_tasks, setting_docs, tracks,
};
use crate::db::json::{
    AgentRole, BookStatus, ConceptionState, JsonText, ResearchTaskKind, ResearchTaskStatus,
    SessionKind, SessionStatus, SettingDocKind, TrackKind,
};
use crate::error::{AppError, ErrorCode};
use crate::events::{BookOp, ServerEvent};
use crate::state::AppState;

use super::sessions::{self, CreateSession};
use super::{conflict, now};

/// `POST /api/books` with `start_conception: true` (spec/08 §5 step 1): the book is created
/// `conceiving` and its conception session starts when a conception model is configured.
///
/// The session needs the book's own transaction to have committed (its kickoff reads the book),
/// so creation and the session start are two transactions; a book whose model appears later gets
/// its session from `conception/start`.
pub async fn start_for_new_book(
    state: &AppState,
    book_id: i32,
) -> Result<(Option<i32>, Vec<String>), AppError> {
    let mut warnings = Vec::new();
    let conn = state.db().conn();
    let book = books::Entity::find_by_id(book_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?;
    let session_id =
        match super::sessions::try_resolve_model(conn, Some(&book), AgentRole::Conception).await? {
            Some(_) => {
                let (session_id, session_warnings) = start_session(state, book_id).await?;
                warnings.extend(session_warnings);
                Some(session_id)
            }
            None => {
                // The book still exists and can be written by hand; the warning names the gap
                // (D-046: creating a book without a conception model still succeeds).
                warnings.push("model.missing_role".to_owned());
                None
            }
        };
    Ok((session_id, warnings))
}

/// `POST /api/books/:id/conception/start` (spec/08 §5): the open session, or a new one.
pub async fn start(state: &AppState, book_id: i32) -> Result<(i32, Vec<String>), AppError> {
    {
        let db = state.db().clone();
        super::write(&db, move |tx| {
            Box::pin(async move {
                let book = books::Entity::find_by_id(book_id)
                    .one(tx)
                    .await?
                    .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?;
                if book.status != BookStatus::Conceiving {
                    return Err(conflict(
                        ErrorCode::BookInvalidState,
                        "only a conceiving book can start conception",
                        json!({ "status": book.status }),
                    ));
                }
                Ok(())
            })
        })
        .await?;
    }
    if let Some(open) = open_session(state.db().conn(), book_id).await? {
        return Ok((open, Vec::new()));
    }
    match start_session(state, book_id).await {
        Ok(started) => Ok(started),
        // A concurrent `conception/start` won the creation race. Its session *is* the open one
        // spec/08 §5 tells this endpoint to return, so the loser answers with it instead of
        // surfacing a 409 for what the author experienced as one click (D-047).
        Err(error) if error.code == ErrorCode::SessionInvalidState => {
            match open_session(state.db().conn(), book_id).await? {
                Some(open) => Ok((open, Vec::new())),
                None => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

/// Starts the book's conception session and records it in `ConceptionState.session_id`.
async fn start_session(state: &AppState, book_id: i32) -> Result<(i32, Vec<String>), AppError> {
    let (session_id, warnings) = sessions::create(
        state,
        CreateSession {
            kind: SessionKind::Conception,
            book_id: Some(book_id),
            chapter_id: None,
            research_task_id: None,
            granularity: None,
            auto_pause: None,
            pause_after_outline: None,
            redraft: false,
            guidance: None,
            kickoff_extra: None,
            pending_comment: None,
            legacy_planning_fixture: false,
            legacy_system_prompt: None,
            legacy_kickoff_text: None,
            chapter_guard: false,
            force_previous: false,
        },
    )
    .await?;
    let db = state.db().clone();
    let events = super::write(&db, move |tx| {
        Box::pin(async move {
            let book = books::Entity::find_by_id(book_id)
                .one(tx)
                .await?
                .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?;
            let mut conception: ConceptionState = book.conception.clone().into_inner();
            conception.session_id = Some(session_id);
            books::ActiveModel {
                id: Set(book_id),
                conception: Set(JsonText(conception)),
                updated_at: Set(now()),
                ..Default::default()
            }
            .update(tx)
            .await?;
            Ok(Vec::<ServerEvent>::new())
        })
    })
    .await?;
    state.events().publish_all(events);
    Ok((session_id, warnings))
}

/// The book's open conception session, when there is one (spec/02 §5 invariant 5).
pub async fn open_session(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Option<i32>, AppError> {
    sessions::open_book_session(conn, book_id, SessionKind::Conception).await
}

/// The book's open `canon_seed` session, when there is one (spec/08 §5, D-047).
pub async fn open_seed_session(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Option<i32>, AppError> {
    sessions::open_book_session(conn, book_id, SessionKind::CanonSeed).await
}

/// The configured model profile of a role for a book, without failing when there is none
/// (`book.model_overrides[role]` → `app_settings["roles.defaults"][role]`, spec/03 §4 step 1).
async fn resolve_role_model(
    state: &AppState,
    book_id: i32,
    role: AgentRole,
) -> Result<Option<i32>, AppError> {
    let conn = state.db().conn();
    let book = books::Entity::find_by_id(book_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?;
    super::sessions::try_resolve_model(conn, Some(&book), role).await
}

/// `POST /api/books/:id/conception/finalize` (spec/08 §5): one transaction plus a session start.
pub async fn finalize(state: &AppState, book_id: i32) -> Result<FinalizeResponse, AppError> {
    // 1. Preconditions need the materials as they are now; a running session is stopped first
    //    (spec/08 §5 step 2), which needs its runner cancelled and awaited outside any
    //    transaction (spec/03 §6).
    let running = running_conception_session(state.db().conn(), book_id).await?;
    if let Some(session_id) = running {
        state
            .manager()
            .cancel(session_id, crate::agent::CancelReason::Stop);
        state
            .manager()
            .wait_for(session_id, std::time::Duration::from_secs(10))
            .await;
    }

    let warnings_state = state.clone();
    let (book_dto, events) = {
        let db = state.db().clone();
        super::write(&db, move |tx| {
            Box::pin(async move { finalize_inner(tx, book_id).await })
        })
        .await?
    };
    state.events().publish_all(events);

    // 2. The seed session starts only when everything committed (its kickoff reads the finalized
    //    book); the missing archivist is a warning, not a failure (D-046).
    let mut warnings = Vec::new();
    let mut seed_session_id = None;
    // With every seedable system off there is nothing to seed: no session and no warning
    // (spec/08 §5 step 5, spec/06 §7). Goals are never seeded.
    if !book_dto.settings.canon_systems.any_seed_system() {
        return Ok(FinalizeResponse {
            book: book_dto,
            seed_session_id,
            warnings,
        });
    }
    match resolve_role_model(&warnings_state, book_id, AgentRole::Archivist).await? {
        Some(_) => {
            let (session_id, _) = seed_session(&warnings_state, book_id).await?;
            seed_session_id = Some(session_id);
        }
        None => warnings.push("model.missing_role".to_owned()),
    }
    Ok(FinalizeResponse {
        book: book_dto,
        seed_session_id,
        warnings,
    })
}

/// The finalize response (spec/10 §2.2).
pub type FinalizeResponse = crate::api::dto::books::ConceptionFinalizeResponse;

/// The id of the book's running conception session, when there is one.
async fn running_conception_session(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Option<i32>, AppError> {
    let row = agent_sessions::Entity::find()
        .filter(agent_sessions::COLUMN.book_id.eq(book_id))
        .filter(agent_sessions::COLUMN.kind.eq(SessionKind::Conception))
        .filter(agent_sessions::COLUMN.status.eq(SessionStatus::Running))
        .order_by_desc(agent_sessions::COLUMN.id)
        .one(conn)
        .await?;
    Ok(row.map(|row| row.id))
}

/// The in-transaction part of finalize (spec/08 §5 steps 1–4).
async fn finalize_inner(
    tx: &impl ConnectionTrait,
    book_id: i32,
) -> Result<(BookDto, Vec<ServerEvent>), AppError> {
    let book = books::Entity::find_by_id(book_id)
        .one(tx)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?;
    if book.status != BookStatus::Conceiving {
        return Err(conflict(
            ErrorCode::BookInvalidState,
            "only a conceiving book can be finalized",
            json!({ "status": book.status }),
        ));
    }

    // `outline`, `world`, `protagonist` non-empty and ≥1 character design, else
    // `conception.incomplete` with what is missing (spec/08 §5 step 1). Unapproved items are
    // allowed: the UI confirms.
    let docs = setting_docs::Entity::find()
        .filter(setting_docs::COLUMN.book_id.eq(book_id))
        .all(tx)
        .await?;
    let empty_doc = |kind: SettingDocKind| {
        docs.iter()
            .find(|doc| doc.kind == kind)
            .is_none_or(|doc| doc.content.trim().is_empty())
    };
    let characters = crate::db::entities::character_designs::Entity::find()
        .filter(
            crate::db::entities::character_designs::COLUMN
                .book_id
                .eq(book_id),
        )
        .one(tx)
        .await?;
    let mut missing = Vec::new();
    for kind in [
        SettingDocKind::Outline,
        SettingDocKind::World,
        SettingDocKind::Protagonist,
    ] {
        if empty_doc(kind) {
            missing.push(kind.as_str().to_owned());
        }
    }
    if characters.is_none() {
        missing.push("characters".to_owned());
    }
    if !missing.is_empty() {
        return Err(conflict(
            ErrorCode::ConceptionIncomplete,
            "the setting materials are not complete",
            json!({ "missing": missing }),
        ));
    }

    // The open conception session completes; a pending form is simply left unanswered
    // (spec/08 §5 step 2).
    let mut events = Vec::new();
    let open = open_session(tx, book_id).await?;
    if let Some(session_id) = open {
        let row = agent_sessions::Entity::find_by_id(session_id)
            .one(tx)
            .await?
            .ok_or_else(|| AppError::not_found(format!("session {session_id}")))?;
        let mut transition = crate::agent::session::transition(
            tx,
            &row,
            SessionStatus::Completed,
            crate::agent::session::TransitionCause::Completed,
        )
        .await?;
        events.append(&mut transition);
    }

    // The book activates and records the finalization (step 3).
    let mut conception: ConceptionState = book.conception.clone().into_inner();
    conception.finalized_at = Some(now());
    conception.session_id = None;
    let updated = books::ActiveModel {
        id: Set(book_id),
        status: Set(BookStatus::Active),
        conception: Set(JsonText(conception)),
        updated_at: Set(now()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    events.push(ServerEvent::BookUpdated {
        book_id,
        op: BookOp::Update,
    });

    // Draft research tasks become ready; each distinct scope is found-or-created and attached
    // (step 4). A new scope's description is its topics joined with ` / ` — model-written text
    // and a language-neutral separator, so no wording is generated in code (D-054).
    let drafts = research_tasks::Entity::find()
        .filter(research_tasks::COLUMN.book_id.eq(book_id))
        .filter(research_tasks::COLUMN.kind.eq(ResearchTaskKind::Topic))
        .filter(research_tasks::COLUMN.status.eq(ResearchTaskStatus::Draft))
        .order_by_asc(research_tasks::COLUMN.id)
        .all(tx)
        .await?;
    let mut scope_by_name = std::collections::BTreeMap::new();
    for task in &drafts {
        let scope_id = match scope_by_name.get(&task.scope_name) {
            Some(id) => *id,
            None => {
                let id = find_or_create_scope(tx, book_id, &task.scope_name, &drafts).await?;
                scope_by_name.insert(task.scope_name.clone(), id);
                id
            }
        };
        let ready = research_tasks::ActiveModel {
            id: Set(task.id),
            status: Set(ResearchTaskStatus::Ready),
            scope_id: Set(Some(scope_id)),
            updated_at: Set(now()),
            ..Default::default()
        }
        .update(tx)
        .await?;
        // The event carries the task as it is after the update (spec/10 §3: the payload is the
        // current `ResearchTaskDto` and the client upserts it), not the `draft` row it replaced.
        events.push(ServerEvent::ResearchUpdated {
            task: super::research::to_dto(ready),
        });
    }

    let book_dto = super::books::to_dto(&updated)?;
    Ok((book_dto, events))
}

/// Finds a scope by name or creates it attached to the book (spec/08 §5 step 4).
async fn find_or_create_scope(
    tx: &impl ConnectionTrait,
    book_id: i32,
    name: &str,
    drafts: &[research_tasks::Model],
) -> Result<i32, AppError> {
    if let Some(row) = knowledge_scopes::Entity::find()
        .filter(knowledge_scopes::COLUMN.name.eq(name))
        .one(tx)
        .await?
    {
        super::knowledge::include_scope(tx, book_id, row.id).await?;
        return Ok(row.id);
    }
    let topics: Vec<&str> = drafts
        .iter()
        .filter(|task| task.scope_name == name)
        .map(|task| task.topic.as_str())
        .collect();
    let timestamp = now();
    let created = knowledge_scopes::ActiveModel {
        name: Set(name.to_owned()),
        description: Set(topics.join(" / ")),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    }
    .insert(tx)
    .await
    .map_err(AppError::from)?;
    super::knowledge::include_scope(tx, book_id, created.id).await?;
    Ok(created.id)
}

/// Runs after a preparation session completes, in its own write after the check's completion
/// commits (spec/08 §5, §4.6). All four conditions are re-checked here; a missing conception model
/// is not an error. The caller logs other failures at `warn` without failing the check.
pub async fn maybe_start_after_knowledge_check(
    state: &AppState,
    session_id: i32,
) -> Result<(), AppError> {
    let db = state.db().clone();
    let state_owned = state.clone();
    let (started, events) = super::write(&db, move |tx| {
        Box::pin(async move {
            let (session, events) =
                start_after_knowledge_check(&state_owned, tx, session_id).await?;
            Ok((session, events))
        })
    })
    .await?;
    state.events().publish_all(events);
    if let Some(session_id) = started {
        state.manager().spawn(state.clone(), session_id);
    }
    Ok(())
}

/// The in-transaction half of the auto-start condition check. `None` means no action, not an
/// error (including the missing-model case).
async fn start_after_knowledge_check(
    state: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    session_id: i32,
) -> Result<(Option<i32>, Vec<ServerEvent>), AppError> {
    let Some(check_session) = agent_sessions::Entity::find_by_id(session_id)
        .one(tx)
        .await?
    else {
        return Ok((None, Vec::new()));
    };
    if check_session.kind != SessionKind::Research
        || check_session.status != SessionStatus::Completed
    {
        return Ok((None, Vec::new()));
    }
    let Some(task_id) = check_session.research_task_id else {
        return Ok((None, Vec::new()));
    };
    let Some(task) = research_tasks::Entity::find_by_id(task_id).one(tx).await? else {
        return Ok((None, Vec::new()));
    };
    if task.kind != ResearchTaskKind::Preparation || task.status != ResearchTaskStatus::Done {
        return Ok((None, Vec::new()));
    }
    let Some(book_id) = task.book_id else {
        return Ok((None, Vec::new()));
    };
    let Some(book) = books::Entity::find_by_id(book_id).one(tx).await? else {
        return Ok((None, Vec::new()));
    };
    if book.status != BookStatus::Conceiving {
        return Ok((None, Vec::new()));
    }
    let mut conception: ConceptionState = book.conception.clone().into_inner();
    if !conception.start_after_knowledge_check {
        return Ok((None, Vec::new()));
    }
    // Condition 4: any conception session, in any status, means the author already exercised the
    // choice (or an earlier auto-start/abandoned run did); auto-start never creates a second one.
    if agent_sessions::Entity::find()
        .filter(agent_sessions::COLUMN.book_id.eq(book_id))
        .filter(agent_sessions::COLUMN.kind.eq(SessionKind::Conception))
        .one(tx)
        .await?
        .is_some()
    {
        return Ok((None, Vec::new()));
    }

    let session = match sessions::create_inner(
        state,
        tx,
        CreateSession {
            kind: SessionKind::Conception,
            book_id: Some(book_id),
            ..CreateSession::default()
        },
    )
    .await
    {
        Ok((session_id, events, _warnings)) => (session_id, events),
        Err(error) if error.code == ErrorCode::ModelMissingRole => {
            return Ok((None, Vec::new()));
        }
        Err(error) => return Err(error),
    };
    conception.start_after_knowledge_check = false;
    conception.session_id = Some(session.0);
    books::ActiveModel {
        id: Set(book_id),
        conception: Set(JsonText(conception)),
        updated_at: Set(now()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    let mut events = session.1;
    events.push(ServerEvent::BookUpdated {
        book_id,
        op: BookOp::Update,
    });
    Ok((Some(session.0), events))
}

/// `POST /api/books/:id/canon/seed` (spec/08 §5): the open seed session, or a new one.
pub async fn seed(state: &AppState, book_id: i32) -> Result<i32, AppError> {
    {
        let db = state.db().clone();
        super::write(&db, move |tx| {
            Box::pin(async move {
                let book = books::Entity::find_by_id(book_id)
                    .one(tx)
                    .await?
                    .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?;
                if book.status != BookStatus::Active {
                    return Err(conflict(
                        ErrorCode::BookInvalidState,
                        "only an active book can seed canon",
                        json!({ "status": book.status }),
                    ));
                }
                Ok(())
            })
        })
        .await?;
    }
    if let Some(open) = open_seed_session(state.db().conn(), book_id).await? {
        return Ok(open);
    }
    match seed_session(state, book_id).await {
        Ok((session_id, _warnings)) => Ok(session_id),
        // Same creation race as `conception/start`: the open seed session is the answer (D-047).
        Err(error) if error.code == ErrorCode::SessionInvalidState => {
            match open_seed_session(state.db().conn(), book_id).await? {
                Some(open) => Ok(open),
                None => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

/// Starts the book's `canon_seed` session (spec/06 §7).
async fn seed_session(state: &AppState, book_id: i32) -> Result<(i32, Vec<String>), AppError> {
    sessions::create(
        state,
        CreateSession {
            kind: SessionKind::CanonSeed,
            book_id: Some(book_id),
            chapter_id: None,
            research_task_id: None,
            granularity: None,
            auto_pause: None,
            pause_after_outline: None,
            redraft: false,
            guidance: None,
            kickoff_extra: None,
            pending_comment: None,
            legacy_planning_fixture: false,
            legacy_system_prompt: None,
            legacy_kickoff_text: None,
            chapter_guard: false,
            force_previous: false,
        },
    )
    .await
}

/// True when a book still has a main track (used by the API layer's guards).
pub async fn has_main_track(conn: &impl ConnectionTrait, book_id: i32) -> Result<bool, AppError> {
    Ok(tracks::Entity::find()
        .filter(tracks::COLUMN.book_id.eq(book_id))
        .filter(tracks::COLUMN.kind.eq(TrackKind::Main))
        .one(conn)
        .await?
        .is_some())
}
