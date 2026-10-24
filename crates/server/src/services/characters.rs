//! Character design service (spec/10 §2.3, PROJECT §8.9, spec/06 §3, spec/08 §1.4 locking).
//!
//! Character designs are the conception studio's ranked cast list. `rank` is `1..n` (1 = most
//! important) and is renumbered on every create, delete and reorder. Every change writes a
//! `change_log` row with `actor: user`.

use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use std::collections::HashSet;

use crate::api::dto::settings_materials::{
    CharacterDto, CharacterInput, CharacterPatch, CharacterReorderInput,
};
use crate::db::entities::character_designs;
use crate::db::json::{ChangeActor, ChangeOp, ChangeTargetType};
use crate::error::{AppError, ErrorCode};
use crate::events::{ServerEvent, SettingsKind};

use super::now;

/// `GET /api/books/:id/characters` — ranked, most important first.
pub async fn list(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Vec<CharacterDto>, AppError> {
    super::books::get(conn, book_id).await?;
    let rows = ranked(conn, book_id).await?;
    rows.iter().map(to_dto).collect()
}

/// `POST /api/books/:id/characters`. A duplicate name is `conflict.exists` (spec/10 §4).
pub async fn create(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    book_id: i32,
    input: CharacterInput,
) -> Result<CharacterDto, AppError> {
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
    input: CharacterInput,
) -> Result<(CharacterDto, Vec<ServerEvent>), AppError> {
    super::books::get(tx, book_id).await?;
    super::setting_docs::ensure_editable(tx, book_id).await?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::validation("a character needs a name"));
    }
    if let Some(existing) = find_by_name(tx, book_id, name).await? {
        return Err(super::conflict(
            ErrorCode::ConflictExists,
            format!("a character named {name:?} already exists"),
            serde_json::json!({ "id": existing.id }),
        ));
    }
    let rank = next_rank(tx, book_id).await?;
    let timestamp = now();
    let model = character_designs::ActiveModel {
        book_id: Set(book_id),
        rank: Set(rank),
        name: Set(name.to_owned()),
        role_label: Set(input.role_label.trim().to_owned()),
        summary: Set(input.summary.trim().to_owned()),
        profile: Set(input.profile.clone()),
        entity_id: Set(None),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    let row = character_designs::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    let after = super::change_log::character_snapshot(&row);
    super::change_log::record(
        tx,
        super::change_log::NewChange {
            book_id,
            track_id: None,
            chapter_id: None,
            session_id: None,
            actor: ChangeActor::User,
            target_type: ChangeTargetType::CharacterDesign,
            target_id: row.id,
            op: ChangeOp::Create,
            before: None,
            after: Some(&after),
            note: input.note.trim(),
            reverts_id: None,
        },
    )
    .await?;
    super::books::touch(tx, book_id, timestamp).await?;
    let dto = to_dto(&row)?;
    Ok((
        dto.clone(),
        vec![ServerEvent::SettingsChanged {
            book_id,
            kind: SettingsKind::Characters,
            doc_kind: None,
            revision: None,
        }],
    ))
}

/// `PATCH /api/characters/:id`.
pub async fn update(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    character_id: i32,
    patch: CharacterPatch,
) -> Result<CharacterDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { update_inner(tx, character_id, patch).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn update_inner(
    tx: &impl ConnectionTrait,
    character_id: i32,
    patch: CharacterPatch,
) -> Result<(CharacterDto, Vec<ServerEvent>), AppError> {
    let existing = load(tx, character_id).await?;
    super::setting_docs::ensure_editable(tx, existing.book_id).await?;
    let before = super::change_log::character_snapshot(&existing);

    let mut active = character_designs::ActiveModel {
        id: Set(character_id),
        updated_at: Set(now()),
        ..Default::default()
    };
    if let Some(name) = &patch.name {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::validation("a character needs a name"));
        }
        if name != existing.name
            && let Some(other) = find_by_name(tx, existing.book_id, name).await?
        {
            return Err(super::conflict(
                ErrorCode::ConflictExists,
                format!("a character named {name:?} already exists"),
                serde_json::json!({ "id": other.id }),
            ));
        }
        active.name = Set(name.to_owned());
    }
    if let Some(role_label) = &patch.role_label {
        active.role_label = Set(role_label.trim().to_owned());
    }
    if let Some(summary) = &patch.summary {
        active.summary = Set(summary.trim().to_owned());
    }
    if let Some(profile) = &patch.profile {
        active.profile = Set(profile.clone());
    }
    if let Some(entity_id) = patch.entity_id {
        active.entity_id = Set(Some(entity_id));
    }
    sea_orm::ActiveModelTrait::update(active, tx).await?;
    let row = load(tx, character_id).await?;
    let after = super::change_log::character_snapshot(&row);
    super::change_log::record(
        tx,
        super::change_log::NewChange {
            book_id: row.book_id,
            track_id: None,
            chapter_id: None,
            session_id: None,
            actor: ChangeActor::User,
            target_type: ChangeTargetType::CharacterDesign,
            target_id: row.id,
            op: ChangeOp::Update,
            before: Some(&before),
            after: Some(&after),
            note: patch.note.trim(),
            reverts_id: None,
        },
    )
    .await?;
    let dto = to_dto(&row)?;
    Ok((
        dto.clone(),
        vec![ServerEvent::SettingsChanged {
            book_id: row.book_id,
            kind: SettingsKind::Characters,
            doc_kind: None,
            revision: None,
        }],
    ))
}

/// `DELETE /api/characters/:id`; the rest of the ranking closes up.
pub async fn delete(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    character_id: i32,
) -> Result<(), AppError> {
    let published = super::write(db, move |tx| {
        Box::pin(async move { delete_inner(tx, character_id).await })
    })
    .await?;
    events.publish_all(published);
    Ok(())
}

