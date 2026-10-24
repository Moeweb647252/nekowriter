//! Session services: creation (spec/03 §4), the control operations (spec/03 §6) and the read
//! endpoints (spec/10 §2.7).
//!
//! The service validates against the status table of spec/03 §3 and writes only what the API is
//! allowed to write while a session is `running` (D-034): `pause_requested`, the writer switches,
//! inbox rows and the §6 status changes. Every other session write belongs to the runner.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, Set,
};
use serde_json::json;

use crate::agent::runner;
use crate::agent::session::{self, TransitionCause};
use crate::agent::tools::registry::{initial_phase, role_of_kind, tools_for_role};
use crate::api::dto::sessions::{
    DebugEchoDto, MessagePageDto, ResumeInput, SessionAnswerInput, SessionMessageInput,
    SessionPatch, SessionSnapshotDto,
};
use crate::db::entities::{
    agent_inbox, agent_messages, agent_sessions, agent_tool_calls, books, chapters, model_profiles,
    providers, research_tasks, tracks,
};
use crate::db::json::{
    AgentRole, BookSettings, Granularity, InboxKind, ModelSnapshot, Phase, Protocol, RoleModels,
    SessionCounters, SessionKind, SessionStatus, StoredBlock, ToolCallStatus,
};
use crate::error::{AppError, ErrorCode};
use crate::events::ServerEvent;
use crate::prompts::PromptSet;
use crate::services;
use crate::state::AppState;

/// The `messages` cap of a session snapshot (spec/10 §2.7).
pub const SNAPSHOT_MESSAGE_LIMIT: usize = 200;

/// The default page size of the message list (spec/10 §2.7).
pub const MESSAGE_PAGE_DEFAULT: usize = 100;

// ───────────────────────────── session creation ─────────────────────────────

impl Default for CreateSession {
    fn default() -> Self {
        Self {
            kind: SessionKind::ChapterWrite,
            book_id: None,
            chapter_id: None,
            research_task_id: None,
            granularity: None,
            auto_pause: None,
            pause_after_outline: None,
            redraft: false,
            kickoff_extra: None,
        }
    }
}

/// Everything the caller wants to start a session for.
#[derive(Debug, Clone)]
pub struct CreateSession {
    pub kind: SessionKind,
    pub book_id: Option<i32>,
    pub chapter_id: Option<i32>,
    pub research_task_id: Option<i32>,
    /// Writer sessions: take these instead of the book's settings when given (spec/03 §4 step 3).
    pub granularity: Option<Granularity>,
    pub auto_pause: Option<bool>,
    pub pause_after_outline: Option<bool>,
    /// The redraft kickoff carries the author's feedback (spec/03 §1.1).
    pub redraft: bool,
    /// Pre-rendered extra kickoff state (task blocks that depend on the caller's flow).
    pub kickoff_extra: Option<serde_json::Value>,
}

/// Creates a session (spec/03 §4) and spawns its runner. Returns the session id and any
/// warnings (`prompt.override_failed`, `context.large_kickoff`).
pub async fn create(
    state: &AppState,
    input: CreateSession,
) -> Result<(i32, Vec<String>), AppError> {
    let db = state.db().clone();
    let state_owned = state.clone();
    let (session_id, events, warnings) = services::write(&db, move |tx| {
        Box::pin(async move { create_inner(&state_owned, tx, input).await })
    })
    .await?;
    state.events().publish_all(events);
    state.manager().spawn(state.clone(), session_id);
    Ok((session_id, warnings))
}

/// The in-transaction part of [`create`].
async fn create_inner(
    state: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    input: CreateSession,
) -> Result<(i32, Vec<ServerEvent>, Vec<String>), AppError> {
    let role = role_of_kind(input.kind);
    let role_enum = AgentRole::from_str_opt(role)
        .ok_or_else(|| AppError::internal(format!("unknown role {role}")))?;
    let mut warnings = Vec::new();

    // 1. Resolve the model: book.model_overrides[role] → app_settings["roles.defaults"][role].
    let book = match input.book_id {
        Some(book_id) => Some(
            books::Entity::find_by_id(book_id)
                .one(tx)
                .await?
                .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?,
        ),
        None => None,
    };
    let model_profile_id = resolve_model(tx, book.as_ref(), role_enum).await?;
    let profile = model_profiles::Entity::find_by_id(model_profile_id)
        .one(tx)
        .await?
        .ok_or_else(|| crate::agent::context::missing_role(role_enum))?;
    let provider = providers::Entity::find_by_id(profile.provider_id)
        .one(tx)
        .await?
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::ModelMissingRole,
                "the model profile's provider no longer exists",
            )
            .with_details(json!({ "role": role }))
        })?;
    let snapshot = model_snapshot(&profile, &provider)?;

    // 2. Freeze the prompts: the role's system prompt and tool definitions through the registry.
    let prompts = state.prompts().for_book(state.db(), input.book_id).await?;
    let system_prompt = prompts
        .render(&format!("{role}.system"), &json!({}))
        .map_err(|err| err.into_app_error())?;
    warnings.extend(system_prompt.warnings());
    let tools = frozen_tools(&prompts, role, &mut warnings)?;

    // 3. Initial fields; writer sessions copy the book's settings unless told otherwise.
    let book_settings = match book.as_ref() {
        Some(book) => crate::db::json::from_json_or_default::<BookSettings>(&book.settings),
        None => BookSettings::default(),
    };
    let phase = initial_phase(input.kind);
    let (granularity, auto_pause, pause_after_outline) = if matches!(
        input.kind,
        SessionKind::ChapterWrite | SessionKind::ChapterRevise
    ) {
        (
            Some(input.granularity.unwrap_or(book_settings.granularity)),
            input.auto_pause.unwrap_or(book_settings.auto_pause),
            input
                .pause_after_outline
                .unwrap_or(book_settings.pause_after_outline),
        )
    } else {
        (None, false, false)
    };

    // Domain side effects of creation happen in this same transaction (spec/03 §4).
    let track = match input.chapter_id {
        Some(_chapter_id) => Some(apply_creation_side_effects(tx, &input).await?),
        None => None,
    };
    let track_id = track.as_ref().map(|track| track.id);

    let base_content_version = match input.chapter_id {
        Some(chapter_id) => chapters::Entity::find_by_id(chapter_id)
            .one(tx)
            .await?
            .map(|chapter| chapter.content_version),
        None => None,
    };
    let _ = input.chapter_id;

    let now = chrono::Utc::now();
    let session = agent_sessions::ActiveModel {
        book_id: Set(input.book_id),
        role: Set(role.to_owned()),
        kind: Set(input.kind.as_str().to_owned()),
        chapter_id: Set(input.chapter_id),
        track_id: Set(track_id),
        research_task_id: Set(input.research_task_id),
        model_profile_id: Set(Some(model_profile_id)),
        model_snapshot: Set(crate::db::json::to_json(&snapshot)
            .map_err(|err| AppError::internal(err.to_string()))?),
        system_prompt: Set(system_prompt.text),
        tools: Set(
            crate::db::json::to_json(&tools).map_err(|err| AppError::internal(err.to_string()))?
        ),
        status: Set(SessionStatus::Running.as_str().to_owned()),
        phase: Set(phase.as_str().to_owned()),
        granularity: Set(granularity.map(|value| value.as_str().to_owned())),
        auto_pause: Set(i32::from(auto_pause)),
        pause_after_outline: Set(i32::from(pause_after_outline)),
        pause_requested: Set(0),
        held_tool_call_id: Set(None),
        base_content_version: Set(base_content_version),
        pending_comment: Set(None),
        view_snapshot: Set(None),
        counters: Set(crate::db::json::to_json(&SessionCounters::default())
            .map_err(|err| AppError::internal(err.to_string()))?),
        usage: Set("{}".to_owned()),
        error: Set(None),
        parent_session_id: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        finished_at: Set(None),
        ..Default::default()
    }
    .insert(tx)
    .await?;

    // 4. Kickoff: the block list of the kind, persisted as `seq 0`, kind `kickoff`.
    let blocks =
        crate::agent::context::assemble_kickoff(tx, &prompts, &session, input.redraft).await?;
    let kickoff = runner::insert_kickoff(tx, session.id, blocks).await?;
    events_for_new_session(&mut warnings, &session, &kickoff);

    // A large kickoff still starts the session but carries `context.large_kickoff` (§10).
    if kickoff_too_large(&session, &kickoff) {
        warnings.push("context.large_kickoff".to_owned());
    }
    let _ = input.kickoff_extra;

    let mut events = Vec::new();
    events.push(ServerEvent::SessionMessage {
        session_id: session.id,
        message: session::message_dto(&kickoff)?,
    });
    let mut dto_warnings = warnings.clone();
    dto_warnings.dedup();
    events.push(ServerEvent::SessionUpdated {
        session: session::to_dto(tx, &session, dto_warnings.clone()).await?,
        warnings: dto_warnings.clone(),
    });
    Ok((session.id, events, dto_warnings))
}

