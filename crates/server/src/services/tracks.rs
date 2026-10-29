//! Track service: list, read, create, patch and delete (spec/10 §2.2, spec/08 §1.3, §6).
//!
//! The book is created with its main track (spec/02 §5 invariant 1); this service creates the
//! side tracks of spec/08 §6. A side track is a first-class track with its own chapters, but it
//! carries a canon mode (`canon` or `isolated`), an optional story-time anchor and an optional
//! focus character (its protagonist for short-term memory, spec/06 §4.2).
//!
//! Rules implemented here:
//! - defaults per spec/08 §6: `isolated`, `after_chapter` = the latest official main chapter
//!   (`before_start` when there is none);
//! - an `after_chapter` anchor must be a main-track chapter of the book, and a focus entity must
//!   be a character visible in the track (`validation.failed` otherwise);
//! - `canon_mode` cannot change once the track has an official chapter (`track.mode_locked`);
//!   the anchor may change at any time and affects only new sessions;
//! - the main track cannot be deleted; side tracks can, and their open sessions are abandoned
//!   before the cascade (spec/08 §1.3).

use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, QueryTrait, Set,
};

use crate::api::dto::tracks::{TrackDto, TrackInput, TrackPatch};
use crate::db::entities::{appearances, chapters, entities, tracks};
use crate::db::json::{AnchorKind, CanonMode, ChapterStatus, EntityKind, TrackKind};
use crate::error::{AppError, ErrorCode};
use crate::events::ServerEvent;

use super::{field_error, now};

/// `GET /api/books/:id/tracks`, main track first.
pub async fn list(conn: &impl ConnectionTrait, book_id: i32) -> Result<Vec<TrackDto>, AppError> {
    super::books::get(conn, book_id).await?;
    let rows = tracks::Entity::find()
        .filter(tracks::COLUMN.book_id.eq(book_id))
        .order_by_asc(tracks::COLUMN.ord)
        .all(conn)
        .await?;
    rows.iter().map(to_dto).collect()
}

/// `GET`/`PATCH /api/tracks/:id` support.
pub async fn get(conn: &impl ConnectionTrait, track_id: i32) -> Result<TrackDto, AppError> {
    let row = load(conn, track_id).await?;
    to_dto(&row)
}

