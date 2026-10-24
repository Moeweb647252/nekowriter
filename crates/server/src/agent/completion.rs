//! Completion handlers: the domain effects of a completing tool call (spec/03 §5.4, spec/08).
//!
//! A call's `complete` effect runs **inside** the transaction that finalizes the call (D-033), so
//! the domain change, the call's result and its status commit together. M3 provides the framework
//! and the chapter `draft` handler (the generic "a draft finished" step); the review, revision,
//! maintenance and research handlers arrive with the milestones that own their tools and are
//! dispatched from [`super::context::apply_completion`].

use chrono::Utc;
use sea_orm::{ActiveModelTrait, ConnectionTrait, EntityTrait, Set};

use crate::db::entities::{agent_sessions, chapter_versions, chapters};
use crate::db::json::{ChapterContent, ChapterStatus, VersionKind};
use crate::error::AppError;
use crate::events::{ContentActor, ServerEvent};
use crate::text;

use super::session;

/// `finish_draft` (or a completing `insert_text`): the chapter gets a `draft` version and moves
/// to `in_review` (spec/03 §5.4, spec/08 §1.1).
///
/// The writer milestone (M4.2) owns the tool that requests this; the handler lives here because
/// finalize is runtime code.
pub async fn finish_draft(
    tx: &sea_orm::DatabaseTransaction,
    session: &agent_sessions::Model,
    chapter_id: i32,
) -> Result<Vec<ServerEvent>, AppError> {
    let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(tx).await? else {
        return Ok(Vec::new());
    };
    let content: ChapterContent = crate::db::json::from_json_or_default(&chapter.content);
    let word_count = text::content_word_count(&content);

    // A `draft` version, so the text the author reads is recoverable (spec/08 §1.2).
    let version_no = next_version_no(tx, chapter_id).await?;
    chapter_versions::ActiveModel {
        chapter_id: Set(chapter_id),
        version_no: Set(version_no),
        kind: Set(VersionKind::Draft.as_str().to_owned()),
        title: Set(chapter.title.clone()),
        content: Set(crate::db::json::to_json(&content)
            .map_err(|err| AppError::internal(err.to_string()))?),
        word_count: Set(word_count),
        note: Set(String::new()),
        session_id: Set(Some(session.id)),
        created_at: Set(Utc::now()),
        ..Default::default()
    }
    .insert(tx)
    .await?;

    let now = Utc::now();
    chapters::ActiveModel {
        id: Set(chapter_id),
        status: Set(ChapterStatus::InReview.as_str().to_owned()),
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

/// The next `version_no` of a chapter (unique per chapter, spec/02 §2).
async fn next_version_no(tx: &impl ConnectionTrait, chapter_id: i32) -> Result<i32, AppError> {
    use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};
    let last = chapter_versions::Entity::find()
        .filter(chapter_versions::Column::ChapterId.eq(chapter_id))
        .order_by_desc(chapter_versions::Column::VersionNo)
        .one(tx)
        .await?
        .map(|row| row.version_no);
    Ok(last.map(|value| value + 1).unwrap_or(1))
}