/// Records the first `session.updated`/`session.message` pair of a new session.
fn events_for_new_session(
    _warnings: &mut [String],
    _session: &agent_sessions::Model,
    _kickoff: &agent_messages::Model,
) {
}

/// Rough token estimate of the kickoff: CJK characters × 1.0 + other characters ÷ 4 (spec/03 §10).
fn kickoff_too_large(session: &agent_sessions::Model, kickoff: &agent_messages::Model) -> bool {
    let Ok(snapshot) = crate::db::json::from_json::<ModelSnapshot>(&session.model_snapshot) else {
        return false;
    };
    let blocks: Vec<StoredBlock> = crate::db::json::from_json_or_default(&kickoff.content);
    let mut estimate = 0i64;
    for block in blocks {
        if let StoredBlock::Text { text, .. } = block {
            let mut cjk = 0i64;
            let mut other = 0i64;
            for ch in text.chars() {
                if is_cjk(ch) {
                    cjk += 1;
                } else {
                    other += 1;
                }
            }
            estimate += cjk + (other + 3) / 4;
        }
    }
    i64::from(snapshot.context_window) > 0 && estimate * 2 > i64::from(snapshot.context_window)
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

/// `book.model_overrides[role]` → `app_settings["roles.defaults"][role]` (spec/03 §4 step 1).
async fn resolve_model(
    tx: &impl ConnectionTrait,
    book: Option<&books::Model>,
    role: AgentRole,
) -> Result<i32, AppError> {
    if let Some(book) = book {
        let overrides: RoleModels = crate::db::json::from_json_or_default(&book.model_overrides);
        if let Some(profile_id) = overrides.get(role) {
            return Ok(profile_id);
        }
    }
    if let Some(profile_id) = read_role_default(tx, role).await? {
        return Ok(profile_id);
    }
    Err(role_defaults_error(role))
}

/// Reads `app_settings["roles.defaults"]` **without** the API masking of
/// [`crate::services::app_settings::get`], which would corrupt a stored secret.
async fn read_role_default(
    conn: &impl ConnectionTrait,
    role: AgentRole,
) -> Result<Option<i32>, AppError> {
    use crate::db::entities::app_settings;
    let row = app_settings::Entity::find_by_id("roles.defaults")
        .one(conn)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let roles: RoleModels = crate::db::json::from_json(&row.value)
        .map_err(|err| AppError::internal(format!("corrupt roles.defaults: {err}")))?;
    Ok(roles.get(role))
}

/// The error of a role without a model (spec/10 §4).
fn role_defaults_error(role: AgentRole) -> AppError {
    crate::agent::context::missing_role(role)
}

/// The frozen model snapshot of a session (spec/03 §4 step 1, spec/02 §4).
fn model_snapshot(
    profile: &model_profiles::Model,
    provider: &providers::Model,
) -> Result<ModelSnapshot, AppError> {
    let extra_headers: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&provider.extra_headers).unwrap_or_default();
    let protocol = match provider.protocol.as_str() {
        "openai" => Protocol::Openai,
        "anthropic" => Protocol::Anthropic,
        _ => Protocol::Mock,
    };
    // `cache_ttl: "auto"` is resolved at creation so the TTL never changes within a session
    // (spec/03 §4 step 1, D-031): 1h for conception and for worker sessions that hold for the
    // author, 5m otherwise.
    let mut options: nekowriter_llm::ModelOptions =
        serde_json::from_str(&profile.options).unwrap_or_default();
    if options.cache_ttl == nekowriter_llm::CacheTtlOption::Auto {
        options.cache_ttl = nekowriter_llm::CacheTtlOption::FiveMinutes;
    }
    Ok(ModelSnapshot {
        provider_id: provider.id,
        protocol,
        base_url: provider.base_url.clone(),
        extra_headers,
        model: profile.model.clone(),
        context_window: profile.context_window,
        max_output_tokens: profile.max_output_tokens,
        options: serde_json::to_value(&options)
            .map_err(|err| AppError::internal(err.to_string()))?,
    })
}

/// The frozen tool definitions of a role (spec/05 §1.3): the schema is the shipped contract, the
/// description is the resolved `tool.<name>` prompt.
fn frozen_tools(
    prompts: &PromptSet,
    role: &str,
    warnings: &mut Vec<String>,
) -> Result<Vec<nekowriter_llm::ToolDef>, AppError> {
    let mut tools = Vec::new();
    for name in tools_for_role(role) {
        let key = format!("tool.{name}");
        let rendered = prompts
            .render(&key, &json!({}))
            .map_err(|err| err.into_app_error())?;
        warnings.extend(rendered.warnings());
        let schema = prompts
            .pack()
            .tool(name)
            .map(|tool| tool.input_schema.clone())
            .ok_or_else(|| AppError::internal(format!("tool {name} is not in the pack")))?;
        tools.push(nekowriter_llm::ToolDef {
            name: (*name).to_owned(),
            description: rendered.text,
            input_schema: schema,
        });
    }
    Ok(tools)
}

