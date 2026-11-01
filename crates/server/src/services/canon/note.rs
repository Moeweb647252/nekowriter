//! Memory-note service (spec/06 §1.5, §2, §3; spec/05 §6 `add_memory_note` /
//! `retire_memory_note`).
//!
//! Notes hold what the **text** has established: facts that must stay true (including knowledge
//! asymmetries, tagged with the entities involved), world rules the prose has added, and anything
//! else worth remembering. Notes are **retired**, never deleted, when the text overturns them; the
//! change note says why (spec/06 §1.5).

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};

use crate::api::dto::canon::{MemoryNoteDto, MemoryNoteInput, MemoryNotePatch, MemoryNoteQuery};
use crate::db::entities::memory_notes;
use crate::db::json::{ChangeOp, ChangeTargetType, NoteStatus};
use crate::events::ServerEvent;

use super::{
    CanonError, CanonResult, CanonWrite, ChangeWrite, Visibility, changed_event, note_snapshot,
    stamp,
};

/// `GET /api/books/:id/memory-notes?kind&status` (spec/10 §2.4).
pub async fn list(
    conn: &impl ConnectionTrait,
    book_id: i32,
    query: &MemoryNoteQuery,
) -> CanonResult<Vec<MemoryNoteDto>> {
    super::super::books::get(conn, book_id).await?;
    // Shared rows by default (spec/06 §2); an isolated track's private notes are read through
    // the track's own views.
    let mut statement = memory_notes::Entity::find()
        .filter(memory_notes::COLUMN.book_id.eq(book_id))
        .filter(Visibility::shared(book_id).visible_condition(memory_notes::Column::TrackId))
        .order_by_asc(memory_notes::COLUMN.id);
    if let Some(kind) = query.kind {
        statement = statement.filter(memory_notes::COLUMN.kind.eq(kind.as_str()));
    }
    if let Some(status) = query.status {
        statement = statement.filter(memory_notes::COLUMN.status.eq(status.as_str()));
    }
    let rows = statement.all(conn).await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(to_dto(conn, &row).await?);
    }
    Ok(items)
}

/// `GET /api/memory-notes/:id`.
pub async fn get(conn: &impl ConnectionTrait, note_id: i32) -> CanonResult<MemoryNoteDto> {
    let row = load(conn, note_id).await?;
    to_dto(conn, &row).await
}

/// `POST /api/books/:id/memory-notes` (spec/10 §2.4).
pub async fn create(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    book_id: i32,
    input: MemoryNoteInput,
) -> CanonResult<MemoryNoteDto> {
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
    input: MemoryNoteInput,
) -> CanonResult<(MemoryNoteDto, Vec<ServerEvent>)> {
    super::super::books::get(tx, book_id).await?;
    let content = input.content.trim();
    if content.is_empty() {
        return Err(CanonError::field("content", "a memory note needs content"));
    }
    super::validate_importance(input.importance)?;
    ensure_entities_visible(tx, write, book_id, &input.entity_ids).await?;
    let timestamp = stamp();
    // A recreated row takes a NEW id, never the deleted one's reused rowid (spec/02 §5.10).
    let new_id = super::fresh_id(tx, "memory_notes", ChangeTargetType::MemoryNote).await?;
    let model = memory_notes::ActiveModel {
        id: Set(new_id),
        book_id: Set(book_id),
        track_id: Set(write.new_row_track()),
        kind: Set(input.kind),
        content: Set(content.to_owned()),
        chapter_id: Set(input.chapter_id.or(write.chapter_id)),
        importance: Set(input.importance),
        status: Set(NoteStatus::Active),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
    };
    let row = memory_notes::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    let entity_ids = unique_ids(&input.entity_ids);
    super::set_linked_entities(tx, super::LinkTable::MemoryNote, row.id, &entity_ids).await?;
    let after = note_snapshot(&row, entity_ids)?;
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id,
            target_type: ChangeTargetType::MemoryNote,
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
        ChangeTargetType::MemoryNote,
        row.id,
        ChangeOp::Create,
        change_id,
    )];
    Ok((to_dto(tx, &row).await?, events))
}

