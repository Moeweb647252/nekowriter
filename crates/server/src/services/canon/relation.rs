//! Relation service (spec/06 §1.2, §2, §3; spec/05 §6 `set_relation` / `end_relation`).
//!
//! Relations are **directed**: `kind` says what the source is to the target (source Lin Fan,
//! target Zhang San, kind "senior fellow disciple"), and `inverse_kind` says what the target is to
//! the source ("junior fellow disciple"). `kind` is **data** about a bond, not part of its
//! identity: at most one **active** relation may exist per ordered `(source, target)` pair in
//! shared canon — and one per isolated track (the partial unique indexes of the schema). A pair
//! whose bond changes wording (`师兄` → `同门师兄`) edits the row it already has; rewording never
//! opens a second active row. The two directions are separate pairs, so `(A, B, 父亲)` and
//! `(B, A, 嫡长子)` are legitimately two rows. Ending a relation keeps the row (`status = ended`,
//! `until_chapter_id`) so history stays queryable and a resumed bond can start a new active row.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, Set,
};

use crate::api::dto::canon::{RelationDto, RelationInput, RelationPatch, RelationQuery};
use crate::db::entities::relations;
use crate::db::json::{ChangeOp, ChangeTargetType, RelationStatus};
use crate::events::ServerEvent;

use super::{
    CanonError, CanonResult, CanonWrite, ChangeWrite, Visibility, changed_event, relation_snapshot,
    stamp,
};