/// Domain side effects on creation (spec/03 §4): `chapter_write` sets the chapter to `drafting`
/// (a redraft saves a `backup` version first and clears the text); `chapter_revise` sets
/// `revising` and saves a `backup`; maintenance/re-sync set `canon_status` to `syncing`; a
/// research session sets its task to `running`.
async fn apply_creation_side_effects(
    tx: &sea_orm::DatabaseTransaction,
    input: &CreateSession,
) -> Result<tracks::Model, AppError> {
    use crate::db::json::ChapterStatus;
    use crate::services::versions::{self, NewVersion};

    let Some(chapter_id) = input.chapter_id else {
        return Err(AppError::internal("a chapter session needs a chapter"));
    };
    let chapter = chapters::Entity::find_by_id(chapter_id)
        .one(tx)
        .await?
        .ok_or_else(|| AppError::not_found(format!("chapter {chapter_id}")))?;
    let track = tracks::Entity::find_by_id(chapter.track_id)
        .one(tx)
        .await?
        .ok_or_else(|| AppError::not_found(format!("track {}", chapter.track_id)))?;
    let now = chrono::Utc::now();

    match input.kind {
        SessionKind::ChapterWrite => {
            if input.redraft {
                let content: crate::db::json::ChapterContent =
                    crate::db::json::from_json_or_default(&chapter.content);
                let word_count = crate::text::content_word_count(&content);
                versions::create_version(
                    tx,
                    NewVersion {
                        chapter_id,
                        kind: crate::db::json::VersionKind::Backup,
                        title: &chapter.title,
                        content: &content,
                        word_count,
                        note: "",
                        session_id: None,
                    },
                )
                .await?;
                chapters::ActiveModel {
                    id: Set(chapter_id),
                    content: Set(crate::db::json::to_json(
                        &crate::db::json::ChapterContent::default(),
                    )
                    .map_err(|err| AppError::internal(err.to_string()))?),
                    content_version: Set(chapter.content_version + 1),
                    word_count: Set(0),
                    status: Set(ChapterStatus::Drafting.as_str().to_owned()),
                    updated_at: Set(now),
                    ..Default::default()
                }
                .update(tx)
                .await?;
            } else {
                chapters::ActiveModel {
                    id: Set(chapter_id),
                    status: Set(ChapterStatus::Drafting.as_str().to_owned()),
                    updated_at: Set(now),
                    ..Default::default()
                }
                .update(tx)
                .await?;
            }
        }
        SessionKind::ChapterRevise => {
            let content: crate::db::json::ChapterContent =
                crate::db::json::from_json_or_default(&chapter.content);
            let word_count = crate::text::content_word_count(&content);
            versions::create_version(
                tx,
                NewVersion {
                    chapter_id,
                    kind: crate::db::json::VersionKind::Backup,
                    title: &chapter.title,
                    content: &content,
                    word_count,
                    note: "",
                    session_id: None,
                },
            )
            .await?;
            chapters::ActiveModel {
                id: Set(chapter_id),
                status: Set(ChapterStatus::Revising.as_str().to_owned()),
                updated_at: Set(now),
                ..Default::default()
            }
            .update(tx)
            .await?;
        }
        SessionKind::ChapterMaintenance | SessionKind::ChapterResync => {
            chapters::ActiveModel {
                id: Set(chapter_id),
                canon_status: Set(crate::db::json::CanonStatus::Syncing.as_str().to_owned()),
                updated_at: Set(now),
                ..Default::default()
            }
            .update(tx)
            .await?;
        }
        _ => {}
    }

    if input.kind == SessionKind::Research
        && let Some(task_id) = input.research_task_id
    {
        research_tasks::ActiveModel {
            id: Set(task_id),
            status: Set("running".to_owned()),
            updated_at: Set(now),
            ..Default::default()
        }
        .update(tx)
        .await?;
    }
    Ok(track)
}

// ───────────────────────────── reads ─────────────────────────────

/// `GET /api/sessions/:id` (spec/10 §2.7).
pub async fn snapshot(
    conn: &DatabaseConnection,
    session_id: i32,
) -> Result<SessionSnapshotDto, AppError> {
    let session = session::load(conn, session_id).await?;
    let all = runner::messages_of(conn, session_id).await?;
    let tail: Vec<agent_messages::Model> = all
        .into_iter()
        .rev()
        .take(SNAPSHOT_MESSAGE_LIMIT)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut items = Vec::new();
    let mut tool_calls = Vec::new();
    for message in &tail {
        let calls = runner::calls_of(conn, message.id).await?;
        for call in &calls {
            tool_calls.push(session::tool_call_dto(call)?);
        }
        items.push(crate::api::dto::sessions::MessageWithCallsDto {
            message: session::message_dto(message)?,
            tool_calls: calls
                .iter()
                .map(session::tool_call_dto)
                .collect::<Result<_, _>>()?,
        });
    }
    let tools: Vec<nekowriter_llm::ToolDef> = crate::db::json::from_json_or_default(&session.tools);
    Ok(SessionSnapshotDto {
        session: session::to_dto(conn, &session, Vec::new()).await?,
        messages: items,
        tool_calls,
        inbox_pending: session::inbox_pending(conn, session_id).await? as i32,
        system_prompt: session.system_prompt.clone(),
        tool_names: tools.into_iter().map(|tool| tool.name).collect(),
        next_cursor: None,
    })
}

/// `GET /api/sessions/:id/messages` (spec/10 §2.7): older first, cursor-based.
pub async fn message_page(
    conn: &DatabaseConnection,
    session_id: i32,
    cursor: Option<String>,
    limit: Option<usize>,
) -> Result<MessagePageDto, AppError> {
    session::load(conn, session_id).await?;
    let limit = limit.unwrap_or(MESSAGE_PAGE_DEFAULT).clamp(1, 500);
    let mut query = agent_messages::Entity::find()
        .filter(agent_messages::Column::SessionId.eq(session_id))
        .order_by_desc(agent_messages::Column::Seq);
    if let Some(cursor) = cursor.and_then(|value| value.parse::<i32>().ok()) {
        query = query.filter(agent_messages::Column::Seq.lt(cursor));
    }
    use sea_orm::QuerySelect;
    let rows = query.limit(limit as u64 + 1).all(conn).await?;
    let mut rows: Vec<agent_messages::Model> = rows;
    let has_more = rows.len() > limit;
    if has_more {
        rows.truncate(limit);
    }
    rows.reverse();
    let next_cursor = if has_more {
        rows.first().map(|row| row.seq.to_string())
    } else {
        None
    };
    let mut items = Vec::new();
    for message in &rows {
        let calls = runner::calls_of(conn, message.id).await?;
        items.push(crate::api::dto::sessions::MessageWithCallsDto {
            message: session::message_dto(message)?,
            tool_calls: calls
                .iter()
                .map(session::tool_call_dto)
                .collect::<Result<_, _>>()?,
        });
    }
    Ok(MessagePageDto { items, next_cursor })
}

