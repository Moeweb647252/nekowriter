//! Research tasks (spec/07 §8, spec/10 §2.5).
//!
//! A research task is the author's request to gather material into a knowledge scope. It is
//! created as a `draft` while the linked book is still `conceiving` (so the author can shape it
//! before the book exists on paper) and `ready` otherwise; `run` starts a researcher session with
//! a frozen snapshot, exactly like every other role.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};

use crate::api::dto::research::{
    ResearchTaskCreate, ResearchTaskDto, ResearchTaskEntryDto, ResearchTaskPatch, ResearchTaskQuery,
};
use crate::db::entities::{books, knowledge_entries, knowledge_scopes, research_tasks};
use crate::db::json::{BookStatus, ResearchTaskStatus, SessionKind, SessionStatus};
use crate::error::{AppError, ErrorCode};
use crate::events::ServerEvent;
use crate::state::AppState;

use super::{now, sessions};

/// The number of existing titles a researcher kickoff carries (spec/07 §8 step 2: ≤ 200).
pub const MAX_EXISTING_TITLES: usize = 200;

// ───────────────────────────── reads ─────────────────────────────

/// `GET /api/research-tasks` (spec/10 §2.5).
pub async fn list(
    conn: &impl ConnectionTrait,
    query: ResearchTaskQuery,
) -> Result<Vec<ResearchTaskDto>, AppError> {
    let mut find = research_tasks::Entity::find();
    if let Some(book_id) = query.book_id {
        find = find.filter(research_tasks::COLUMN.book_id.eq(book_id));
    }
    if let Some(status) = query.status {
        find = find.filter(research_tasks::COLUMN.status.eq(status.as_str()));
    }
    Ok(find
        .order_by_desc(research_tasks::COLUMN.created_at)
        .order_by_desc(research_tasks::COLUMN.id)
        .all(conn)
        .await?
        .into_iter()
        .map(to_dto)
        .collect())
}

/// `GET /api/research-tasks/:id` (spec/10 §2.5).
pub async fn get(conn: &impl ConnectionTrait, task_id: i32) -> Result<ResearchTaskDto, AppError> {
    with_entries(conn, load(conn, task_id).await?).await
}

/// The task with the entries it created, once it is `done` (spec/10 §2.5: "with the report and
/// the created entries when done"). The list and the events stay cheap and leave the field empty;
/// the task page reads this endpoint.
async fn with_entries(
    conn: &impl ConnectionTrait,
    row: research_tasks::Model,
) -> Result<ResearchTaskDto, AppError> {
    let mut dto = to_dto(row);
    if dto.status == ResearchTaskStatus::Done {
        dto.entries = knowledge_entries::Entity::find()
            .filter(knowledge_entries::COLUMN.research_task_id.eq(dto.id))
            .order_by_asc(knowledge_entries::COLUMN.id)
            .all(conn)
            .await?
            .into_iter()
            .map(|entry| ResearchTaskEntryDto {
                id: entry.id,
                title: entry.title,
            })
            .collect();
    }
    Ok(dto)
}

async fn load(
    conn: &impl ConnectionTrait,
    task_id: i32,
) -> Result<research_tasks::Model, AppError> {
    research_tasks::Entity::find_by_id(task_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("research task {task_id}")))
}

// ───────────────────────────── writes ─────────────────────────────

/// `POST /api/research-tasks` (spec/10 §2.5).
pub async fn create(
    state: &AppState,
    input: ResearchTaskCreate,
) -> Result<(ResearchTaskDto, Vec<String>), AppError> {
    let db = state.db().clone();
    let (dto, events, warnings) = super::write(&db, move |tx| {
        Box::pin(async move { create_inner(tx, input).await })
    })
    .await?;
    state.events().publish_all(events);
    Ok((dto, warnings))
}

