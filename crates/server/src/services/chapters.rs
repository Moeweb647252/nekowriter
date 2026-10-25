//! Chapter service: CRUD, reorder, content writes (PATCH/PUT with `base_version`), submit,
//! approve, versions and restore (spec/08 §1.1, §1.2, spec/10 §2.2, spec/02 §5 invariants 3–4).

use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use std::collections::HashSet;

use crate::api::dto::chapters::ChapterWriteInput;
use crate::api::dto::chapters::ChapterWriteResponse;
use crate::api::dto::chapters::{
    ApproveChapterResponse, ChapterContentPatch, ChapterContentResult, ChapterContentTextPut,
    ChapterDto, ChapterInput, ChapterListItemDto, ChapterPatch, ChapterReorderInput,
    ChapterVersionMetaDto, ManualVersionInput,
};
use crate::db::entities::{agent_sessions, appearances, chapter_versions, chapters, tracks};
use crate::db::json::{
    AgentRole, CanonStatus, ChangeActor, ChangeOp, ChangeTargetType, ChapterContent, ChapterStatus,
    SessionStatus, Snapshot, TrackKind, VersionKind,
};
use crate::error::{AppError, ErrorCode};
use crate::events::{BookOp, ContentActor, ServerEvent};
use crate::text;

use super::{json, now, parse_json};

/// `GET /api/tracks/:id/chapters` (spec/10 §2.2: no content).
pub async fn list(
    conn: &impl ConnectionTrait,
    track_id: i32,
) -> Result<Vec<ChapterListItemDto>, AppError> {
    super::tracks::load(conn, track_id).await?;
    let rows = chapters::Entity::find()
        .filter(chapters::Column::TrackId.eq(track_id))
        .order_by_asc(chapters::Column::Ord)
        .all(conn)
        .await?;
    Ok(rows.into_iter().map(ChapterListItemDto::from).collect())
}

/// `GET /api/chapters/:id`.
pub async fn get(conn: &impl ConnectionTrait, chapter_id: i32) -> Result<ChapterDto, AppError> {
    let row = load(conn, chapter_id).await?;
    to_dto(conn, &row).await
}