/// `GET /api/books/:id/sessions` (spec/10 §2.7).
pub async fn list(
    conn: &DatabaseConnection,
    book_id: i32,
    kind: Option<SessionKind>,
    status: Option<SessionStatus>,
    chapter_id: Option<i32>,
) -> Result<Vec<crate::api::dto::SessionDto>, AppError> {
    let mut query = agent_sessions::Entity::find()
        .filter(agent_sessions::Column::BookId.eq(book_id))
        .order_by_desc(agent_sessions::Column::Id);
    if let Some(kind) = kind {
        query = query.filter(agent_sessions::Column::Kind.eq(kind.as_str()));
    }
    if let Some(status) = status {
        query = query.filter(agent_sessions::Column::Status.eq(status.as_str()));
    }
    if let Some(chapter_id) = chapter_id {
        query = query.filter(agent_sessions::Column::ChapterId.eq(chapter_id));
    }
    let rows = query.all(conn).await?;
    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(session::to_dto(conn, &row, Vec::new()).await?);
    }
    Ok(sessions)
}

// ───────────────────────────── control operations ─────────────────────────────

/// Runs one control operation that only changes a non-running session, then spawns its runner.
async fn resume_after(state: &AppState, events: Vec<ServerEvent>, session_id: i32) {
    state.events().publish_all(events);
    state.manager().spawn(state.clone(), session_id);
}

/// Publishes the events of a control operation that spawns no runner.
async fn publish(state: &AppState, events: Vec<ServerEvent>) {
    state.events().publish_all(events);
}

/// `POST /api/sessions/:id/pause` (spec/03 §6): `pause_requested ← 1`, running only.
pub async fn pause(
    state: &AppState,
    session_id: i32,
) -> Result<crate::api::dto::SessionDto, AppError> {
    let db = state.db().clone();
    let events = services::write(&db, move |tx| {
        Box::pin(async move { pause_inner(tx, session_id).await })
    })
    .await?;
    publish(state, events.0).await;
    Ok(events.1)
}

/// The in-transaction part of `pause`.
async fn pause_inner(
    tx: &sea_orm::DatabaseTransaction,
    session_id: i32,
) -> Result<(Vec<ServerEvent>, crate::api::dto::SessionDto), AppError> {
    let session = session::load(tx, session_id).await?;
    if session.status != SessionStatus::Running.as_str() {
        return Err(invalid_state(&session, "pause"));
    }
    agent_sessions::ActiveModel {
        id: Set(session_id),
        pause_requested: Set(1),
        updated_at: Set(chrono::Utc::now()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    let session = session::load(tx, session_id).await?;
    let dto = session::to_dto(tx, &session, Vec::new()).await?;
    Ok((
        vec![ServerEvent::SessionUpdated {
            session: dto.clone(),
            warnings: Vec::new(),
        }],
        dto,
    ))
}

/// `POST /api/sessions/:id/resume` (spec/03 §6): `held` (approve or feedback) or `paused`.
pub async fn resume(
    state: &AppState,
    session_id: i32,
    input: ResumeInput,
) -> Result<crate::api::dto::SessionDto, AppError> {
    state
        .manager()
        .wait_cancelled(session_id, std::time::Duration::from_secs(10))
        .await;
    let db = state.db().clone();
    let state_owned = state.clone();
    let feedback = input.feedback.clone();
    let (events, dto) = services::write(&db, move |tx| {
        let feedback = feedback.clone();
        Box::pin(async move { resume_inner(&state_owned, tx, session_id, feedback).await })
    })
    .await?;
    resume_after(state, events, session_id).await;
    Ok(dto)
}

/// The in-transaction part of `resume`.
async fn resume_inner(
    state: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    session_id: i32,
    feedback: Option<String>,
) -> Result<(Vec<ServerEvent>, crate::api::dto::SessionDto), AppError> {
    let session = session::load(tx, session_id).await?;
    let status = session::status_of(&session);
    let mut events = Vec::new();
    match status {
        SessionStatus::Held => {
            let Some(call_id) = session.held_tool_call_id else {
                return Err(invalid_state(&session, "resume"));
            };
            let mut held = runner::load_call(tx, call_id).await?;
            if held.status != ToolCallStatus::Held.as_str() {
                return Err(invalid_state(&session, "resume"));
            }
            let prompts = state_prompts(state, &session).await?;
            let mut appendices: Vec<crate::db::json::Appendix> =
                crate::db::json::from_json_or_default(&held.appendices);
            // Feedback appended, and deferred effects revoked first (spec/03 §5.4).
            if let Some(feedback) = feedback
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                let effects: crate::db::json::ToolEffects =
                    crate::db::json::from_json_or_default(held.effects.as_deref().unwrap_or("{}"));
                if effects.complete.is_some() {
                    let rendered = prompts
                        .render("notice.completion_revoked", &json!({}))
                        .map_err(|err| err.into_app_error())?;
                    appendices.push(crate::db::json::Appendix {
                        kind: crate::db::json::AppendixKind::EffectNotice,
                        text: rendered.text,
                        inbox_id: None,
                    });
                }
                if let Some(notice) = rejected_notice(&held.name) {
                    let rendered = prompts
                        .render(notice, &json!({}))
                        .map_err(|err| err.into_app_error())?;
                    appendices.push(crate::db::json::Appendix {
                        kind: crate::db::json::AppendixKind::EffectNotice,
                        text: rendered.text,
                        inbox_id: None,
                    });
                }
                let rendered = prompts
                    .render("notice.user_feedback", &json!({ "text": feedback }))
                    .map_err(|err| err.into_app_error())?;
                appendices.push(crate::db::json::Appendix {
                    kind: crate::db::json::AppendixKind::UserFeedback,
                    text: rendered.text,
                    inbox_id: None,
                });
            }
            // Finalize the held call: effects revoked when feedback was given, else applied.
            let revoke = feedback
                .as_deref()
                .map(str::trim)
                .is_some_and(|text| !text.is_empty());
            let now = chrono::Utc::now();
            let mut effects: crate::db::json::ToolEffects =
                crate::db::json::from_json_or_default(held.effects.as_deref().unwrap_or("{}"));
            if revoke {
                effects = crate::db::json::ToolEffects::default();
            }
            agent_tool_calls::ActiveModel {
                id: Set(held.id),
                status: Set(if held.is_error != 0 {
                    ToolCallStatus::Error.as_str().to_owned()
                } else {
                    ToolCallStatus::Done.as_str().to_owned()
                }),
                appendices: Set(crate::db::json::to_json(&appendices)
                    .map_err(|err| AppError::internal(err.to_string()))?),
                effects: Set(Some(
                    crate::db::json::to_json(&crate::db::json::ToolEffects::default())
                        .map_err(|err| AppError::internal(err.to_string()))?,
                )),
                finished_at: Set(Some(now)),
                ..Default::default()
            }
            .update(tx)
            .await?;
            // Apply the kept effects (a phase change) when no feedback revoked them.
            if !revoke {
                if let Some(phase) = effects.phase_to {
                    agent_sessions::ActiveModel {
                        id: Set(session_id),
                        phase: Set(phase.as_str().to_owned()),
                        ..Default::default()
                    }
                    .update(tx)
                    .await?;
                }
                if let Some(completion) = effects.complete {
                    let mut completion_events =
                        crate::agent::context::apply_completion(tx, &session, &held, completion)
                            .await?;
                    events.append(&mut completion_events);
                }
            }
            held = runner::load_call(tx, held.id).await?;
            events.push(ServerEvent::SessionToolCall {
                session_id,
                tool_call: session::tool_call_dto(&held)?,
            });
            let running = set_status(tx, &session, SessionStatus::Running, true).await?;
            events.push(ServerEvent::SessionUpdated {
                session: session::to_dto(tx, &running, Vec::new()).await?,
                warnings: Vec::new(),
            });
        }
        SessionStatus::Paused => {
            if let Some(feedback) = feedback
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                // A paused session queues feedback as an inbox user message (spec/03 §6).
                insert_inbox(tx, session_id, InboxKind::UserMessage, feedback).await?;
            }
            // A circuit-breaker pause clears its error on resume (spec/03 §6).
            agent_sessions::ActiveModel {
                id: Set(session_id),
                error: Set(None),
                ..Default::default()
            }
            .update(tx)
            .await?;
            let running = set_status(tx, &session, SessionStatus::Running, true).await?;
            events.push(ServerEvent::SessionUpdated {
                session: session::to_dto(tx, &running, Vec::new()).await?,
                warnings: Vec::new(),
            });
        }
        _ => return Err(invalid_state(&session, "resume")),
    }
    Ok((
        events,
        session::to_dto(tx, &session::load(tx, session_id).await?, Vec::new()).await?,
    ))
}

