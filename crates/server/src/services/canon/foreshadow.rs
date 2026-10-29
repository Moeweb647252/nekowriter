//! Foreshadowing service (spec/06 §1.5, §2, §3; spec/05 §6 `add_foreshadowing` /
//! `resolve_foreshadowing`).
//!
//! A foreshadowing item is `open` → `resolved` (with the resolution and the resolving chapter) or
//! `abandoned`. `planted_chapter_id` is where the hook was planted and `resolved_chapter_id` where
//! it paid off; both are chapter references, so a chapter delete nulls them without touching the
//! item (spec/02 §2).

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};

use crate::api::dto::canon::{
    ForeshadowingDto, ForeshadowingInput, ForeshadowingPatch, ForeshadowingQuery,
};
use crate::db::entities::foreshadowing;
use crate::db::json::{ChangeOp, ChangeTargetType, ForeshadowingStatus};
use crate::events::ServerEvent;

use super::{
    CanonError, CanonResult, CanonWrite, ChangeWrite, Visibility, changed_event,
    foreshadowing_snapshot, stamp,
};

/// `GET /api/books/:id/foreshadowing?status` (spec/10 §2.4).
pub async fn list(
    conn: &impl ConnectionTrait,
    book_id: i32,
    query: &ForeshadowingQuery,
) -> CanonResult<Vec<ForeshadowingDto>> {
    super::super::books::get(conn, book_id).await?;
    // Shared rows by default (spec/06 §2); an isolated track's private hooks are read through
    // the track's own views.
    let mut statement = foreshadowing::Entity::find()
        .filter(foreshadowing::COLUMN.book_id.eq(book_id))
        .filter(Visibility::shared(book_id).visible_condition(foreshadowing::Column::TrackId))
        .order_by_asc(foreshadowing::COLUMN.id);
    if let Some(status) = query.status {
        statement = statement.filter(foreshadowing::COLUMN.status.eq(status.as_str()));
    }
    let rows = statement.all(conn).await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(to_dto(conn, &row).await?);
    }
    Ok(items)
}

/// `GET /api/foreshadowing/:id`.
pub async fn get(conn: &impl ConnectionTrait, item_id: i32) -> CanonResult<ForeshadowingDto> {
    let row = load(conn, item_id).await?;
    to_dto(conn, &row).await
}

/// `POST /api/books/:id/foreshadowing` (spec/10 §2.4).
pub async fn create(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    book_id: i32,
    input: ForeshadowingInput,
) -> CanonResult<ForeshadowingDto> {
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
    input: ForeshadowingInput,
) -> CanonResult<(ForeshadowingDto, Vec<ServerEvent>)> {
    super::super::books::get(tx, book_id).await?;
    let title = input.title.trim();
    if title.is_empty() {
        return Err(CanonError::field(
            "title",
            "a foreshadowing item needs a title",
        ));
    }
    super::validate_importance(input.importance)?;
    ensure_entities_visible(tx, write, book_id, &input.entity_ids).await?;
    let timestamp = stamp();
    // A recreated row takes a NEW id, never the deleted one's reused rowid (spec/02 §5.10).
    let new_id = super::fresh_id(tx, "foreshadowing", ChangeTargetType::Foreshadowing).await?;
    let model = foreshadowing::ActiveModel {
        id: Set(new_id),
        book_id: Set(book_id),
        track_id: Set(write.new_row_track()),
        title: Set(title.to_owned()),
        description: Set(input.description.trim().to_owned()),
        planned_payoff: Set(input.planned_payoff.trim().to_owned()),
        status: Set(ForeshadowingStatus::Open),
        planted_chapter_id: Set(input.planted_chapter_id.or(write.chapter_id)),
        resolved_chapter_id: Set(None),
        resolution: Set(String::new()),
        importance: Set(input.importance),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
    };
    let row = foreshadowing::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    let entity_ids = unique_ids(&input.entity_ids);
    super::set_linked_entities(tx, super::LinkTable::Foreshadowing, row.id, &entity_ids).await?;
    let after = foreshadowing_snapshot(&row, entity_ids)?;
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id,
            target_type: ChangeTargetType::Foreshadowing,
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
        ChangeTargetType::Foreshadowing,
        row.id,
        ChangeOp::Create,
        change_id,
    )];
    Ok((to_dto(tx, &row).await?, events))
}

