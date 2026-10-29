//! Change-log revert (spec/06 §3, spec/10 §2.4; spec/02 §5.9–§5.10).
//!
//! `POST /api/changes/:id/revert`:
//!
//! 1. allowed only when the change is the newest non-reverted change of its target — otherwise
//!    `409 change.not_latest` with `details.later_change_ids` (the UI links them);
//! 2. `create` → delete the target; `update` → restore the `before` snapshot; `delete` → recreate
//!    from `before` with a **new id** (join rows such as entity links are restored when their
//!    entities still exist; relations of a deleted entity are **not** restored, spec/02 §5.10);
//! 3. the revert is itself a new change with `reverts_id`, and the original gets
//!    `reverted_by_id` (the only permitted mutation of a change row).
//!
//! Reverted changes never reach mid-term memory (spec/06 §3).

use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::api::dto::changes::ChangeDto;
use crate::db::entities::change_log;
use crate::db::json::{ChangeOp, ChangeTargetType, JsonText, Snapshot};
use crate::error::{AppError, ErrorCode};
use crate::events::ServerEvent;

use super::{CanonError, CanonResult, CanonWrite, changed_event};

/// `POST /api/changes/:id/revert` (spec/10 §2.4): returns the **revert** change.
pub async fn revert(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    change_id: i32,
) -> CanonResult<ChangeDto> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { revert_inner(tx, change_id).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn revert_inner(
    tx: &impl ConnectionTrait,
    change_id: i32,
) -> CanonResult<(ChangeDto, Vec<ServerEvent>)> {
    let original = change_log::Entity::find_by_id(change_id)
        .one(tx)
        .await?
        .ok_or_else(|| CanonError::not_found(format!("change #{change_id}")))?;
    let target_type = original.target_type;
    let op = original.op;
    if original.reverts_id.is_some() {
        return Err(CanonError::App(AppError::validation(
            "a revert cannot itself be reverted",
        )));
    }
    // A reverted change is never the newest one any more; naming the revert that superseded it is
    // the honest answer for the UI ("revert the later changes first" links them).
    if let Some(revert_of_it) = original.reverted_by_id {
        return Err(not_latest(vec![revert_of_it]));
    }
    // 1. Only the newest non-reverted change of the target may be reverted (spec/06 §3).
    let later = later_change_ids(tx, &original, target_type).await?;
    if !later.is_empty() {
        return Err(not_latest(later));
    }

    // 2. Compute the inverse. The inverse operation itself records the revert row (D-079):
    //    `before` is the target's state before the revert, `after` its state afterwards.
    let before = original.before.clone().map(JsonText::into_inner);
    let after = original.after.clone().map(JsonText::into_inner);
    let write = revert_write(original.book_id, original.track_id)
        .with_chapter(original.chapter_id)
        .as_revert(original.id);
    // The revert is the author's action on an existing history, so the row keeps the original's
    // session when there was one.
    let write = CanonWrite {
        session_id: original.session_id,
        ..write
    };
    match op {
        ChangeOp::Create => {
            // create → delete the target.
            inverse_delete(tx, &write, target_type, original.target_id).await?
        }
        ChangeOp::Update => {
            let Some(before) = before.as_ref() else {
                return Err(CanonError::internal(
                    "an update change has no before snapshot",
                ));
            };
            inverse_restore(
                tx,
                &write,
                target_type,
                original.target_id,
                before,
                after.as_ref(),
            )
            .await?
        }
        ChangeOp::Delete => {
            let Some(before) = before.as_ref() else {
                return Err(CanonError::internal(
                    "a delete change has no before snapshot",
                ));
            };
            // delete → recreate from `before`, with a NEW id (spec/02 §5.10).
            inverse_recreate(tx, &write, target_type, before).await?
        }
    };

    // 3. The inverse operation above wrote the revert row; link the original to it. The only
    //    permitted mutation of a change row is this `reverted_by_id` (spec/02 §5.9).
    let revert_id = revert_row_of(tx, original.id).await?;
    super::change_log_service::mark_reverted(tx, original.id, revert_id).await?;
    let dto = super::change_log_service::get(tx, revert_id).await?;
    let events = vec![changed_event(
        original.book_id,
        target_type,
        dto.target_id,
        dto.op,
        revert_id,
    )];
    Ok((dto, events))
}

/// The change row the inverse operation just wrote (`reverts_id = change_id`), inside the
/// transaction that wrote it.
async fn revert_row_of(conn: &impl ConnectionTrait, change_id: i32) -> CanonResult<i32> {
    crate::db::entities::change_log::Entity::find()
        .filter(
            crate::db::entities::change_log::COLUMN
                .reverts_id
                .eq(change_id),
        )
        .order_by_desc(crate::db::entities::change_log::COLUMN.id)
        .one(conn)
        .await?
        .map(|row| row.id)
        .ok_or_else(|| {
            CanonError::internal(format!(
                "the revert of change #{change_id} was not recorded"
            ))
        })
}

/// The writer a revert uses: the original change's book, or its isolated track (spec/06 §2).
fn revert_write(book_id: i32, track_id: Option<i32>) -> CanonWrite {
    match track_id {
        Some(track_id) => {
            CanonWrite::author().with_visibility(super::Visibility::isolated(book_id, track_id))
        }
        None => CanonWrite::author().with_visibility(super::Visibility::shared(book_id)),
    }
}

/// The later changes of the same target that block a revert, newest first (spec/06 §3).
async fn later_change_ids(
    conn: &impl ConnectionTrait,
    original: &change_log::Model,
    target_type: ChangeTargetType,
) -> CanonResult<Vec<i32>> {
    let rows = change_log::Entity::find()
        .filter(change_log::COLUMN.target_type.eq(target_type.as_str()))
        .filter(change_log::COLUMN.target_id.eq(original.target_id))
        .filter(change_log::COLUMN.id.gt(original.id))
        .order_by_desc(change_log::COLUMN.id)
        .all(conn)
        .await?;
    Ok(rows.into_iter().map(|row| row.id).collect())
}

/// `409 change.not_latest` with `details.later_change_ids` (spec/10 §4).
fn not_latest(later: Vec<i32>) -> CanonError {
    CanonError::App(
        AppError::new(ErrorCode::ChangeNotLatest, "a later change exists")
            .with_details(super::change_log_service::not_latest_details(later)),
    )
}

/// The `track_id` a snapshot carries (`None` for the snapshot kinds that have none).
fn snapshot_track(snapshot: &Snapshot) -> Option<i32> {
    match snapshot {
        Snapshot::Entity { track_id, .. }
        | Snapshot::Relation { track_id, .. }
        | Snapshot::TimelineEvent { track_id, .. }
        | Snapshot::Note { track_id, .. }
        | Snapshot::Foreshadowing { track_id, .. } => *track_id,
        _ => None,
    }
}

/// `create` → delete the target.
async fn inverse_delete(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    target_type: ChangeTargetType,
    target_id: i32,
) -> CanonResult<()> {
    // The delete itself records the revert row (D-079), with the snapshot it takes as `before`.
    match target_type {
        ChangeTargetType::Entity => super::entity::delete_inner(tx, write, target_id).await?,
        ChangeTargetType::Relation => super::relation::delete_inner(tx, write, target_id).await?,
        ChangeTargetType::TimelineEvent => {
            super::timeline::delete_inner(tx, write, target_id).await?
        }
        ChangeTargetType::MemoryNote => super::note::delete_inner(tx, write, target_id).await?,
        ChangeTargetType::Foreshadowing => {
            super::foreshadow::delete_inner(tx, write, target_id).await?
        }
        ChangeTargetType::AppearanceSet | ChangeTargetType::ChapterSummary => {
            return Err(not_revertible(target_type));
        }
        ChangeTargetType::SettingDoc | ChangeTargetType::CharacterDesign => {
            return Err(setting_history(target_type));
        }
    };
    Ok(())
}

/// `update` → restore the `before` snapshot.
async fn inverse_restore(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    target_type: ChangeTargetType,
    target_id: i32,
    before: &Snapshot,
    after: Option<&Snapshot>,
) -> CanonResult<()> {
    let current = after.cloned();
    let restored = match target_type {
        ChangeTargetType::Entity => {
            super::entity::restore_inner(tx, write, target_id, before).await?
        }
        ChangeTargetType::Relation => {
            super::relation::restore_inner(tx, write, target_id, before).await?
        }
        ChangeTargetType::TimelineEvent => {
            super::timeline::restore_inner(tx, write, target_id, before).await?
        }
        ChangeTargetType::MemoryNote => {
            super::note::restore_inner(tx, write, target_id, before).await?
        }
        ChangeTargetType::Foreshadowing => {
            super::foreshadow::restore_inner(tx, write, target_id, before).await?
        }
        // An appearance set's "target" is its chapter, and restoring it also restores the entity
        // bounds (D-059).
        ChangeTargetType::AppearanceSet => {
            super::appearance::restore_set_inner(tx, write, target_id, before).await?;
            before.clone()
        }
        ChangeTargetType::ChapterSummary => {
            super::restore_summary_inner(tx, target_id, before).await?;
            before.clone()
        }
        ChangeTargetType::SettingDoc | ChangeTargetType::CharacterDesign => {
            return Err(setting_history(target_type));
        }
    };
    // `restore_inner` writes the row; the change row is recorded here, and `record_change` turns
    // it into the revert (D-079). An appearance set goes through `restore_set_inner`, which
    // records its own change for the same reason.
    if target_type != ChangeTargetType::AppearanceSet {
        record_revert(
            tx,
            write,
            target_type,
            target_id,
            ChangeOp::Update,
            current.as_ref(),
            Some(&restored),
            snapshot_track(before),
        )
        .await?;
    }
    Ok(())
}

/// `delete` → recreate from `before`, with a new id (spec/02 §5.10).
async fn inverse_recreate(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    target_type: ChangeTargetType,
    before: &Snapshot,
) -> CanonResult<()> {
    let (new_id, restored) = match target_type {
        ChangeTargetType::Entity => {
            let (id, snapshot) = super::entity::recreate_inner(tx, write, before).await?;
            (id, snapshot)
        }
        ChangeTargetType::Relation => {
            let (id, snapshot) = super::relation::recreate_inner(tx, write, before).await?;
            (id, snapshot)
        }
        ChangeTargetType::TimelineEvent => {
            let (id, snapshot) = super::timeline::recreate_inner(tx, write, before).await?;
            (id, snapshot)
        }
        ChangeTargetType::MemoryNote => {
            let (id, snapshot) = super::note::recreate_inner(tx, write, before).await?;
            (id, snapshot)
        }
        ChangeTargetType::Foreshadowing => {
            let (id, snapshot) = super::foreshadow::recreate_inner(tx, write, before).await?;
            (id, snapshot)
        }
        ChangeTargetType::AppearanceSet | ChangeTargetType::ChapterSummary => {
            return Err(not_revertible(target_type));
        }
        ChangeTargetType::SettingDoc | ChangeTargetType::CharacterDesign => {
            return Err(setting_history(target_type));
        }
    };
    record_revert(
        tx,
        write,
        target_type,
        new_id,
        ChangeOp::Create,
        None,
        Some(&restored),
        snapshot_track(before),
    )
    .await?;
    Ok(())
}

/// Records the change row of an inverse operation, which `record_change` stamps as the revert.
// Eight parameters is what the inverse operation needs (target, snapshots, actor and the revert's
// own identity); grouping them into a struct would only move the same fields one level down, and
// this is a private helper with one caller per target type.
#[allow(clippy::too_many_arguments)]
async fn record_revert(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    target_type: ChangeTargetType,
    target_id: i32,
    op: ChangeOp,
    before: Option<&Snapshot>,
    after: Option<&Snapshot>,
    track_id: Option<i32>,
) -> CanonResult<i32> {
    let book_id = write
        .visibility
        .map(|visibility| visibility.book_id)
        .ok_or_else(|| CanonError::internal("a revert needs a book"))?;
    super::record_change(
        tx,
        write,
        super::ChangeWrite {
            book_id,
            target_type,
            target_id,
            op,
            before,
            after,
            note: "",
            reverts_id: None,
            track_id,
            chapter_id: None,
        },
    )
    .await
}

/// Appearance sets and chapter summaries can be created only by the chapter lifecycle.
fn not_revertible(target_type: ChangeTargetType) -> CanonError {
    CanonError::App(AppError::validation(format!(
        "a {} change cannot be reverted",
        target_type.as_str()
    )))
}

/// Setting materials have their own history (spec/08 §8.9); M5 reverts canon targets.
fn setting_history(target_type: ChangeTargetType) -> CanonError {
    CanonError::App(AppError::validation(format!(
        "a {} change is reverted from its own history page",
        target_type.as_str()
    )))
}
