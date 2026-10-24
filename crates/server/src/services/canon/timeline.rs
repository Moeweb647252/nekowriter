//! Timeline service (spec/06 §1.4, spec/02 §1, spec/10 §2.4; spec/05 §6).
//!
//! `position` is **story chronology**, not chapter order: a flashback narrated in chapter 40 to
//! events 20 years earlier gets a position near the start.
//!
//! - Inserting between two neighbors uses the midpoint of their positions.
//! - When the gap falls below `1e-6`, the book's whole timeline is renormalized to multiples of
//!   1024 first, then the insert happens (spec/02 §1).
//! - `move` places an event with `after_id` / `before_id`; both at once is
//!   `error.mutually_exclusive`.
//!
//! Story order is what the API returns (and what the UI reorders by drag and drop).

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};

use crate::api::dto::canon::{
    TimelineEventDto, TimelineEventInput, TimelineEventPatch, TimelineMoveInput, TimelineQuery,
};
use crate::db::entities::timeline_events;
use crate::db::json::{ChangeOp, ChangeTargetType};
use crate::events::ServerEvent;

use super::{
    CanonError, CanonResult, CanonWrite, ChangeWrite, changed_event, stamp, timeline_snapshot,
};

/// The spacing a renormalized timeline uses (spec/02 §1: multiples of 1024).
pub const POSITION_STEP: f64 = 1024.0;

/// The gap below which the timeline is renormalized (spec/02 §1).
pub const MIN_GAP: f64 = 1e-6;

// ───────────────────────────── reads ─────────────────────────────

/// `GET /api/books/:id/timeline?track_id&entity_id` (spec/10 §2.4): story order.
pub async fn list(
    conn: &impl ConnectionTrait,
    book_id: i32,
    query: &TimelineQuery,
) -> CanonResult<Vec<TimelineEventDto>> {
    super::super::books::get(conn, book_id).await?;
    let rows = ordered(conn, book_id, query.track_id).await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(entity_id) = query.entity_id {
            let ids =
                super::linked_entity_ids(conn, super::LinkTable::TimelineEvent, row.id).await?;
            if !ids.contains(&entity_id) {
                continue;
            }
        }
        items.push(to_dto(conn, &row).await?);
    }
    Ok(items)
}

/// Every timeline event of a book in story order, optionally narrowed to one track.
pub async fn ordered(
    conn: &impl ConnectionTrait,
    book_id: i32,
    track_id: Option<i32>,
) -> CanonResult<Vec<timeline_events::Model>> {
    let mut statement = timeline_events::Entity::find()
        .filter(timeline_events::Column::BookId.eq(book_id))
        .order_by_asc(timeline_events::Column::Position)
        .order_by_asc(timeline_events::Column::Id);
    if let Some(track_id) = track_id {
        // A track's view is its own events plus the shared ones (spec/06 §2).
        statement = statement.filter(
            sea_orm::Condition::any()
                .add(timeline_events::Column::TrackId.is_null())
                .add(timeline_events::Column::TrackId.eq(track_id)),
        );
    }
    Ok(statement.all(conn).await?)
}

/// `GET /api/timeline-events/:id`.
pub async fn get(conn: &impl ConnectionTrait, event_id: i32) -> CanonResult<TimelineEventDto> {
    let row = load(conn, event_id).await?;
    to_dto(conn, &row).await
}

// ───────────────────────────── create ─────────────────────────────