/// The rejection notice of a held call whose effects were revoked (spec/03 §5.4).
fn rejected_notice(tool: &str) -> Option<&'static str> {
    match tool {
        "submit_outline" => Some("writer.notice.outline_rejected"),
        "submit_revision_plan" => Some("writer.notice.revision_plan_rejected"),
        _ => None,
    }
}

/// `POST /api/sessions/:id/stop` (spec/03 §6): running only; cancels the runner's token and
/// returns at once (D-023).
pub async fn stop(
    state: &AppState,
    session_id: i32,
) -> Result<crate::api::dto::SessionDto, AppError> {
    let db = state.db().clone();
    let dto = services::write(&db, move |tx| {
        Box::pin(async move {
            let session = session::load(tx, session_id).await?;
            if session.status != SessionStatus::Running.as_str() {
                return Err(invalid_state(&session, "stop"));
            }
            session::to_dto(tx, &session, Vec::new()).await
        })
    })
    .await?;
    // The runner is the only writer of a running session: it performs the transition at its next
    // await point (spec/03 §6 / D-034).
    state
        .manager()
        .cancel(session_id, crate::agent::CancelReason::Stop);
    Ok(dto)
}

/// `POST /api/sessions/:id/continue` (spec/03 §6): stopped, failed or interrupted → running.
pub async fn continue_session(
    state: &AppState,
    session_id: i32,
) -> Result<crate::api::dto::SessionDto, AppError> {
    // A stop is a cancellation: let the cancelled runner finish its last write before the
    // status change, so the two cannot interleave (spec/03 §6, D-034).
    state
        .manager()
        .wait_cancelled(session_id, std::time::Duration::from_secs(10))
        .await;
    let db = state.db().clone();
    let (events, dto) = services::write(&db, move |tx| {
        Box::pin(async move {
            let session = session::load(tx, session_id).await?;
            let status = session::status_of(&session);
            if !matches!(
                status,
                SessionStatus::Stopped | SessionStatus::Failed | SessionStatus::Interrupted
            ) {
                return Err(invalid_state(&session, "continue"));
            }
            agent_sessions::ActiveModel {
                id: Set(session_id),
                error: Set(None),
                ..Default::default()
            }
            .update(tx)
            .await?;
            let running = set_status(tx, &session, SessionStatus::Running, false).await?;
            let dto = session::to_dto(tx, &running, Vec::new()).await?;
            Ok((
                vec![ServerEvent::SessionUpdated {
                    session: dto.clone(),
                    warnings: Vec::new(),
                }],
                dto,
            ))
        })
    })
    .await?;
    resume_after(state, events, session_id).await;
    Ok(dto)
}

/// `POST /api/sessions/:id/abandon` (spec/03 §6): any open status → abandoned.
pub async fn abandon(
    state: &AppState,
    session_id: i32,
) -> Result<crate::api::dto::SessionDto, AppError> {
    // A running session is stopped first: cancel, wait for the runner to exit (spec/03 §6).
    let session = session::load(state.db().conn(), session_id).await?;
    if session.status == SessionStatus::Running.as_str() {
        state
            .manager()
            .cancel(session_id, crate::agent::CancelReason::Abandon);
        state
            .manager()
            .wait_for(session_id, std::time::Duration::from_secs(10))
            .await;
    }
    let db = state.db().clone();
    let (events, dto) = services::write(&db, move |tx| {
        Box::pin(async move {
            let session = session::load(tx, session_id).await?;
            if session::is_terminal(session::status_of(&session)) {
                return Err(invalid_state(&session, "abandon"));
            }
            let events = session::transition(
                tx,
                &session,
                SessionStatus::Abandoned,
                TransitionCause::Stop,
            )
            .await?;
            let dto =
                session::to_dto(tx, &session::load(tx, session_id).await?, Vec::new()).await?;
            Ok((events, dto))
        })
    })
    .await?;
    publish(state, events).await;
    Ok(dto)
}