/// `GET /api/books/:id/relations?entity_id=` (spec/10 §2.4). Whole lists, sorted client-side.
pub async fn list(
    conn: &impl ConnectionTrait,
    book_id: i32,
    query: &RelationQuery,
) -> CanonResult<Vec<RelationDto>> {
    super::super::books::get(conn, book_id).await?;
    // The canon pages show shared rows by default (spec/06 §2); a private relation belongs to
    // its isolated track and is read through that track's own views.
    let mut statement = relations::Entity::find()
        .filter(relations::Column::BookId.eq(book_id))
        .filter(Visibility::shared(book_id).visible_condition(relations::Column::TrackId))
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
    // A relation is one row, and only that row is written; the endpoints are read. An isolated
    // track therefore records a **private** relation over shared entities (`track_id = T`,
    // spec/06 §2) without modifying them — the per-track unique indexes exist exactly for that
    // case (spec/06 §1.2).
    if let Some(visibility) = write.visibility {
        visibility.ensure_visible(source.track_id)?;
        visibility.ensure_visible(target.track_id)?;
    } else if source.track_id.is_some() || target.track_id.is_some() {
        // The author's API has no track context, so it writes shared canon; a shared relation
        // must not reach an isolated track's private entity (spec/06 §2).
        return Err(CanonError::validation(
            "a shared relation can only link shared entities",
        ));
    }
    ensure_no_active_pair(
        tx,
        source.book_id,
        track_id,
        input.source_id,
        input.target_id,
        None,
    )
    .await?;
    let timestamp = stamp();
    let model = relations::ActiveModel {
        book_id: Set(book_id),
        track_id: Set(track_id),
        source_id: Set(input.source_id),
        target_id: Set(input.target_id),
        kind: Set(kind.to_owned()),
        inverse_kind: Set(input.inverse_kind.trim().to_owned()),
        description: Set(input.description.trim().to_owned()),
        status: Set(RelationStatus::Active),
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
            note: input.note.as_deref().unwrap_or_default().trim(),
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
    // Reactivating an `ended` row must not put a second active relation on a pair that has since
    // gained one (spec/06 §1.2). The row being patched is excluded: it is the target of the write.
    if patch.status == Some(RelationStatus::Active) {
        ensure_no_active_pair(
            tx,
            existing.book_id,
            existing.track_id,
            existing.source_id,
            existing.target_id,
            Some(relation_id),
        )
        .await?;
    }
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
        active.status = Set(status);
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
            note: patch.note.as_deref().unwrap_or_default().trim(),
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

/// `set_relation` (spec/05 §6): an upsert of the visible active relation of `(source, target)`.
///
/// The pair identifies the relation; `kind` is data about it (spec/06 §1.2). A kind that differs
/// from the row's current wording therefore **rewrites** that row — it does not open a second
/// active relation, which is what the archivist used to do and what left the author with two
/// near-identical lines per pair. In an isolated track an existing **shared** relation is read-only
/// (`error.isolated_readonly`) and a new one becomes a private row. `since_chapter_id` is the
/// session chapter on create.
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
    let existing = find_active_pair(tx, book_id, write, source_id, target_id).await?;
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
                // The pair already has a relation: a new wording replaces the old one on the same
                // row, so the pair keeps exactly one active line.
                kind: Some(kind.to_owned()),
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
                note: Some(note.to_owned()),
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
                note: Some(note.to_owned()),
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
        note: Some(note.to_owned()),
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

/// The visible active relation of a writer **matching this exact wording** (spec/06 §1.2).
///
/// Only `end_relation` needs this: its contract names the bond it ends by its kind, so a stale
/// wording must report `relation_not_found` rather than silently ending whatever the pair holds.
/// Callers that mean "the relation of this pair" use [`find_active_pair`] instead — the pair, not
/// the wording, is the identity.
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
        .filter(relations::Column::Status.eq(RelationStatus::Active));
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

/// Rejects a second active relation for one ordered pair in the same canon (spec/06 §1.2).
///
/// The check is on `(source, target)` and **not** on `kind`: `kind` is data about the bond, so a
/// reworded kind is an edit of the existing row, never a second active row. Without this, an agent
/// that re-creates a pair with slightly different wording leaves a two-line relation list where
/// both lines are active and mean the same thing (the regression this guard exists for).
///
/// `track_id` keeps the two canons apart exactly as the partial unique indexes do: a shared write
/// (`None`) matches only shared rows, a private one only that track's rows. An isolated track
/// therefore keeps its own private relation over a pair that already has a shared one
/// (spec/06 §2) — the shared row is invisible to it and `set_relation_inner` reports the shared
/// row as read-only instead.
async fn ensure_no_active_pair(
    tx: &impl ConnectionTrait,
    book_id: i32,
    track_id: Option<i32>,
    source_id: i32,
    target_id: i32,
    except_id: Option<i32>,
) -> CanonResult<()> {
    let mut statement = relations::Entity::find()
        .filter(relations::Column::BookId.eq(book_id))
        .filter(relations::Column::SourceId.eq(source_id))
        .filter(relations::Column::TargetId.eq(target_id))
        .filter(relations::Column::Status.eq(RelationStatus::Active));
    statement = match track_id {
        Some(track_id) => statement.filter(relations::Column::TrackId.eq(track_id)),
        None => statement.filter(relations::Column::TrackId.is_null()),
    };
    if let Some(except_id) = except_id {
        statement = statement.filter(relations::Column::Id.ne(except_id));
    }
    if let Some(row) = statement.one(tx).await? {
        // The message names the kind the pair already carries, so a caller that has the old
        // wording in hand can call `set_relation` / `PATCH` and edit instead of re-creating.
        return Err(super::conflict(
            format!(
                "{} is already active (#{}); edit that relation instead of creating a second one",
                relation_label(row.source_id, row.target_id, &row.kind),
                row.id
            ),
            row.id,
        ));
    }
    Ok(())
}

/// The active relation of one pair in a canon, whatever its kind (spec/06 §1.2 uniqueness).
///
/// Returned by `set_relation_inner` so a reworded kind edits the row the pair already has instead
/// of failing: the tool's contract is "set the relation of this pair", and the wording is data.
pub async fn find_active_pair(
    conn: &impl ConnectionTrait,
    book_id: i32,
    write: &CanonWrite,
    source_id: i32,
    target_id: i32,
) -> CanonResult<Option<relations::Model>> {
    let mut statement = relations::Entity::find()
        .filter(relations::Column::BookId.eq(book_id))
        .filter(relations::Column::SourceId.eq(source_id))
        .filter(relations::Column::TargetId.eq(target_id))
        .filter(relations::Column::Status.eq(RelationStatus::Active));
    if let Some(condition) = write.condition(relations::Column::TrackId) {
        statement = statement.filter(condition);
    }
    // A shared row wins over a private one: it is the canon relation (spec/06 §2).
    let rows = statement
        .order_by_asc(relations::Column::TrackId)
        .all(conn)
        .await?;
    Ok(rows.into_iter().next())
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
        status: row.status,
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
    // A revert that would restore `active` on a row whose pair already has an active relation
    // would break the pair-level rule this module enforces (spec/06 §1.2). The row being restored
    // is excluded: it is the row the revert writes.
    if status == &RelationStatus::Active {
        ensure_no_active_pair(
            tx,
            existing.book_id,
            existing.track_id,
            *source_id,
            *target_id,
            Some(relation_id),
        )
        .await?;
    }
    relations::ActiveModel {
        id: Set(relation_id),
        source_id: Set(*source_id),
        target_id: Set(*target_id),
        kind: Set(kind.clone()),
        inverse_kind: Set(inverse_kind.clone()),
        description: Set(description.clone()),
        status: Set(*status),
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
    // Recreating an **active** row for a pair that has since acquired another active relation
    // would put two active lines on one pair (spec/06 §1.2). An `ended` snapshot is history and
    // never collides, which is the common case: a deleted, already-ended relation.
    if status == &RelationStatus::Active {
        ensure_no_active_pair(tx, source.book_id, track_id, *source_id, *target_id, None).await?;
    }
    let timestamp = stamp();
    // A recreated row takes a NEW id, never the deleted one's reused rowid (spec/02 §5.10).
    let new_id = super::fresh_id(tx, "relations", ChangeTargetType::Relation).await?;
    let model = relations::ActiveModel {
        id: Set(new_id),
        book_id: Set(source.book_id),
        track_id: Set(track_id),
        source_id: Set(*source_id),
        target_id: Set(*target_id),
        kind: Set(kind.clone()),
        inverse_kind: Set(inverse_kind.clone()),
        description: Set(description.clone()),
        status: Set(*status),
        since_chapter_id: Set(*since_chapter_id),
        until_chapter_id: Set(*until_chapter_id),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
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
