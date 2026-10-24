//! Relation service (spec/06 §1.2, §2, §3; spec/05 §6 `set_relation` / `end_relation`).
//!
//! Relations are **directed**: `kind` says what the source is to the target ("林凡 是 张三 的 师兄"),
//! and `inverse_kind` says what the target is to the source ("师弟"). Several relations of
//! different kinds may exist between one pair, but only one **active** relation per
//! `(source, target, kind)` in shared canon — and one per isolated track (the partial unique
//! indexes of `0001_init.sql`). Ending a relation keeps the row (`status = ended`,
//! `until_chapter_id`) so history stays queryable.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, Set,
};

use crate::api::dto::canon::{RelationDto, RelationInput, RelationPatch, RelationQuery};
use crate::db::entities::relations;
use crate::db::json::{ChangeOp, ChangeTargetType, RelationStatus};
use crate::events::ServerEvent;

use super::{
    CanonError, CanonResult, CanonWrite, ChangeWrite, changed_event, relation_snapshot, stamp,
};

/// `GET /api/books/:id/relations?entity_id=` (spec/10 §2.4). Whole lists, sorted client-side.
pub async fn list(
    conn: &impl ConnectionTrait,
    book_id: i32,
    query: &RelationQuery,
) -> CanonResult<Vec<RelationDto>> {
    super::super::books::get(conn, book_id).await?;
    let mut statement = relations::Entity::find()
        .filter(relations::Column::BookId.eq(book_id))
        .order_by_asc(relations::Column::Id);
    if let Some(entity_id) = query.entity_id {
        statement = statement.filter(
            Condition::any()
                .add(relations::Column::SourceId.eq(entity_id))
                .add(relations::Column::TargetId.eq(entity_id)),
        );
    }
    let rows = statement.all(conn).await?;
    rows.iter().map(to_dto).collect()
}

/// `GET /api/relations/:id`.
pub async fn get(conn: &impl ConnectionTrait, relation_id: i32) -> CanonResult<RelationDto> {
    to_dto(&load(conn, relation_id).await?)
}

/// `POST /api/books/:id/relations` (spec/10 §2.4).
pub async fn create(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    book_id: i32,
    input: RelationInput,
) -> CanonResult<RelationDto> {
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
    input: RelationInput,
) -> CanonResult<(RelationDto, Vec<ServerEvent>)> {
    super::super::books::get(tx, book_id).await?;
    let kind = input.kind.trim();
    if kind.is_empty() {
        return Err(CanonError::field("kind", "a relation needs a kind"));
    }
    if input.source_id == input.target_id {
        return Err(CanonError::field(
            "target_id",
            "a relation needs two different entities",
        ));
    }
    let source = super::load_entity(tx, input.source_id).await?;
    let target = super::load_entity(tx, input.target_id).await?;
    if source.book_id != book_id || target.book_id != book_id {
        return Err(CanonError::validation(
            "both entities must belong to this book",
        ));
    }
    let track_id = write.new_row_track();
    // Both endpoints of a private relation must be visible to the same track (spec/06 §2).
    if let Some(visibility) = write.visibility {
        visibility.ensure_visible(source.track_id)?;
        visibility.ensure_visible(target.track_id)?;
        visibility.ensure_writable(
            source.track_id,
            &super::entity::entity_label(&source.name, source.id),
        )?;
        visibility.ensure_writable(
            target.track_id,
            &super::entity::entity_label(&target.name, target.id),
        )?;
    }
    ensure_no_active_duplicate(tx, source.book_id, track_id, &input).await?;
    let timestamp = stamp();
    let model = relations::ActiveModel {
        book_id: Set(book_id),
        track_id: Set(track_id),
        source_id: Set(input.source_id),
        target_id: Set(input.target_id),
        kind: Set(kind.to_owned()),
        inverse_kind: Set(input.inverse_kind.trim().to_owned()),
        description: Set(input.description.trim().to_owned()),
        status: Set(RelationStatus::Active.as_str().to_owned()),
        since_chapter_id: Set(input.since_chapter_id),
        until_chapter_id: Set(input.until_chapter_id),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    let row = relations::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    let after = relation_snapshot(&row)?;
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id,
            target_type: ChangeTargetType::Relation,
            target_id: row.id,
            op: ChangeOp::Create,
            before: None,
            after: Some(&after),
            note: input.note.trim(),
            reverts_id: None,
            track_id: row.track_id,
            chapter_id: None,
        },
    )
    .await?;
    let events = vec![changed_event(
        book_id,
        ChangeTargetType::Relation,
        row.id,
        ChangeOp::Create,
        change_id,
    )];
    Ok((to_dto(&row)?, events))
}

