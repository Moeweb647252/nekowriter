//! Setting document service (spec/10 §2.3, spec/08 §9, spec/06 §3, spec/08 §1.4 locking).
//!
//! A book always has exactly three documents (`outline`, `world`, `protagonist`), created with the
//! book. A `PUT` carries `base_revision` (optimistic concurrency), bumps `revision`, and writes a
//! `change_log` row. Editing is locked only while the book's conception session is running.

use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};

use crate::api::dto::settings_materials::{SettingDocDto, SettingDocInput};
use crate::db::entities::setting_docs;
use crate::db::json::{ChangeActor, ChangeOp, ChangeTargetType, SettingDocKind};
use crate::error::{AppError, ErrorCode};
use crate::events::{ServerEvent, SettingsKind};

use super::now;

/// `GET /api/books/:id/setting-docs` — the three documents, in the spec's order.
pub async fn list(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Vec<SettingDocDto>, AppError> {
    super::books::get(conn, book_id).await?;
    let rows = super::books::setting_docs_of(conn, book_id).await?;
    rows.iter().map(to_dto).collect()
}

/// `GET /api/books/:id/setting-docs/:kind`.
pub async fn get(
    conn: &impl ConnectionTrait,
    book_id: i32,
    kind: SettingDocKind,
) -> Result<SettingDocDto, AppError> {
    let row = load(conn, book_id, kind).await?;
    to_dto(&row)
}

/// `PUT /api/books/:id/setting-docs/:kind`.
pub async fn put(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    book_id: i32,
    kind: SettingDocKind,
    input: SettingDocInput,
) -> Result<SettingDocDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { put_inner(tx, book_id, kind, input).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn put_inner(
    tx: &impl ConnectionTrait,
    book_id: i32,
    kind: SettingDocKind,
    input: SettingDocInput,
) -> Result<(SettingDocDto, Vec<ServerEvent>), AppError> {
    let existing = load(tx, book_id, kind).await?;
    if existing.revision != input.base_revision {
        return Err(super::conflict(
            ErrorCode::ConflictVersion,
            "the document changed since it was loaded",
            serde_json::json!({ "revision": existing.revision }),
        ));
    }
    ensure_editable(tx, book_id).await?;
    if input.content == existing.content {
        // A no-op save keeps the revision (and emits nothing): the UI autosaves on blur.
        return Ok((to_dto(&existing)?, Vec::new()));
    }

    let revision = existing.revision + 1;
    let timestamp = now();
    let active = setting_docs::ActiveModel {
        id: Set(existing.id),
        content: Set(input.content.clone()),
        revision: Set(revision),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    sea_orm::ActiveModelTrait::update(active, tx).await?;

    let before = super::change_log::setting_doc_snapshot(&existing.content);
    let after = super::change_log::setting_doc_snapshot(&input.content);
    super::change_log::record(
        tx,
        super::change_log::NewChange {
            book_id,
            track_id: None,
            chapter_id: None,
            session_id: None,
            actor: ChangeActor::User,
            target_type: ChangeTargetType::SettingDoc,
            target_id: existing.id,
            op: ChangeOp::Update,
            before: Some(&before),
            after: Some(&after),
            note: input.note.trim(),
            reverts_id: None,
        },
    )
    .await?;
    super::books::touch(tx, book_id, timestamp).await?;

    let row = load(tx, book_id, kind).await?;
    let dto = to_dto(&row)?;
    Ok((
        dto.clone(),
        vec![ServerEvent::SettingsChanged {
            book_id,
            kind: SettingsKind::SettingDoc,
            doc_kind: Some(kind),
            revision: Some(revision),
        }],
    ))
}

/// `GET /api/books/:id/setting-docs/:kind/history` — the change log filtered to this document.
pub async fn history(
    conn: &impl ConnectionTrait,
    book_id: i32,
    kind: SettingDocKind,
    limit: usize,
) -> Result<Vec<crate::api::dto::changes::ChangeDto>, AppError> {
    let doc = load(conn, book_id, kind).await?;
    super::change_log::for_target(conn, ChangeTargetType::SettingDoc, doc.id, limit).await
}

/// Records an agent-written document (M4/M9 call this); the caller owns the transaction.
pub async fn write_from_agent(
    tx: &impl ConnectionTrait,
    book_id: i32,
    kind: SettingDocKind,
    content: &str,
    note: &str,
    session_id: Option<i32>,
) -> Result<SettingDocDto, AppError> {
    let existing = load(tx, book_id, kind).await?;
    let revision = existing.revision + 1;
    let timestamp = now();
    let active = setting_docs::ActiveModel {
        id: Set(existing.id),
        content: Set(content.to_owned()),
        revision: Set(revision),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    sea_orm::ActiveModelTrait::update(active, tx).await?;

    let before = super::change_log::setting_doc_snapshot(&existing.content);
    let after = super::change_log::setting_doc_snapshot(content);
    super::change_log::record(
        tx,
        super::change_log::NewChange {
            book_id,
            track_id: None,
            chapter_id: None,
            session_id,
            actor: ChangeActor::Agent,
            target_type: ChangeTargetType::SettingDoc,
            target_id: existing.id,
            op: ChangeOp::Update,
            before: Some(&before),
            after: Some(&after),
            note,
            reverts_id: None,
        },
    )
    .await?;
    let row = load(tx, book_id, kind).await?;
    to_dto(&row)
}

/// Loads one document or reports `not_found`.
pub async fn load(
    conn: &impl ConnectionTrait,
    book_id: i32,
    kind: SettingDocKind,
) -> Result<setting_docs::Model, AppError> {
    super::books::get(conn, book_id).await?;
    setting_docs::Entity::find()
        .filter(setting_docs::COLUMN.book_id.eq(book_id))
        .filter(setting_docs::COLUMN.kind.eq(kind.as_str()))
        .one(conn)
        .await?
        .ok_or_else(|| {
            AppError::not_found(format!(
                "the {:?} setting document of book #{book_id}",
                kind
            ))
        })
}

/// Setting materials are locked only while the conception session runs (spec/08 §1.4).
pub async fn ensure_editable(tx: &impl ConnectionTrait, book_id: i32) -> Result<(), AppError> {
    if let Some(session_id) = super::conception_session_running(tx, book_id).await? {
        return Err(super::conflict(
            ErrorCode::SettingsLocked,
            "the conception agent is working on this book",
            serde_json::json!({ "session_id": session_id }),
        ));
    }
    Ok(())
}

/// Maps a row to its DTO.
pub fn to_dto(row: &setting_docs::Model) -> Result<SettingDocDto, AppError> {
    Ok(SettingDocDto {
        id: row.id,
        book_id: row.book_id,
        kind: row.kind,
        content: row.content.clone(),
        revision: row.revision,
        approved_revision: row.approved_revision,
        updated_at: row.updated_at,
    })
}

/// The canonical DTO of the current row (used by tests).
pub async fn dto(
    conn: &impl ConnectionTrait,
    book_id: i32,
    kind: SettingDocKind,
) -> Result<SettingDocDto, AppError> {
    let row = load(conn, book_id, kind).await?;
    to_dto(&row)
}

/// Parses a path segment (`outline`, `world`, `protagonist`) into a kind.
pub fn parse_kind(raw: &str) -> Result<SettingDocKind, AppError> {
    SettingDocKind::from_str_opt(raw).ok_or_else(|| {
        AppError::validation(format!(
            "unknown setting document {raw:?}; expected outline, world or protagonist"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_segments_parse_into_kinds() {
        assert_eq!(
            parse_kind("outline").expect("outline"),
            SettingDocKind::Outline
        );
        assert_eq!(parse_kind("world").expect("world"), SettingDocKind::World);
        assert_eq!(
            parse_kind("protagonist").expect("protagonist"),
            SettingDocKind::Protagonist
        );
        let error = parse_kind("characters").expect_err("unknown kind");
        assert_eq!(error.code, ErrorCode::ValidationFailed);
    }

    #[test]
    fn there_are_exactly_three_setting_document_kinds() {
        assert_eq!(super::super::books::SETTING_DOC_KINDS.len(), 3);
    }
}