/// `PATCH /api/memory-notes/:id` (spec/10 §2.4).
pub async fn update(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    note_id: i32,
    patch: MemoryNotePatch,
) -> CanonResult<MemoryNoteDto> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { update_inner(tx, &CanonWrite::author(), note_id, patch).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn update_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    note_id: i32,
    patch: MemoryNotePatch,
) -> CanonResult<(MemoryNoteDto, Vec<ServerEvent>)> {
    let existing = load(tx, note_id).await?;
    write.ensure_writable(existing.track_id, &format!("memory note #{}", existing.id))?;
    if let Some(importance) = patch.importance {
        super::validate_importance(importance)?;
    }
    if let Some(entity_ids) = &patch.entity_ids {
        ensure_entities_visible(tx, write, existing.book_id, entity_ids).await?;
    }
    let current_ids = super::linked_entity_ids(tx, super::LinkTable::MemoryNote, note_id).await?;
    let before = note_snapshot(&existing, current_ids)?;
    let mut active = memory_notes::ActiveModel {
        id: Set(note_id),
        updated_at: Set(stamp()),
        ..Default::default()
    };
    if let Some(kind) = patch.kind {
        active.kind = Set(kind);
    }
    if let Some(content) = &patch.content {
        let content = content.trim();
        if content.is_empty() {
            return Err(CanonError::field("content", "a memory note needs content"));
        }
        active.content = Set(content.to_owned());
    }
    if let Some(chapter_id) = patch.chapter_id {
        active.chapter_id = Set(chapter_id);
    }
    if let Some(importance) = patch.importance {
        active.importance = Set(importance);
    }
    if let Some(status) = patch.status {
        active.status = Set(status);
    }
    active.update(tx).await?;
    if let Some(entity_ids) = &patch.entity_ids {
        super::set_linked_entities(
            tx,
            super::LinkTable::MemoryNote,
            note_id,
            &unique_ids(entity_ids),
        )
        .await?;
    }
    let row = load(tx, note_id).await?;
    let entity_ids = super::linked_entity_ids(tx, super::LinkTable::MemoryNote, note_id).await?;
    let after = note_snapshot(&row, entity_ids)?;
    if before == after {
        return Ok((to_dto(tx, &row).await?, Vec::new()));
    }
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id: row.book_id,
            target_type: ChangeTargetType::MemoryNote,
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
        ChangeTargetType::MemoryNote,
        row.id,
        ChangeOp::Update,
        change_id,
    )];
    Ok((to_dto(tx, &row).await?, events))
}

/// `DELETE /api/memory-notes/:id`.
pub async fn delete(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    note_id: i32,
) -> CanonResult<()> {
    let (_, published) = super::write(db, move |tx| {
        Box::pin(async move {
            let events = delete_inner(tx, &CanonWrite::author(), note_id).await?;
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
    note_id: i32,
) -> CanonResult<Vec<ServerEvent>> {
    let existing = load(tx, note_id).await?;
    write.ensure_writable(existing.track_id, &format!("memory note #{}", existing.id))?;
    let ids = super::linked_entity_ids(tx, super::LinkTable::MemoryNote, note_id).await?;
    let before = note_snapshot(&existing, ids)?;
    memory_notes::Entity::delete_by_id(note_id).exec(tx).await?;
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id: existing.book_id,
            target_type: ChangeTargetType::MemoryNote,
            target_id: note_id,
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
        ChangeTargetType::MemoryNote,
        note_id,
        ChangeOp::Delete,
        change_id,
    )])
}

/// `retire_memory_note` (spec/05 §6): status → retired, the reason as the change note.
pub async fn retire_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    note_id: i32,
    reason: &str,
) -> CanonResult<(MemoryNoteDto, Vec<ServerEvent>)> {
    let existing = load(tx, note_id).await?;
    write.ensure_writable(existing.track_id, &format!("memory note #{}", existing.id))?;
    if existing.status == NoteStatus::Retired {
        return Ok((to_dto(tx, &existing).await?, Vec::new()));
    }
    let patch = MemoryNotePatch {
        status: Some(NoteStatus::Retired),
        note: Some(reason.to_owned()),
        ..MemoryNotePatch::default()
    };
    update_inner(tx, write, note_id, patch).await
}