async fn create_inner(
    tx: &impl ConnectionTrait,
    input: ResearchTaskCreate,
) -> Result<(ResearchTaskDto, Vec<ServerEvent>, Vec<String>), AppError> {
    let scope_name = input.scope_name.trim().to_owned();
    let topic = input.topic.trim().to_owned();
    let mut fields = serde_json::Map::new();
    if scope_name.is_empty() {
        fields.insert(
            "scope_name".to_owned(),
            serde_json::Value::String("a research task needs a scope name".to_owned()),
        );
    }
    if topic.is_empty() {
        fields.insert(
            "topic".to_owned(),
            serde_json::Value::String("a research task needs a topic".to_owned()),
        );
    }
    // The language rule of spec/10 §2.5: required without a book, ignored with one (a book has a
    // writing language, and the researcher writes in the book's).
    let mut language = input
        .language
        .as_deref()
        .map(str::trim)
        .filter(|language| !language.is_empty())
        .map(str::to_owned);
    if let Some(book_id) = input.book_id {
        let book = books::Entity::find_by_id(book_id)
            .one(tx)
            .await?
            .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?;
        // Setting materials include the draft research topics, which the conception agent owns
        // (`set_research_topics` replaces them): they are locked while it runs (spec/08 §1.4).
        super::setting_docs::ensure_editable(tx, book_id).await?;
        language = Some(book.writing_language);
    } else if language.is_none() {
        fields.insert(
            "language".to_owned(),
            serde_json::Value::String("a task without a book needs a language".to_owned()),
        );
    }
    if !fields.is_empty() {
        return Err(AppError::invalid_fields(fields));
    }
    let language = language.unwrap_or_else(|| "en".to_owned());

    // A draft while the book is still being conceived, `ready` otherwise (spec/10 §2.5).
    let status = match input.book_id {
        Some(book_id) => match books::Entity::find_by_id(book_id).one(tx).await? {
            Some(book) if book.status == BookStatus::Conceiving => ResearchTaskStatus::Draft,
            _ => ResearchTaskStatus::Ready,
        },
        None => ResearchTaskStatus::Ready,
    };
    let timestamp = now();
    let row = research_tasks::Entity::insert(research_tasks::ActiveModel {
        book_id: Set(input.book_id),
        scope_name: Set(scope_name),
        topic: Set(topic),
        requirements: Set(input.requirements.trim().to_owned()),
        language: Set(language),
        status: Set(status),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    })
    .exec_with_returning(tx)
    .await?;
    let events = vec![ServerEvent::ResearchUpdated {
        task: to_dto(row.clone()),
    }];
    Ok((to_dto(row), events, Vec::new()))
}

/// `PATCH /api/research-tasks/:id` (spec/10 §2.5).
pub async fn patch(
    state: &AppState,
    task_id: i32,
    input: ResearchTaskPatch,
) -> Result<ResearchTaskDto, AppError> {
    let db = state.db().clone();
    let (dto, events) = super::write(&db, move |tx| {
        Box::pin(async move { patch_inner(tx, task_id, input).await })
    })
    .await?;
    state.events().publish_all(events);
    Ok(dto)
}

async fn patch_inner(
    tx: &impl ConnectionTrait,
    task_id: i32,
    input: ResearchTaskPatch,
) -> Result<(ResearchTaskDto, Vec<ServerEvent>), AppError> {
    let existing = load(tx, task_id).await?;
    // A task with an open session is being edited by the researcher; the author's edits wait
    // (spec/10 §2.5).
    if is_open(&existing) {
        return Err(open_session_error(&existing));
    }
    ensure_topics_editable(tx, &existing).await?;
    let mut active = research_tasks::ActiveModel {
        id: Set(task_id),
        updated_at: Set(now()),
        ..Default::default()
    };
    if let Some(topic) = input.topic {
        let topic = topic.trim().to_owned();
        if topic.is_empty() {
            return Err(field_error("topic", "a research task needs a topic"));
        }
        active.topic = Set(topic);
    }
    if let Some(requirements) = input.requirements {
        active.requirements = Set(requirements.trim().to_owned());
    }
    if let Some(scope_name) = input.scope_name {
        let scope_name = scope_name.trim().to_owned();
        if scope_name.is_empty() {
            return Err(field_error(
                "scope_name",
                "a research task needs a scope name",
            ));
        }
        active.scope_name = Set(scope_name);
    }
    if let Some(status) = input.status {
        // The only status the author may set is `ready`, which confirms a draft (spec/10 §2.5).
        if status != ResearchTaskStatus::Ready {
            return Err(field_error(
                "status",
                "only `ready` can be set; the researcher and the runtime own the other statuses",
            ));
        }
        if existing.status != ResearchTaskStatus::Draft {
            return Err(field_error(
                "status",
                "only a draft can be confirmed as ready",
            ));
        }
        active.status = Set(ResearchTaskStatus::Ready);
    }
    let row = active.update(tx).await?;
    let events = vec![ServerEvent::ResearchUpdated {
        task: to_dto(row.clone()),
    }];
    Ok((with_entries(tx, row).await?, events))
}