/// `POST /api/books/:id/timeline` (spec/10 §2.4).
pub async fn create(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    book_id: i32,
    input: TimelineEventInput,
) -> CanonResult<TimelineEventDto> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { create_inner(tx, &CanonWrite::author(), book_id, input).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn create_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    book_id: i32,
    input: TimelineEventInput,
) -> CanonResult<(TimelineEventDto, Vec<ServerEvent>)> {
    super::super::books::get(tx, book_id).await?;
    let title = input.title.trim();
    if title.is_empty() {
        return Err(CanonError::field("title", "a timeline event needs a title"));
    }
    if input.after_id.is_some() && input.before_id.is_some() {
        return Err(mutually_exclusive(&["after_id", "before_id"]));
    }
    super::validate_importance(input.importance)?;
    if let Some(chapter_id) = input.chapter_id {
        ensure_chapter_in_book(tx, book_id, chapter_id).await?;
    }
    ensure_entities_visible(tx, write, book_id, &input.entity_ids).await?;
    let track_id = write.new_row_track();
    let position = position_for(tx, book_id, input.after_id, input.before_id, None).await?;
    let timestamp = stamp();
    let model = timeline_events::ActiveModel {
        book_id: Set(book_id),
        track_id: Set(track_id),
        position: Set(position),
        story_time: Set(input.story_time.trim().to_owned()),
        title: Set(title.to_owned()),
        description: Set(input.description.trim().to_owned()),
        chapter_id: Set(input.chapter_id.or(write.chapter_id)),
        importance: Set(input.importance),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    let row = timeline_events::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    let entity_ids = unique_ids(&input.entity_ids);
    super::set_linked_entities(tx, super::LinkTable::TimelineEvent, row.id, &entity_ids).await?;
    let after = timeline_snapshot(&row, entity_ids);
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id,
            target_type: ChangeTargetType::TimelineEvent,
            target_id: row.id,
            op: ChangeOp::Create,
            before: None,
            after: Some(&after),
            note: input.note.as_deref().unwrap_or_default().trim(),
            reverts_id: None,
            track_id: row.track_id,
            chapter_id: None,
        },
    )
    .await?;
    let events = vec![changed_event(
        book_id,
        ChangeTargetType::TimelineEvent,
        row.id,
        ChangeOp::Create,
        change_id,
    )];
    Ok((to_dto(tx, &row).await?, events))
}

// ───────────────────────────── update ─────────────────────────────

/// `PATCH /api/timeline-events/:id` (spec/10 §2.4).
pub async fn update(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    event_id: i32,
    patch: TimelineEventPatch,
) -> CanonResult<TimelineEventDto> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { update_inner(tx, &CanonWrite::author(), event_id, patch).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn update_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    event_id: i32,
    patch: TimelineEventPatch,
) -> CanonResult<(TimelineEventDto, Vec<ServerEvent>)> {
    let existing = load(tx, event_id).await?;
    write.ensure_writable(
        existing.track_id,
        &format!("timeline event #{}", existing.id),
    )?;
    if patch.after_id.is_some() && patch.before_id.is_some() {
        return Err(mutually_exclusive(&["after_id", "before_id"]));
    }
    if let Some(importance) = patch.importance {
        super::validate_importance(importance)?;
    }
    if let Some(Some(chapter_id)) = patch.chapter_id {
        ensure_chapter_in_book(tx, existing.book_id, chapter_id).await?;
    }
    if let Some(entity_ids) = &patch.entity_ids {
        ensure_entities_visible(tx, write, existing.book_id, entity_ids).await?;
    }
    let current_ids =
        super::linked_entity_ids(tx, super::LinkTable::TimelineEvent, event_id).await?;
    let before = timeline_snapshot(&existing, current_ids);
    let mut active = timeline_events::ActiveModel {
        id: Set(event_id),
        updated_at: Set(stamp()),
        ..Default::default()
    };
    if let Some(title) = &patch.title {
        let title = title.trim();
        if title.is_empty() {
            return Err(CanonError::field("title", "a timeline event needs a title"));
        }
        active.title = Set(title.to_owned());
    }
    if let Some(description) = &patch.description {
        active.description = Set(description.trim().to_owned());
    }
    if let Some(story_time) = &patch.story_time {
        active.story_time = Set(story_time.trim().to_owned());
    }
    if let Some(chapter_id) = patch.chapter_id {
        active.chapter_id = Set(chapter_id);
    }
    if let Some(importance) = patch.importance {
        active.importance = Set(importance);
    }
    let moved = patch.after_id.is_some() || patch.before_id.is_some();
    if moved {
        let position = position_for(
            tx,
            existing.book_id,
            patch.after_id,
            patch.before_id,
            Some(event_id),
        )
        .await?;
        active.position = Set(position);
    }
    active.update(tx).await?;
    if let Some(entity_ids) = &patch.entity_ids {
        super::set_linked_entities(
            tx,
            super::LinkTable::TimelineEvent,
            event_id,
            &unique_ids(entity_ids),
        )
        .await?;
    }
    let row = load(tx, event_id).await?;
    let entity_ids =
        super::linked_entity_ids(tx, super::LinkTable::TimelineEvent, event_id).await?;
    let after = timeline_snapshot(&row, entity_ids);
    if before == after {
        return Ok((to_dto(tx, &row).await?, Vec::new()));
    }
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id: row.book_id,
            target_type: ChangeTargetType::TimelineEvent,
            target_id: row.id,
            op: ChangeOp::Update,
            before: Some(&before),
            after: Some(&after),
            note: patch.note.as_deref().unwrap_or_default().trim(),
            reverts_id: None,
            track_id: row.track_id,
            chapter_id: None,
        },
    )
    .await?;
    let events = vec![changed_event(
        row.book_id,
        ChangeTargetType::TimelineEvent,
        row.id,
        ChangeOp::Update,
        change_id,
    )];
    Ok((to_dto(tx, &row).await?, events))
}