/// `POST /api/sessions/:id/message` (spec/03 §6).
pub async fn send_message(
    state: &AppState,
    session_id: i32,
    input: SessionMessageInput,
) -> Result<crate::api::dto::SessionDto, AppError> {
    let text = input.text.trim().to_owned();
    if text.is_empty() {
        return Err(AppError::validation("the message is empty"));
    }
    let db = state.db().clone();
    let (events, dto, spawn) = services::write(&db, move |tx| {
        let text = text.clone();
        Box::pin(async move {
            let session = session::load(tx, session_id).await?;
            let status = session::status_of(&session);
            match status {
                SessionStatus::AwaitingInput => {
                    // The form must be answered (spec/03 §6).
                    return Err(invalid_state(&session, "message"));
                }
                SessionStatus::Running
                | SessionStatus::Held
                | SessionStatus::Paused
                | SessionStatus::Idle
                | SessionStatus::Stopped
                | SessionStatus::Failed
                | SessionStatus::Interrupted => {}
                _ => return Err(invalid_state(&session, "message")),
            }
            insert_inbox(tx, session_id, InboxKind::UserMessage, &text).await?;
            // An idle conception session starts working on the reply (spec/03 §6).
            let spawn = status == SessionStatus::Idle;
            if spawn {
                set_status(tx, &session, SessionStatus::Running, false).await?;
            }
            let dto =
                session::to_dto(tx, &session::load(tx, session_id).await?, Vec::new()).await?;
            Ok((
                vec![ServerEvent::SessionUpdated {
                    session: dto.clone(),
                    warnings: Vec::new(),
                }],
                dto,
                spawn,
            ))
        })
    })
    .await?;
    if spawn {
        resume_after(state, events, session_id).await;
    } else {
        publish(state, events).await;
    }
    Ok(dto)
}

/// `POST /api/sessions/:id/answer` (spec/03 §6): awaiting_input only.
pub async fn answer(
    state: &AppState,
    session_id: i32,
    input: SessionAnswerInput,
) -> Result<crate::api::dto::SessionDto, AppError> {
    let db = state.db().clone();
    let state_owned = state.clone();
    let input = input.clone();
    let (events, dto) = services::write(&db, move |tx| {
        let input = input.clone();
        Box::pin(async move { answer_inner(&state_owned, tx, session_id, input).await })
    })
    .await?;
    resume_after(state, events, session_id).await;
    Ok(dto)
}

/// The in-transaction part of `answer`.
async fn answer_inner(
    state: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    session_id: i32,
    input: SessionAnswerInput,
) -> Result<(Vec<ServerEvent>, crate::api::dto::SessionDto), AppError> {
    let session = session::load(tx, session_id).await?;
    if session.status != SessionStatus::AwaitingInput.as_str() {
        return Err(invalid_state(&session, "answer"));
    }
    if session.held_tool_call_id != Some(input.tool_call_id) {
        return Err(AppError::validation(
            "the answered tool call is not the one waiting for input",
        ));
    }
    let call = runner::load_call(tx, input.tool_call_id).await?;
    if call.status != ToolCallStatus::AwaitingInput.as_str() {
        return Err(invalid_state(&session, "answer"));
    }
    let prompts = state_prompts(state, &session).await?;
    // The answer becomes the call's result (spec/03 §6). `ask_user` and `request_review`
    // validate their payload in their handlers; M3 validates the envelope here.
    let rendered = render_answer(&prompts, &call.name, &input.payload)?;
    let effects: crate::db::json::ToolEffects =
        crate::db::json::from_json_or_default(call.effects.as_deref().unwrap_or("{}"));
    agent_tool_calls::ActiveModel {
        id: Set(call.id),
        status: Set(ToolCallStatus::Done.as_str().to_owned()),
        raw_result_text: Set(Some(rendered)),
        effects: Set(Some(
            crate::db::json::to_json(&crate::db::json::ToolEffects::default())
                .map_err(|err| AppError::internal(err.to_string()))?,
        )),
        finished_at: Set(Some(chrono::Utc::now())),
        ..Default::default()
    }
    .update(tx)
    .await?;
    let mut events = Vec::new();
    if let Some(completion) = effects.complete {
        let mut completion_events =
            crate::agent::context::apply_completion(tx, &session, &call, completion).await?;
        events.append(&mut completion_events);
    }
    let call = runner::load_call(tx, call.id).await?;
    events.push(ServerEvent::SessionToolCall {
        session_id,
        tool_call: session::tool_call_dto(&call)?,
    });
    let running = set_status(tx, &session, SessionStatus::Running, true).await?;
    let dto = session::to_dto(tx, &running, Vec::new()).await?;
    events.push(ServerEvent::SessionUpdated {
        session: dto.clone(),
        warnings: Vec::new(),
    });
    Ok((events, dto))
}

/// Renders the answered call's result (spec/05 §7 templates).
fn render_answer(
    prompts: &PromptSet,
    tool: &str,
    payload: &serde_json::Value,
) -> Result<String, AppError> {
    let key = match tool {
        "ask_user" => "result.ask_user",
        "request_review" => "result.request_review",
        other => {
            return Err(AppError::validation(format!(
                "{other} does not accept an answer"
            )));
        }
    };
    let mut context = payload.clone();
    if let serde_json::Value::Object(map) = &mut context {
        map.entry("general").or_insert(json!(""));
        map.entry("all_approved").or_insert(json!(true));
    }
    prompts
        .render_value(key, crate::prompts::template_value(&context))
        .map(|rendered| rendered.text)
        .map_err(|err| err.into_app_error())
}

/// `PATCH /api/sessions/:id` (spec/03 §6, §8): the writer switches.
pub async fn patch(
    state: &AppState,
    session_id: i32,
    patch: SessionPatch,
) -> Result<crate::api::dto::SessionDto, AppError> {
    let db = state.db().clone();
    let state_owned = state.clone();
    services::write(&db, move |tx| {
        let patch = patch.clone();
        let state = state_owned.clone();
        Box::pin(async move {
            let session = session::load(tx, session_id).await?;
            if session::is_terminal(session::status_of(&session)) {
                return Err(invalid_state(&session, "patch"));
            }
            let is_writer = matches!(
                session::kind_of(&session),
                SessionKind::ChapterWrite | SessionKind::ChapterRevise
            );
            if !is_writer {
                return Err(invalid_state(&session, "patch"));
            }
            let mut sets: Vec<&'static str> = Vec::new();
            let mut values: Vec<sea_orm::Value> = Vec::new();
            let mut notice: Option<Granularity> = None;
            if let Some(granularity) = patch.granularity {
                sets.push("granularity = ?");
                values.push(granularity.as_str().to_owned().into());
                let phase = session::phase_of(&session);
                if matches!(phase, Phase::Drafting | Phase::Revising) {
                    notice = Some(granularity);
                }
            }
            if let Some(auto_pause) = patch.auto_pause {
                sets.push("auto_pause = ?");
                values.push(i32::from(auto_pause).into());
            }
            if let Some(pause_after_outline) = patch.pause_after_outline {
                sets.push("pause_after_outline = ?");
                values.push(i32::from(pause_after_outline).into());
            }
            if sets.is_empty() {
                return Err(AppError::validation("nothing to change"));
            }
            sets.push("updated_at = ?");
            values.push(chrono::Utc::now().into());
            session::guard_columns(tx, session_id, &session.status, &sets.join(", "), values)
                .await?;
            // A granularity change during drafting or revising queues the notice (spec/03 §8).
            if let Some(granularity) = notice {
                let prompts = state_prompts(&state, &session).await?;
                let (tail_open, last_pid) = (false, String::new());
                let key = granularity.as_str();
                let context = json!({
                    "granularity": {
                        "key": key,
                        "label": prompts.label("granularity", key),
                        "revising": matches!(
                            session::phase_of(&session),
                            Phase::RevisionPlanning | Phase::Revising
                        ),
                        "tail_open": tail_open,
                        "last_pid": last_pid,
                    },
                });
                let rendered = prompts
                    .render_value(
                        "writer.notice.granularity",
                        crate::prompts::template_value(&context),
                    )
                    .map_err(|err| err.into_app_error())?;
                insert_inbox(tx, session_id, InboxKind::Notice, &rendered.text).await?;
            }
            let dto =
                session::to_dto(tx, &session::load(tx, session_id).await?, Vec::new()).await?;
            Ok((
                vec![ServerEvent::SessionUpdated {
                    session: dto.clone(),
                    warnings: Vec::new(),
                }],
                dto,
            ))
        })
    })
    .await
    .map(|(events, dto)| {
        state.events().publish_all(events);
        dto
    })
}