/// `POST /api/tracks/:id/chapters` (spec/08 §1.1): `planned`, `ord` = last + 1, empty content.
pub async fn create(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    track_id: i32,
    input: ChapterInput,
) -> Result<ChapterDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { create_inner(tx, track_id, input).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn create_inner(
    tx: &impl ConnectionTrait,
    track_id: i32,
    input: ChapterInput,
) -> Result<(ChapterDto, Vec<ServerEvent>), AppError> {
    let track = super::tracks::load(tx, track_id).await?;
    let ord = next_ord(tx, track_id).await?;
    let content = ChapterContent::default();
    let timestamp = now();
    let model = chapters::ActiveModel {
        book_id: Set(track.book_id),
        track_id: Set(track_id),
        ord: Set(ord),
        title: Set(input.title.trim().to_owned()),
        status: Set(ChapterStatus::Planned.as_str().to_owned()),
        guidance: Set(input.guidance.trim().to_owned()),
        outline: Set(String::new()),
        reference_notes: Set(String::new()),
        summary: Set(String::new()),
        content: Set(json(&content)?),
        content_version: Set(0),
        word_count: Set(0),
        official_version_id: Set(None),
        canon_status: Set(CanonStatus::None.as_str().to_owned()),
        synced_version_id: Set(None),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    let row = chapters::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    super::books::touch(tx, track.book_id, timestamp).await?;
    let dto = to_dto(tx, &row).await?;
    Ok((
        dto.clone(),
        vec![
            ServerEvent::ChapterUpdated {
                chapter: dto.list_item.clone(),
                outline_changed: false,
            },
            ServerEvent::BookUpdated {
                book_id: track.book_id,
                op: BookOp::Update,
            },
        ],
    ))
}

/// `PATCH /api/chapters/:id` (title, guidance, outline, reference notes, summary).
pub async fn update(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    chapter_id: i32,
    patch: ChapterPatch,
) -> Result<ChapterDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { update_inner(tx, chapter_id, patch).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn update_inner(
    tx: &impl ConnectionTrait,
    chapter_id: i32,
    patch: ChapterPatch,
) -> Result<(ChapterDto, Vec<ServerEvent>), AppError> {
    let existing = load(tx, chapter_id).await?;
    // The writer drafts from the outline and the reference notes; while its session runs the
    // author may not change them (spec/08 §1.4).
    let edits_agent_inputs = patch.outline.is_some() || patch.reference_notes.is_some();
    if edits_agent_inputs
        && let Some(session_id) = super::running_writer_session(tx, chapter_id).await?
    {
        return Err(super::conflict(
            ErrorCode::ChapterLocked,
            "the writer is working on this chapter",
            serde_json::json!({ "session_id": session_id }),
        ));
    }
    let outline_changed = patch
        .outline
        .as_ref()
        .is_some_and(|outline| *outline != existing.outline);

    let mut active = chapters::ActiveModel {
        id: Set(chapter_id),
        updated_at: Set(now()),
        ..Default::default()
    };
    if let Some(title) = &patch.title {
        active.title = Set(title.trim().to_owned());
    }
    if let Some(guidance) = &patch.guidance {
        active.guidance = Set(guidance.trim().to_owned());
    }
    if let Some(outline) = &patch.outline {
        active.outline = Set(outline.clone());
    }
    if let Some(notes) = &patch.reference_notes {
        active.reference_notes = Set(notes.clone());
    }
    if let Some(summary) = &patch.summary {
        active.summary = Set(summary.clone());
    }
    sea_orm::ActiveModelTrait::update(active, tx).await?;
    if patch.title.is_some() || patch.summary.is_some() {
        // A title or summary change updates the chapter's index rows in this transaction
        // (spec/07 §2); a non-official chapter has none and the helper is a no-op.
        crate::search::maintenance::sync_chapter(tx, chapter_id).await?;
        crate::search::index::wake();
    }

    let row = load(tx, chapter_id).await?;
    let dto = to_dto(tx, &row).await?;
    Ok((
        dto.clone(),
        vec![ServerEvent::ChapterUpdated {
            chapter: dto.list_item,
            outline_changed,
        }],
    ))
}

/// `DELETE /api/chapters/:id` (spec/08 §1.1): refuses when an anchor or a session is open, then
/// renumbers the rest of the track.
pub async fn delete(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    chapter_id: i32,
) -> Result<(), AppError> {
    let published = super::write(db, move |tx| {
        Box::pin(async move { delete_inner(tx, chapter_id).await })
    })
    .await?;
    events.publish_all(published);
    Ok(())
}

pub async fn delete_inner(
    tx: &impl ConnectionTrait,
    chapter_id: i32,
) -> Result<Vec<ServerEvent>, AppError> {
    use sea_orm::ActiveModelTrait;

    let existing = load(tx, chapter_id).await?;
    if let Some(session_id) = super::open_chapter_session(tx, chapter_id).await? {
        return Err(super::conflict(
            ErrorCode::ChapterSessionExists,
            "the chapter has an open session",
            serde_json::json!({ "session_id": session_id }),
        ));
    }
    let anchor_track_ids = anchor_tracks(tx, chapter_id).await?;
    if !anchor_track_ids.is_empty() {
        return Err(super::conflict(
            ErrorCode::ChapterIsAnchor,
            "the chapter anchors a side track",
            serde_json::json!({ "track_ids": anchor_track_ids }),
        ));
    }

    // The appearance rows cascade with the chapter; the entities that appeared in it lose a
    // bound, so their `first_chapter_id` / `last_chapter_id` are recomputed (spec/08 §1.1,
    // D-059). The entity ids are read before the cascade removes the rows.
    let appeared: Vec<i32> = appearances::Entity::find()
        .filter(appearances::Column::ChapterId.eq(chapter_id))
        .all(tx)
        .await?
        .into_iter()
        .map(|row| row.entity_id)
        .collect();
    chapters::Entity::delete_by_id(chapter_id).exec(tx).await?;
    super::canon::entity::recompute_bounds(tx, existing.book_id, &appeared)
        .await
        .map_err(AppError::from)?;
    renumber_subsequent(tx, existing.track_id, existing.ord).await?;

    // Repair the chapter's own track and book timestamps.
    let timestamp = now();
    let track_active = tracks::ActiveModel {
        id: Set(existing.track_id),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    track_active.update(tx).await?;

    Ok(vec![
        ServerEvent::BookUpdated {
            book_id: existing.book_id,
            op: BookOp::Update,
        },
        ServerEvent::ChapterUpdated {
            chapter: ChapterListItemDto {
                id: existing.id,
                book_id: existing.book_id,
                track_id: existing.track_id,
                number: existing.ord,
                title: existing.title.clone(),
                status: ChapterStatus::from_str_opt(&existing.status)
                    .unwrap_or(ChapterStatus::Planned),
                canon_status: CanonStatus::from_str_opt(&existing.canon_status)
                    .unwrap_or(CanonStatus::None),
                word_count: existing.word_count,
                updated_at: timestamp,
            },
            // The UI invalidates the whole list on any chapter event.
            outline_changed: false,
        },
    ])
}

/// `POST /api/tracks/:id/chapters/reorder` (spec/08 §1.1): official chapters stay put and no
/// non-official chapter may end up before an official one.
pub async fn reorder(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    track_id: i32,
    input: ChapterReorderInput,
) -> Result<Vec<ChapterListItemDto>, AppError> {
    let (list, published) = super::write(db, move |tx| {
        Box::pin(async move { reorder_inner(tx, track_id, input).await })
    })
    .await?;
    events.publish_all(published);
    Ok(list)
}

pub async fn reorder_inner(
    tx: &impl ConnectionTrait,
    track_id: i32,
    input: ChapterReorderInput,
) -> Result<(Vec<ChapterListItemDto>, Vec<ServerEvent>), AppError> {
    use sea_orm::ActiveModelTrait;

    let track = super::tracks::load(tx, track_id).await?;
    let rows = ordered_chapters(tx, track_id).await?;
    let existing_ids: Vec<i32> = rows.iter().map(|row| row.id).collect();
    let mut seen = HashSet::new();
    for id in &input.ids {
        if !seen.insert(*id) {
            return Err(AppError::validation("the reorder list repeats a chapter"));
        }
    }
    if seen.len() != existing_ids.len() || !existing_ids.iter().all(|id| seen.contains(id)) {
        return Err(AppError::validation(
            "the reorder list must contain every chapter of the track exactly once",
        ));
    }

    let by_id: std::collections::HashMap<i32, chapters::Model> =
        rows.into_iter().map(|row| (row.id, row)).collect();
    let mut remaining_official_positions: Vec<usize> = existing_ids
        .iter()
        .enumerate()
        .filter(|(_, id)| {
            by_id
                .get(id)
                .is_some_and(|row| row.status == ChapterStatus::Official.as_str())
        })
        .map(|(index, _)| index)
        .collect();

    // Official chapters may not move: their new positions must be exactly the old ones.
    let new_official_positions: Vec<usize> = input
        .ids
        .iter()
        .enumerate()
        .filter(|(_, id)| {
            by_id
                .get(id)
                .is_some_and(|row| row.status == ChapterStatus::Official.as_str())
        })
        .map(|(index, _)| index)
        .collect();
    if remaining_official_positions != new_official_positions {
        // `chapter.invalid_state` documents `details.status` (spec/10 §4); the ids of the
        // chapters that may not move are the actionable part, so send both.
        let status = if by_id
            .values()
            .any(|row| row.status == ChapterStatus::Official.as_str())
        {
            ChapterStatus::Official
        } else {
            ChapterStatus::Planned
        };
        return Err(super::conflict(
            ErrorCode::ChapterInvalidState,
            "official chapters may not move, and no chapter may move before an official one",
            serde_json::json!({
                "status": status.as_str(),
                "official_chapter_ids": input
                    .ids
                    .iter()
                    .filter(|id| by_id.get(id).is_some_and(|row| {
                        row.status == ChapterStatus::Official.as_str()
                    }))
                    .collect::<Vec<_>>()
            }),
        ));
    }
    remaining_official_positions.clear();

    // Two-phase renumber (spec/02 §1): first push every ord negative, then write the target.
    for row in by_id.values() {
        let active = chapters::ActiveModel {
            id: Set(row.id),
            ord: Set(-row.ord),
            ..Default::default()
        };
        active.update(tx).await?;
    }
    let timestamp = now();
    for (index, id) in input.ids.iter().enumerate() {
        let ord = index as i32 + 1;
        let active = chapters::ActiveModel {
            id: Set(*id),
            ord: Set(ord),
            updated_at: Set(timestamp),
            ..Default::default()
        };
        active.update(tx).await?;
    }

    let list = self::list_chapters(tx, track_id).await?;
    let events = list
        .iter()
        .map(|chapter| ServerEvent::ChapterUpdated {
            chapter: chapter.clone(),
            outline_changed: false,
        })
        .chain(std::iter::once(ServerEvent::BookUpdated {
            book_id: track.book_id,
            op: BookOp::Update,
        }))
        .collect();
    Ok((list, events))
}

/// `PATCH /api/chapters/:id/content` (spec/08 §1.1, spec/10 §2.2).
pub async fn patch_content(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    chapter_id: i32,
    patch: ChapterContentPatch,
) -> Result<ChapterContentResult, AppError> {
    let (result, published) = super::write(db, move |tx| {
        Box::pin(async move { patch_content_inner(tx, chapter_id, patch).await })
    })
    .await?;
    events.publish_all(published);
    Ok(result)
}

pub async fn patch_content_inner(
    tx: &impl ConnectionTrait,
    chapter_id: i32,
    patch: ChapterContentPatch,
) -> Result<(ChapterContentResult, Vec<ServerEvent>), AppError> {
    let existing = load(tx, chapter_id).await?;
    ensure_editable(tx, chapter_id).await?;
    ensure_version(&existing, patch.base_version)?;
    let mut content: ChapterContent = parse_json("chapters.content", &existing.content)?;
    let applied = text::apply_inputs(&mut content, &patch.ops).map_err(map_text_error)?;
    write_content(
        tx,
        &existing,
        content,
        applied.ops,
        applied.word_count,
        ContentActor::User,
    )
    .await
}

/// `PUT /api/chapters/:id/content/text` (spec/10 §2.2): full-text save with pid alignment.
pub async fn put_content_text(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    chapter_id: i32,
    put: ChapterContentTextPut,
) -> Result<ChapterContentResult, AppError> {
    let (result, published) = super::write(db, move |tx| {
        Box::pin(async move { put_content_text_inner(tx, chapter_id, put).await })
    })
    .await?;
    events.publish_all(published);
    Ok(result)
}

pub async fn put_content_text_inner(
    tx: &impl ConnectionTrait,
    chapter_id: i32,
    put: ChapterContentTextPut,
) -> Result<(ChapterContentResult, Vec<ServerEvent>), AppError> {
    let existing = load(tx, chapter_id).await?;
    ensure_editable(tx, chapter_id).await?;
    ensure_version(&existing, put.base_version)?;
    let mut content: ChapterContent = parse_json("chapters.content", &existing.content)?;
    let aligned = text::set_full_text(&mut content, &put.text);
    write_content(
        tx,
        &existing,
        content,
        aligned.ops,
        aligned.word_count,
        ContentActor::User,
    )
    .await
}

/// Persists a new content state: version + 1, word count, canon staleness and the SSE event.
///
/// A save that changes nothing is not a change: it must not bump `content_version` (that would
/// invalidate every other tab's `base_version` and emit a pointless `chapter.content`), must not
/// mark synced canon stale, and must not clear `tail_open`. The response echoes the unchanged
/// state so the caller's optimistic-concurrency loop stays correct.
async fn write_content(
    tx: &impl ConnectionTrait,
    existing: &chapters::Model,
    content: ChapterContent,
    ops: Vec<crate::api::dto::ContentOp>,
    word_count: i32,
    actor: ContentActor,
) -> Result<(ChapterContentResult, Vec<ServerEvent>), AppError> {
    use sea_orm::ActiveModelTrait;

    let stored: ChapterContent = parse_json("chapters.content", &existing.content)?;
    let unchanged = ops.is_empty() && stored == content;
    if unchanged {
        return Ok((
            ChapterContentResult {
                version: existing.content_version,
                ops: Vec::new(),
                word_count: existing.word_count,
            },
            Vec::new(),
        ));
    }

    let version = existing.content_version + 1;
    let timestamp = now();
    let mut active = chapters::ActiveModel {
        id: Set(existing.id),
        content: Set(json(&content)?),
        content_version: Set(version),
        word_count: Set(word_count),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    // An author edit of an official chapter whose canon is synced makes the canon stale
    // (spec/08 §1.1 "Edit text").
    let mut canon_status =
        CanonStatus::from_str_opt(&existing.canon_status).unwrap_or(CanonStatus::None);
    if actor == ContentActor::User
        && existing.status == ChapterStatus::Official.as_str()
        && canon_status == CanonStatus::Synced
    {
        canon_status = CanonStatus::Stale;
        active.canon_status = Set(canon_status.as_str().to_owned());
    }
    // `planned ── manual edit ──► drafting` (lifecycle diagram of spec/08 §1): a planned chapter
    // enters drafting as soon as the author writes text into it by hand. An agent's write starts
    // drafting through the write endpoint of M4 instead.
    if actor == ContentActor::User
        && status_of(existing) == ChapterStatus::Planned
        && !content.paragraphs.is_empty()
    {
        active.status = Set(ChapterStatus::Drafting.as_str().to_owned());
    }
    active.update(tx).await?;
    if existing.status == ChapterStatus::Official.as_str() {
        // The official text changed: its passage documents are rebuilt in this transaction, and
        // unchanged passages keep their vectors (spec/07 §2, D-039).
        crate::search::maintenance::sync_passages(tx, existing.id).await?;
        crate::search::index::wake();
    }
    let row = load(tx, existing.id).await?;
    let list_item = ChapterListItemDto::from(row);
    Ok((
        ChapterContentResult {
            version,
            ops: ops.clone(),
            word_count,
        },
        vec![
            ServerEvent::ChapterContentChanged {
                chapter_id: existing.id,
                version,
                ops,
                word_count,
                tail_open: content.tail_open,
                actor,
            },
            ServerEvent::ChapterUpdated {
                chapter: list_item,
                outline_changed: false,
            },
        ],
    ))
}

/// `POST /api/chapters/:id/write` (spec/08 §1.1): start a `chapter_write` session in planning.
///
/// Preconditions: the chapter is `planned` or `drafting`, it has no open writer session, and the
/// previous chapter of the track is `official` with `canon_status = synced` — unless `force`, which
/// the Write dialog sends after explaining why the check failed. The side-track anchor chapter's
/// readiness (the first chapter of a side story) arrives with M10, which adds the anchor rule to
/// the same check.
pub async fn write(
    state: &crate::state::AppState,
    chapter_id: i32,
    input: ChapterWriteInput,
) -> Result<ChapterWriteResponse, AppError> {
    let chapter = load(state.db().conn(), chapter_id).await?;
    let status = status_of(&chapter);
    if !matches!(status, ChapterStatus::Planned | ChapterStatus::Drafting) {
        return Err(invalid_state(
            "only a planned or drafting chapter can be written by the agent",
            status,
        ));
    }
    if let Some(session_id) = super::open_writer_session(state.db().conn(), chapter_id).await? {
        return Err(super::conflict(
            ErrorCode::ChapterSessionExists,
            "the chapter already has an open writer session",
            serde_json::json!({ "session_id": session_id }),
        ));
    }
    // The previous chapter must be official and canonically synced before the next one is written,
    // unless the author explicitly forces it (spec/08 §1.1).
    if !input.force
        && let Some(reason) = previous_chapter_blocker(state.db().conn(), &chapter).await?
    {
        return Err(super::conflict(
            ErrorCode::ChapterPreviousNotReady,
            "the previous chapter is not ready",
            reason,
        ));
    }
    // The guidance travels into the creation transaction, so the kickoff reads it and a request
    // that fails stores nothing (spec/08 §1.1).
    let (session_id, warnings) = super::sessions::create(
        state,
        super::sessions::CreateSession {
            kind: crate::db::json::SessionKind::ChapterWrite,
            book_id: Some(chapter.book_id),
            chapter_id: Some(chapter_id),
            research_task_id: None,
            granularity: input.granularity,
            auto_pause: input.auto_pause,
            pause_after_outline: input.pause_after_outline,
            redraft: false,
            guidance: input.guidance,
            kickoff_extra: None,
            pending_comment: None,
        },
    )
    .await?;
    Ok(ChapterWriteResponse {
        session_id,
        warnings,
    })
}

/// Why the previous chapter blocks a write, as the `details` of `chapter.previous_not_ready`
/// (spec/08 §1.1). `None` means the chapter may be written.
async fn previous_chapter_blocker(
    conn: &impl ConnectionTrait,
    chapter: &chapters::Model,
) -> Result<Option<serde_json::Value>, AppError> {
    let previous = chapters::Entity::find()
        .filter(chapters::Column::TrackId.eq(chapter.track_id))
        .filter(chapters::Column::Ord.lt(chapter.ord))
        .order_by_desc(chapters::Column::Ord)
        .one(conn)
        .await?;
    let Some(previous) = previous else {
        // The first chapter of the track has no predecessor.
        return Ok(None);
    };
    let official = previous.status == ChapterStatus::Official.as_str();
    let synced = previous.canon_status == crate::db::json::CanonStatus::Synced.as_str();
    if official && synced {
        return Ok(None);
    }
    Ok(Some(serde_json::json!({
        // spec/10 §3: `details.chapter_id` and `details.reason`. The reason is a code the Write
        // dialog words itself, so it can explain *why* and still offer "Write anyway".
        "chapter_id": previous.id,
        "chapter_number": previous.ord,
        "reason": if official { "canon_not_synced" } else { "not_official" },
        "status": previous.status,
        "canon_status": previous.canon_status,
    })))
}

/// `POST /api/chapters/:id/submit` (spec/08 §1.1): version `draft`/`revision`, status `in_review`.
pub async fn submit(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    chapter_id: i32,
) -> Result<ChapterDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { submit_inner(tx, chapter_id).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn submit_inner(
    tx: &impl ConnectionTrait,
    chapter_id: i32,
) -> Result<(ChapterDto, Vec<ServerEvent>), AppError> {
    use sea_orm::ActiveModelTrait;

    let existing = load(tx, chapter_id).await?;
    let status = status_of(&existing);
    if !matches!(status, ChapterStatus::Drafting | ChapterStatus::Revising) {
        return Err(invalid_state(
            "only a drafting or revising chapter can be submitted",
            status,
        ));
    }
    // An open, non-running writer session is abandoned by the UI first; a running one blocks.
    if let Some(session_id) = super::running_writer_session(tx, chapter_id).await? {
        return Err(super::conflict(
            ErrorCode::ChapterSessionExists,
            "the writer is running; stop it before submitting",
            serde_json::json!({ "session_id": session_id }),
        ));
    }
    let content: ChapterContent = parse_json("chapters.content", &existing.content)?;
    if content.paragraphs.is_empty() {
        return Err(invalid_state("the chapter is empty", status));
    }
    let kind = if status == ChapterStatus::Revising {
        VersionKind::Revision
    } else {
        VersionKind::Draft
    };
    let word_count = text::content_word_count(&content);
    super::versions::create_version(
        tx,
        super::versions::NewVersion {
            chapter_id,
            kind,
            title: &existing.title,
            content: &content,
            word_count,
            note: "",
            session_id: None,
        },
    )
    .await?;

    let timestamp = now();
    let active = chapters::ActiveModel {
        id: Set(chapter_id),
        status: Set(ChapterStatus::InReview.as_str().to_owned()),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    active.update(tx).await?;
    let row = load(tx, chapter_id).await?;
    let dto = to_dto(tx, &row).await?;
    Ok((
        dto.clone(),
        vec![ServerEvent::ChapterUpdated {
            chapter: dto.list_item,
            outline_changed: false,
        }],
    ))
}

/// `POST /api/chapters/:id/approve` (spec/08 §1.1): an `official` version, `official_version_id`,
/// status `official`, canon `pending`.
pub async fn approve(
    state: &crate::state::AppState,
    chapter_id: i32,
) -> Result<ApproveChapterResponse, AppError> {
    // An open reviewer session's verdict is moot once the chapter is approved: its report would
    // be for the text that just became official (spec/08 §1.1).
    for reviewer in super::reviews::open_reviewer_sessions(state.db().conn(), chapter_id).await? {
        super::sessions::abandon(state, reviewer.id).await?;
    }
    let state_owned = state.clone();
    let (response, published) = super::write(state.db(), move |tx| {
        Box::pin(async move { approve_inner(&state_owned, tx, chapter_id).await })
    })
    .await?;
    state.events().publish_all(published);
    if let Some(session_id) = response.maintenance_session_id {
        state.manager().spawn(state.clone(), session_id);
    }
    Ok(response)
}

pub async fn approve_inner(
    state: &crate::state::AppState,
    tx: &sea_orm::DatabaseTransaction,
    chapter_id: i32,
) -> Result<(ApproveChapterResponse, Vec<ServerEvent>), AppError> {
    use sea_orm::ActiveModelTrait;

    let existing = load(tx, chapter_id).await?;
    let status = status_of(&existing);
    if status != ChapterStatus::InReview {
        return Err(invalid_state(
            "only a chapter in review can be approved",
            status,
        ));
    }
    if let Some(session_id) = super::open_writer_session(tx, chapter_id).await? {
        return Err(super::conflict(
            ErrorCode::ChapterSessionExists,
            "the chapter has an open writer session",
            serde_json::json!({ "session_id": session_id }),
        ));
    }
    let content: ChapterContent = parse_json("chapters.content", &existing.content)?;
    if content.paragraphs.is_empty() {
        return Err(invalid_state("the chapter is empty", status));
    }
    let word_count = text::content_word_count(&content);
    let version = super::versions::create_version(
        tx,
        super::versions::NewVersion {
            chapter_id,
            kind: VersionKind::Official,
            title: &existing.title,
            content: &content,
            word_count,
            note: "",
            session_id: None,
        },
    )
    .await?;

    let timestamp = now();
    let active = chapters::ActiveModel {
        id: Set(chapter_id),
        status: Set(ChapterStatus::Official.as_str().to_owned()),
        canon_status: Set(CanonStatus::Pending.as_str().to_owned()),
        official_version_id: Set(Some(version.id)),
        synced_version_id: Set(None),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    active.update(tx).await?;
    // The chapter just became official: index its summary and passages in this transaction
    // (spec/07 §2). The archivist fills the summary later; that recomputes the summary document.
    crate::search::maintenance::sync_chapter(tx, chapter_id).await?;
    crate::search::index::wake();

    let row = load(tx, chapter_id).await?;
    let dto = to_dto(tx, &row).await?;
    // `auto_maintenance` starts the archivist in the approval transaction (spec/08 §1.1): the
    // official version, the canon status and the session either all commit or none does.
    let mut warnings = Vec::new();
    let mut events = vec![
        ServerEvent::ChapterUpdated {
            chapter: dto.list_item.clone(),
            outline_changed: false,
        },
        ServerEvent::BookUpdated {
            book_id: existing.book_id,
            op: BookOp::Update,
        },
    ];
    let mut maintenance_session_id = None;
    if super::maintenance::auto_maintenance_enabled(tx, existing.book_id).await? {
        let (session_id, session_events, session_warnings) =
            super::maintenance::start_after_approval(state, tx, &existing).await?;
        maintenance_session_id = session_id;
        events.extend(session_events);
        warnings.extend(session_warnings);
    }
    warnings.sort();
    warnings.dedup();
    Ok((
        ApproveChapterResponse {
            chapter: dto.clone(),
            maintenance_session_id,
            warnings,
        },
        events,
    ))
}

/// `POST /api/chapters/:id/versions` — the author's manual snapshot.
pub async fn save_version(
    db: &crate::db::Db,
    chapter_id: i32,
    input: ManualVersionInput,
) -> Result<ChapterVersionMetaDto, AppError> {
    let row = super::write(db, move |tx| {
        Box::pin(async move { save_version_inner(tx, chapter_id, input).await })
    })
    .await?;
    super::versions::to_meta(&row)
}

pub async fn save_version_inner(
    tx: &impl ConnectionTrait,
    chapter_id: i32,
    input: ManualVersionInput,
) -> Result<chapter_versions::Model, AppError> {
    let existing = load(tx, chapter_id).await?;
    let content: ChapterContent = parse_json("chapters.content", &existing.content)?;
    let word_count = text::content_word_count(&content);
    super::versions::create_version(
        tx,
        super::versions::NewVersion {
            chapter_id,
            kind: VersionKind::Manual,
            title: &existing.title,
            content: &content,
            word_count,
            note: input.note.trim(),
            session_id: None,
        },
    )
    .await
}

/// `POST /api/chapter-versions/:id/restore` (spec/08 §1.2): a `backup` first, then the version's
/// content. It counts as an author edit, so synced canon becomes stale.
pub async fn restore(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    version_id: i32,
    actor: ChangeActor,
) -> Result<ChapterDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { restore_inner(tx, version_id, actor).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn restore_inner(
    tx: &impl ConnectionTrait,
    version_id: i32,
    actor: ChangeActor,
) -> Result<(ChapterDto, Vec<ServerEvent>), AppError> {
    use sea_orm::ActiveModelTrait;

    let version = super::versions::load(tx, version_id).await?;
    let existing = load(tx, version.chapter_id).await?;
    if let Some(session_id) = super::running_writer_session(tx, existing.id).await? {
        return Err(super::conflict(
            ErrorCode::ChapterLocked,
            "the writer is working; pause it before restoring a version",
            serde_json::json!({ "session_id": session_id }),
        ));
    }
    let current: ChapterContent = parse_json("chapters.content", &existing.content)?;
    let current_words = text::content_word_count(&current);
    super::versions::create_version(
        tx,
        super::versions::NewVersion {
            chapter_id: existing.id,
            kind: VersionKind::Backup,
            title: &existing.title,
            content: &current,
            word_count: current_words,
            note: "",
            session_id: None,
        },
    )
    .await?;

    let restored: ChapterContent = parse_json("chapter_versions.content", &version.content)?;
    let word_count = text::content_word_count(&restored);
    let version_no = existing.content_version + 1;
    let timestamp = now();

    // The restore is an author edit: record it even when `actor` is an agent, because the
    // content change is the same (spec/08 §1.2).
    let _ = actor;
    let mut canon_status =
        CanonStatus::from_str_opt(&existing.canon_status).unwrap_or(CanonStatus::None);
    if existing.status == ChapterStatus::Official.as_str() && canon_status == CanonStatus::Synced {
        canon_status = CanonStatus::Stale;
    }

    let ops = restore_ops(&current, &restored);
    let active = chapters::ActiveModel {
        id: Set(existing.id),
        content: Set(json(&restored)?),
        content_version: Set(version_no),
        word_count: Set(word_count),
        canon_status: Set(canon_status.as_str().to_owned()),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    active.update(tx).await?;
    if existing.status == ChapterStatus::Official.as_str() {
        crate::search::maintenance::sync_passages(tx, existing.id).await?;
        crate::search::index::wake();
    }

    let row = load(tx, existing.id).await?;
    let dto = to_dto(tx, &row).await?;
    Ok((
        dto.clone(),
        vec![
            ServerEvent::ChapterContentChanged {
                chapter_id: existing.id,
                version: version_no,
                ops,
                word_count,
                tail_open: restored.tail_open,
                actor: ContentActor::User,
            },
            ServerEvent::ChapterUpdated {
                chapter: dto.list_item,
                outline_changed: false,
            },
        ],
    ))
}

/// The ops that take a client from `current` to `restored` (spec/10 §3).
///
/// A restored version is an arbitrary earlier document, so it can reorder paragraphs; the shared
/// builder emits a delete plus an insert for a moved pid, which is the only form `ContentOp` has
/// that reproduces the order on a replaying client.
fn restore_ops(
    current: &ChapterContent,
    restored: &ChapterContent,
) -> Vec<crate::api::dto::ContentOp> {
    crate::text::transform_ops(
        &current.paragraphs,
        &restored.paragraphs,
        current.tail_open,
        restored.tail_open,
    )
}

/// A JSON snapshot of a chapter summary change (spec/02 §4 `SnapshotChapterSummary`).
pub fn summary_snapshot(summary: &str) -> Snapshot {
    Snapshot::ChapterSummary {
        summary: summary.to_owned(),
    }
}

// ───────────────────────────── helpers ─────────────────────────────

/// Loads a chapter row or reports `not_found`.
pub async fn load(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
) -> Result<chapters::Model, AppError> {
    chapters::Entity::find_by_id(chapter_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("chapter #{chapter_id}")))
}

/// The ordered chapters of a track.
async fn ordered_chapters(
    conn: &impl ConnectionTrait,
    track_id: i32,
) -> Result<Vec<chapters::Model>, AppError> {
    Ok(chapters::Entity::find()
        .filter(chapters::Column::TrackId.eq(track_id))
        .order_by_asc(chapters::Column::Ord)
        .all(conn)
        .await?)
}

async fn list_chapters(
    conn: &impl ConnectionTrait,
    track_id: i32,
) -> Result<Vec<ChapterListItemDto>, AppError> {
    Ok(ordered_chapters(conn, track_id)
        .await?
        .into_iter()
        .map(ChapterListItemDto::from)
        .collect())
}

/// The next unused `ord` of a track (contiguous `1..n`, spec/02 §1).
async fn next_ord(conn: &impl ConnectionTrait, track_id: i32) -> Result<i32, AppError> {
    Ok(super::books::last_ord(conn, track_id).await? + 1)
}

/// Shifts the chapters after `removed_ord` down by one after a delete (two-phase, spec/02 §1).
async fn renumber_subsequent(
    tx: &impl ConnectionTrait,
    track_id: i32,
    removed_ord: i32,
) -> Result<(), AppError> {
    use sea_orm::ActiveModelTrait;
    let rows = chapters::Entity::find()
        .filter(chapters::Column::TrackId.eq(track_id))
        .filter(chapters::Column::Ord.gt(removed_ord))
        .order_by_asc(chapters::Column::Ord)
        .all(tx)
        .await?;
    // Two phases (spec/02 §1): shift every `ord` negative first so no intermediate value
    // collides with a live one, then write the final value from the original number.
    let targets: Vec<(i32, i32, i32)> = rows
        .iter()
        .map(|row| (row.id, -row.ord, row.ord - 1))
        .collect();
    for (id, negative, _) in &targets {
        let active = chapters::ActiveModel {
            id: Set(*id),
            ord: Set(*negative),
            ..Default::default()
        };
        active.update(tx).await?;
    }
    for (id, _, target) in &targets {
        let active = chapters::ActiveModel {
            id: Set(*id),
            ord: Set(*target),
            ..Default::default()
        };
        active.update(tx).await?;
    }
    Ok(())
}

/// The tracks anchored to a chapter (spec/08 §1.1 `chapter.is_anchor`).
async fn anchor_tracks(conn: &impl ConnectionTrait, chapter_id: i32) -> Result<Vec<i32>, AppError> {
    let rows = tracks::Entity::find()
        .filter(tracks::Column::AnchorChapterId.eq(chapter_id))
        .all(conn)
        .await?;
    Ok(rows.into_iter().map(|row| row.id).collect())
}

/// Rejects content writes while the chapter's writer session is running (spec/08 §1.4).
async fn ensure_editable(tx: &impl ConnectionTrait, chapter_id: i32) -> Result<(), AppError> {
    if let Some(session_id) = super::running_writer_session(tx, chapter_id).await? {
        return Err(super::conflict(
            ErrorCode::ChapterLocked,
            "the writer is working on this chapter",
            serde_json::json!({ "session_id": session_id }),
        ));
    }
    Ok(())
}

/// Optimistic concurrency: a stale `base_version` is `conflict.version` with the current version.
fn ensure_version(existing: &chapters::Model, base_version: i32) -> Result<(), AppError> {
    if existing.content_version != base_version {
        return Err(super::conflict(
            ErrorCode::ConflictVersion,
            "the chapter changed since it was loaded",
            serde_json::json!({ "version": existing.content_version }),
        ));
    }
    Ok(())
}

fn status_of(row: &chapters::Model) -> ChapterStatus {
    ChapterStatus::from_str_opt(&row.status).unwrap_or(ChapterStatus::Planned)
}

fn invalid_state(message: &str, status: ChapterStatus) -> AppError {
    super::conflict(
        ErrorCode::ChapterInvalidState,
        message,
        serde_json::json!({ "status": status.as_str() }),
    )
}

/// Maps a text-layer failure to the API error code the spec assigns to it.
fn map_text_error(error: text::TextError) -> AppError {
    match error {
        text::TextError::PidNotFound(pid) => super::conflict(
            ErrorCode::ValidationFailed,
            format!("paragraph {pid} does not exist"),
            serde_json::json!({ "pid": pid }),
        ),
        text::TextError::InvalidPid(pid) => super::conflict(
            ErrorCode::ValidationFailed,
            format!("{pid} is not a paragraph id"),
            serde_json::json!({ "pid": pid }),
        ),
        text::TextError::EmptyText(what) => {
            AppError::validation(format!("{what} needs non-empty text"))
        }
        text::TextError::PidExhausted => AppError::internal("no paragraph ids left"),
    }
}

/// Maps a chapter row to its full DTO, leaving `open_sessions` empty.
///
/// Used where the caller builds a DTO for a chapter that cannot have sessions yet (a freshly
/// created chapter) or has already loaded them itself.
pub fn to_dto_without_sessions(row: &chapters::Model) -> Result<ChapterDto, AppError> {
    Ok(ChapterDto {
        list_item: ChapterListItemDto::from(row.clone()),
        guidance: row.guidance.clone(),
        outline: row.outline.clone(),
        reference_notes: row.reference_notes.clone(),
        summary: row.summary.clone(),
        content: parse_json("chapters.content", &row.content)?,
        content_version: row.content_version,
        official_version_id: row.official_version_id,
        open_sessions: Vec::new(),
    })
}

/// Maps a chapter row to its full DTO, with the chapter's open sessions (spec/10 §2.8).
pub async fn to_dto(
    conn: &impl ConnectionTrait,
    row: &chapters::Model,
) -> Result<ChapterDto, AppError> {
    let mut dto = to_dto_without_sessions(row)?;
    dto.open_sessions = open_sessions(conn, row.id).await?;
    Ok(dto)
}

/// The roles that own a chapter and therefore appear in `ChapterDto.open_sessions`
/// (spec/10 §2.8). Conception, research and canon-seed sessions are not per-chapter.
pub const CHAPTER_ROLES: [AgentRole; 3] =
    [AgentRole::Writer, AgentRole::Reviewer, AgentRole::Archivist];

/// The statuses that are closed (spec/03 §3). A session in one of them is never "open".
pub const TERMINAL_STATUSES: [SessionStatus; 3] = [
    SessionStatus::Completed,
    SessionStatus::Superseded,
    SessionStatus::Abandoned,
];

/// The chapter's open writer, reviewer and archivist sessions, oldest first (spec/10 §2.8).
///
/// `chapter_revise` sessions carry the writer role, so the role filter already covers them.
pub async fn open_sessions(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
) -> Result<Vec<crate::api::dto::SessionDto>, AppError> {
    let roles: Vec<String> = CHAPTER_ROLES
        .iter()
        .map(|role| role.as_str().to_owned())
        .collect();
    let closed: Vec<String> = TERMINAL_STATUSES
        .iter()
        .map(|status| status.as_str().to_owned())
        .collect();
    let rows = agent_sessions::Entity::find()
        .filter(agent_sessions::Column::ChapterId.eq(chapter_id))
        .filter(agent_sessions::Column::Role.is_in(roles))
        .filter(agent_sessions::Column::Status.is_not_in(closed))
        .order_by_asc(agent_sessions::Column::Id)
        .all(conn)
        .await?;
    let mut sessions = Vec::with_capacity(rows.len());
    for row in &rows {
        // Warnings are a creation-time property (D-073), so an embedded session has none.
        sessions.push(crate::agent::session::to_dto(conn, row, Vec::new()).await?);
    }
    Ok(sessions)
}

/// Records a `chapter_summary` change when maintenance writes a summary (M6 calls this).
pub async fn record_summary_change(
    tx: &impl ConnectionTrait,
    chapter: &chapters::Model,
    before: &str,
    after: &str,
    actor: ChangeActor,
    note: &str,
) -> Result<(), AppError> {
    if before == after {
        return Ok(());
    }
    let before = summary_snapshot(before);
    let after = summary_snapshot(after);
    super::change_log::record(
        tx,
        super::change_log::NewChange {
            book_id: chapter.book_id,
            track_id: Some(chapter.track_id),
            chapter_id: Some(chapter.id),
            session_id: None,
            actor,
            target_type: ChangeTargetType::ChapterSummary,
            target_id: chapter.id,
            op: ChangeOp::Update,
            before: Some(&before),
            after: Some(&after),
            note,
            reverts_id: None,
        },
    )
    .await?;
    crate::search::maintenance::sync_chapter_summary(tx, chapter.id).await?;
    crate::search::index::wake();
    Ok(())
}

/// True when the track's kind is `main` (used by rules that differ per track).
pub async fn track_is_main(conn: &impl ConnectionTrait, track_id: i32) -> Result<bool, AppError> {
    let track = super::tracks::load(conn, track_id).await?;
    Ok(super::enum_of::<TrackKind>("tracks.kind", &track.kind)? == TrackKind::Main)
}

/// Convenience for tests and future services: a chapter's full text as a paragraph list.
pub fn paragraph_texts(content: &ChapterContent) -> Vec<&str> {
    content
        .paragraphs
        .iter()
        .map(|paragraph| paragraph.text.as_str())
        .collect()
}

/// Reads a chapter version's content for the diff view (UIs fetch it through the API).
pub async fn version_content(
    conn: &impl ConnectionTrait,
    version_id: i32,
) -> Result<ChapterContent, AppError> {
    let row = super::versions::load(conn, version_id).await?;
    parse_json("chapter_versions.content", &row.content)
}

#[cfg(test)]
mod tests {
    use crate::db::json::{ChapterContent, Paragraph};

    use super::restore_ops;

    fn doc(paragraphs: &[(&str, &str)], tail_open: bool) -> ChapterContent {
        ChapterContent {
            paragraphs: paragraphs
                .iter()
                .map(|(pid, text)| Paragraph {
                    pid: (*pid).to_owned(),
                    text: (*text).to_owned(),
                })
                .collect(),
            next_pid: paragraphs.len() as i32 + 1,
            tail_open,
        }
    }

    /// Replaying the emitted ops on the current document must produce the restored one
    /// (spec/10 §3: the client applies the ops when `version = local + 1`).
    fn assert_replays(current: &ChapterContent, restored: &ChapterContent) {
        let ops = restore_ops(current, restored);
        let mut replay = current.clone();
        crate::text::apply_ops(&mut replay, &ops).expect("ops replay");
        // `next_pid` is a server-side counter the ops do not carry; the visible state is the
        // paragraph list and the tail.
        assert_eq!(
            replay.paragraphs, restored.paragraphs,
            "ops {ops:?} must reconstruct the version's paragraphs"
        );
        assert_eq!(replay.tail_open, restored.tail_open, "ops {ops:?}: tail");
    }

    #[test]
    fn restore_reproduces_a_reordered_version() {
        // Restoring an earlier version can put the paragraphs back in another order; a bare
        // `update` per paragraph would leave the client with the current order.
        let current = doc(&[("p1", "one, edited"), ("p2", "two, edited")], false);
        let restored = doc(&[("p2", "two"), ("p1", "one")], false);
        assert_replays(&current, &restored);
    }

    #[test]
    fn restore_ops_replay_for_every_shape() {
        let documents = [
            doc(&[("p1", "alpha"), ("p2", "beta"), ("p3", "gamma")], false),
            doc(&[("p2", "beta"), ("p3", "gamma"), ("p1", "alpha")], false),
            doc(&[("p3", "gamma"), ("p1", "alpha")], false),
            doc(&[("p4", "brand new")], true),
            doc(&[], false),
        ];
        for current in &documents {
            for restored in &documents {
                assert_replays(current, restored);
            }
        }
    }

    #[test]
    fn restore_reports_a_tail_change() {
        let ops = restore_ops(&doc(&[("p1", "one")], true), &doc(&[("p1", "one")], false));
        assert_eq!(
            ops,
            vec![crate::api::dto::ContentOp::Tail { open: false }],
            "a tail-only change is still reported"
        );
    }
}