/// `POST /api/timeline-events/:id/move` (spec/10 §2.4): after_id / before_id placement.
pub async fn move_event(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    event_id: i32,
    input: TimelineMoveInput,
) -> CanonResult<TimelineEventDto> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { move_inner(tx, &CanonWrite::author(), event_id, input).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn move_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    event_id: i32,
    input: TimelineMoveInput,
) -> CanonResult<(TimelineEventDto, Vec<ServerEvent>)> {
    if input.after_id.is_some() && input.before_id.is_some() {
        return Err(mutually_exclusive(&["after_id", "before_id"]));
    }
    let patch = TimelineEventPatch {
        after_id: input.after_id,
        before_id: input.before_id,
        note: input.note,
        ..TimelineEventPatch::default()
    };
    update_inner(tx, write, event_id, patch).await
}

// ───────────────────────────── delete ─────────────────────────────

/// `DELETE /api/timeline-events/:id`.
pub async fn delete(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    event_id: i32,
) -> CanonResult<()> {
    let (_, published) = super::write(db, move |tx| {
        Box::pin(async move {
            let events = delete_inner(tx, &CanonWrite::author(), event_id).await?;
            Ok(((), events))
        })
    })
    .await?;
    events.publish_all(published);
    Ok(())
}

pub async fn delete_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    event_id: i32,
) -> CanonResult<Vec<ServerEvent>> {
    let existing = load(tx, event_id).await?;
    write.ensure_writable(
        existing.track_id,
        &format!("timeline event #{}", existing.id),
    )?;
    let ids = super::linked_entity_ids(tx, super::LinkTable::TimelineEvent, event_id).await?;
    let before = timeline_snapshot(&existing, ids);
    timeline_events::Entity::delete_by_id(event_id)
        .exec(tx)
        .await?;
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id: existing.book_id,
            target_type: ChangeTargetType::TimelineEvent,
            target_id: event_id,
            op: ChangeOp::Delete,
            before: Some(&before),
            after: None,
            note: "",
            reverts_id: None,
            track_id: existing.track_id,
            chapter_id: None,
        },
    )
    .await?;
    Ok(vec![changed_event(
        existing.book_id,
        ChangeTargetType::TimelineEvent,
        event_id,
        ChangeOp::Delete,
        change_id,
    )])
}

// ───────────────────────────── positions (spec/02 §1) ─────────────────────────────