/// Starts a test-only session with the debug tools frozen, a chosen kind and phase, and optional
/// chapter (the runtime tests of PLAN M3.1 drive ``auto_pause``/hold/stop paths this way). Behind
/// the `testing` feature so a release build cannot create one.
#[cfg(any(test, feature = "testing"))]
pub async fn create_debug_session(
    state: &AppState,
    spec: DebugSessionSpec,
) -> Result<i32, AppError> {
    let db = state.db().clone();
    let state_owned = state.clone();
    let (session_id, events) = services::write(&db, move |tx| {
        Box::pin(async move { debug_session_inner(&state_owned, tx, spec).await })
    })
    .await?;
    state.events().publish_all(events);
    state.manager().spawn(state.clone(), session_id);
    Ok(session_id)
}

/// The parameters of a test-only session.
#[cfg(any(test, feature = "testing"))]
#[derive(Debug, Clone)]
pub struct DebugSessionSpec {
    pub book_id: i32,
    pub chapter_id: Option<i32>,
    pub kind: SessionKind,
    pub phase: Phase,
    pub granularity: Option<Granularity>,
    pub auto_pause: bool,
    pub pause_after_outline: bool,
}

/// The in-transaction part of the test-only session creation.
#[cfg(any(test, feature = "testing"))]
async fn debug_session_inner(
    state: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    spec: DebugSessionSpec,
) -> Result<(i32, Vec<ServerEvent>), AppError> {
    let book = books::Entity::find_by_id(spec.book_id)
        .one(tx)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book {}", spec.book_id)))?;
    let track = match spec.chapter_id {
        Some(chapter_id) => chapters::Entity::find_by_id(chapter_id)
            .one(tx)
            .await?
            .map(|chapter| chapter.track_id),
        None => tracks::Entity::find()
            .filter(tracks::Column::BookId.eq(spec.book_id))
            .filter(tracks::Column::Kind.eq("main"))
            .one(tx)
            .await?
            .map(|track| track.id),
    };
    let profile_id = resolve_model(tx, Some(&book), AgentRole::Writer).await?;
    let profile = model_profiles::Entity::find_by_id(profile_id)
        .one(tx)
        .await?
        .ok_or_else(|| role_defaults_error(AgentRole::Writer))?;
    let provider = providers::Entity::find_by_id(profile.provider_id)
        .one(tx)
        .await?
        .ok_or_else(|| role_defaults_error(AgentRole::Writer))?;
    let snapshot = model_snapshot(&profile, &provider)?;

    let specs = state.tool_specs();
    let prompts = state
        .prompts()
        .for_book(state.db(), Some(spec.book_id))
        .await?;
    let mut tools = Vec::new();
    for name in crate::agent::tools::registry::DEBUG_TOOLS {
        let schema = specs
            .schema(name)
            .cloned()
            .ok_or_else(|| AppError::internal(format!("the debug tool {name} has no schema")))?;
        tools.push(nekowriter_llm::ToolDef {
            name: (*name).to_owned(),
            description: format!("test-only tool {name}"),
            input_schema: schema,
        });
    }
    let now = chrono::Utc::now();
    let session = agent_sessions::ActiveModel {
        book_id: Set(Some(spec.book_id)),
        role: Set(role_of_kind(spec.kind).to_owned()),
        kind: Set(spec.kind.as_str().to_owned()),
        chapter_id: Set(spec.chapter_id),
        track_id: Set(track),
        research_task_id: Set(None),
        model_profile_id: Set(Some(profile_id)),
        model_snapshot: Set(crate::db::json::to_json(&snapshot)
            .map_err(|err| AppError::internal(err.to_string()))?),
        system_prompt: Set(prompts
            .render("writer.system", &json!({}))
            .map_err(|err| err.into_app_error())?
            .text),
        tools: Set(
            crate::db::json::to_json(&tools).map_err(|err| AppError::internal(err.to_string()))?
        ),
        status: Set(SessionStatus::Running.as_str().to_owned()),
        phase: Set(spec.phase.as_str().to_owned()),
        granularity: Set(spec.granularity.map(|value| value.as_str().to_owned())),
        auto_pause: Set(i32::from(spec.auto_pause)),
        pause_after_outline: Set(i32::from(spec.pause_after_outline)),
        pause_requested: Set(0),
        held_tool_call_id: Set(None),
        base_content_version: Set(match spec.chapter_id {
            Some(chapter_id) => chapters::Entity::find_by_id(chapter_id)
                .one(tx)
                .await?
                .map(|chapter| chapter.content_version),
            None => None,
        }),
        pending_comment: Set(None),
        view_snapshot: Set(None),
        counters: Set(crate::db::json::to_json(&SessionCounters::default())
            .map_err(|err| AppError::internal(err.to_string()))?),
        usage: Set("{}".to_owned()),
        error: Set(None),
        parent_session_id: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        finished_at: Set(None),
        ..Default::default()
    }
    .insert(tx)
    .await?;
    if let Some(chapter_id) = spec.chapter_id {
        chapters::ActiveModel {
            id: Set(chapter_id),
            status: Set(crate::db::json::ChapterStatus::Drafting.as_str().to_owned()),
            updated_at: Set(now),
            ..Default::default()
        }
        .update(tx)
        .await?;
    }
    let kickoff = runner::insert_kickoff(
        tx,
        session.id,
        vec![StoredBlock::Text {
            text: "Debug session: use the test-only tools.".to_owned(),
            cache: None,
        }],
    )
    .await?;
    let mut events = vec![ServerEvent::SessionMessage {
        session_id: session.id,
        message: session::message_dto(&kickoff)?,
    }];
    events.push(ServerEvent::SessionUpdated {
        session: session::to_dto(tx, &session, Vec::new()).await?,
        warnings: Vec::new(),
    });
    Ok((session.id, events))
}

