//! Completion handlers: the domain effects of a completing tool call (spec/03 §5.4, spec/08).
//!
//! A call's `complete` effect runs **inside** the transaction that finalizes the call (D-033), so
//! the domain change, the call's result and its status commit together. M3 provides the framework
//! and the chapter `draft` handler (the generic "a draft finished" step); the review, revision,
//! maintenance and research handlers arrive with the milestones that own their tools and are
//! dispatched from [`super::context::apply_completion`].

use chrono::Utc;
use sea_orm::{ActiveModelTrait, ConnectionTrait, EntityTrait, Set};

use crate::db::entities::{agent_sessions, agent_tool_calls, chapter_versions, chapters};
use crate::db::json::{CanonStatus, ChapterContent, ChapterStatus, JsonText, VersionKind};
use crate::error::AppError;
use crate::events::{ContentActor, ServerEvent};
use crate::text;

use crate::api::dto::ChapterListItemDto;

use super::session;

/// `finish_draft` (or a completing `insert_text`): the chapter gets a `draft` version and moves
/// to `in_review` (spec/03 §5.4, spec/08 §1.1).
///
/// The writer milestone (M4.2) owns the tool that requests this; the handler lives here because
/// finalize is runtime code.
pub async fn finish_draft(
    tx: &sea_orm::DatabaseTransaction,
    session: &agent_sessions::Model,
    call: &agent_tool_calls::Model,
    chapter_id: i32,
) -> Result<Vec<ServerEvent>, AppError> {
    finish_version(tx, session, call, chapter_id, VersionKind::Draft, "note").await
}

/// Saves a text version on completion (`draft` or `revision`) and moves the chapter to
/// `in_review` (spec/08 §1.1). `note_field` is the input field whose text becomes the version's
/// note.
async fn finish_version(
    tx: &sea_orm::DatabaseTransaction,
    session: &agent_sessions::Model,
    call: &agent_tool_calls::Model,
    chapter_id: i32,
    kind: VersionKind,
    note_field: &str,
) -> Result<Vec<ServerEvent>, AppError> {
    let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(tx).await? else {
        return Ok(Vec::new());
    };
    let content: ChapterContent = chapter.content.clone().into_inner();
    let word_count = text::content_word_count(&content);

    // A `draft`/`revision` version, so the text the author reads is recoverable (spec/08 §1.2).
    let version_no = next_version_no(tx, chapter_id).await?;
    chapter_versions::ActiveModel {
        chapter_id: Set(chapter_id),
        version_no: Set(version_no),
        kind: Set(kind),
        title: Set(chapter.title.clone()),
        content: Set(JsonText(content.clone())),
        word_count: Set(word_count),
        note: Set(completion_note(call, note_field)),
        session_id: Set(Some(session.id)),
        created_at: Set(Utc::now()),
        ..Default::default()
    }
    .insert(tx)
    .await?;

    let now = Utc::now();
    chapters::ActiveModel {
        id: Set(chapter_id),
        status: Set(ChapterStatus::InReview),
        word_count: Set(word_count),
        updated_at: Set(now),
        ..Default::default()
    }
    .update(tx)
    .await?;

    let chapter = chapters::Entity::find_by_id(chapter_id)
        .one(tx)
        .await?
        .ok_or_else(|| AppError::internal("chapter vanished after update"))?;
    Ok(vec![
        ServerEvent::ChapterUpdated {
            chapter: crate::api::dto::ChapterListItemDto::from(chapter.clone()),
            outline_changed: false,
        },
        ServerEvent::SessionUpdated {
            session: session::to_dto(tx, session, Vec::new()).await?,
            warnings: Vec::new(),
        },
        ServerEvent::ChapterContentChanged {
            chapter_id,
            version: chapter.content_version,
            ops: Vec::new(),
            word_count,
            tail_open: content.tail_open,
            actor: ContentActor::Agent,
        },
    ])
}

/// `finish_revision` (spec/05 §4, spec/08 §1.1): a `revision` version with the summary as its
/// note, then `in_review`.
pub async fn finish_revision(
    tx: &sea_orm::DatabaseTransaction,
    session: &agent_sessions::Model,
    call: &agent_tool_calls::Model,
    chapter_id: i32,
) -> Result<Vec<ServerEvent>, AppError> {
    finish_version(
        tx,
        session,
        call,
        chapter_id,
        VersionKind::Revision,
        "summary",
    )
    .await
}