/// The position a new or moved event gets (spec/02 §1, spec/06 §1.4).
///
/// `after_id` / `before_id` name the neighbors; with neither, the event goes to the end. When the
/// neighbors leave no room (`gap < 1e-6`) the book's timeline is renormalized to multiples of
/// [`POSITION_STEP`] and the midpoint is recomputed.
pub async fn position_for(
    tx: &impl ConnectionTrait,
    book_id: i32,
    after_id: Option<i32>,
    before_id: Option<i32>,
    moving: Option<i32>,
) -> CanonResult<f64> {
    let after = match after_id {
        Some(id) => Some(load_in_book(tx, book_id, id).await?),
        None => None,
    };
    let before = match before_id {
        Some(id) => Some(load_in_book(tx, book_id, id).await?),
        None => None,
    };
    if let (Some(after), Some(before)) = (&after, &before)
        && after.position >= before.position
    {
        return Err(CanonError::field(
            "before_id",
            "before_id must come after after_id in story order",
        ));
    }
    if let Some(event) = after.as_ref().or(before.as_ref())
        && Some(event.id) == moving
    {
        return Err(CanonError::validation(
            "an event cannot be moved next to itself",
        ));
    }
    let neighbors =
        neighbor_positions(tx, book_id, after.as_ref(), before.as_ref(), moving).await?;
    let (low, high) = neighbors;
    if gap_ok(low, high) {
        return Ok(midpoint(low, high));
    }
    // Renormalize to multiples of 1024, then place the event again.
    renormalize(tx, book_id).await?;
    let after = match after_id {
        Some(id) => Some(load_in_book(tx, book_id, id).await?),
        None => None,
    };
    let before = match before_id {
        Some(id) => Some(load_in_book(tx, book_id, id).await?),
        None => None,
    };
    let (low, high) =
        neighbor_positions(tx, book_id, after.as_ref(), before.as_ref(), moving).await?;
    if !gap_ok(low, high) {
        // Renormalizing cannot fail to open a gap unless the book has a single event; place it.
        return Err(CanonError::internal(
            "the timeline still has no room for the event after renormalizing",
        ));
    }
    Ok(midpoint(low, high))
}

/// The `(previous, next)` positions around the place an event goes (spec/06 §1.4).
async fn neighbor_positions(
    tx: &impl ConnectionTrait,
    book_id: i32,
    after: Option<&timeline_events::Model>,
    before: Option<&timeline_events::Model>,
    moving: Option<i32>,
) -> CanonResult<(f64, f64)> {
    let previous = match after {
        Some(after) => after.position,
        None => match before {
            // Before the first event: use the first event's position minus a step.
            Some(before) => {
                let first = first_before(tx, book_id, before.position, moving).await?;
                match first {
                    Some(position) => position - POSITION_STEP,
                    None => before.position - POSITION_STEP,
                }
            }
            None => last_position(tx, book_id, moving).await?.unwrap_or(0.0),
        },
    };
    let next = match before {
        Some(before) => before.position,
        None => match after {
            Some(after) => next_after(tx, book_id, after.position, moving)
                .await?
                .unwrap_or(f64::MAX),
            None => f64::MAX,
        },
    };
    Ok((previous, next))
}

/// A gap is usable when it is not below the renormalization threshold.
fn gap_ok(low: f64, high: f64) -> bool {
    if high == f64::MAX {
        return true;
    }
    if high <= low {
        return false;
    }
    high - low >= MIN_GAP
}

/// The midpoint between two positions (spec/06 §1.4).
fn midpoint(low: f64, high: f64) -> f64 {
    if high == f64::MAX {
        return low + POSITION_STEP;
    }
    (low + high) / 2.0
}

/// Renormalizes a book's timeline to `1024, 2048, …` (spec/02 §1).
pub async fn renormalize(tx: &impl ConnectionTrait, book_id: i32) -> CanonResult<()> {
    let rows = timeline_events::Entity::find()
        .filter(timeline_events::Column::BookId.eq(book_id))
        .order_by_asc(timeline_events::Column::Position)
        .order_by_asc(timeline_events::Column::Id)
        .all(tx)
        .await?;
    for (index, row) in rows.iter().enumerate() {
        let target = POSITION_STEP * (index as f64 + 1.0);
        if (row.position - target).abs() < f64::EPSILON {
            continue;
        }
        timeline_events::ActiveModel {
            id: Set(row.id),
            position: Set(target),
            ..Default::default()
        }
        .update(tx)
        .await?;
    }
    Ok(())
}

/// The last position of a book's timeline (ignoring the event being moved).
async fn last_position(
    tx: &impl ConnectionTrait,
    book_id: i32,
    moving: Option<i32>,
) -> CanonResult<Option<f64>> {
    let mut statement = timeline_events::Entity::find()
        .filter(timeline_events::Column::BookId.eq(book_id))
        .order_by_desc(timeline_events::Column::Position)
        .order_by_desc(timeline_events::Column::Id);
    if let Some(moving) = moving {
        statement = statement.filter(timeline_events::Column::Id.ne(moving));
    }
    Ok(statement.one(tx).await?.map(|row| row.position))
}