/// `POST /api/books/:id/tracks` (spec/08 §6).
pub async fn create(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    book_id: i32,
    input: TrackInput,
) -> Result<TrackDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { create_inner(tx, book_id, input).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn create_inner(
    tx: &impl ConnectionTrait,
    book_id: i32,
    input: TrackInput,
) -> Result<(TrackDto, Vec<ServerEvent>), AppError> {
    super::books::get(tx, book_id).await?;
    let kind = input.kind.unwrap_or(TrackKind::Side);
    let timestamp = now();
    let (canon_mode, anchor_kind, anchor_chapter_id, focus_entity_id, ord) = match kind {
        TrackKind::Main => {
            // Restoring a missing main track (M1, spec/10 §2.2): it has no side-story
            // settings, and its title is localized by the UI (D-037).
            if input.canon_mode.is_some()
                || input.anchor_kind.is_some()
                || input.anchor_chapter_id.is_some()
                || input.focus_entity_id.is_some()
            {
                return Err(AppError::validation(
                    "the main track has no canon mode, anchor or focus entity",
                ));
            }
            ensure_no_main(tx, book_id).await?;
            (CanonMode::Canon, AnchorKind::None, None, None, 0)
        }
        TrackKind::Side => {
            if input.title.trim().is_empty() {
                return Err(field_error("title", "a side story needs a title"));
            }
            let canon_mode = input.canon_mode.unwrap_or(CanonMode::Isolated);
            let (anchor_kind, anchor_chapter_id) =
                resolve_anchor(tx, book_id, input.anchor_kind, input.anchor_chapter_id).await?;
            if let Some(focus_entity_id) = input.focus_entity_id {
                validate_focus_entity(tx, book_id, None, focus_entity_id).await?;
            }
            (
                canon_mode,
                anchor_kind,
                anchor_chapter_id,
                input.focus_entity_id,
                next_ord(tx, book_id).await?,
            )
        }
    };
    let row = tracks::ActiveModel {
        book_id: Set(book_id),
        kind: Set(kind),
        title: Set(if kind == TrackKind::Main {
            String::new()
        } else {
            input.title.trim().to_owned()
        }),
        synopsis: Set(input.synopsis.trim().to_owned()),
        canon_mode: Set(canon_mode),
        anchor_kind: Set(anchor_kind),
        anchor_chapter_id: Set(anchor_chapter_id),
        focus_entity_id: Set(focus_entity_id),
        ord: Set(ord),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    let row = tracks::Entity::insert(row).exec_with_returning(tx).await?;
    Ok((
        to_dto(&row)?,
        vec![ServerEvent::BookUpdated {
            book_id,
            op: crate::events::BookOp::Update,
        }],
    ))
}

/// `PATCH /api/tracks/:id` (spec/08 §6).
///
/// The main track only carries a synopsis (D-037). A side track's title, synopsis, canon mode,
/// anchor and focus character are editable; the canon mode is locked once the track has an
/// official chapter, and the other fields only affect sessions created afterwards.
pub async fn update(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    track_id: i32,
    patch: TrackPatch,
) -> Result<TrackDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { update_inner(tx, track_id, patch).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn update_inner(
    tx: &impl ConnectionTrait,
    track_id: i32,
    patch: TrackPatch,
) -> Result<(TrackDto, Vec<ServerEvent>), AppError> {
    let existing = load(tx, track_id).await?;
    let kind: TrackKind = existing.kind;
    if kind == TrackKind::Main {
        if patch.title.is_some()
            || patch.canon_mode.is_some()
            || patch.anchor_kind.is_some()
            || patch.anchor_chapter_id.is_some()
            || patch.focus_entity_id.is_some()
            || patch.clear_focus_entity
        {
            // D-037: the main track's title is empty and localized by the UI; it has no anchor
            // and no canon mode of its own.
            return Err(AppError::validation(
                "the main track only has an editable synopsis",
            ));
        }
        let mut active = tracks::ActiveModel {
            id: Set(track_id),
            updated_at: Set(now()),
            ..Default::default()
        };
        if let Some(synopsis) = patch.synopsis {
            active.synopsis = Set(synopsis.trim().to_owned());
        }
        sea_orm::ActiveModelTrait::update(active, tx).await?;
        return finish_update(tx, track_id).await;
    }

    let mut active = tracks::ActiveModel {
        id: Set(track_id),
        updated_at: Set(now()),
        ..Default::default()
    };
    if let Some(ref title) = patch.title {
        let title = title.trim();
        if title.is_empty() {
            return Err(field_error("title", "a side story needs a title"));
        }
        active.title = Set(title.to_owned());
    }
    if let Some(ref synopsis) = patch.synopsis {
        active.synopsis = Set(synopsis.trim().to_owned());
    }
    if let Some(ref canon_mode) = patch.canon_mode {
        let current: CanonMode = existing.canon_mode;
        if *canon_mode != current {
            // A track with an official chapter is already in the canon it chose (spec/08 §6).
            if has_official_chapter(tx, track_id).await? {
                return Err(super::conflict(
                    ErrorCode::TrackModeLocked,
                    "the track has an official chapter; its canon mode cannot change",
                    serde_json::json!({ "canon_mode": current.as_str() }),
                ));
            }
            active.canon_mode = Set(*canon_mode);
        }
    }
    apply_anchor_patch(tx, &existing, &patch, &mut active).await?;
    if patch.clear_focus_entity {
        if patch.focus_entity_id.is_some() {
            return Err(field_error(
                "focus_entity_id",
                "set or clear the focus character, not both",
            ));
        }
        active.focus_entity_id = Set(None);
    } else if let Some(entity_id) = patch.focus_entity_id {
        validate_focus_entity(tx, existing.book_id, Some(&existing), entity_id).await?;
        active.focus_entity_id = Set(Some(entity_id));
    }
    sea_orm::ActiveModelTrait::update(active, tx).await?;
    finish_update(tx, track_id).await
}

/// Resolves the anchor fields of a side-track PATCH into the active model (spec/08 §6).
///
/// `anchor_kind` chooses the mode; `after_chapter` needs a chapter, and changing the chapter of an
/// existing `after_chapter` anchor only sends `anchor_chapter_id`. A kind that names no chapter
/// clears the stored one, so the CHECK of spec/02 never fires.
async fn apply_anchor_patch(
    tx: &impl ConnectionTrait,
    existing: &tracks::Model,
    patch: &TrackPatch,
    active: &mut tracks::ActiveModel,
) -> Result<(), AppError> {
    let current_kind: AnchorKind = existing.anchor_kind;
    match (patch.anchor_kind, patch.anchor_chapter_id) {
        (Some(AnchorKind::AfterChapter), Some(chapter_id)) => {
            validate_anchor_chapter(tx, existing.book_id, chapter_id).await?;
            active.anchor_kind = Set(AnchorKind::AfterChapter);
            active.anchor_chapter_id = Set(Some(chapter_id));
        }
        (Some(AnchorKind::AfterChapter), None) => {
            // Keeping the current anchor is fine; switching to `after_chapter` needs a chapter.
            if current_kind != AnchorKind::AfterChapter || existing.anchor_chapter_id.is_none() {
                return Err(field_error(
                    "anchor_chapter_id",
                    "an after_chapter anchor needs the anchor chapter",
                ));
            }
        }
        (Some(_), Some(_)) => {
            return Err(field_error(
                "anchor_chapter_id",
                "only an after_chapter anchor names a chapter",
            ));
        }
        (Some(other), None) => {
            active.anchor_kind = Set(other);
            active.anchor_chapter_id = Set(None);
        }
        (None, Some(chapter_id)) => {
            if current_kind != AnchorKind::AfterChapter {
                return Err(field_error(
                    "anchor_kind",
                    "choose an after_chapter anchor before naming its chapter",
                ));
            }
            validate_anchor_chapter(tx, existing.book_id, chapter_id).await?;
            active.anchor_chapter_id = Set(Some(chapter_id));
        }
        (None, None) => {}
    }
    Ok(())
}

/// Reads the updated track and returns its DTO with the `book.updated` event.
async fn finish_update(
    tx: &impl ConnectionTrait,
    track_id: i32,
) -> Result<(TrackDto, Vec<ServerEvent>), AppError> {
    let row = load(tx, track_id).await?;
    Ok((
        to_dto(&row)?,
        vec![ServerEvent::BookUpdated {
            book_id: row.book_id,
            op: crate::events::BookOp::Update,
        }],
    ))
}

/// `DELETE /api/tracks/:id`.
///
/// The main track can never be deleted (`validation.failed`, spec/08 §1.3). A side track can be
/// deleted at any time: its open sessions are stopped and abandoned first, then the cascade
/// removes its chapters, its private canon rows and its change rows (spec/08 §1.3, D-043).
pub async fn delete(state: &crate::state::AppState, track_id: i32) -> Result<(), AppError> {
    let existing = load(state.db().conn(), track_id).await?;
    let kind: TrackKind = existing.kind;
    if kind == TrackKind::Main {
        return Err(AppError::validation("the main track cannot be deleted"));
    }
    // No runner may write into the cascade: stop and abandon every open session of the track
    // before the delete transaction (spec/08 §1.3).
    for session_id in super::sessions::open_ids_for_track(state.db().conn(), track_id).await? {
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
        Box::pin(async move { delete_inner(tx, track_id).await })
    })
    .await?;
    state.events().publish_all(published);
    Ok(())
}

pub async fn delete_inner(
    tx: &impl ConnectionTrait,
    track_id: i32,
) -> Result<Vec<ServerEvent>, AppError> {
    let existing = load(tx, track_id).await?;
    let kind: TrackKind = existing.kind;
    if kind == TrackKind::Main {
        return Err(AppError::validation("the main track cannot be deleted"));
    }
    // Shared entities that appeared only in this track's chapters get their bounds recomputed
    // after the cascade (spec/06 §1.1, D-059, D-080). The ids are read before the rows vanish.
    let appeared: Vec<i32> = appearances::Entity::find()
        .filter(
            appearances::COLUMN.chapter_id.in_subquery(
                chapters::Entity::find()
                    .select_only()
                    .column(chapters::Column::Id)
                    .filter(chapters::COLUMN.track_id.eq(track_id))
                    .into_query(),
            ),
        )
        .all(tx)
        .await?
        .into_iter()
        .map(|row| row.entity_id)
        .collect();
    tracks::Entity::delete_by_id(track_id).exec(tx).await?;
    super::canon::entity::recompute_bounds(tx, existing.book_id, &appeared)
        .await
        .map_err(AppError::from)?;
    Ok(vec![ServerEvent::BookUpdated {
        book_id: existing.book_id,
        op: crate::events::BookOp::Update,
    }])
}

/// Loads a track or reports `not_found`.
pub async fn load(conn: &impl ConnectionTrait, track_id: i32) -> Result<tracks::Model, AppError> {
    tracks::Entity::find_by_id(track_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("track #{track_id}")))
}

/// True when a track has at least one official chapter (spec/08 §6 mode lock).
pub async fn has_official_chapter(
    conn: &impl ConnectionTrait,
    track_id: i32,
) -> Result<bool, AppError> {
    let count = chapters::Entity::find()
        .filter(chapters::COLUMN.track_id.eq(track_id))
        .filter(chapters::COLUMN.status.eq(ChapterStatus::Official))
        .count(conn)
        .await?;
    Ok(count > 0)
}

/// True when the track is a side track in isolated canon mode (spec/06 §2).
pub fn is_isolated(row: &tracks::Model) -> bool {
    row.kind == TrackKind::Side && row.canon_mode == CanonMode::Isolated
}

/// The main track of a book, when it exists (a book always has one, spec/02 §5).
pub async fn main_track(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Option<tracks::Model>, AppError> {
    Ok(tracks::Entity::find()
        .filter(tracks::COLUMN.book_id.eq(book_id))
        .filter(tracks::COLUMN.kind.eq(TrackKind::Main))
        .one(conn)
        .await?)
}

/// Resolves the anchor of a new side track (spec/08 §6).
///
/// Default: `after_chapter` on the latest official main chapter, or `before_start` when no main
/// chapter is official yet. An explicit `anchor_kind = after_chapter` must name its chapter; a
/// chapter without a kind means `after_chapter`. A kind that names no chapter must not carry one.
async fn resolve_anchor(
    conn: &impl ConnectionTrait,
    book_id: i32,
    kind: Option<AnchorKind>,
    chapter_id: Option<i32>,
) -> Result<(AnchorKind, Option<i32>), AppError> {
    let Some(kind) = kind else {
        if let Some(chapter_id) = chapter_id {
            validate_anchor_chapter(conn, book_id, chapter_id).await?;
            return Ok((AnchorKind::AfterChapter, Some(chapter_id)));
        }
        return Ok(match latest_official_main_chapter(conn, book_id).await? {
            Some(chapter) => (AnchorKind::AfterChapter, Some(chapter.id)),
            None => (AnchorKind::BeforeStart, None),
        });
    };
    match kind {
        AnchorKind::AfterChapter => {
            let Some(chapter_id) = chapter_id else {
                return Err(field_error(
                    "anchor_chapter_id",
                    "an after_chapter anchor needs the anchor chapter",
                ));
            };
            validate_anchor_chapter(conn, book_id, chapter_id).await?;
            Ok((AnchorKind::AfterChapter, Some(chapter_id)))
        }
        other => {
            if chapter_id.is_some() {
                return Err(field_error(
                    "anchor_chapter_id",
                    "only an after_chapter anchor names a chapter",
                ));
            }
            Ok((other, None))
        }
    }
}

/// The latest official main-track chapter of the book (the default `after_chapter` anchor).
async fn latest_official_main_chapter(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Option<chapters::Model>, AppError> {
    let Some(main) = main_track(conn, book_id).await? else {
        return Ok(None);
    };
    Ok(chapters::Entity::find()
        .filter(chapters::COLUMN.track_id.eq(main.id))
        .filter(chapters::COLUMN.status.eq(ChapterStatus::Official))
        .order_by_desc(chapters::COLUMN.ord)
        .one(conn)
        .await?)
}

/// An `after_chapter` anchor must be a main-track chapter of this book (spec/08 §6).
async fn validate_anchor_chapter(
    conn: &impl ConnectionTrait,
    book_id: i32,
    chapter_id: i32,
) -> Result<(), AppError> {
    let chapter = chapters::Entity::find_by_id(chapter_id)
        .one(conn)
        .await?
        .ok_or_else(|| field_error("anchor_chapter_id", "the anchor chapter does not exist"))?;
    let track = tracks::Entity::find_by_id(chapter.track_id)
        .one(conn)
        .await?
        .ok_or_else(|| field_error("anchor_chapter_id", "the anchor chapter has no track"))?;
    if track.book_id != book_id || track.kind != TrackKind::Main {
        return Err(field_error(
            "anchor_chapter_id",
            "the anchor must be a main-track chapter of this book",
        ));
    }
    Ok(())
}

/// The focus character must be a character visible in the track (spec/08 §6).
///
/// A shared character is always visible; an isolated track may also focus one of its own private
/// characters. A `track` of `None` is a track being created, which has no private rows yet.
async fn validate_focus_entity(
    conn: &impl ConnectionTrait,
    book_id: i32,
    track: Option<&tracks::Model>,
    entity_id: i32,
) -> Result<(), AppError> {
    let Some(row) = entities::Entity::find_by_id(entity_id).one(conn).await? else {
        return Err(field_error(
            "focus_entity_id",
            "the focus character does not exist",
        ));
    };
    if row.book_id != book_id {
        return Err(field_error(
            "focus_entity_id",
            "the focus character belongs to another book",
        ));
    }
    if row.kind != EntityKind::Character {
        return Err(field_error(
            "focus_entity_id",
            "the focus entity must be a character",
        ));
    }
    if let Some(private_track) = row.track_id {
        let visible = track.is_some_and(|track| {
            track.id == private_track
                && track.kind == TrackKind::Side
                && track.canon_mode == CanonMode::Isolated
        });
        if !visible {
            return Err(field_error(
                "focus_entity_id",
                "the focus character is not visible in this track",
            ));
        }
    }
    Ok(())
}

/// The `ord` of a new side track: the next free number after the existing tracks (§ `ord` = 1,
/// 2, … in creation order; the main track is 0).
async fn next_ord(conn: &impl ConnectionTrait, book_id: i32) -> Result<i32, AppError> {
    let max = super::query_i64(
        conn,
        "SELECT COALESCE(MAX(ord), 0) AS value FROM tracks WHERE book_id = ?",
        vec![book_id.into()],
    )
    .await?
    .unwrap_or(0);
    i32::try_from(max + 1).map_err(|_| AppError::internal("track ord overflow"))
}

/// The open sessions of a track's chapters (status not terminal).
async fn ensure_no_main(conn: &impl ConnectionTrait, book_id: i32) -> Result<(), AppError> {
    let existing = tracks::Entity::find()
        .filter(tracks::COLUMN.book_id.eq(book_id))
        .filter(tracks::COLUMN.kind.eq(TrackKind::Main))
        .one(conn)
        .await?;
    if let Some(row) = existing {
        // `conflict.exists` documents `details.id` (spec/10 §4): the id of the row that holds the
        // name, so the UI can link to it instead of guessing.
        return Err(AppError::conflict(
            ErrorCode::ConflictExists,
            "the book already has a main track",
        )
        .with_details(serde_json::json!({ "id": row.id })));
    }
    Ok(())
}

/// Maps a track row to its DTO.
pub fn to_dto(row: &tracks::Model) -> Result<TrackDto, AppError> {
    Ok(TrackDto {
        id: row.id,
        book_id: row.book_id,
        kind: row.kind,
        title: row.title.clone(),
        synopsis: row.synopsis.clone(),
        canon_mode: row.canon_mode,
        anchor_kind: row.anchor_kind,
        anchor_chapter_id: row.anchor_chapter_id,
        focus_entity_id: row.focus_entity_id,
        ord: row.ord,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}