/// `PATCH /api/relations/:id` (spec/10 §2.4).
pub async fn update(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    relation_id: i32,
    patch: RelationPatch,
) -> CanonResult<RelationDto> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { update_inner(tx, &CanonWrite::author(), relation_id, patch).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn update_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    relation_id: i32,
    patch: RelationPatch,
) -> CanonResult<(RelationDto, Vec<ServerEvent>)> {
    let existing = load(tx, relation_id).await?;
    write.ensure_writable(existing.track_id, &format!("relation #{}", existing.id))?;
    let before = relation_snapshot(&existing)?;
    let mut active = relations::ActiveModel {
        id: Set(relation_id),
        updated_at: Set(stamp()),
        ..Default::default()
    };
    if let Some(kind) = &patch.kind {
        let kind = kind.trim();
        if kind.is_empty() {
            return Err(CanonError::field("kind", "a relation needs a kind"));
        }
        active.kind = Set(kind.to_owned());
    }
    if let Some(inverse_kind) = &patch.inverse_kind {
        active.inverse_kind = Set(inverse_kind.trim().to_owned());
    }
    if let Some(description) = &patch.description {
        active.description = Set(description.trim().to_owned());
    }
    if let Some(status) = patch.status {
        active.status = Set(status.as_str().to_owned());
    }
    if let Some(since) = patch.since_chapter_id {
        active.since_chapter_id = Set(since);
    }
    if let Some(until) = patch.until_chapter_id {
        active.until_chapter_id = Set(until);
    }
    active.update(tx).await?;
    let row = load(tx, relation_id).await?;
    let after = relation_snapshot(&row)?;
    if before == after {
        return Ok((to_dto(&row)?, Vec::new()));
    }
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id: row.book_id,
            target_type: ChangeTargetType::Relation,
            target_id: row.id,
            op: ChangeOp::Update,
            before: Some(&before),
            after: Some(&after),
            note: patch.note.trim(),
            reverts_id: None,
            track_id: row.track_id,
            chapter_id: None,
        },
    )
    .await?;
    let events = vec![changed_event(
        row.book_id,
        ChangeTargetType::Relation,
        row.id,
        ChangeOp::Update,
        change_id,
    )];
    Ok((to_dto(&row)?, events))
}

/// `DELETE /api/relations/:id`: one `delete` change with the full snapshot.
pub async fn delete(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    relation_id: i32,
) -> CanonResult<()> {
    let (_, published) = super::write(db, move |tx| {
        Box::pin(async move {
            let events = delete_inner(tx, &CanonWrite::author(), relation_id).await?;
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
    relation_id: i32,
) -> CanonResult<Vec<ServerEvent>> {
    let existing = load(tx, relation_id).await?;
    write.ensure_writable(existing.track_id, &format!("relation #{}", existing.id))?;
    let before = relation_snapshot(&existing)?;
    relations::Entity::delete_by_id(relation_id)
        .exec(tx)
        .await?;
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id: existing.book_id,
            target_type: ChangeTargetType::Relation,
            target_id: relation_id,
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
        ChangeTargetType::Relation,
        relation_id,
        ChangeOp::Delete,
        change_id,
    )])
}

// ───────────────────────────── tool-level upsert / end (spec/05 §6) ─────────────────────────────

/// The identity of a relation a tool names: what the source is to the target (spec/06 §1.2).
#[derive(Debug, Clone, Copy)]
pub struct RelationKey<'a> {
    pub source_id: i32,
    pub target_id: i32,
    pub kind: &'a str,
}

/// The written details of a relation upsert (spec/05 §6 `set_relation`).
#[derive(Debug, Clone, Copy)]
pub struct RelationDetails<'a> {
    pub inverse_kind: &'a str,
    pub description: &'a str,
    /// The author/agent change note; empty when there is none.
    pub note: &'a str,
}