/// The next position after `position` (ignoring the event being moved).
async fn next_after(
    tx: &impl ConnectionTrait,
    book_id: i32,
    position: f64,
    moving: Option<i32>,
) -> CanonResult<Option<f64>> {
    let mut statement = timeline_events::Entity::find()
        .filter(timeline_events::Column::BookId.eq(book_id))
        .filter(timeline_events::Column::Position.gt(position))
        .order_by_asc(timeline_events::Column::Position)
        .order_by_asc(timeline_events::Column::Id);
    if let Some(moving) = moving {
        statement = statement.filter(timeline_events::Column::Id.ne(moving));
    }
    Ok(statement.one(tx).await?.map(|row| row.position))
}

/// The position right before `position` (ignoring the event being moved).
async fn first_before(
    tx: &impl ConnectionTrait,
    book_id: i32,
    position: f64,
    moving: Option<i32>,
) -> CanonResult<Option<f64>> {
    let mut statement = timeline_events::Entity::find()
        .filter(timeline_events::Column::BookId.eq(book_id))
        .filter(timeline_events::Column::Position.lt(position))
        .order_by_desc(timeline_events::Column::Position)
        .order_by_desc(timeline_events::Column::Id);
    if let Some(moving) = moving {
        statement = statement.filter(timeline_events::Column::Id.ne(moving));
    }
    Ok(statement.one(tx).await?.map(|row| row.position))
}

// ───────────────────────────── helpers ─────────────────────────────

/// `after_id` and `before_id` cannot be combined (spec/05, `error.mutually_exclusive`).
pub fn mutually_exclusive(fields: &[&str]) -> CanonError {
    CanonError::App(
        crate::error::AppError::validation(format!("{} cannot be combined", fields.join(" and ")))
            .with_details(serde_json::json!({ "fields": fields })),
    )
}

fn unique_ids(ids: &[i32]) -> Vec<i32> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for id in ids {
        if seen.insert(*id) {
            result.push(*id);
        }
    }
    result
}

async fn ensure_chapter_in_book(
    tx: &impl ConnectionTrait,
    book_id: i32,
    chapter_id: i32,
) -> CanonResult<()> {
    let chapter = crate::db::entities::chapters::Entity::find_by_id(chapter_id)
        .one(tx)
        .await?;
    match chapter {
        Some(chapter) if chapter.book_id == book_id => Ok(()),
        _ => Err(CanonError::field(
            "chapter_id",
            "the chapter does not belong to this book",
        )),
    }
}

async fn ensure_entities_visible(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    book_id: i32,
    entity_ids: &[i32],
) -> CanonResult<()> {
    let visibility = write
        .visibility
        .unwrap_or_else(|| super::Visibility::shared(book_id));
    for entity_id in entity_ids {
        super::load_visible_entity(tx, visibility, *entity_id)
            .await
            .map_err(|_| {
                CanonError::validation(format!("entity #{entity_id} is not visible in this canon"))
            })?;
    }
    let _ = book_id;
    Ok(())
}

/// Loads one timeline event or reports `not_found`.
pub async fn load(
    conn: &impl ConnectionTrait,
    event_id: i32,
) -> CanonResult<timeline_events::Model> {
    timeline_events::Entity::find_by_id(event_id)
        .one(conn)
        .await?
        .ok_or_else(|| CanonError::not_found(format!("timeline event #{event_id}")))
}

/// Loads one timeline event and checks it belongs to the book.
async fn load_in_book(
    conn: &impl ConnectionTrait,
    book_id: i32,
    event_id: i32,
) -> CanonResult<timeline_events::Model> {
    let row = load(conn, event_id).await?;
    if row.book_id != book_id {
        return Err(CanonError::not_found(format!("timeline event #{event_id}")));
    }
    Ok(row)
}

