//! The change log (spec/06 §3, spec/02 §2 `change_log`, spec/10 §2.4).
//!
//! Every create, update or delete of a canon or setting target writes one row with full
//! `before`/`after` snapshots and a `note`. The `note` only ever holds written text (the author's
//! change note, an agent's note, a proposal's rationale, or the merge form `#B → #A`); the UI
//! derives localized labels from `actor`, `op`, `target_type` and `reverts_id` (DECISIONS D-032).

use chrono::{DateTime, Utc};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde_json::Value;

use crate::api::dto::changes::{ChangeDto, ChangePageDto, ChangeQuery};
use crate::db::entities::change_log;
use crate::db::json::{ChangeActor, ChangeOp, ChangeTargetType, JsonText, Snapshot, to_json};
use crate::error::AppError;

use super::now;

/// The largest page the API serves (spec/10 §1).
const MAX_LIMIT: i32 = 500;
const DEFAULT_LIMIT: i32 = 100;

/// One row to write, before serialization.
pub struct NewChange<'a> {
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub chapter_id: Option<i32>,
    pub session_id: Option<i32>,
    pub actor: ChangeActor,
    pub target_type: ChangeTargetType,
    pub target_id: i32,
    pub op: ChangeOp,
    pub before: Option<&'a Snapshot>,
    pub after: Option<&'a Snapshot>,
    pub note: &'a str,
    pub reverts_id: Option<i32>,
}

/// Appends one change inside the caller's transaction and returns its id.
pub async fn record(tx: &impl ConnectionTrait, change: NewChange<'_>) -> Result<i32, AppError> {
    let before = change.before.map(|snapshot| JsonText(snapshot.clone()));
    let after = change.after.map(|snapshot| JsonText(snapshot.clone()));
    let model = change_log::ActiveModel {
        id: sea_orm::ActiveValue::NotSet,
        book_id: Set(change.book_id),
        track_id: Set(change.track_id),
        chapter_id: Set(change.chapter_id),
        session_id: Set(change.session_id),
        actor: Set(change.actor),
        target_type: Set(change.target_type),
        target_id: Set(change.target_id),
        op: Set(change.op),
        before: Set(before),
        after: Set(after),
        note: Set(change.note.to_owned()),
        reverts_id: Set(change.reverts_id),
        reverted_by_id: Set(None),
        created_at: Set(now()),
    };
    let inserted = change_log::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    Ok(inserted.id)
}

/// `GET /api/books/:id/changes` (spec/10 §2.4): newest first, cursor-paged.
pub async fn page(
    conn: &impl ConnectionTrait,
    book_id: i32,
    query: &ChangeQuery,
) -> Result<ChangePageDto, AppError> {
    super::books::get(conn, book_id).await?;
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let mut statement = change_log::Entity::find()
        .filter(change_log::COLUMN.book_id.eq(book_id))
        .order_by_desc(change_log::COLUMN.id)
        .limit(u64::try_from(limit + 1).unwrap_or(u64::MAX));
    if let Some(chapter_id) = query.chapter_id {
        statement = statement.filter(change_log::COLUMN.chapter_id.eq(chapter_id));
    }
    if let Some(session_id) = query.session_id {
        statement = statement.filter(change_log::COLUMN.session_id.eq(session_id));
    }
    if let Some(target_type) = query.target_type {
        statement = statement.filter(change_log::COLUMN.target_type.eq(target_type.as_str()));
    }
    if let Some(actor) = query.actor {
        statement = statement.filter(change_log::COLUMN.actor.eq(actor.as_str()));
    }
    if let Some(cursor) = &query.cursor {
        let id = parse_cursor(cursor)?;
        statement = statement.filter(change_log::COLUMN.id.lt(id));
    }
    let mut rows = statement.all(conn).await?;
    let has_more = rows.len() > limit as usize;
    if has_more {
        rows.truncate(limit as usize);
    }
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(to_dto(&row)?);
    }
    let next_cursor = if has_more {
        items.last().map(|change| change.id.to_string())
    } else {
        None
    };
    Ok(ChangePageDto { items, next_cursor })
}

