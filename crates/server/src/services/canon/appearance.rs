//! Appearance service (spec/06 §1.3, §2, §3; spec/05 §6 `record_appearances`).
//!
//! One row per (chapter, entity) with a role: `pov`, `major`, `minor` or `mentioned`. The
//! archivist replaces the whole set per chapter, and the appearances drive two things:
//!
//! - the mid-term memory window (spec/06 §4.3);
//! - `first_chapter_id` / `last_chapter_id`, recomputed here on every change (D-059).
//!
//! Appearances belong to the **chapter**, not to a canon row, so an isolated track may record
//! shared entities appearing in its own chapters (spec/05 §6). The bounds of a shared entity
//! nevertheless count only shared-canon chapters (D-036).

use std::collections::HashSet;

use sea_orm::{ActiveModelTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};

use crate::api::dto::canon::{AppearanceDto, AppearanceInput};
use crate::db::entities::appearances;
use crate::db::json::{AppearanceRole, ChangeOp, ChangeTargetType};
use crate::events::ServerEvent;

use super::entity::recompute_bounds;
use super::{
    CanonError, CanonResult, CanonWrite, ChangeWrite, appearance_set_snapshot, changed_event,
};

/// The appearance set of one chapter, oldest first.
pub async fn for_chapter(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
) -> CanonResult<Vec<appearances::Model>> {
    Ok(appearances::Entity::find()
        .filter(appearances::COLUMN.chapter_id.eq(chapter_id))
        .order_by_asc(appearances::COLUMN.entity_id)
        .all(conn)
        .await?)
}

/// `record_appearances` (spec/05 §6): replaces this chapter's appearance set.
///
/// Recomputes the bounds of every entity in the old **and** new set, so an entity that left the
/// chapter loses its bound when it appeared nowhere else (D-059).
pub async fn record_set_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    chapter_id: i32,
    inputs: &[AppearanceInput],
) -> CanonResult<(Vec<appearances::Model>, Vec<ServerEvent>)> {
    let chapter = crate::db::entities::chapters::Entity::find_by_id(chapter_id)
        .one(tx)
        .await?
        .ok_or_else(|| CanonError::not_found(format!("chapter #{chapter_id}")))?;
    let previous = for_chapter(tx, chapter_id).await?;
    let before = appearance_set_snapshot(&previous);

    let mut seen = HashSet::new();
    let mut wanted = Vec::with_capacity(inputs.len());
    for input in inputs {
        if !seen.insert(input.entity_id) {
            continue;
        }
        // Every entity of the set must be visible to the writer (spec/06 §2).
        let entity = super::load_visible_entity(
            tx,
            write
                .visibility
                .unwrap_or_else(|| super::Visibility::shared(chapter.book_id)),
            input.entity_id,
        )
        .await
        .map_err(|_| {
            CanonError::validation(format!(
                "entity #{} is not visible in this canon",
                input.entity_id
            ))
        })?;
        if entity.book_id != chapter.book_id {
            return Err(CanonError::validation(format!(
                "entity #{} belongs to another book",
                input.entity_id
            )));
        }
        wanted.push(input.clone());
    }

    appearances::Entity::delete_many()
        .filter(appearances::COLUMN.chapter_id.eq(chapter_id))
        .exec(tx)
        .await?;
    for input in &wanted {
        appearances::ActiveModel {
            chapter_id: Set(chapter_id),
            entity_id: Set(input.entity_id),
            role: Set(input.role),
            note: Set(input.note.trim().to_owned()),
            ..Default::default()
        }
        .insert(tx)
        .await?;
    }
    let current = for_chapter(tx, chapter_id).await?;
    let after = appearance_set_snapshot(&current);

    let mut affected: Vec<i32> = previous.iter().map(|row| row.entity_id).collect();
    affected.extend(current.iter().map(|row| row.entity_id));
    recompute_bounds(tx, chapter.book_id, &affected).await?;

    let events = if before == after {
        Vec::new()
    } else {
        let change_id = super::record_change(
            tx,
            write,
            ChangeWrite {
                book_id: chapter.book_id,
                target_type: ChangeTargetType::AppearanceSet,
                // `appearance_set` and `chapter_summary` targets are keyed by chapter id
                // (spec/06 §3).
                target_id: chapter_id,
                op: ChangeOp::Update,
                before: Some(&before),
                after: Some(&after),
                note: "",
                reverts_id: None,
                track_id: None,
                chapter_id: Some(chapter_id),
            },
        )
        .await?;
        vec![changed_event(
            chapter.book_id,
            ChangeTargetType::AppearanceSet,
            chapter_id,
            ChangeOp::Update,
            change_id,
        )]
    };
    Ok((current, events))
}

/// Restores an appearance set from a snapshot (the revert path).
pub async fn restore_set_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    chapter_id: i32,
    restored: &crate::db::json::Snapshot,
) -> CanonResult<Vec<ServerEvent>> {
    let inputs: Vec<AppearanceInput> = match restored {
        crate::db::json::Snapshot::AppearanceSet { appearances: rows } => rows
            .iter()
            .map(|row| AppearanceInput {
                entity_id: row.entity_id,
                role: row.role,
                note: row.note.clone(),
            })
            .collect(),
        _ => {
            return Err(CanonError::internal(
                "an appearance-set revert needs an appearance-set snapshot",
            ));
        }
    };
    let (_, events) = record_set_inner(tx, write, chapter_id, &inputs).await?;
    Ok(events)
}

/// The role order used when several appearances compete (`pov` is the most prominent).
pub fn role_rank(role: AppearanceRole) -> u8 {
    match role {
        AppearanceRole::Pov => 4,
        AppearanceRole::Major => 3,
        AppearanceRole::Minor => 2,
        AppearanceRole::Mentioned => 1,
    }
}

/// Maps an appearance row to its DTO (spec/10 §2.4).
pub fn to_dto(row: &appearances::Model) -> CanonResult<AppearanceDto> {
    Ok(AppearanceDto {
        id: row.id,
        chapter_id: row.chapter_id,
        entity_id: row.entity_id,
        role: row.role,
        note: row.note.clone(),
    })
}
