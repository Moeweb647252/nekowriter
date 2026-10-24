//! Tiered memory (spec/06 §4): the snapshot of canon behind `block.memory`, built for a target
//! position (track, chapter) from canon **as it is now**.
//!
//! M4 implements the short-term layer and the window bookkeeping: recent chapter summaries, the
//! previous chapter's tail and the protagonist's current state. The mid-term evidence ([§4.3])
//! and the long-term index ([§4.4]) read canon tables that arrive with M5, so their lists stay
//! empty until then; the rendering of every empty part is already fixed by `block.memory`
//! (spec/06 §4.5), so the model never has to guess.
//!
//! Labels are rendered by the prompt registry (`shared.chapter_label`, `shared.label`); nothing in
//! this module writes model-facing text.

use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::db::entities::{chapters, entities, tracks};
use crate::db::json::{BookSettings, ChapterContent, ChapterStatus};
use crate::error::AppError;
use crate::prompts::PromptSet;

/// How many characters of the previous chapter's tail `previous_tail` may carry (spec/06 §4.2).
pub const PREVIOUS_TAIL_LIMIT: usize = 800;

/// One chapter of the short-term window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryItem {
    /// Rendered by `shared.chapter_label`.
    pub label: String,
    pub title: String,
    pub summary: String,
}

/// The last paragraphs of the latest chapter before the target (spec/06 §4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailItem {
    pub label: String,
    pub text: String,
}

/// The protagonist's current state (spec/06 §4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityStateItem {
    pub id: i32,
    pub name: String,
    pub status: String,
    /// Rendered by `shared.label` (`entity_status`).
    pub status_label: String,
    pub state: String,
    /// A side track whose anchor lies before the newest change (M10).
    pub after_anchor: bool,
    pub attributes: Vec<(String, String)>,
}

/// The whole memory snapshot of spec/06 §4.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemorySnapshot {
    /// True when no earlier chapter of the track is official yet.
    pub is_first: bool,
    pub window_mid: i32,
    pub recent_summaries: Vec<SummaryItem>,
    pub previous_tail: Option<TailItem>,
    pub protagonist: Option<EntityStateItem>,
}

/// Builds the memory snapshot for a target chapter (spec/06 §4.1).
pub async fn snapshot(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    track: &tracks::Model,
    chapter: &chapters::Model,
) -> Result<MemorySnapshot, AppError> {
    let book_settings = book_settings(conn, chapter.book_id).await?;
    // P: the official chapters of the target's track before it (spec/06 §4.1).
    let prior = official_prior_chapters(conn, track.id, chapter.ord).await?;

    // The short-term window is the last N chapters of P.
    let recent_count = book_settings.recent_summary_count.max(0) as usize;
    let short_term: Vec<&chapters::Model> = prior
        .iter()
        .rev()
        .take(recent_count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    // The templates render `label` and `title` side by side (`{{ label }}《{{ title }}》`), so the
    // label never carries the title itself (spec/09 §1 `with_title`).
    let recent_summaries = short_term
        .iter()
        .map(|row| SummaryItem {
            label: chapter_label(prompts, track, row),
            title: row.title.clone(),
            summary: row.summary.clone(),
        })
        .collect();

    let previous_tail = match prior.last() {
        Some(row) => Some(TailItem {
            label: chapter_label(prompts, track, row),
            text: tail_text(&content_of(row)?, PREVIOUS_TAIL_LIMIT),
        }),
        None => None,
    };

    Ok(MemorySnapshot {
        is_first: prior.is_empty(),
        window_mid: book_settings.midterm_window.max(0),
        recent_summaries,
        previous_tail,
        protagonist: protagonist(conn, prompts, track).await?,
    })
}

/// The official chapters of a track with a smaller `ord`, oldest first (spec/06 §4.1 P).
async fn official_prior_chapters(
    conn: &impl ConnectionTrait,
    track_id: i32,
    ord: i32,
) -> Result<Vec<chapters::Model>, AppError> {
    Ok(chapters::Entity::find()
        .filter(chapters::Column::TrackId.eq(track_id))
        .filter(chapters::Column::Ord.lt(ord))
        .filter(chapters::Column::Status.eq(ChapterStatus::Official.as_str()))
        .order_by_asc(chapters::Column::Ord)
        .all(conn)
        .await?)
}

/// The bare chapter reference of a row (main track or side track, numbering only), without the
/// title. The rendering comes from `shared.chapter_label`, never from code.
fn chapter_label(prompts: &PromptSet, track: &tracks::Model, chapter: &chapters::Model) -> String {
    prompts.chapter_label(
        &track.kind,
        &track.title,
        i64::from(chapter.ord),
        &chapter.title,
        false,
    )
}

/// The last paragraphs of a chapter, as many as fit `limit` characters (at least one).
pub fn tail_text(content: &ChapterContent, limit: usize) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let mut total = 0usize;
    for paragraph in content.paragraphs.iter().rev() {
        let length = paragraph.text.chars().count();
        if !parts.is_empty() && total + length > limit {
            break;
        }
        total += length;
        parts.push(paragraph.text.as_str());
    }
    parts.reverse();
    parts.join("\n\n")
}

