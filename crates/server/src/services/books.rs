//! Book service: create (with the main track and the three setting documents), read, patch
//! (archive), delete, and the overview (spec/08 §1.3, §5, §7; spec/02 §5 invariants 1–2).

use super::field_error;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use std::collections::BTreeMap;

use crate::api::dto::books::{
    AttentionItemDto, AttentionKind, BookDto, BookOverviewDto, BookPatch, BookProgressDto,
    BookSummaryDto, ChapterPointerDto, CreateBookResponse, NextActionDto, NextActionKind,
    StatusCountDto, TrackProgressDto,
};
use crate::db::entities::{
    agent_sessions, books, chapters, character_designs, entities as canon_entities, research_tasks,
    setting_docs, setting_proposals, tracks,
};
use crate::db::json::{
    AnchorKind, BookSettings, BookStatus, CanonMode, CanonStatus, ChapterStatus, ConceptionState,
    ResearchTaskStatus, RoleModels, SessionKind, SessionStatus, TrackKind, Usage,
};
use crate::error::{AppError, ErrorCode};
use crate::events::{BookOp, ServerEvent};

use super::app_settings;
use super::{conflict, json, now, parse_json};

/// The three setting documents every book has (spec/02 §5 invariant 2).
pub const SETTING_DOC_KINDS: [&str; 3] = ["outline", "world", "protagonist"];

/// `POST /api/books` (spec/08 §5). `start_conception` makes the book `conceiving` and starts
/// its conception session (when a conception model is configured; the response's `session_id`
/// and any `model.missing_role` warning come from the conception service, which needs this
/// transaction committed first).
pub async fn create(
    state: &crate::state::AppState,
    input: crate::api::dto::books::BookInput,
) -> Result<CreateBookResponse, AppError> {
    let db = state.db().clone();
    let (mut response, published) = super::write(&db, move |tx| {
        Box::pin(async move { create_inner(tx, input).await })
    })
    .await?;
    state.events().publish_all(published);
    if response.book.status == BookStatus::Conceiving {
        let (session_id, warnings) =
            super::conception::start_for_new_book(state, response.book.id).await?;
        response.session_id = session_id;
        response.warnings.extend(warnings);
    }
    Ok(response)
}