/// `DELETE /api/research-tasks/:id` (spec/10 §2.5).
pub async fn delete(state: &AppState, task_id: i32) -> Result<(), AppError> {
    let db = state.db().clone();
    let events = super::write(&db, move |tx| {
        Box::pin(async move { delete_inner(tx, task_id).await })
    })
    .await?;
    state.events().publish_all(events);
    Ok(())
}

// ───────────────────────────── defer and resume (D-138) ─────────────────────────────

/// `POST /api/research-tasks/:id/defer` and `.../resume` (spec/10 §2.5).
///
/// Both endpoints name a **target status** rather than an action on the current one, which is what
/// makes a repeat call harmless: `defer` on a deferred task and `resume` on a ready one return the
/// current row unchanged (no write, no event), so a double-click or a retried request cannot fail.
/// A transition that is not in the table is the documented `409 research.invalid_state` naming the
/// task's current status (spec/07 §8, D-138).
pub async fn transition(
    state: &AppState,
    task_id: i32,
    transition: Transition,
) -> Result<ResearchTaskDto, AppError> {
    let db = state.db().clone();
    let (dto, events) = super::write(&db, move |tx| {
        Box::pin(async move { transition_inner(tx, task_id, transition).await })
    })
    .await?;
    state.events().publish_all(events);
    Ok(dto)
}

/// The two author transitions of the deferred status (spec/10 §2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// `ready → deferred`: "I know about this, not now".
    Defer,
    /// `deferred → ready`: "I do want to run this after all".
    Resume,
}

impl Transition {
    /// The wire word of the endpoint and of the error detail.
    pub fn as_str(self) -> &'static str {
        match self {
            Transition::Defer => "defer",
            Transition::Resume => "resume",
        }
    }

    /// The status this transition moves the task to.
    fn target(self) -> ResearchTaskStatus {
        match self {
            Transition::Defer => ResearchTaskStatus::Deferred,
            Transition::Resume => ResearchTaskStatus::Ready,
        }
    }

    /// The statuses this transition may start from (spec/07 §8 step 1, D-138).
    ///
    /// `defer` accepts `ready` only: a `draft` topic is unconfirmed (the author confirms it to
    /// `ready` or deletes it), and `running`/`done`/`failed`/`cancelled` are not "pending, but not
    /// now" — deferring them would claim finished work is still ahead or resurrect a dropped
    /// intent. `resume` accepts `deferred` only; every other status is already resolved.
    fn sources(self) -> &'static [ResearchTaskStatus] {
        match self {
            Transition::Defer => &[ResearchTaskStatus::Ready],
            Transition::Resume => &[ResearchTaskStatus::Deferred],
        }
    }
}