/// `POST /api/debug/sessions/echo` (spec/10 §2.7, debug builds only): a test-only session whose
/// tools exercise the runtime without any real tool.
pub async fn debug_echo(state: &AppState, book_id: i32) -> Result<DebugEchoDto, AppError> {
    let db = state.db().clone();
    let state_owned = state.clone();
    let (session_id, events) = services::write(&db, move |tx| {
        Box::pin(async move { debug_echo_inner(&state_owned, tx, book_id).await })
    })
    .await?;
    state.events().publish_all(events);
    state.manager().spawn(state.clone(), session_id);
    Ok(DebugEchoDto { session_id })
}

/// The in-transaction part of the debug session creation.
async fn debug_echo_inner(
    state: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    book_id: i32,
) -> Result<(i32, Vec<ServerEvent>), AppError> {
    let book = books::Entity::find_by_id(book_id)
        .one(tx)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?;
    let track = tracks::Entity::find()
        .filter(tracks::Column::BookId.eq(book_id))
        .filter(tracks::Column::Kind.eq("main"))
        .one(tx)
        .await?;
    let profile_id = resolve_model(tx, Some(&book), AgentRole::Writer).await?;
    let profile = model_profiles::Entity::find_by_id(profile_id)
        .one(tx)
        .await?
        .ok_or_else(|| role_defaults_error(AgentRole::Writer))?;
    let provider = providers::Entity::find_by_id(profile.provider_id)
        .one(tx)
        .await?
        .ok_or_else(|| role_defaults_error(AgentRole::Writer))?;
    let snapshot = model_snapshot(&profile, &provider)?;

    // The debug session freezes exactly the test-only tools.
    let specs = state.tool_specs();
    let prompts = state.prompts().for_book(state.db(), Some(book_id)).await?;
    let mut tools = Vec::new();
    for name in crate::agent::tools::registry::DEBUG_TOOLS {
        let schema = specs
            .schema(name)
            .cloned()
            .ok_or_else(|| AppError::internal(format!("the debug tool {name} has no schema")))?;
        tools.push(nekowriter_llm::ToolDef {
            name: (*name).to_owned(),
            description: format!("test-only tool {name}"),
            input_schema: schema,
        });
    }
    let now = chrono::Utc::now();
    let session = agent_sessions::ActiveModel {
        book_id: Set(Some(book_id)),
        role: Set(AgentRole::Writer.as_str().to_owned()),
        kind: Set(SessionKind::ChapterWrite.as_str().to_owned()),
        chapter_id: Set(None),
        track_id: Set(track.map(|track| track.id)),
        research_task_id: Set(None),
        model_profile_id: Set(Some(profile_id)),
        model_snapshot: Set(crate::db::json::to_json(&snapshot)
            .map_err(|err| AppError::internal(err.to_string()))?),
        system_prompt: Set(prompts
            .render("writer.system", &json!({}))
            .map_err(|err| err.into_app_error())?
            .text),
        tools: Set(
            crate::db::json::to_json(&tools).map_err(|err| AppError::internal(err.to_string()))?
        ),
        status: Set(SessionStatus::Running.as_str().to_owned()),
        phase: Set(Phase::Drafting.as_str().to_owned()),
        granularity: Set(Some(Granularity::Paragraph.as_str().to_owned())),
        auto_pause: Set(0),
        pause_after_outline: Set(0),
        pause_requested: Set(0),
        held_tool_call_id: Set(None),
        base_content_version: Set(None),
        pending_comment: Set(None),
        view_snapshot: Set(None),
        counters: Set(crate::db::json::to_json(&SessionCounters::default())
            .map_err(|err| AppError::internal(err.to_string()))?),
        usage: Set("{}".to_owned()),
        error: Set(None),
        parent_session_id: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        finished_at: Set(None),
        ..Default::default()
    }
    .insert(tx)
    .await?;
    let kickoff = runner::insert_kickoff(
        tx,
        session.id,
        vec![StoredBlock::Text {
            text: "Debug session: use the test-only tools.".to_owned(),
            cache: None,
        }],
    )
    .await?;
    let mut events = vec![ServerEvent::SessionMessage {
        session_id: session.id,
        message: session::message_dto(&kickoff)?,
    }];
    events.push(ServerEvent::SessionUpdated {
        session: session::to_dto(tx, &session, Vec::new()).await?,
        warnings: Vec::new(),
    });
    Ok((session.id, events))
}

// ───────────────────────────── helpers ─────────────────────────────

/// Sets a session's status from an operation that is not the runner, clearing `pause_requested`
/// when it enters `running` (spec/03 §3).
async fn set_status(
    tx: &sea_orm::DatabaseTransaction,
    session: &agent_sessions::Model,
    to: SessionStatus,
    clear_pause: bool,
) -> Result<agent_sessions::Model, AppError> {
    let now = chrono::Utc::now();
    let mut active = agent_sessions::ActiveModel {
        id: Set(session.id),
        status: Set(to.as_str().to_owned()),
        updated_at: Set(now),
        ..Default::default()
    };
    if clear_pause {
        active.pause_requested = Set(0);
    }
    // A status change that leaves `running` clears the pause flag and captures the snapshot, so
    // those operations must go through `transition`; these paths all enter `running`.
    debug_assert!(to == SessionStatus::Running);
    active.update(tx).await?;
    session::load(tx, session.id).await
}

/// Inserts one inbox row.
async fn insert_inbox(
    tx: &sea_orm::DatabaseTransaction,
    session_id: i32,
    kind: InboxKind,
    content: &str,
) -> Result<agent_inbox::Model, AppError> {
    Ok(agent_inbox::ActiveModel {
        session_id: Set(session_id),
        kind: Set(kind.as_str().to_owned()),
        content: Set(content.to_owned()),
        created_at: Set(chrono::Utc::now()),
        consumed_at: Set(None),
        consumed_by_tool_call_id: Set(None),
        consumed_by_message_id: Set(None),
        ..Default::default()
    }
    .insert(tx)
    .await?)
}

/// The prompt set of a session's book.
async fn state_prompts(
    state: &AppState,
    session: &agent_sessions::Model,
) -> Result<std::sync::Arc<PromptSet>, AppError> {
    state.prompts().for_book(state.db(), session.book_id).await
}

/// `409 session.invalid_state` with `details.status` and `details.operation` (spec/10 §4).
fn invalid_state(session: &agent_sessions::Model, operation: &str) -> AppError {
    AppError::new(
        ErrorCode::SessionInvalidState,
        format!(
            "{operation} is not allowed while the session is {}",
            session.status
        ),
    )
    .with_details(json!({
        "status": session.status,
        "operation": operation,
    }))
}