pub async fn delete_inner(
    tx: &impl ConnectionTrait,
    character_id: i32,
) -> Result<Vec<ServerEvent>, AppError> {
    let existing = load(tx, character_id).await?;
    super::setting_docs::ensure_editable(tx, existing.book_id).await?;
    character_designs::Entity::delete_by_id(character_id)
        .exec(tx)
        .await?;
    let before = super::change_log::character_snapshot(&existing);
    super::change_log::record(
        tx,
        super::change_log::NewChange {
            book_id: existing.book_id,
            track_id: None,
            chapter_id: None,
            session_id: None,
            actor: ChangeActor::User,
            target_type: ChangeTargetType::CharacterDesign,
            target_id: existing.id,
            op: ChangeOp::Delete,
            before: Some(&before),
            after: None,
            note: "",
            reverts_id: None,
        },
    )
    .await?;
    renumber(tx, existing.book_id).await?;
    Ok(vec![ServerEvent::SettingsChanged {
        book_id: existing.book_id,
        kind: SettingsKind::Characters,
        doc_kind: None,
        revision: None,
    }])
}

/// `POST /api/books/:id/characters/reorder` (dnd-kit in the studio, spec/11 §5.4).
pub async fn reorder(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    book_id: i32,
    input: CharacterReorderInput,
) -> Result<Vec<CharacterDto>, AppError> {
    let (list, published) = super::write(db, move |tx| {
        Box::pin(async move { reorder_inner(tx, book_id, input).await })
    })
    .await?;
    events.publish_all(published);
    Ok(list)
}

pub async fn reorder_inner(
    tx: &impl ConnectionTrait,
    book_id: i32,
    input: CharacterReorderInput,
) -> Result<(Vec<CharacterDto>, Vec<ServerEvent>), AppError> {
    use sea_orm::ActiveModelTrait;

    super::books::get(tx, book_id).await?;
    super::setting_docs::ensure_editable(tx, book_id).await?;
    let rows = ranked(tx, book_id).await?;
    let mut seen = HashSet::new();
    for id in &input.ids {
        if !seen.insert(*id) {
            return Err(AppError::validation("the reorder list repeats a character"));
        }
    }
    if seen.len() != rows.len() || !rows.iter().all(|row| seen.contains(&row.id)) {
        return Err(AppError::validation(
            "the reorder list must contain every character exactly once",
        ));
    }
    let timestamp = now();
    for (index, id) in input.ids.iter().enumerate() {
        let active = character_designs::ActiveModel {
            id: Set(*id),
            rank: Set(index as i32 + 1),
            updated_at: Set(timestamp),
            ..Default::default()
        };
        active.update(tx).await?;
    }
    let list = ranked(tx, book_id).await?;
    let dtos = list.iter().map(to_dto).collect::<Result<Vec<_>, _>>()?;
    Ok((
        dtos,
        vec![ServerEvent::SettingsChanged {
            book_id,
            kind: SettingsKind::Characters,
            doc_kind: None,
            revision: None,
        }],
    ))
}

/// `GET /api/books/:id/characters/:id/history` — the change log of one design.
pub async fn history(
    conn: &impl ConnectionTrait,
    character_id: i32,
    limit: usize,
) -> Result<Vec<crate::api::dto::changes::ChangeDto>, AppError> {
    load(conn, character_id).await?;
    super::change_log::for_target(conn, ChangeTargetType::CharacterDesign, character_id, limit)
        .await
}

/// Loads one design or reports `not_found`.
pub async fn load(
    conn: &impl ConnectionTrait,
    character_id: i32,
) -> Result<character_designs::Model, AppError> {
    character_designs::Entity::find_by_id(character_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("character #{character_id}")))
}

/// The ranked designs of a book.
async fn ranked(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Vec<character_designs::Model>, AppError> {
    Ok(character_designs::Entity::find()
        .filter(character_designs::Column::BookId.eq(book_id))
        .order_by_asc(character_designs::Column::Rank)
        .all(conn)
        .await?)
}

async fn find_by_name(
    conn: &impl ConnectionTrait,
    book_id: i32,
    name: &str,
) -> Result<Option<character_designs::Model>, AppError> {
    Ok(character_designs::Entity::find()
        .filter(character_designs::Column::BookId.eq(book_id))
        .filter(character_designs::Column::Name.eq(name))
        .one(conn)
        .await?)
}

async fn next_rank(conn: &impl ConnectionTrait, book_id: i32) -> Result<i32, AppError> {
    let rows = ranked(conn, book_id).await?;
    Ok(rows.iter().map(|row| row.rank).max().unwrap_or(0) + 1)
}

/// Renumbers the book's designs to contiguous `1..n` (spec/02 §1).
async fn renumber(tx: &impl ConnectionTrait, book_id: i32) -> Result<(), AppError> {
    use sea_orm::ActiveModelTrait;
    let rows = ranked(tx, book_id).await?;
    let timestamp = now();
    for (index, row) in rows.iter().enumerate() {
        let target = index as i32 + 1;
        if row.rank == target {
            continue;
        }
        let active = character_designs::ActiveModel {
            id: Set(row.id),
            rank: Set(target),
            updated_at: Set(timestamp),
            ..Default::default()
        };
        active.update(tx).await?;
    }
    Ok(())
}

/// Maps a row to its DTO.
pub fn to_dto(row: &character_designs::Model) -> Result<CharacterDto, AppError> {
    Ok(CharacterDto {
        id: row.id,
        book_id: row.book_id,
        rank: row.rank,
        name: row.name.clone(),
        role_label: row.role_label.clone(),
        summary: row.summary.clone(),
        profile: row.profile.clone(),
        entity_id: row.entity_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}