async fn transition_inner(
    tx: &impl ConnectionTrait,
    task_id: i32,
    transition: Transition,
) -> Result<(ResearchTaskDto, Vec<ServerEvent>), AppError> {
    let task = load(tx, task_id).await?;
    let current = task.status;
    // A task whose session is open is being written by the researcher: the same conflict `run`,
    // PATCH and DELETE report (spec/07 §8 step 1, spec/10 §2.5).
    if is_open(&task) {
        return Err(open_session_error(&task));
    }
    let target = transition.target();
    if current == target {
        // Already there: the row is the answer, and nothing is written or announced.
        return Ok((with_entries(tx, task).await?, Vec::new()));
    }
    if !transition.sources().contains(&current) {
        return Err(AppError::new(
            ErrorCode::ResearchInvalidState,
            format!(
                "a {} research task cannot be moved to {}",
                current.as_str(),
                target.as_str()
            ),
        )
        .with_details(serde_json::json!({
            "status": current.as_str(),
            "operation": transition.as_str(),
        })));
    }
    let row = research_tasks::ActiveModel {
        id: Set(task_id),
        status: Set(target),
        updated_at: Set(now()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    Ok((
        with_entries(tx, row.clone()).await?,
        vec![ServerEvent::ResearchUpdated { task: to_dto(row) }],
    ))
}

async fn delete_inner(
    tx: &impl ConnectionTrait,
    task_id: i32,
) -> Result<Vec<ServerEvent>, AppError> {
    let existing = load(tx, task_id).await?;
    if is_open(&existing) {
        return Err(open_session_error(&existing));
    }
    ensure_topics_editable(tx, &existing).await?;
    research_tasks::Entity::delete_by_id(task_id)
        .exec(tx)
        .await?;
    Ok(vec![ServerEvent::ResearchUpdated {
        task: to_dto(existing),
    }])
}

/// `POST /api/research-tasks/:id/run` (spec/07 §8, spec/10 §2.5).
pub async fn run(state: &AppState, task_id: i32) -> Result<i32, AppError> {
    // The scope is resolved (or created) and attached to the linked book **before** the session
    // starts, in its own short transaction: the kickoff then reads a real scope, and no
    // transaction is open across session creation's own write (spec/07 §8 step 2).
    // The book (if any) is read before the session starts: the session carries it so the book's
    // agent panel lists the researcher and events reach that book's stream (spec/10 §3).
    let book_id = get(state.db_conn(), task_id).await?.book_id;
    let scope_id = {
        let db = state.db().clone();
        super::write(&db, move |tx| {
            Box::pin(async move { prepare_scope(tx, task_id).await })
        })
        .await?
    };
    let (session_id, _warnings) = sessions::create(
        state,
        sessions::CreateSession {
            kind: SessionKind::Research,
            book_id,
            chapter_id: None,
            research_task_id: Some(task_id),
            granularity: None,
            auto_pause: None,
            pause_after_outline: None,
            redraft: false,
            guidance: None,
            kickoff_extra: Some(serde_json::json!({ "scope_id": scope_id })),
            pending_comment: None,
            legacy_planning_fixture: false,
            legacy_system_prompt: None,
            legacy_kickoff_text: None,
            chapter_guard: false,
            force_previous: false,
        },
    )
    .await?;
    Ok(session_id)
}

/// The scope of a run: the existing one by name, or a new one attached to the linked book
/// (spec/07 §8 step 2).
async fn prepare_scope(tx: &impl ConnectionTrait, task_id: i32) -> Result<i32, AppError> {
    let task = load(tx, task_id).await?;
    // Running is refused while a session is open (spec/07 §8 step 1).
    if is_open(&task) {
        return Err(open_session_error(&task));
    }
    // `deferred` is runnable on purpose: the author's "not now" is not a lock, and forcing an
    // un-defer step before every run would add friction without any safety gain (D-138). The run
    // starts the session, which moves the task to `running` through its own transition
    // (spec/07 §8 step 5), so the deferred state never outlives the first run.
    let runnable = match task.status {
        ResearchTaskStatus::Draft
        | ResearchTaskStatus::Ready
        | ResearchTaskStatus::Deferred
        | ResearchTaskStatus::Done
        | ResearchTaskStatus::Cancelled => true,
        ResearchTaskStatus::Running | ResearchTaskStatus::Failed => false,
    };
    if !runnable {
        return Err(AppError::new(
            ErrorCode::ResearchInvalidState,
            format!("a {} research task cannot be run", task.status),
        ));
    }
    let scope_id = match task.scope_id {
        Some(scope_id) => scope_id,
        None => {
            let scope = resolve_scope(tx, &task.scope_name).await?;
            research_tasks::ActiveModel {
                id: Set(task_id),
                scope_id: Set(Some(scope)),
                updated_at: Set(now()),
                ..Default::default()
            }
            .update(tx)
            .await?;
            scope
        }
    };
    // The linked book must be able to search the scope the researcher writes into (spec/07 §8).
    if let Some(book_id) = task.book_id {
        super::knowledge::include_scope(tx, book_id, scope_id).await?;
    }
    Ok(scope_id)
}

/// Finds a scope by name, or creates it (spec/07 §8 step 2).
async fn resolve_scope(tx: &impl ConnectionTrait, name: &str) -> Result<i32, AppError> {
    let name = name.trim();
    if let Some(row) = knowledge_scopes::Entity::find()
        .filter(knowledge_scopes::COLUMN.name.eq(name))
        .one(tx)
        .await?
    {
        return Ok(row.id);
    }
    let timestamp = now();
    let row = knowledge_scopes::Entity::insert(knowledge_scopes::ActiveModel {
        name: Set(name.to_owned()),
        description: Set(String::new()),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    })
    .exec_with_returning(tx)
    .await?;
    Ok(row.id)
}

// ───────────────────────────── the runtime side ─────────────────────────────

/// True while the task has a session that has not finished (spec/07 §8 step 1).
pub fn is_open(task: &research_tasks::Model) -> bool {
    match task.status {
        ResearchTaskStatus::Running | ResearchTaskStatus::Failed => task.session_id.is_some(),
        ResearchTaskStatus::Draft
        | ResearchTaskStatus::Ready
        | ResearchTaskStatus::Deferred
        | ResearchTaskStatus::Done
        | ResearchTaskStatus::Cancelled => false,
    }
}

/// Rejects an author write to a draft research topic while the book's conception session runs
/// (spec/08 §1.4: the draft topics are setting materials and are locked then, like the documents
/// and the cast). A task without a book has no conception session to race.
async fn ensure_topics_editable(
    tx: &impl ConnectionTrait,
    task: &research_tasks::Model,
) -> Result<(), AppError> {
    match task.book_id {
        Some(book_id) => super::setting_docs::ensure_editable(tx, book_id).await,
        None => Ok(()),
    }
}

/// The `409 research.invalid_state` a blocked operation reports (spec/10 §2.5).
fn open_session_error(task: &research_tasks::Model) -> AppError {
    AppError::new(
        ErrorCode::ResearchInvalidState,
        "the research task has an open session",
    )
    .with_details(serde_json::json!({ "session_id": task.session_id }))
}

/// Marks the task `done` and stores the report (`finish_research`, spec/07 §8 step 5).
///
/// `entries_created` counts the task's entries across **every** run, not just this one
/// (spec/07 §8), so a second run of the same task keeps its history.
pub async fn finish(
    tx: &sea_orm::DatabaseTransaction,
    session: &crate::db::entities::agent_sessions::Model,
    report: &str,
) -> Result<Vec<ServerEvent>, AppError> {
    let Some(task_id) = session.research_task_id else {
        return Ok(Vec::new());
    };
    if research_tasks::Entity::find_by_id(task_id)
        .one(tx)
        .await?
        .is_none()
    {
        return Ok(Vec::new());
    }
    let total = knowledge_entries::Entity::find()
        .filter(knowledge_entries::COLUMN.research_task_id.eq(task_id))
        .count(tx)
        .await?;
    let row = research_tasks::ActiveModel {
        id: Set(task_id),
        status: Set(ResearchTaskStatus::Done),
        report: Set(report.trim().to_owned()),
        entries_created: Set(i32::try_from(total).unwrap_or(i32::MAX)),
        updated_at: Set(now()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    Ok(vec![ServerEvent::ResearchUpdated { task: to_dto(row) }])
}

/// Keeps the task's status in step with its session (spec/07 §8 step 1, spec/03 §6).
///
/// Called by the session writer when a session ends, so a task never claims to be `running`
/// without a session.
pub async fn on_session_status(
    conn: &impl ConnectionTrait,
    task_id: i32,
    session_id: i32,
    status: SessionStatus,
) -> Result<Vec<ServerEvent>, AppError> {
    let Some(task) = research_tasks::Entity::find_by_id(task_id)
        .one(conn)
        .await?
    else {
        return Ok(Vec::new());
    };
    // Only the task's own live session moves it; an older session's exit must not reopen or fail a
    // task that has since been resumed.
    if task.session_id != Some(session_id) {
        return Ok(Vec::new());
    }
    let Some(next) = research_status_for_session(status) else {
        return Ok(Vec::new());
    };
    if task.status == next {
        return Ok(Vec::new());
    }
    let row = research_tasks::ActiveModel {
        id: Set(task_id),
        status: Set(next),
        updated_at: Set(now()),
        ..Default::default()
    }
    .update(conn)
    .await?;
    Ok(vec![ServerEvent::ResearchUpdated { task: to_dto(row) }])
}

/// The task status a session status maps to; every other session status leaves the task alone.
fn research_status_for_session(status: SessionStatus) -> Option<ResearchTaskStatus> {
    match status {
        SessionStatus::Running => Some(ResearchTaskStatus::Running),
        SessionStatus::Failed | SessionStatus::Interrupted => Some(ResearchTaskStatus::Failed),
        SessionStatus::Abandoned => Some(ResearchTaskStatus::Cancelled),
        SessionStatus::Idle
        | SessionStatus::Held
        | SessionStatus::AwaitingInput
        | SessionStatus::Paused
        | SessionStatus::Completed
        | SessionStatus::Stopped
        | SessionStatus::Superseded => None,
    }
}

/// The existing titles of a scope, capped (spec/07 §8 step 2: at most 200).
pub async fn existing_titles(
    conn: &impl ConnectionTrait,
    scope_id: i32,
) -> Result<Vec<String>, AppError> {
    Ok(knowledge_entries::Entity::find()
        .filter(knowledge_entries::COLUMN.scope_id.eq(scope_id))
        .order_by_asc(knowledge_entries::COLUMN.title)
        .limit(MAX_EXISTING_TITLES as u64)
        .all(conn)
        .await?
        .into_iter()
        .map(|row| row.title)
        .collect())
}

/// The number of entries a task has created across every run (spec/07 §8).
pub async fn entry_count(conn: &impl ConnectionTrait, task_id: i32) -> Result<i32, AppError> {
    let total = knowledge_entries::Entity::find()
        .filter(knowledge_entries::COLUMN.research_task_id.eq(task_id))
        .count(conn)
        .await?;
    Ok(i32::try_from(total).unwrap_or(i32::MAX))
}

// ───────────────────────────── mapping ─────────────────────────────

/// The DTO of a row, without the created entries (events and the list do not need them; the
/// single-task endpoints fill them through [`with_entries`]).
pub fn to_dto(row: research_tasks::Model) -> ResearchTaskDto {
    ResearchTaskDto {
        id: row.id,
        book_id: row.book_id,
        scope_id: row.scope_id,
        scope_name: row.scope_name,
        topic: row.topic,
        kind: row.kind,
        origin: row.origin,
        requirements: row.requirements,
        language: row.language,
        status: row.status,
        session_id: row.session_id,
        report: row.report,
        entries_created: row.entries_created,
        entries: Vec::new(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn field_error(field: &str, message: &str) -> AppError {
    AppError::invalid_fields(serde_json::Map::from_iter([(
        field.to_owned(),
        serde_json::Value::String(message.to_owned()),
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::entity::Iterable;

    #[test]
    fn every_session_status_maps_explicitly() {
        let expected = |status: SessionStatus| match status {
            SessionStatus::Running => Some(ResearchTaskStatus::Running),
            SessionStatus::Failed | SessionStatus::Interrupted => Some(ResearchTaskStatus::Failed),
            SessionStatus::Abandoned => Some(ResearchTaskStatus::Cancelled),
            SessionStatus::Idle
            | SessionStatus::Held
            | SessionStatus::AwaitingInput
            | SessionStatus::Paused
            | SessionStatus::Completed
            | SessionStatus::Stopped
            | SessionStatus::Superseded => None,
        };
        for status in SessionStatus::iter() {
            assert_eq!(
                research_status_for_session(status),
                expected(status),
                "the mapping is explicit for {status:?}"
            );
        }
    }
}
