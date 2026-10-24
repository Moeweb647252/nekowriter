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

use crate::api::dto::canon::RevertInput;
use crate::api::dto::changes::ChangeDto;
use crate::db::entities::change_log;
use crate::db::json::{ChangeOp, ChangeTargetType, Snapshot};
use crate::error::{AppError, ErrorCode};
use crate::events::ServerEvent;

use super::{CanonError, CanonResult, CanonWrite, changed_event};

/// `POST /api/changes/:id/revert` (spec/10 §2.4): returns the **revert** change.
pub async fn revert(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    change_id: i32,
    input: RevertInput,
) -> CanonResult<ChangeDto> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { revert_inner(tx, change_id, input).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn revert_inner(
    tx: &impl ConnectionTrait,
    change_id: i32,
    input: RevertInput,
) -> CanonResult<(ChangeDto, Vec<ServerEvent>)> {
    let original = change_log::Entity::find_by_id(change_id)
        .one(tx)
        .await?
        .ok_or_else(|| CanonError::not_found(format!("change #{change_id}")))?;
    let target_type = parse_target_type(&original.target_type)?;
    let op = parse_op(&original.op)?;
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

    // 2. Compute the inverse. The revert change describes it the same way any other change does:
    //    `before` is the target's state before the revert, `after` its state afterwards.
    let before = parse_snapshot(original.before.as_deref())?;
    let after = parse_snapshot(original.after.as_deref())?;
    let write = revert_write(original.book_id, original.track_id);
    let revert_of_target = match op {
        ChangeOp::Create => {
            // create → delete the target.
            inverse_delete(tx, &write, target_type, original.target_id, after.as_ref()).await?
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

    // 3. The revert itself is a new change with `reverts_id`; the original gets `reverted_by_id`.
    let revert_id = super::change_log_service::record(
        tx,
        super::change_log_service::NewChange {
            book_id: original.book_id,
            track_id: revert_of_target.track_id,
            chapter_id: original.chapter_id,
            session_id: original.session_id,
            actor: crate::db::json::ChangeActor::System,
            target_type,
            target_id: revert_of_target.target_id,
            op: revert_of_target.op,
            before: revert_of_target.before.as_ref(),
            after: revert_of_target.after.as_ref(),
            note: input.note.trim(),
            reverts_id: Some(original.id),
        },
    )
    .await?;
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

/// The writer a revert uses: the original change's book, or its isolated track (spec/06 §2).
fn revert_write(book_id: i32, track_id: Option<i32>) -> CanonWrite {
    match track_id {
        Some(track_id) => {
            CanonWrite::author().with_visibility(super::Visibility::isolated(book_id, track_id))
        }
        None => CanonWrite::author().with_visibility(super::Visibility::shared(book_id)),
    }
}

/// The change row a revert writes.
struct RevertOfTarget {
    target_id: i32,
    op: ChangeOp,
    before: Option<Snapshot>,
    after: Option<Snapshot>,
    track_id: Option<i32>,
}

/// The later changes of the same target that block a revert, newest first (spec/06 §3).
async fn later_change_ids(
    conn: &impl ConnectionTrait,
    original: &change_log::Model,
    target_type: ChangeTargetType,
) -> CanonResult<Vec<i32>> {
    let rows = change_log::Entity::find()
        .filter(change_log::Column::TargetType.eq(target_type.as_str()))
        .filter(change_log::Column::TargetId.eq(original.target_id))
        .filter(change_log::Column::Id.gt(original.id))
        .order_by_desc(change_log::Column::Id)
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

fn parse_target_type(raw: &str) -> CanonResult<ChangeTargetType> {
    ChangeTargetType::from_str_opt(raw)
        .ok_or_else(|| CanonError::internal(format!("unknown change_log.target_type {raw:?}")))
}

fn parse_op(raw: &str) -> CanonResult<ChangeOp> {
    ChangeOp::from_str_opt(raw)
        .ok_or_else(|| CanonError::internal(format!("unknown change_log.op {raw:?}")))
}

fn parse_snapshot(raw: Option<&str>) -> CanonResult<Option<Snapshot>> {
    match raw {
        Some(raw) => crate::db::json::from_json::<Snapshot>(raw)
            .map(Some)
            .map_err(|err| CanonError::internal(format!("corrupt change snapshot: {err}"))),
        None => Ok(None),
    }
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
    after: Option<&Snapshot>,
) -> CanonResult<RevertOfTarget> {
    let track_id = after.and_then(snapshot_track);
    let before = match target_type {
        ChangeTargetType::Entity => super::entity::snapshot_now(tx, target_id)
            .await?
            .or(after.cloned()),
        ChangeTargetType::Relation => super::relation::snapshot_now(tx, target_id)
            .await?
            .or(after.cloned()),
        ChangeTargetType::TimelineEvent => super::timeline::snapshot_now(tx, target_id)
            .await?
            .or(after.cloned()),
        ChangeTargetType::MemoryNote => super::note::snapshot_now(tx, target_id)
            .await?
            .or(after.cloned()),
        ChangeTargetType::Foreshadowing => super::foreshadow::snapshot_now(tx, target_id)
            .await?
            .or(after.cloned()),
        ChangeTargetType::AppearanceSet | ChangeTargetType::ChapterSummary => {
            return Err(not_revertible(target_type));
        }
        ChangeTargetType::SettingDoc | ChangeTargetType::CharacterDesign => {
            return Err(setting_history(target_type));
        }
    };
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
        _ => Vec::new(),
    };
    Ok(RevertOfTarget {
        target_id,
        op: ChangeOp::Delete,
        before,
        after: None,
        track_id,
    })
}

/// `update` → restore the `before` snapshot.
async fn inverse_restore(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    target_type: ChangeTargetType,
    target_id: i32,
    before: &Snapshot,
    after: Option<&Snapshot>,
) -> CanonResult<RevertOfTarget> {
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
    Ok(RevertOfTarget {
        target_id,
        op: ChangeOp::Update,
        before: current,
        after: Some(restored),
        track_id: snapshot_track(before),
    })
}

/// `delete` → recreate from `before`, with a new id (spec/02 §5.10).
async fn inverse_recreate(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    target_type: ChangeTargetType,
    before: &Snapshot,
) -> CanonResult<RevertOfTarget> {
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
    Ok(RevertOfTarget {
        target_id: new_id,
        op: ChangeOp::Create,
        before: None,
        after: Some(restored),
        track_id: snapshot_track(before),
    })
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
