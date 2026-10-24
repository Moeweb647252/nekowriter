//! Track service: list, read, patch and delete (spec/10 §2.2, spec/08 §1.3, §6).
//!
//! M1 implements the main track. Side stories arrive in M10; creating one here returns
//! `validation.failed` so the API surface matches the spec without half-implemented behavior.

use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set,
};

use crate::api::dto::tracks::{TrackDto, TrackInput, TrackPatch};
use crate::db::entities::{chapters, tracks};
use crate::db::json::{AnchorKind, CanonMode, ChapterStatus, TrackKind};
use crate::error::AppError;
use crate::events::ServerEvent;

use super::now;

/// `GET /api/books/:id/tracks`, main track first.
pub async fn list(conn: &impl ConnectionTrait, book_id: i32) -> Result<Vec<TrackDto>, AppError> {
    super::books::get(conn, book_id).await?;
    let rows = tracks::Entity::find()
        .filter(tracks::Column::BookId.eq(book_id))
        .order_by_asc(tracks::Column::Ord)
        .all(conn)
        .await?;
    rows.iter().map(to_dto).collect()
}

/// `GET`/`PATCH /api/tracks/:id` support.
pub async fn get(conn: &impl ConnectionTrait, track_id: i32) -> Result<TrackDto, AppError> {
    let row = load(conn, track_id).await?;
    to_dto(&row)
}

/// `POST /api/books/:id/tracks`. Only the main track exists in M1 (spec/10 §2.2).
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
    if input.kind != TrackKind::Main {
        // Side stories are M10 (spec/08 §6).
        return Err(AppError::validation(
            "side tracks are not available yet; only the main track can be created",
        ));
    }
    ensure_no_main(tx, book_id).await?;
    let timestamp = now();
    let model = tracks::ActiveModel {
        book_id: Set(book_id),
        kind: Set(TrackKind::Main.as_str().to_owned()),
        title: Set(String::new()),
        synopsis: Set(input.synopsis.trim().to_owned()),
        canon_mode: Set(CanonMode::Canon.as_str().to_owned()),
        anchor_kind: Set(AnchorKind::None.as_str().to_owned()),
        anchor_chapter_id: Set(None),
        focus_entity_id: Set(None),
        ord: Set(0),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    let row = tracks::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    Ok((
        to_dto(&row)?,
        vec![ServerEvent::BookUpdated {
            book_id,
            op: crate::events::BookOp::Update,
        }],
    ))
}

/// `PATCH /api/tracks/:id`. The main track only carries a synopsis (spec/08 §6, D-037).
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
    let kind: TrackKind = super::enum_of("tracks.kind", &existing.kind)?;
    if kind == TrackKind::Main
        && (patch.title.is_some()
            || patch.canon_mode.is_some()
            || patch.anchor_kind.is_some()
            || patch.anchor_chapter_id.is_some()
            || patch.focus_entity_id.is_some())
    {
        // D-037: the main track's title is empty and localized by the UI; it has no anchor and
        // no canon mode of its own.
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
    let row = load(tx, track_id).await?;
    Ok((
        to_dto(&row)?,
        vec![ServerEvent::BookUpdated {
            book_id: row.book_id,
            op: crate::events::BookOp::Update,
        }],
    ))
}

/// `DELETE /api/tracks/:id`. The main track can never be deleted (`validation.failed`, spec/08
/// §1.3); side tracks leave with their chapters in M10.
pub async fn delete(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    track_id: i32,
) -> Result<(), AppError> {
    let published = super::write(db, move |tx| {
        Box::pin(async move { delete_inner(tx, track_id).await })
    })
    .await?;
    events.publish_all(published);
    Ok(())
}

pub async fn delete_inner(
    tx: &impl ConnectionTrait,
    track_id: i32,
) -> Result<Vec<ServerEvent>, AppError> {
    let existing = load(tx, track_id).await?;
    let kind: TrackKind = super::enum_of("tracks.kind", &existing.kind)?;
    if kind == TrackKind::Main {
        return Err(AppError::validation("the main track cannot be deleted"));
    }
    tracks::Entity::delete_by_id(track_id).exec(tx).await?;
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
        .filter(chapters::Column::TrackId.eq(track_id))
        .filter(chapters::Column::Status.eq(ChapterStatus::Official.as_str()))
        .count(conn)
        .await?;
    Ok(count > 0)
}

async fn ensure_no_main(conn: &impl ConnectionTrait, book_id: i32) -> Result<(), AppError> {
    let existing = tracks::Entity::find()
        .filter(tracks::Column::BookId.eq(book_id))
        .filter(tracks::Column::Kind.eq(TrackKind::Main.as_str()))
        .one(conn)
        .await?;
    if let Some(row) = existing {
        // `conflict.exists` documents `details.id` (spec/10 §4): the id of the row that holds the
        // name, so the UI can link to it instead of guessing.
        return Err(AppError::conflict(
            crate::error::ErrorCode::ConflictExists,
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
        kind: super::enum_of("tracks.kind", &row.kind)?,
        title: row.title.clone(),
        synopsis: row.synopsis.clone(),
        canon_mode: super::enum_of("tracks.canon_mode", &row.canon_mode)?,
        anchor_kind: super::enum_of("tracks.anchor_kind", &row.anchor_kind)?,
        anchor_chapter_id: row.anchor_chapter_id,
        focus_entity_id: row.focus_entity_id,
        ord: row.ord,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}