/// Loads one note or reports `not_found`.
pub async fn load(conn: &impl ConnectionTrait, note_id: i32) -> CanonResult<memory_notes::Model> {
    memory_notes::Entity::find_by_id(note_id)
        .one(conn)
        .await?
        .ok_or_else(|| CanonError::not_found(format!("memory note #{note_id}")))
}

/// Maps a row to its DTO, including its entity links.
pub async fn to_dto(
    conn: &impl ConnectionTrait,
    row: &memory_notes::Model,
) -> CanonResult<MemoryNoteDto> {
    Ok(MemoryNoteDto {
        id: row.id,
        book_id: row.book_id,
        track_id: row.track_id,
        kind: row.kind,
        content: row.content.clone(),
        chapter_id: row.chapter_id,
        importance: row.importance,
        status: row.status,
        entity_ids: super::linked_entity_ids(conn, super::LinkTable::MemoryNote, row.id).await?,
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

/// Restores a memory note from a `before` snapshot (the revert of an update).
pub async fn restore_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    note_id: i32,
    before: &crate::db::json::Snapshot,
) -> CanonResult<crate::db::json::Snapshot> {
    let crate::db::json::Snapshot::Note {
        kind,
        content,
        chapter_id,
        importance,
        status,
        entity_ids,
        ..
    } = before
    else {
        return Err(CanonError::internal("a note revert needs a note snapshot"));
    };
    let existing = load(tx, note_id).await?;
    write.ensure_writable(existing.track_id, &format!("memory note #{}", existing.id))?;
    memory_notes::ActiveModel {
        id: Set(note_id),
        kind: Set(*kind),
        content: Set(content.clone()),
        chapter_id: Set(*chapter_id),
        importance: Set(*importance),
        status: Set(*status),
        updated_at: Set(stamp()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    let ids = unique_ids(entity_ids);
    super::set_linked_entities(tx, super::LinkTable::MemoryNote, note_id, &ids).await?;
    let row = load(tx, note_id).await?;
    note_snapshot(&row, ids)
}

/// Recreates a deleted memory note from its `before` snapshot, with a **new id**.
pub async fn recreate_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    before: &crate::db::json::Snapshot,
) -> CanonResult<(i32, crate::db::json::Snapshot)> {
    let crate::db::json::Snapshot::Note {
        kind,
        content,
        chapter_id,
        importance,
        status,
        entity_ids,
        track_id,
    } = before
    else {
        return Err(CanonError::internal("a note revert needs a note snapshot"));
    };
    let book_id = write
        .visibility
        .map(|visibility| visibility.book_id)
        .ok_or_else(|| CanonError::internal("a note revert needs a book"))?;
    super::super::books::get(tx, book_id).await?;
    let timestamp = stamp();
    // A recreated row takes a NEW id, never the deleted one's reused rowid (spec/02 §5.10).
    let new_id = super::fresh_id(tx, "memory_notes", ChangeTargetType::MemoryNote).await?;
    let model = memory_notes::ActiveModel {
        id: Set(new_id),
        book_id: Set(book_id),
        track_id: Set(track_id.or_else(|| write.new_row_track())),
        kind: Set(*kind),
        content: Set(content.clone()),
        chapter_id: Set(*chapter_id),
        importance: Set(*importance),
        status: Set(*status),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
    };
    let row = memory_notes::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    let restored = super::existing_entity_ids(tx, &unique_ids(entity_ids)).await?;
    super::set_linked_entities(tx, super::LinkTable::MemoryNote, row.id, &restored).await?;
    Ok((row.id, note_snapshot(&row, restored)?))
}