/// The inner create; runs inside the caller's transaction.
pub async fn create_inner(
    tx: &impl ConnectionTrait,
    input: crate::api::dto::books::BookInput,
) -> Result<(CreateBookResponse, Vec<ServerEvent>), AppError> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err(field_error("title", "a book needs a title"));
    }
    let language = input.writing_language.trim();
    if language.is_empty() {
        return Err(field_error(
            "writing_language",
            "a writing language is required",
        ));
    }
    validate_language(language)?;

    let settings = app_settings::book_defaults(tx).await?;
    let timestamp = now();
    let status = if input.start_conception {
        BookStatus::Conceiving
    } else {
        BookStatus::Active
    };
    let model = books::ActiveModel {
        title: Set(title.to_owned()),
        idea: Set(input.idea.trim().to_owned()),
        genre: Set(input.genre.trim().to_owned()),
        writing_language: Set(language.to_owned()),
        status: Set(status.as_str().to_owned()),
        settings: Set(json(&settings)?),
        model_overrides: Set(json(&RoleModels::default())?),
        conception: Set(json(&ConceptionState::default())?),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    let book = books::Entity::insert(model).exec_with_returning(tx).await?;

    // Invariant 1: exactly one main track, created with the book (spec/02 §5).
    let main_track = tracks::ActiveModel {
        book_id: Set(book.id),
        kind: Set(TrackKind::Main.as_str().to_owned()),
        title: Set(String::new()),
        synopsis: Set(String::new()),
        canon_mode: Set(CanonMode::Canon.as_str().to_owned()),
        anchor_kind: Set(AnchorKind::None.as_str().to_owned()),
        anchor_chapter_id: Set(None),
        focus_entity_id: Set(None),
        ord: Set(0),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    tracks::Entity::insert(main_track).exec(tx).await?;

    // Invariant 2: exactly three setting documents, empty (spec/02 §5).
    for kind in SETTING_DOC_KINDS {
        let doc = setting_docs::ActiveModel {
            book_id: Set(book.id),
            kind: Set(kind.to_owned()),
            content: Set(String::new()),
            revision: Set(0),
            approved_revision: Set(None),
            updated_at: Set(timestamp),
            ..Default::default()
        };
        setting_docs::Entity::insert(doc).exec(tx).await?;
    }

    // New books select the skills whose `enabled_by_default = 1` (design §4.1).
    super::skills::insert_defaults_for_book(tx, book.id, timestamp).await?;

    // A book with conception starts `conceiving` and gets its session after this transaction
    // commits; without it, the book starts `active` with empty materials the author writes
    // (spec/08 §5).
    let dto = to_dto(&book)?;
    Ok((
        CreateBookResponse {
            book: dto,
            session_id: None,
            warnings: Vec::new(),
        },
        vec![ServerEvent::BookUpdated {
            book_id: book.id,
            op: BookOp::Create,
        }],
    ))
}

/// `GET /api/books?status=` (spec/10 §2.2). Archived books are hidden unless requested.
pub async fn list(
    conn: &impl ConnectionTrait,
    status: Option<BookStatus>,
) -> Result<Vec<BookSummaryDto>, AppError> {
    let mut query = books::Entity::find().order_by_desc(books::Column::UpdatedAt);
    if let Some(status) = status {
        query = query.filter(books::Column::Status.eq(status.as_str()));
    }
    let rows = query.all(conn).await?;
    let mut summaries = Vec::with_capacity(rows.len());
    for row in rows {
        summaries.push(to_summary(conn, row).await?);
    }
    Ok(summaries)
}

/// `GET /api/books/:id`.
pub async fn get(conn: &impl ConnectionTrait, book_id: i32) -> Result<BookDto, AppError> {
    let row = books::Entity::find_by_id(book_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book #{book_id}")))?;
    to_dto(&row)
}

/// `PATCH /api/books/:id` (title, genre, writing language, settings, model overrides, archive).
pub async fn update(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    book_id: i32,
    patch: BookPatch,
) -> Result<BookDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { update_inner(tx, book_id, patch).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn update_inner(
    tx: &impl ConnectionTrait,
    book_id: i32,
    patch: BookPatch,
) -> Result<(BookDto, Vec<ServerEvent>), AppError> {
    let existing = books::Entity::find_by_id(book_id)
        .one(tx)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book #{book_id}")))?;
    let current_status = super::enum_of::<BookStatus>("books.status", &existing.status)?;

    let mut active = books::ActiveModel {
        id: Set(book_id),
        updated_at: Set(now()),
        ..Default::default()
    };
    if let Some(title) = &patch.title {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::validation("a book needs a title"));
        }
        active.title = Set(title.to_owned());
    }
    if let Some(genre) = &patch.genre {
        active.genre = Set(genre.trim().to_owned());
    }
    if let Some(language) = &patch.writing_language {
        let language = language.trim();
        validate_language(language)?;
        active.writing_language = Set(language.to_owned());
    }
    if let Some(settings) = &patch.settings {
        validate_settings(settings)?;
        active.settings = Set(json(settings)?);
    }
    if let Some(overrides) = &patch.model_overrides {
        active.model_overrides = Set(json(overrides)?);
    }
    if let Some(status) = patch.status {
        // Archiving only hides the book; unarchiving sets it back to active (spec/08 §1.3).
        match status {
            BookStatus::Archived => {
                if current_status != BookStatus::Active {
                    return Err(conflict(
                        ErrorCode::BookInvalidState,
                        "only an active book can be archived",
                        serde_json::json!({ "status": current_status.as_str() }),
                    ));
                }
            }
            BookStatus::Active => {
                if current_status != BookStatus::Archived {
                    return Err(conflict(
                        ErrorCode::BookInvalidState,
                        "only an archived book can be unarchived",
                        serde_json::json!({ "status": current_status.as_str() }),
                    ));
                }
            }
            BookStatus::Conceiving => {
                // `conceiving` is set by the conception studio, never by a patch (spec/08 §5).
                return Err(AppError::validation(
                    "status can only be set to archived or active",
                ));
            }
        }
        active.status = Set(status.as_str().to_owned());
    }
    active.update(tx).await?;

    let row = books::Entity::find_by_id(book_id)
        .one(tx)
        .await?
        .ok_or_else(|| AppError::internal("the book vanished during the update"))?;
    Ok((
        to_dto(&row)?,
        vec![ServerEvent::BookUpdated {
            book_id,
            op: BookOp::Update,
        }],
    ))
}

/// `DELETE /api/books/:id`: cascades to tracks, chapters, setting materials and canon.
pub async fn delete(state: &crate::state::AppState, book_id: i32) -> Result<(), AppError> {
    // spec/08 §1.3: the book's open sessions are stopped, their runners awaited, and the sessions
    // abandoned before the cascade, so no runner writes to deleted rows. This mirrors the track
    // delete (`tracks::delete`), which has the same rule.
    for session_id in super::sessions::open_ids_for_book(state.db().conn(), book_id).await? {
        match super::sessions::abandon(state, session_id).await {
            Ok(_) => {}
            // Another caller finalized it between the query and the call, which is the state this
            // loop wants anyway.
            Err(error) if error.code == ErrorCode::SessionInvalidState => {}
            Err(error) => return Err(error),
        }
    }
    let db = state.db().clone();
    let published = super::write(&db, move |tx| {
        Box::pin(async move { delete_inner(tx, book_id).await })
    })
    .await?;
    state.events().publish_all(published);
    Ok(())
}

pub async fn delete_inner(
    tx: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Vec<ServerEvent>, AppError> {
    let existing = books::Entity::find_by_id(book_id).one(tx).await?;
    if existing.is_none() {
        return Err(AppError::not_found(format!("book #{book_id}")));
    }
    books::Entity::delete_by_id(book_id).exec(tx).await?;
    Ok(vec![ServerEvent::BookUpdated {
        book_id,
        op: BookOp::Delete,
    }])
}

/// `GET /api/books/:id/overview` (spec/08 §7): the book, its next action, everything that needs
/// the author, progress, recent changes and its token usage.
pub async fn overview(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<BookOverviewDto, AppError> {
    let row = books::Entity::find_by_id(book_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book #{book_id}")))?;
    let book = to_dto(&row)?;
    let progress = progress(conn, book_id).await?;
    let next_action = next_action(conn, &row).await?;
    let attention = attention(conn, book_id).await?;
    let recent_changes = super::change_log::recent_for_book(conn, book_id, 10).await?;
    let usage = book_usage(conn, book_id).await?;
    Ok(BookOverviewDto {
        book,
        next_action,
        attention,
        progress,
        recent_changes,
        usage,
    })
}

/// The session statuses that need the author's attention (spec/08 §7 `attention[]`).
const ATTENTION_SESSION_STATUSES: [&str; 6] = [
    "held",
    "awaiting_input",
    "paused",
    "failed",
    "interrupted",
    "stopped",
];

/// The chapter pointer of a chapter row, with its status parsed for the DTO.
fn pointer(chapter: &chapters::Model) -> ChapterPointerDto {
    ChapterPointerDto {
        id: chapter.id,
        number: chapter.ord,
        title: chapter.title.clone(),
        status: ChapterStatus::from_str_opt(&chapter.status).unwrap_or(ChapterStatus::Planned),
    }
}

/// The book's chapters in reading order (track `ord`, then chapter `ord`).
async fn ordered_chapters(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Vec<(tracks::Model, chapters::Model)>, AppError> {
    let track_rows = tracks::Entity::find()
        .filter(tracks::Column::BookId.eq(book_id))
        .order_by_asc(tracks::Column::Ord)
        .all(conn)
        .await?;
    let mut chapter_rows = chapters::Entity::find()
        .filter(chapters::Column::BookId.eq(book_id))
        .all(conn)
        .await?;
    chapter_rows.sort_by_key(|chapter| (chapter.track_id, chapter.ord, chapter.id));
    let mut ordered = Vec::with_capacity(chapter_rows.len());
    for track in track_rows {
        for chapter in chapter_rows.iter().filter(|c| c.track_id == track.id) {
            ordered.push((track.clone(), chapter.clone()));
        }
    }
    Ok(ordered)
}

/// Everything that needs the author, in the order of spec/08 §7: open sessions, chapters in
/// review, canon that is not synced, then the pending-proposal and ready-research counts.
pub async fn attention(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Vec<AttentionItemDto>, AppError> {
    let ordered = ordered_chapters(conn, book_id).await?;
    let mut items = Vec::new();

    let sessions = agent_sessions::Entity::find()
        .filter(agent_sessions::Column::BookId.eq(book_id))
        .filter(agent_sessions::Column::Status.is_in(ATTENTION_SESSION_STATUSES))
        .order_by_asc(agent_sessions::Column::Id)
        .all(conn)
        .await?;
    for session in sessions {
        let chapter = session
            .chapter_id
            .and_then(|id| ordered.iter().find(|(_, chapter)| chapter.id == id))
            .map(|(_, chapter)| pointer(chapter));
        items.push(AttentionItemDto {
            kind: AttentionKind::OpenSession,
            count: 1,
            session_id: Some(session.id),
            session_kind: SessionKind::from_str_opt(&session.kind),
            session_status: SessionStatus::from_str_opt(&session.status),
            chapter,
            track_id: session.track_id,
        });
    }

    for (track, chapter) in &ordered {
        if chapter.status == ChapterStatus::InReview.as_str() {
            items.push(AttentionItemDto {
                kind: AttentionKind::ChapterInReview,
                count: 1,
                session_id: None,
                session_kind: None,
                session_status: None,
                chapter: Some(pointer(chapter)),
                track_id: Some(track.id),
            });
        }
        let canon = CanonStatus::from_str_opt(&chapter.canon_status).unwrap_or(CanonStatus::None);
        if chapter.status == ChapterStatus::Official.as_str()
            && matches!(
                canon,
                CanonStatus::Pending | CanonStatus::Failed | CanonStatus::Stale
            )
        {
            items.push(AttentionItemDto {
                kind: AttentionKind::CanonUnsynced,
                count: 1,
                session_id: None,
                session_kind: None,
                session_status: None,
                chapter: Some(pointer(chapter)),
                track_id: Some(track.id),
            });
        }
    }

    let proposals = setting_proposals::Entity::find()
        .filter(setting_proposals::Column::BookId.eq(book_id))
        .filter(setting_proposals::Column::Status.eq("pending"))
        .count(conn)
        .await?;
    if proposals > 0 {
        items.push(AttentionItemDto {
            kind: AttentionKind::PendingProposals,
            count: proposals as i32,
            session_id: None,
            session_kind: None,
            session_status: None,
            chapter: None,
            track_id: None,
        });
    }

    let research = research_tasks::Entity::find()
        .filter(research_tasks::Column::BookId.eq(book_id))
        .filter(research_tasks::Column::Status.eq(ResearchTaskStatus::Ready.as_str()))
        .count(conn)
        .await?;
    if research > 0 {
        items.push(AttentionItemDto {
            kind: AttentionKind::ReadyResearch,
            count: research as i32,
            session_id: None,
            session_kind: None,
            session_status: None,
            chapter: None,
            track_id: None,
        });
    }

    Ok(items)
}

/// The single next action, first match of spec/08 §7.
pub async fn next_action(
    conn: &impl ConnectionTrait,
    book: &books::Model,
) -> Result<Option<NextActionDto>, AppError> {
    let action = |kind: NextActionKind| NextActionDto {
        kind,
        chapter: None,
        session_id: None,
        track_id: None,
        ready_research: None,
        research_task_id: None,
    };
    // 1. The book is still being conceived.
    if book.status == BookStatus::Conceiving.as_str() {
        return Ok(Some(action(NextActionKind::ContinueConception)));
    }
    let ordered = ordered_chapters(conn, book.id).await?;

    // 2. The canon was never seeded and the materials to seed it exist.
    let seeded = agent_sessions::Entity::find()
        .filter(agent_sessions::Column::BookId.eq(book.id))
        .filter(agent_sessions::Column::Kind.eq(SessionKind::CanonSeed.as_str()))
        .filter(agent_sessions::Column::Status.eq(SessionStatus::Completed.as_str()))
        .count(conn)
        .await?
        > 0;
    if !seeded {
        let designs = character_designs::Entity::find()
            .filter(character_designs::Column::BookId.eq(book.id))
            .count(conn)
            .await?;
        let entities = canon_entities::Entity::find()
            .filter(canon_entities::Column::BookId.eq(book.id))
            .count(conn)
            .await?;
        if designs > 0 && entities == 0 {
            return Ok(Some(action(NextActionKind::SeedCanon)));
        }
    }

    // 3. Confirmed research tasks are waiting to run, and the writer reads what they produce
    //    (PROJECT §8.4 F2 step 6: after finalize the overview suggests "Run research tasks" and
    //    "Write chapter 1"; spec/08 §7 rule 3, D-136). `Ready` only: a `draft` topic is not
    //    confirmed by the author yet and cannot be run (spec/07 §8), so counting it would make the
    //    step a dead end. The count travels with the action so the card can name it.
    let research = research_tasks::Entity::find()
        .filter(research_tasks::Column::BookId.eq(book.id))
        .filter(research_tasks::Column::Status.eq(ResearchTaskStatus::Ready.as_str()))
        .all(conn)
        .await?;
    if let Some(first) = research.first() {
        return Ok(Some(NextActionDto {
            kind: NextActionKind::RunResearch,
            chapter: None,
            session_id: None,
            track_id: None,
            ready_research: Some(research.len() as i32),
            // Only a single task gets a direct target: with several, the author has to choose on
            // the research page, so naming one of them would be an arbitrary pick (D-136).
            research_task_id: if research.len() == 1 {
                Some(first.id)
            } else {
                None
            },
        }));
    }

    // 4. A chapter waits for the author's review.
    if let Some((track, chapter)) = ordered
        .iter()
        .find(|(_, chapter)| chapter.status == ChapterStatus::InReview.as_str())
    {
        return Ok(Some(NextActionDto {
            kind: NextActionKind::ReviewChapter,
            chapter: Some(pointer(chapter)),
            session_id: None,
            track_id: Some(track.id),
            ready_research: None,
            research_task_id: None,
        }));
    }

    // 5. Work in progress: the first chapter with an open outline session or one that is being
    //    written or revised (spec/08 §7 rule 5, D-146). An outline session is work in progress and
    //    is offered as `write_outline`, not as `continue_chapter`.
    for (track, chapter) in &ordered {
        let open = agent_sessions::Entity::find()
            .filter(agent_sessions::Column::ChapterId.eq(chapter.id))
            .filter(agent_sessions::Column::Role.is_in(["planner", "writer"]))
            .filter(agent_sessions::Column::Status.is_not_in([
                SessionStatus::Completed.as_str(),
                SessionStatus::Superseded.as_str(),
                SessionStatus::Abandoned.as_str(),
            ]))
            .order_by_desc(agent_sessions::Column::Id)
            .one(conn)
            .await?;
        let writing = matches!(
            ChapterStatus::from_str_opt(&chapter.status),
            Some(ChapterStatus::Drafting) | Some(ChapterStatus::Revising)
        );
        if open.is_none() && !writing {
            continue;
        }
        if open
            .as_ref()
            .is_some_and(|session| session.kind == SessionKind::ChapterOutline.as_str())
        {
            return Ok(Some(NextActionDto {
                kind: NextActionKind::WriteOutline,
                chapter: Some(pointer(chapter)),
                session_id: open.map(|session| session.id),
                track_id: Some(track.id),
                ready_research: None,
                research_task_id: None,
            }));
        }
        return Ok(Some(NextActionDto {
            kind: NextActionKind::ContinueChapter,
            chapter: Some(pointer(chapter)),
            session_id: open.map(|session| session.id),
            track_id: Some(track.id),
            ready_research: None,
            research_task_id: None,
        }));
    }

    // 6. The latest official main chapter's canon is not synced.
    let main_track = tracks::Entity::find()
        .filter(tracks::Column::BookId.eq(book.id))
        .filter(tracks::Column::Kind.eq(TrackKind::Main.as_str()))
        .one(conn)
        .await?;
    if let Some(main) = &main_track
        && let Some((_, chapter)) = ordered
            .iter()
            .filter(|(track, chapter)| {
                track.id == main.id && chapter.status == ChapterStatus::Official.as_str()
            })
            .filter(|(_, chapter)| {
                matches!(
                    CanonStatus::from_str_opt(&chapter.canon_status),
                    Some(CanonStatus::Pending)
                        | Some(CanonStatus::Failed)
                        | Some(CanonStatus::Stale)
                )
            })
            .max_by_key(|(_, chapter)| chapter.ord)
    {
        return Ok(Some(NextActionDto {
            kind: NextActionKind::SyncCanon,
            chapter: Some(pointer(chapter)),
            session_id: None,
            track_id: Some(main.id),
            ready_research: None,
            research_task_id: None,
        }));
    }

    // 7. The first planned chapter: an empty outline is outlined first, otherwise it is written
    //    (spec/08 §7 rule 7, D-146). This sits after `sync_canon` because writing chapter N+1
    //    before chapter N's canon is synced trips `previous_not_ready` anyway.
    if let Some((track, chapter)) = ordered
        .iter()
        .find(|(_, chapter)| chapter.status == ChapterStatus::Planned.as_str())
    {
        let kind = if chapter.outline.trim().is_empty() {
            NextActionKind::WriteOutline
        } else {
            NextActionKind::WriteChapter
        };
        return Ok(Some(NextActionDto {
            kind,
            chapter: Some(pointer(chapter)),
            session_id: None,
            track_id: Some(track.id),
            ready_research: None,
            research_task_id: None,
        }));
    }

    // 8. Otherwise the main story continues (or the first track, for a book without one).
    let track_id = main_track
        .map(|track| track.id)
        .or_else(|| ordered.first().map(|(track, _)| track.id));
    Ok(Some(NextActionDto {
        kind: NextActionKind::WriteNext,
        chapter: None,
        session_id: None,
        track_id,
        ready_research: None,
        research_task_id: None,
    }))
}

/// Token totals over every session of the book (spec/08 §7 `usage`).
pub async fn book_usage(conn: &impl ConnectionTrait, book_id: i32) -> Result<Usage, AppError> {
    let sessions = agent_sessions::Entity::find()
        .filter(agent_sessions::Column::BookId.eq(book_id))
        .all(conn)
        .await?;
    let mut total = Usage::default();
    for session in sessions {
        let usage: Usage = crate::db::json::from_json_or_default(&session.usage);
        total.requests += usage.requests;
        total.input_tokens += usage.input_tokens;
        total.cached_input_tokens += usage.cached_input_tokens;
        total.cache_write_tokens += usage.cache_write_tokens;
        total.output_tokens += usage.output_tokens;
        total.reasoning_tokens += usage.reasoning_tokens;
        total.estimated |= usage.estimated;
    }
    total.last_prompt_tokens =
        total.input_tokens + total.cached_input_tokens + total.cache_write_tokens;
    Ok(total)
}

/// Chapter counts and words per track (spec/08 §7 `progress`).
pub async fn progress(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<BookProgressDto, AppError> {
    let track_rows = tracks::Entity::find()
        .filter(tracks::Column::BookId.eq(book_id))
        .order_by_asc(tracks::Column::Ord)
        .all(conn)
        .await?;
    let chapter_rows = chapters::Entity::find()
        .filter(chapters::Column::BookId.eq(book_id))
        .all(conn)
        .await?;

    let mut per_track: BTreeMap<i32, Vec<&chapters::Model>> = BTreeMap::new();
    for chapter in &chapter_rows {
        per_track.entry(chapter.track_id).or_default().push(chapter);
    }

    let mut tracks_progress = Vec::with_capacity(track_rows.len());
    let mut total_chapters = 0i32;
    let mut official_chapters = 0i32;
    let mut total_words = 0i32;
    let mut official_words = 0i32;
    for track in &track_rows {
        let rows = per_track.get(&track.id).cloned().unwrap_or_default();
        let mut by_status: Vec<StatusCountDto> = Vec::new();
        for status in [
            ChapterStatus::Planned,
            ChapterStatus::Drafting,
            ChapterStatus::InReview,
            ChapterStatus::Revising,
            ChapterStatus::Official,
        ] {
            let count = rows
                .iter()
                .filter(|chapter| chapter.status == status.as_str())
                .count() as i32;
            by_status.push(StatusCountDto { status, count });
            if status == ChapterStatus::Official {
                official_chapters += count;
            }
        }
        let words: i32 = rows.iter().map(|chapter| chapter.word_count).sum();
        let official: i32 = rows
            .iter()
            .filter(|chapter| chapter.status == ChapterStatus::Official.as_str())
            .map(|chapter| chapter.word_count)
            .sum();
        total_chapters += rows.len() as i32;
        total_words += words;
        official_words += official;
        tracks_progress.push(TrackProgressDto {
            track_id: track.id,
            kind: super::enum_of("tracks.kind", &track.kind)?,
            title: track.title.clone(),
            chapters: rows.len() as i32,
            words,
            by_status,
        });
    }
    Ok(BookProgressDto {
        total_chapters,
        official_chapters,
        total_words,
        official_words,
        tracks: tracks_progress,
    })
}

/// Builds one library card.
async fn to_summary(
    conn: &impl ConnectionTrait,
    row: books::Model,
) -> Result<BookSummaryDto, AppError> {
    let chapter_rows = chapters::Entity::find()
        .filter(chapters::Column::BookId.eq(row.id))
        .all(conn)
        .await?;
    let main_track = tracks::Entity::find()
        .filter(tracks::Column::BookId.eq(row.id))
        .filter(tracks::Column::Kind.eq(TrackKind::Main.as_str()))
        .one(conn)
        .await?;
    let chapter_count = chapter_rows.len() as i32;
    let official_chapter_count = chapter_rows
        .iter()
        .filter(|chapter| chapter.status == ChapterStatus::Official.as_str())
        .count() as i32;
    let word_count: i32 = chapter_rows.iter().map(|chapter| chapter.word_count).sum();
    let latest_chapter = main_track.and_then(|track| {
        chapter_rows
            .iter()
            .filter(|chapter| chapter.track_id == track.id)
            .max_by_key(|chapter| chapter.ord)
            .map(|chapter| ChapterPointerDto {
                id: chapter.id,
                number: chapter.ord,
                title: chapter.title.clone(),
                status: ChapterStatus::from_str_opt(&chapter.status)
                    .unwrap_or(ChapterStatus::Planned),
            })
    });
    // The card's badge counts everything the overview's attention list would show (spec/08 §7,
    // spec/10 §2.2).
    let attention_count: i32 = attention(conn, row.id)
        .await?
        .iter()
        .map(|item| item.count)
        .sum();
    Ok(BookSummaryDto {
        id: row.id,
        title: row.title,
        genre: row.genre,
        writing_language: row.writing_language,
        status: super::enum_of("books.status", &row.status)?,
        chapter_count,
        official_chapter_count,
        word_count,
        latest_chapter,
        attention_count,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Maps a book row to its DTO.
pub fn to_dto(row: &books::Model) -> Result<BookDto, AppError> {
    Ok(BookDto {
        id: row.id,
        title: row.title.clone(),
        idea: row.idea.clone(),
        genre: row.genre.clone(),
        writing_language: row.writing_language.clone(),
        status: super::enum_of("books.status", &row.status)?,
        settings: parse_json("books.settings", &row.settings)?,
        model_overrides: parse_json("books.model_overrides", &row.model_overrides)?,
        conception: parse_json("books.conception", &row.conception)?,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Rejects settings the writer service could not honor.
pub fn validate_settings(settings: &BookSettings) -> Result<(), AppError> {
    if settings.target_words_min <= 0 {
        return Err(AppError::validation("target_words_min must be positive"));
    }
    if settings.target_words_max < settings.target_words_min {
        return Err(AppError::validation(
            "target_words_max must be at least target_words_min",
        ));
    }
    if settings.recent_summary_count < 0
        || settings.midterm_window < 0
        || settings.recent_chapter_count < 0
    {
        return Err(AppError::validation(
            "memory windows and recent_chapter_count must not be negative",
        ));
    }
    Ok(())
}

/// Validates a BCP-47 writing language tag: a 2–3 letter primary subtag and optional subtags.
pub fn validate_language(language: &str) -> Result<(), AppError> {
    let valid = !language.is_empty()
        && language.len() <= 35
        && language.split('-').all(|part| {
            !part.is_empty() && part.len() <= 8 && part.chars().all(|c| c.is_ascii_alphanumeric())
        })
        && language
            .split('-')
            .next()
            .is_some_and(|primary| (2..=3).contains(&primary.len()));
    if valid {
        Ok(())
    } else {
        Err(field_error(
            "writing_language",
            format!("writing_language must be a BCP-47 tag such as zh-CN or en, got {language:?}"),
        ))
    }
}

/// The three setting documents of a book, in the spec's order.
pub async fn setting_docs_of(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Vec<setting_docs::Model>, AppError> {
    let mut rows = setting_docs::Entity::find()
        .filter(setting_docs::Column::BookId.eq(book_id))
        .all(conn)
        .await?;
    rows.sort_by_key(|row| {
        SETTING_DOC_KINDS
            .iter()
            .position(|kind| *kind == row.kind)
            .unwrap_or(usize::MAX)
    });
    Ok(rows)
}

/// Updates a book row's `updated_at` in the caller's transaction.
pub async fn touch(
    tx: &impl ConnectionTrait,
    book_id: i32,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> Result<(), AppError> {
    use sea_orm::ActiveModelTrait;
    let mut active = books::ActiveModel {
        id: Set(book_id),
        ..Default::default()
    };
    active.updated_at = Set(timestamp);
    active.update(tx).await?;
    Ok(())
}

/// The canonical status of a chapter, for services that only carry the string column.
pub fn canon_status_of(raw: &str) -> CanonStatus {
    CanonStatus::from_str_opt(raw).unwrap_or(CanonStatus::None)
}

/// Loads the main track of a book.
pub async fn main_track(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<tracks::Model, AppError> {
    tracks::Entity::find()
        .filter(tracks::Column::BookId.eq(book_id))
        .filter(tracks::Column::Kind.eq(TrackKind::Main.as_str()))
        .one(conn)
        .await?
        .ok_or_else(|| AppError::internal(format!("book #{book_id} has no main track")))
}

/// Selects the highest `ord` of a track's chapters (0 when it has none).
pub async fn last_ord(conn: &impl ConnectionTrait, track_id: i32) -> Result<i32, AppError> {
    let row = chapters::Entity::find()
        .filter(chapters::Column::TrackId.eq(track_id))
        .order_by_desc(chapters::Column::Ord)
        .select_only()
        .column(chapters::Column::Ord)
        .into_tuple::<i32>()
        .one(conn)
        .await?;
    Ok(row.unwrap_or(0))
}