/// `PATCH /api/foreshadowing/:id` (spec/10 §2.4).
pub async fn update(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    item_id: i32,
    patch: ForeshadowingPatch,
) -> CanonResult<ForeshadowingDto> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { update_inner(tx, &CanonWrite::author(), item_id, patch).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn update_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    item_id: i32,
    patch: ForeshadowingPatch,
) -> CanonResult<(ForeshadowingDto, Vec<ServerEvent>)> {
    let existing = load(tx, item_id).await?;
    write.ensure_writable(
        existing.track_id,
        &format!("foreshadowing #{}", existing.id),
    )?;
    if let Some(importance) = patch.importance {
        super::validate_importance(importance)?;
    }
    if let Some(entity_ids) = &patch.entity_ids {
        ensure_entities_visible(tx, write, existing.book_id, entity_ids).await?;
    }
    let current_ids =
        super::linked_entity_ids(tx, super::LinkTable::Foreshadowing, item_id).await?;
    let before = foreshadowing_snapshot(&existing, current_ids)?;
    let mut active = foreshadowing::ActiveModel {
        id: Set(item_id),
        updated_at: Set(stamp()),
        ..Default::default()
    };
    if let Some(title) = &patch.title {
        let title = title.trim();
        if title.is_empty() {
            return Err(CanonError::field(
                "title",
                "a foreshadowing item needs a title",
            ));
        }
        active.title = Set(title.to_owned());
    }
    if let Some(description) = &patch.description {
        active.description = Set(description.trim().to_owned());
    }
    if let Some(planned_payoff) = &patch.planned_payoff {
        active.planned_payoff = Set(planned_payoff.trim().to_owned());
    }
    if let Some(status) = patch.status {
        active.status = Set(status);
    }
    if let Some(planted) = patch.planted_chapter_id {
        active.planted_chapter_id = Set(planted);
    }
    if let Some(resolved) = patch.resolved_chapter_id {
        active.resolved_chapter_id = Set(resolved);
    }
    if let Some(resolution) = &patch.resolution {
        active.resolution = Set(resolution.trim().to_owned());
    }
    if let Some(importance) = patch.importance {
        active.importance = Set(importance);
    }
    active.update(tx).await?;
    if let Some(entity_ids) = &patch.entity_ids {
        super::set_linked_entities(
            tx,
            super::LinkTable::Foreshadowing,
            item_id,
            &unique_ids(entity_ids),
        )
        .await?;
    }
    let row = load(tx, item_id).await?;
    let entity_ids = super::linked_entity_ids(tx, super::LinkTable::Foreshadowing, item_id).await?;
    let after = foreshadowing_snapshot(&row, entity_ids)?;
    if before == after {
        return Ok((to_dto(tx, &row).await?, Vec::new()));
    }
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id: row.book_id,
            target_type: ChangeTargetType::Foreshadowing,
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
        ChangeTargetType::Foreshadowing,
        row.id,
        ChangeOp::Update,
        change_id,
    )];
    Ok((to_dto(tx, &row).await?, events))
}

/// `DELETE /api/foreshadowing/:id`.
pub async fn delete(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    item_id: i32,
) -> CanonResult<()> {
    let (_, published) = super::write(db, move |tx| {
        Box::pin(async move {
            let events = delete_inner(tx, &CanonWrite::author(), item_id).await?;
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
    item_id: i32,
) -> CanonResult<Vec<ServerEvent>> {
    let existing = load(tx, item_id).await?;
    write.ensure_writable(
        existing.track_id,
        &format!("foreshadowing #{}", existing.id),
    )?;
    let ids = super::linked_entity_ids(tx, super::LinkTable::Foreshadowing, item_id).await?;
    let before = foreshadowing_snapshot(&existing, ids)?;
    foreshadowing::Entity::delete_by_id(item_id)
        .exec(tx)
        .await?;
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id: existing.book_id,
            target_type: ChangeTargetType::Foreshadowing,
            target_id: item_id,
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
        ChangeTargetType::Foreshadowing,
        item_id,
        ChangeOp::Delete,
        change_id,
    )])
}

/// `resolve_foreshadowing` (spec/05 §6): `status` parsed from `resolved` / `abandoned`, the
/// resolution recorded, `resolved_chapter_id` = the session chapter.
pub async fn resolve_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    item_id: i32,
    status: ForeshadowingStatus,
    resolution: &str,
) -> CanonResult<(ForeshadowingDto, Vec<ServerEvent>)> {
    let patch = ForeshadowingPatch {
        status: Some(status),
        resolution: Some(resolution.to_owned()),
        resolved_chapter_id: Some(write.chapter_id),
        note: Some(resolution.to_owned()),
        ..ForeshadowingPatch::default()
    };
    update_inner(tx, write, item_id, patch).await
}

/// Loads one item or reports `not_found`.
pub async fn load(conn: &impl ConnectionTrait, item_id: i32) -> CanonResult<foreshadowing::Model> {
    foreshadowing::Entity::find_by_id(item_id)
        .one(conn)
        .await?
        .ok_or_else(|| CanonError::not_found(format!("foreshadowing #{item_id}")))
}