/// `set_relation` (spec/05 §6): an upsert of the visible active relation `(source, target, kind)`.
///
/// In an isolated track an existing **shared** relation is read-only (`error.isolated_readonly`)
/// and a new one becomes a private row. `since_chapter_id` is the session chapter on create.
pub async fn set_relation_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    book_id: i32,
    key: RelationKey<'_>,
    details: RelationDetails<'_>,
) -> CanonResult<(RelationDto, Vec<ServerEvent>)> {
    let source_id = key.source_id;
    let target_id = key.target_id;
    let kind = key.kind.trim();
    let inverse_kind = details.inverse_kind;
    let description = details.description;
    let note = details.note;
    if kind.is_empty() {
        return Err(CanonError::field("kind", "a relation needs a kind"));
    }
    let existing = find_active(tx, book_id, write, source_id, target_id, kind).await?;
    match existing {
        Some(row) => {
            if !write
                .ensure_writable(row.track_id, &relation_label(source_id, target_id, kind))
                .is_ok()
            {
                return Err(CanonError::isolated(relation_label(
                    source_id, target_id, kind,
                )));
            }
            let patch = RelationPatch {
                inverse_kind: if inverse_kind.is_empty() {
                    None
                } else {
                    Some(inverse_kind.to_owned())
                },
                description: if description.is_empty() {
                    None
                } else {
                    Some(description.to_owned())
                },
                note: note.to_owned(),
                ..RelationPatch::default()
            };
            let (dto, events) = update_inner(tx, write, row.id, patch).await?;
            Ok((dto, events))
        }
        None => {
            let input = RelationInput {
                source_id,
                target_id,
                kind: kind.to_owned(),
                inverse_kind: inverse_kind.to_owned(),
                description: description.to_owned(),
                since_chapter_id: write.chapter_id,
                until_chapter_id: None,
                note: note.to_owned(),
            };
            create_inner(tx, write, book_id, input).await
        }
    }
}

/// `end_relation` (spec/05 §6): `status → ended`, `until_chapter_id` = the session chapter.
pub async fn end_relation_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    book_id: i32,
    key: RelationKey<'_>,
    note: &str,
) -> CanonResult<(RelationDto, Vec<ServerEvent>)> {
    let source_id = key.source_id;
    let target_id = key.target_id;
    let kind = key.kind.trim();
    let row = find_active(tx, book_id, write, source_id, target_id, kind)
        .await?
        .ok_or_else(|| CanonError::not_found(relation_label(source_id, target_id, kind)))?;
    write.ensure_writable(row.track_id, &relation_label(source_id, target_id, kind))?;
    let patch = RelationPatch {
        status: Some(RelationStatus::Ended),
        until_chapter_id: Some(write.chapter_id),
        note: note.to_owned(),
        ..RelationPatch::default()
    };
    update_inner(tx, write, row.id, patch).await
}

/// Loads one relation row or reports `not_found`.
pub async fn load(conn: &impl ConnectionTrait, relation_id: i32) -> CanonResult<relations::Model> {
    relations::Entity::find_by_id(relation_id)
        .one(conn)
        .await?
        .ok_or_else(|| CanonError::not_found(format!("relation #{relation_id}")))
}

/// The visible active relation `(source, target, kind)` of a writer (spec/06 §1.2 uniqueness).
pub async fn find_active(
    conn: &impl ConnectionTrait,
    book_id: i32,
    write: &CanonWrite,
    source_id: i32,
    target_id: i32,
    kind: &str,
) -> CanonResult<Option<relations::Model>> {
    let mut statement = relations::Entity::find()
        .filter(relations::Column::BookId.eq(book_id))
        .filter(relations::Column::SourceId.eq(source_id))
        .filter(relations::Column::TargetId.eq(target_id))
        .filter(relations::Column::Kind.eq(kind))
        .filter(relations::Column::Status.eq(RelationStatus::Active.as_str()));
    if let Some(condition) = write.condition(relations::Column::TrackId) {
        statement = statement.filter(condition);
    }
    // A shared row wins over a private one: it is the canon relation (spec/06 §2).
    let rows = statement
        .order_by_asc(relations::Column::TrackId)
        .all(conn)
        .await?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    Ok(Some(row))
}

/// Rejects a second active relation of the same shape in the same canon (spec/06 §1.2).
async fn ensure_no_active_duplicate(
    tx: &impl ConnectionTrait,
    book_id: i32,
    track_id: Option<i32>,
    input: &RelationInput,
) -> CanonResult<()> {
    let mut statement = relations::Entity::find()
        .filter(relations::Column::BookId.eq(book_id))
        .filter(relations::Column::SourceId.eq(input.source_id))
        .filter(relations::Column::TargetId.eq(input.target_id))
        .filter(relations::Column::Kind.eq(input.kind.trim()))
        .filter(relations::Column::Status.eq(RelationStatus::Active.as_str()));
    statement = match track_id {
        Some(track_id) => statement.filter(relations::Column::TrackId.eq(track_id)),
        None => statement.filter(relations::Column::TrackId.is_null()),
    };
    if let Some(row) = statement.one(tx).await? {
        // A new active relation of the same (source, target, kind) is not a duplicate error in
        // the API: the author edits the existing one (spec/06 §1.2 "Only one active relation").
        return Err(super::conflict(
            format!(
                "an active relation of this kind already exists (#{}); edit it instead",
                row.id
            ),
            row.id,
        ));
    }
    Ok(())
}