/// `finish_maintenance` (spec/06 §6): the chapter's canon is synced to the official version it
/// was maintained from. When the author edited the text meanwhile, `content_version` no longer
/// equals the session's `base_content_version` and the chapter becomes `stale` (spec/03 §14,
/// D-026) — an author edit during maintenance is allowed and is exactly what produces this.
pub async fn finish_maintenance(
    tx: &sea_orm::DatabaseTransaction,
    session: &agent_sessions::Model,
    _call: &agent_tool_calls::Model,
    chapter_id: i32,
) -> Result<Vec<ServerEvent>, AppError> {
    let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(tx).await? else {
        return Ok(Vec::new());
    };
    let synced = session.base_content_version == Some(chapter.content_version);
    let canon_status = if synced {
        CanonStatus::Synced
    } else {
        CanonStatus::Stale
    };
    chapters::ActiveModel {
        id: Set(chapter_id),
        canon_status: Set(canon_status),
        synced_version_id: Set(chapter.official_version_id),
        updated_at: Set(Utc::now()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    let row = chapters::Entity::find_by_id(chapter_id)
        .one(tx)
        .await?
        .ok_or_else(|| AppError::internal("chapter vanished after maintenance"))?;
    Ok(vec![ServerEvent::ChapterUpdated {
        chapter: ChapterListItemDto::from(row),
        outline_changed: false,
    }])
}

/// The `canon_seed` completion (spec/06 §7): link every remaining character design to the entity
/// that carries its exact name or one of its aliases.
///
/// `create_entity` already links a shared character whose name matches an unlinked design as it is
/// created (spec/05 §6); this pass catches the rest — a design the archivist named differently, or
/// created only after the design existed. The rank-1 design's entity is not forced here: setting
/// `is_protagonist` is the archivist's `create_entity(is_protagonist: true)`, which only the seed
/// session may ask for (spec/05 §6, spec/06 §7).
pub async fn finish_seed(
    tx: &sea_orm::DatabaseTransaction,
    session: &agent_sessions::Model,
) -> Result<Vec<ServerEvent>, AppError> {
    use crate::db::entities::{character_designs, entities};
    use crate::db::json::EntityKind;
    use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};

    let Some(book_id) = session.book_id else {
        return Ok(Vec::new());
    };
    let designs = character_designs::Entity::find()
        .filter(character_designs::COLUMN.book_id.eq(book_id))
        .filter(character_designs::COLUMN.entity_id.is_null())
        .order_by_asc(character_designs::COLUMN.rank)
        .all(tx)
        .await?;
    if designs.is_empty() {
        return Ok(Vec::new());
    }
    let candidates = entities::Entity::find()
        .filter(entities::COLUMN.book_id.eq(book_id))
        .filter(entities::COLUMN.kind.eq(EntityKind::Character))
        .filter(entities::COLUMN.track_id.is_null())
        .all(tx)
        .await?;
    let mut linked = false;
    for design in designs {
        let found = candidates.iter().find(|entity| {
            entity.name == design.name
                || entity
                    .aliases
                    .clone()
                    .into_inner()
                    .iter()
                    .any(|alias| alias == &design.name)
        });
        let Some(entity) = found else {
            continue;
        };
        character_designs::ActiveModel {
            id: Set(design.id),
            entity_id: Set(Some(entity.id)),
            updated_at: Set(Utc::now()),
            ..Default::default()
        }
        .update(tx)
        .await?;
        linked = true;
    }
    if !linked {
        return Ok(Vec::new());
    }
    Ok(vec![ServerEvent::SettingsChanged {
        book_id,
        kind: crate::events::SettingsKind::Characters,
        doc_kind: None,
        revision: None,
    }])
}

/// The `note` of the version a completing call saves (spec/08 §1.1).
///
/// `finish_draft` carries the writer's own summary of the chapter in `note` (spec/05 §4); a
/// chapter-completing `insert_text` has no such field, so its version note stays empty. A revision
/// will read `finish_revision`'s `summary` here once M6.2 owns that handler.
fn completion_note(call: &agent_tool_calls::Model, field: &str) -> String {
    let Ok(input) = serde_json::from_str::<serde_json::Value>(&call.input) else {
        return String::new();
    };
    input
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

/// The next `version_no` of a chapter (unique per chapter, spec/02 §2).
async fn next_version_no(tx: &impl ConnectionTrait, chapter_id: i32) -> Result<i32, AppError> {
    use sea_orm::{QueryFilter, QueryOrder};
    let last = chapter_versions::Entity::find()
        .filter(chapter_versions::COLUMN.chapter_id.eq(chapter_id))
        .order_by_desc(chapter_versions::COLUMN.version_no)
        .one(tx)
        .await?
        .map(|row| row.version_no);
    Ok(last.map(|value| value + 1).unwrap_or(1))
}