/// Maps a row to its DTO, including its entity links.
pub async fn to_dto(
    conn: &impl ConnectionTrait,
    row: &timeline_events::Model,
) -> CanonResult<TimelineEventDto> {
    Ok(TimelineEventDto {
        id: row.id,
        book_id: row.book_id,
        track_id: row.track_id,
        position: row.position,
        story_time: row.story_time.clone(),
        title: row.title.clone(),
        description: row.description.clone(),
        chapter_id: row.chapter_id,
        importance: row.importance,
        entity_ids: super::linked_entity_ids(conn, super::LinkTable::TimelineEvent, row.id).await?,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

// ───────────────────────────── revert support (spec/06 §3) ─────────────────────────────

/// The current snapshot of a timeline event, when it still exists.
pub async fn snapshot_now(
    conn: &impl ConnectionTrait,
    event_id: i32,
) -> CanonResult<Option<crate::db::json::Snapshot>> {
    match timeline_events::Entity::find_by_id(event_id)
        .one(conn)
        .await?
    {
        Some(row) => {
            let ids =
                super::linked_entity_ids(conn, super::LinkTable::TimelineEvent, event_id).await?;
            Ok(Some(timeline_snapshot(&row, ids)))
        }
        None => Ok(None),
    }
}

/// Restores a timeline event from a `before` snapshot (the revert of an update).
pub async fn restore_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    event_id: i32,
    before: &crate::db::json::Snapshot,
) -> CanonResult<crate::db::json::Snapshot> {
    let crate::db::json::Snapshot::TimelineEvent {
        position,
        story_time,
        title,
        description,
        chapter_id,
        importance,
        entity_ids,
        ..
    } = before
    else {
        return Err(CanonError::internal(
            "a timeline revert needs a timeline snapshot",
        ));
    };
    let existing = load(tx, event_id).await?;
    write.ensure_writable(
        existing.track_id,
        &format!("timeline event #{}", existing.id),
    )?;
    timeline_events::ActiveModel {
        id: Set(event_id),
        position: Set(*position),
        story_time: Set(story_time.clone()),
        title: Set(title.clone()),
        description: Set(description.clone()),
        chapter_id: Set(*chapter_id),
        importance: Set(*importance),
        updated_at: Set(stamp()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    super::set_linked_entities(
        tx,
        super::LinkTable::TimelineEvent,
        event_id,
        &unique_ids(entity_ids),
    )
    .await?;
    let row = load(tx, event_id).await?;
    Ok(timeline_snapshot(&row, unique_ids(entity_ids)))
}

/// Recreates a deleted timeline event from its `before` snapshot, with a **new id**.
pub async fn recreate_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    before: &crate::db::json::Snapshot,
) -> CanonResult<(i32, crate::db::json::Snapshot)> {
    let crate::db::json::Snapshot::TimelineEvent {
        position,
        story_time,
        title,
        description,
        chapter_id,
        importance,
        entity_ids,
        track_id,
    } = before
    else {
        return Err(CanonError::internal(
            "a timeline revert needs a timeline snapshot",
        ));
    };
    let book_id = write
        .visibility
        .map(|visibility| visibility.book_id)
        .ok_or_else(|| CanonError::internal("a timeline revert needs a book"))?;
    super::super::books::get(tx, book_id).await?;
    let timestamp = stamp();
    let model = timeline_events::ActiveModel {
        book_id: Set(book_id),
        track_id: Set(track_id.or_else(|| write.new_row_track())),
        position: Set(*position),
        story_time: Set(story_time.clone()),
        title: Set(title.clone()),
        description: Set(description.clone()),
        chapter_id: Set(*chapter_id),
        importance: Set(*importance),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    let row = timeline_events::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    // Join rows are restored when their entities still exist (spec/02 §5.10).
    let mut restored = Vec::new();
    for entity_id in unique_ids(entity_ids) {
        if crate::db::entities::entities::Entity::find_by_id(entity_id)
            .one(tx)
            .await?
            .is_some()
        {
            restored.push(entity_id);
        }
    }
    super::set_linked_entities(tx, super::LinkTable::TimelineEvent, row.id, &restored).await?;
    Ok((row.id, timeline_snapshot(&row, restored)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gap_below_the_threshold_is_not_usable() {
        assert!(gap_ok(0.0, 1.0));
        assert!(!gap_ok(0.0, 1e-7));
        assert!(!gap_ok(5.0, 5.0));
        assert!(
            gap_ok(0.0, f64::MAX),
            "the end of the timeline always has room"
        );
    }

    #[test]
    fn the_midpoint_sits_between_the_neighbors() {
        assert_eq!(midpoint(0.0, 1024.0), 512.0);
        assert_eq!(midpoint(10.0, 10.5), 10.25);
        assert_eq!(midpoint(3.0, f64::MAX), 3.0 + POSITION_STEP);
    }
}