/// `kind` from the perspective of a pair, for an error message.
fn relation_label(source_id: i32, target_id: i32, kind: &str) -> String {
    format!("#{source_id} → #{target_id} ({kind})")
}

/// Maps a row to its DTO.
pub fn to_dto(row: &relations::Model) -> CanonResult<RelationDto> {
    Ok(RelationDto {
        id: row.id,
        book_id: row.book_id,
        track_id: row.track_id,
        source_id: row.source_id,
        target_id: row.target_id,
        kind: row.kind.clone(),
        inverse_kind: row.inverse_kind.clone(),
        description: row.description.clone(),
        status: super::enum_of("relations.status", &row.status)?,
        since_chapter_id: row.since_chapter_id,
        until_chapter_id: row.until_chapter_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

// ───────────────────────────── revert support (spec/06 §3) ─────────────────────────────

/// The current snapshot of a relation, when it still exists.
pub async fn snapshot_now(
    conn: &impl ConnectionTrait,
    relation_id: i32,
) -> CanonResult<Option<crate::db::json::Snapshot>> {
    match relations::Entity::find_by_id(relation_id).one(conn).await? {
        Some(row) => Ok(Some(relation_snapshot(&row)?)),
        None => Ok(None),
    }
}

/// Restores a relation from a `before` snapshot (the revert of an update).
pub async fn restore_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    relation_id: i32,
    before: &crate::db::json::Snapshot,
) -> CanonResult<crate::db::json::Snapshot> {
    let crate::db::json::Snapshot::Relation {
        source_id,
        target_id,
        kind,
        inverse_kind,
        description,
        status,
        since_chapter_id,
        until_chapter_id,
        ..
    } = before
    else {
        return Err(CanonError::internal(
            "a relation revert needs a relation snapshot",
        ));
    };
    let existing = load(tx, relation_id).await?;
    write.ensure_writable(existing.track_id, &format!("relation #{}", existing.id))?;
    relations::ActiveModel {
        id: Set(relation_id),
        source_id: Set(*source_id),
        target_id: Set(*target_id),
        kind: Set(kind.clone()),
        inverse_kind: Set(inverse_kind.clone()),
        description: Set(description.clone()),
        status: Set(status.as_str().to_owned()),
        since_chapter_id: Set(*since_chapter_id),
        until_chapter_id: Set(*until_chapter_id),
        updated_at: Set(stamp()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    let row = load(tx, relation_id).await?;
    relation_snapshot(&row)
}

/// Recreates a deleted relation from its `before` snapshot, with a **new id** (spec/02 §5.10).
///
/// Both endpoints must still exist; otherwise the revert fails with `not_found` rather than
/// resurrecting a dangling relation.
pub async fn recreate_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    before: &crate::db::json::Snapshot,
) -> CanonResult<(i32, crate::db::json::Snapshot)> {
    let crate::db::json::Snapshot::Relation {
        source_id,
        target_id,
        kind,
        inverse_kind,
        description,
        status,
        since_chapter_id,
        until_chapter_id,
        track_id,
    } = before
    else {
        return Err(CanonError::internal(
            "a relation revert needs a relation snapshot",
        ));
    };
    let source = super::load_entity(tx, *source_id).await?;
    let target = super::load_entity(tx, *target_id).await?;
    let track_id = track_id.or_else(|| write.new_row_track());
    let timestamp = stamp();
    let model = relations::ActiveModel {
        book_id: Set(source.book_id),
        track_id: Set(track_id),
        source_id: Set(*source_id),
        target_id: Set(*target_id),
        kind: Set(kind.clone()),
        inverse_kind: Set(inverse_kind.clone()),
        description: Set(description.clone()),
        status: Set(status.as_str().to_owned()),
        since_chapter_id: Set(*since_chapter_id),
        until_chapter_id: Set(*until_chapter_id),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    let row = relations::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    let _ = target;
    Ok((row.id, relation_snapshot(&row)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_relation_label_names_both_ends_and_the_kind() {
        assert_eq!(relation_label(12, 15, "师兄"), "#12 → #15 (师兄)");
    }
}