/// The protagonist the writer of the target chapter is told about (spec/06 §4.2).
///
/// M4 uses the visible protagonist (`is_protagonist = 1`); the side-track focus character and the
/// `after_anchor` flag arrive with M10 (spec/06 §4.1).
async fn protagonist(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    track: &tracks::Model,
) -> Result<Option<EntityStateItem>, AppError> {
    let visible = sea_orm::Condition::any()
        .add(entities::Column::TrackId.is_null())
        .add(entities::Column::TrackId.eq(track.id));
    let row = entities::Entity::find()
        .filter(entities::Column::BookId.eq(track.book_id))
        .filter(entities::Column::IsProtagonist.eq(1))
        .filter(visible)
        .one(conn)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let attributes: std::collections::BTreeMap<String, String> =
        crate::db::json::from_json_or_default(&row.attributes);
    Ok(Some(EntityStateItem {
        id: row.id,
        name: row.name,
        status_label: prompts.label("entity_status", &row.status),
        status: row.status,
        state: row.state,
        after_anchor: false,
        attributes: attributes.into_iter().collect(),
    }))
}

/// The effective book settings of a chapter's book (defaults when the row vanished).
async fn book_settings(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<BookSettings, AppError> {
    match crate::db::entities::books::Entity::find_by_id(book_id)
        .one(conn)
        .await?
    {
        Some(book) => Ok(crate::db::json::from_json_or_default(&book.settings)),
        None => Ok(BookSettings::default()),
    }
}

/// A chapter's stored content.
pub fn content_of(chapter: &chapters::Model) -> Result<ChapterContent, AppError> {
    crate::db::json::from_json(&chapter.content)
        .map_err(|err| AppError::internal(format!("corrupt chapters.content: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::json::Paragraph;

    fn content(paragraphs: &[&str]) -> ChapterContent {
        ChapterContent {
            paragraphs: paragraphs
                .iter()
                .enumerate()
                .map(|(index, text)| Paragraph {
                    pid: format!("p{}", index + 1),
                    text: (*text).to_owned(),
                })
                .collect(),
            next_pid: paragraphs.len() as i32 + 1,
            tail_open: false,
        }
    }

    #[test]
    fn the_tail_takes_the_last_paragraphs_that_fit() {
        let doc = content(&["one", "two", "three"]);
        assert_eq!(tail_text(&doc, 3), "three");
        assert_eq!(tail_text(&doc, 100), "one\n\ntwo\n\nthree");
    }

    #[test]
    fn the_tail_always_keeps_at_least_one_paragraph() {
        let doc = content(&["a very long paragraph that exceeds the limit by itself"]);
        assert_eq!(
            tail_text(&doc, 5),
            "a very long paragraph that exceeds the limit by itself"
        );
    }

    #[test]
    fn an_empty_chapter_has_an_empty_tail() {
        assert_eq!(tail_text(&content(&[]), 100), "");
    }
}