/// Maps a row to its DTO, including its entity links.
pub async fn to_dto(
    conn: &impl ConnectionTrait,
    row: &foreshadowing::Model,
) -> CanonResult<ForeshadowingDto> {
    Ok(ForeshadowingDto {
        id: row.id,
        book_id: row.book_id,
        track_id: row.track_id,
        title: row.title.clone(),
        description: row.description.clone(),
        planned_payoff: row.planned_payoff.clone(),
        status: row.status,
        planted_chapter_id: row.planted_chapter_id,
        resolved_chapter_id: row.resolved_chapter_id,
        resolution: row.resolution.clone(),
        importance: row.importance,
        entity_ids: super::linked_entity_ids(conn, super::LinkTable::Foreshadowing, row.id).await?,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
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
    Ok(())
}

// ───────────────────────────── revert support (spec/06 §3) ─────────────────────────────

/// The current snapshot of a foreshadowing item, when it still exists.
pub async fn snapshot_now(
    conn: &impl ConnectionTrait,
    item_id: i32,
) -> CanonResult<Option<crate::db::json::Snapshot>> {
    match foreshadowing::Entity::find_by_id(item_id).one(conn).await? {
        Some(row) => {
            let ids =
                super::linked_entity_ids(conn, super::LinkTable::Foreshadowing, item_id).await?;
            Ok(Some(foreshadowing_snapshot(&row, ids)?))
        }
        None => Ok(None),
    }
}

/// Restores a foreshadowing item from a `before` snapshot (the revert of an update).
pub async fn restore_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    item_id: i32,
    before: &crate::db::json::Snapshot,
) -> CanonResult<crate::db::json::Snapshot> {
    let crate::db::json::Snapshot::Foreshadowing {
        title,
        description,
        planned_payoff,
        status,
        planted_chapter_id,
        resolved_chapter_id,
        resolution,
        importance,
        entity_ids,
        ..
    } = before
    else {
        return Err(CanonError::internal(
            "a foreshadowing revert needs a foreshadowing snapshot",
        ));
    };
    let existing = load(tx, item_id).await?;
    write.ensure_writable(
        existing.track_id,
        &format!("foreshadowing #{}", existing.id),
    )?;
    foreshadowing::ActiveModel {
        id: Set(item_id),
        title: Set(title.clone()),
        description: Set(description.clone()),
        planned_payoff: Set(planned_payoff.clone()),
        status: Set(*status),
        planted_chapter_id: Set(*planted_chapter_id),
        resolved_chapter_id: Set(*resolved_chapter_id),
        resolution: Set(resolution.clone()),
        importance: Set(*importance),
        updated_at: Set(stamp()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    let ids = unique_ids(entity_ids);
    super::set_linked_entities(tx, super::LinkTable::Foreshadowing, item_id, &ids).await?;
    let row = load(tx, item_id).await?;
    foreshadowing_snapshot(&row, ids)
}

/// Recreates a deleted foreshadowing item from its `before` snapshot, with a **new id**.
pub async fn recreate_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    before: &crate::db::json::Snapshot,
) -> CanonResult<(i32, crate::db::json::Snapshot)> {
    let crate::db::json::Snapshot::Foreshadowing {
        title,
        description,
        planned_payoff,
        status,
        planted_chapter_id,
        resolved_chapter_id,
        resolution,
        importance,
        entity_ids,
        track_id,
    } = before
    else {
        return Err(CanonError::internal(
            "a foreshadowing revert needs a foreshadowing snapshot",
        ));
    };
    let book_id = write
        .visibility
        .map(|visibility| visibility.book_id)
        .ok_or_else(|| CanonError::internal("a foreshadowing revert needs a book"))?;
    super::super::books::get(tx, book_id).await?;
    let timestamp = stamp();
    // A recreated row takes a NEW id, never the deleted one's reused rowid (spec/02 §5.10).
    let new_id = super::fresh_id(tx, "foreshadowing", ChangeTargetType::Foreshadowing).await?;
    let model = foreshadowing::ActiveModel {
        id: Set(new_id),
        book_id: Set(book_id),
        track_id: Set(track_id.or_else(|| write.new_row_track())),
        title: Set(title.clone()),
        description: Set(description.clone()),
        planned_payoff: Set(planned_payoff.clone()),
        status: Set(*status),
        planted_chapter_id: Set(*planted_chapter_id),
        resolved_chapter_id: Set(*resolved_chapter_id),
        resolution: Set(resolution.clone()),
        importance: Set(*importance),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
    };
    let row = foreshadowing::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    let restored = super::existing_entity_ids(tx, &unique_ids(entity_ids)).await?;
    super::set_linked_entities(tx, super::LinkTable::Foreshadowing, row.id, &restored).await?;
    Ok((row.id, foreshadowing_snapshot(&row, restored)?))
}