/// `GET /api/changes/:id` — one change row (spec/10 §2.4 revert responses use the same DTO).
pub async fn get(conn: &impl ConnectionTrait, change_id: i32) -> Result<ChangeDto, AppError> {
    let row = change_log::Entity::find_by_id(change_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("change #{change_id}")))?;
    to_dto(&row)
}

/// The latest changes of a book, newest first (`overview.recent_changes`, spec/08 §7).
pub async fn recent_for_book(
    conn: &impl ConnectionTrait,
    book_id: i32,
    limit: usize,
) -> Result<Vec<ChangeDto>, AppError> {
    let rows = change_log::Entity::find()
        .filter(change_log::COLUMN.book_id.eq(book_id))
        .order_by_desc(change_log::COLUMN.id)
        .limit(limit as u64)
        .all(conn)
        .await?;
    rows.iter().map(to_dto).collect()
}

/// The history of one setting document, newest first (spec/08 §9 `History`).
pub async fn for_target(
    conn: &impl ConnectionTrait,
    target_type: ChangeTargetType,
    target_id: i32,
    limit: usize,
) -> Result<Vec<ChangeDto>, AppError> {
    let rows = change_log::Entity::find()
        .filter(change_log::COLUMN.target_type.eq(target_type.as_str()))
        .filter(change_log::COLUMN.target_id.eq(target_id))
        .order_by_desc(change_log::COLUMN.id)
        .limit(limit as u64)
        .all(conn)
        .await?;
    rows.iter().map(to_dto).collect()
}

/// A snapshot for the `before`/`after` columns of a setting document (spec/02 §4).
pub fn setting_doc_snapshot(content: &str) -> Snapshot {
    Snapshot::SettingDoc {
        content: content.to_owned(),
    }
}

/// A snapshot for a character design (spec/02 §4).
pub fn character_snapshot(row: &crate::db::entities::character_designs::Model) -> Snapshot {
    Snapshot::CharacterDesign {
        rank: row.rank,
        name: row.name.clone(),
        role_label: row.role_label.clone(),
        summary: row.summary.clone(),
        profile: row.profile.clone(),
        entity_id: row.entity_id,
    }
}

/// Which columns the change row needs to describe what happened to a target.
pub fn snapshot_json(snapshot: Option<&Snapshot>) -> Result<Option<String>, AppError> {
    snapshot
        .map(to_json)
        .transpose()
        .map_err(|err| AppError::internal(format!("cannot serialize a snapshot: {err}")))
}

/// Maps a change row to its DTO.
pub fn to_dto(row: &change_log::Model) -> Result<ChangeDto, AppError> {
    Ok(ChangeDto {
        id: row.id,
        book_id: row.book_id,
        track_id: row.track_id,
        chapter_id: row.chapter_id,
        session_id: row.session_id,
        actor: row.actor,
        target_type: row.target_type,
        target_id: row.target_id,
        op: row.op,
        before: row.before.clone().map(JsonText::into_inner),
        after: row.after.clone().map(JsonText::into_inner),
        note: row.note.clone(),
        reverts_id: row.reverts_id,
        reverted_by_id: row.reverted_by_id,
        created_at: row.created_at,
    })
}

fn parse_cursor(cursor: &str) -> Result<i32, AppError> {
    cursor
        .parse::<i32>()
        .map_err(|_| AppError::validation("the cursor is not valid"))
}

/// The `details` shape of `change.not_latest` (spec/10 §4).
pub fn not_latest_details(later_change_ids: Vec<i32>) -> Value {
    serde_json::json!({ "later_change_ids": later_change_ids })
}

/// Marks `original` as reverted by `revert_id` (the only permitted mutation of a change row).
pub async fn mark_reverted(
    tx: &impl ConnectionTrait,
    original: i32,
    revert_id: i32,
) -> Result<(), AppError> {
    use sea_orm::ActiveModelTrait;
    let mut active = change_log::ActiveModel {
        id: Set(original),
        ..Default::default()
    };
    active.reverted_by_id = Set(Some(revert_id));
    active.update(tx).await?;
    Ok(())
}

/// Formats a change row's timestamp for logs.
pub fn changed_at(row: &change_log::Model) -> DateTime<Utc> {
    row.created_at
}
