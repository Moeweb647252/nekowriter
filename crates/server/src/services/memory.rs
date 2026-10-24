//! Tiered memory (spec/06 §4): the snapshot of canon behind `block.memory`, built for a target
//! position (track, chapter) from canon **as it is now**.
//!
//! The three tiers of spec/06 §4:
//!
//! - short-term (§4.2): recent chapter summaries, the previous chapter's tail, the protagonist;
//! - mid-term  (§4.3): recent canon change notes, appearance counts, absent important characters;
//! - long-term (§4.4): the index (key notes, open foreshadowing, the cast, archive counts).
//!
//! The window rules are §4.1: **P** is the official chapters of the target's track before it,
//! topped up from the main story for a side track (D-038); the short-term window is the last N of
//! P and the mid-term window the last M.
//!
//! Labels are rendered by the prompt registry (`shared.chapter_label`, `shared.label`); nothing in
//! this module writes model-facing text.

use std::collections::{BTreeMap, HashMap, HashSet};

use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};

use crate::db::entities::{
    appearances, change_log, chapters, entities, foreshadowing, memory_notes, timeline_events,
    tracks,
};
use crate::db::json::{
    BookSettings, ChapterContent, ChapterStatus, EntityKind, EntityStatus, ForeshadowingStatus,
    NoteKind, NoteStatus,
};
use crate::error::AppError;
use crate::prompts::PromptSet;

use super::canon::Visibility;

/// How many characters of the previous chapter's tail `previous_tail` may carry (spec/06 §4.2).
pub const PREVIOUS_TAIL_LIMIT: usize = 800;

/// The mid-term change cap (spec/06 §4.3).
pub const RECENT_CHANGES_LIMIT: usize = 40;

/// The appearance-count cap (spec/06 §4.3).
pub const APPEARANCES_LIMIT: usize = 30;

/// The absent-important cap (spec/06 §4.3).
pub const ABSENT_LIMIT: usize = 10;

/// The key-note cap (spec/06 §4.4).
pub const KEY_NOTES_LIMIT: usize = 15;

/// The open-foreshadowing cap (spec/06 §4.4).
pub const OPEN_FORESHADOWING_LIMIT: usize = 20;

/// The cast cap (spec/06 §4.4).
pub const CAST_LIMIT: usize = 40;

/// The entity count `block.cast_index` lists (spec/06 §4.5).
pub const CAST_INDEX_LIMIT: usize = 300;

/// An entity counts as important from this importance on (spec/06 §4.3, §4.4).
pub const IMPORTANT: i32 = 4;

// ───────────────────────────── the snapshot ─────────────────────────────

/// One chapter of the short-term window (spec/06 §4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryItem {
    pub chapter_id: i32,
    /// Rendered by `shared.chapter_label`.
    pub label: String,
    pub title: String,
    pub summary: String,
}

/// The last paragraphs of the latest chapter before the target (spec/06 §4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailItem {
    pub chapter_id: i32,
    pub label: String,
    pub text: String,
}

/// The protagonist's current state (spec/06 §4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityStateItem {
    pub id: i32,
    pub name: String,
    /// The stored status column (`active`, `dead`, …).
    pub status: String,
    /// Rendered by `shared.label` (`entity_status`).
    pub status_label: String,
    pub state: String,
    /// A side track whose anchor lies before the newest change (spec/06 §4.2, D-038).
    pub after_anchor: bool,
    pub attributes: Vec<(String, String)>,
}

/// One mid-term change note (spec/06 §4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeItem {
    pub chapter_id: Option<i32>,
    /// Rendered by `shared.chapter_label`.
    pub chapter_label: String,
    /// `Name(#id)`, or the relation phrased from its source entity.
    pub target: String,
    pub note: String,
}

/// One character's appearances inside the mid-term window (spec/06 §4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppearanceItem {
    pub id: i32,
    pub name: String,
    pub count: i32,
    pub chapters: Vec<AppearanceChapterItem>,
}

/// One chapter of an appearance count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppearanceChapterItem {
    pub chapter_id: i32,
    pub label: String,
    /// The stored role (`pov` / `major` / `minor` / `mentioned`).
    pub role: crate::db::json::AppearanceRole,
    /// Rendered by `shared.label` (`appearance_role`).
    pub role_label: String,
}

/// An important character that has not appeared lately (spec/06 §4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbsenceItem {
    pub id: i32,
    pub name: String,
    pub last_chapter_id: i32,
    pub last_label: String,
}

/// One key memory note of the long-term index (spec/06 §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteItem {
    pub id: i32,
    /// The stored kind (`fact` / `world_rule`), so the DTO can carry the enum value.
    pub kind: NoteKind,
    /// Rendered by `shared.label` (`note_kind`).
    pub kind_label: String,
    pub content: String,
}

/// One open foreshadowing item of the long-term index (spec/06 §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeshadowItem {
    pub id: i32,
    pub title: String,
    pub importance: i32,
    pub planted_chapter_id: Option<i32>,
    /// Rendered by `shared.chapter_label`.
    pub planted_label: String,
}

/// One cast row of the long-term index (spec/06 §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastItem {
    pub id: i32,
    pub name: String,
    pub kind: EntityKind,
    /// Rendered by `shared.label` (`entity_kind`).
    pub kind_label: String,
    pub summary: String,
    pub importance: i32,
    pub last_chapter_id: Option<i32>,
}

/// The archive size of `block.memory` (spec/06 §4.4 `counts`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CanonCounts {
    pub character: i32,
    pub organization: i32,
    pub location: i32,
    pub item: i32,
    pub other: i32,
    pub event: i32,
    pub fact: i32,
    pub world_rule: i32,
    pub note_other: i32,
    pub foreshadowing_open: i32,
}

/// A side track's time anchor (spec/06 §2, §4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorInfo {
    /// `before_start` | `after_chapter` | `after_end`.
    pub kind: String,
    pub chapter_id: Option<i32>,
    /// Rendered by `shared.chapter_label`.
    pub label: String,
}

/// The target chapter of the snapshot (spec/06 §4.2 `chapter`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChapterBrief {
    pub id: i32,
    pub number: i32,
    /// Rendered by `shared.chapter_label`.
    pub label: String,
    pub title: String,
    pub guidance: String,
    pub outline: String,
    pub target_min: i32,
    pub target_max: i32,
    pub has_outline: bool,
}

/// The whole memory snapshot of spec/06 §4.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemorySnapshot {
    pub chapter: ChapterBrief,
    /// True when no earlier chapter of the position is official yet (spec/06 §4.2).
    pub is_first: bool,
    pub window_mid: i32,
    pub recent_summaries: Vec<SummaryItem>,
    pub previous_tail: Option<TailItem>,
    pub protagonist: Option<EntityStateItem>,
    pub recent_changes: Vec<ChangeItem>,
    pub appearances: Vec<AppearanceItem>,
    pub absent_important: Vec<AbsenceItem>,
    pub key_notes: Vec<NoteItem>,
    pub open_foreshadowing: Vec<ForeshadowItem>,
    pub cast: Vec<CastItem>,
    pub counts: CanonCounts,
    pub anchor: Option<AnchorInfo>,
}

// ───────────────────────────── building the snapshot ─────────────────────────────

/// A chapter of P with its rendered label (spec/06 §4.1).
#[derive(Debug, Clone)]
struct ChapterEntry {
    chapter: chapters::Model,
    label: String,
}

/// Builds the memory snapshot for a target chapter (spec/06 §4).
pub async fn snapshot(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    track: &tracks::Model,
    chapter: &chapters::Model,
) -> Result<MemorySnapshot, AppError> {
    let settings = book_settings(conn, chapter.book_id).await?;
    let visibility = super::canon::visibility_of_track(chapter.book_id, Some(track));
    let prior = prior_chapters(conn, prompts, track, chapter, &settings).await?;
    let labels: HashMap<i32, String> = prior
        .iter()
        .map(|entry| (entry.chapter.id, entry.label.clone()))
        .collect();

    // §4.1: the short-term window is the last N chapters of P, the mid-term window the last M.
    let short_start = prior
        .len()
        .saturating_sub(settings.recent_summary_count.max(0) as usize);
    let recent_summaries = prior[short_start..]
        .iter()
        .map(|entry| SummaryItem {
            chapter_id: entry.chapter.id,
            label: entry.label.clone(),
            title: entry.chapter.title.clone(),
            summary: entry.chapter.summary.clone(),
        })
        .collect();
    let previous_tail = match prior.last() {
        Some(entry) => Some(TailItem {
            chapter_id: entry.chapter.id,
            label: entry.label.clone(),
            text: tail_text(&content_of(&entry.chapter)?, PREVIOUS_TAIL_LIMIT),
        }),
        None => None,
    };
    let mid_start = prior
        .len()
        .saturating_sub(settings.midterm_window.max(0) as usize);
    let window: Vec<i32> = prior[mid_start..]
        .iter()
        .map(|entry| entry.chapter.id)
        .collect();

    Ok(MemorySnapshot {
        chapter: chapter_brief(prompts, track, chapter, &settings),
        is_first: prior.is_empty(),
        window_mid: settings.midterm_window.max(0),
        recent_summaries,
        previous_tail,
        protagonist: protagonist(conn, prompts, track, chapter).await?,
        recent_changes: recent_changes(conn, prompts, chapter, &visibility, &window, &labels)
            .await?,
        appearances: appearance_counts(conn, prompts, chapter, &visibility, &window, &labels)
            .await?,
        absent_important: absent_important(conn, chapter, &visibility, &prior, &window, &labels)
            .await?,
        key_notes: key_notes(conn, prompts, &visibility).await?,
        open_foreshadowing: open_foreshadowing(conn, prompts, &visibility, &labels).await?,
        cast: cast(conn, prompts, &visibility).await?,
        counts: counts(conn, &visibility).await?,
        anchor: anchor(conn, prompts, track).await?,
    })
}

// ───────────────────────────── §4.1 windows ─────────────────────────────

/// P: the official chapters before the target, topped up from the main story for a side track
/// (spec/06 §4.1).
async fn prior_chapters(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    track: &tracks::Model,
    chapter: &chapters::Model,
    settings: &BookSettings,
) -> Result<Vec<ChapterEntry>, AppError> {
    let own = official_before(conn, track.id, chapter.ord).await?;
    let entries: Vec<ChapterEntry> = own
        .into_iter()
        .map(|row| ChapterEntry {
            label: label_of(prompts, track, &row),
            chapter: row,
        })
        .collect();
    // A side story's first chapter still gets a short-term window: main-track chapters up to the
    // anchor are added in front until P has max(N, M) (spec/06 §4.1, D-038).
    let wanted = settings
        .midterm_window
        .max(settings.recent_summary_count)
        .max(0) as usize;
    if track.kind != "side" || entries.len() >= wanted {
        return Ok(entries);
    }
    // "Main-track official chapters up to the anchor are added in front" (spec/06 §4.1): an
    // `after_chapter` anchor fills from the anchor chapter and everything before it, `after_end`
    // from the whole main story, and `before_start` from nothing (the side story starts before
    // the main story). A side track without an anchor has no position to fill from.
    if !matches!(track.anchor_kind.as_str(), "after_chapter" | "after_end") {
        return Ok(entries);
    }
    let Some(main) = main_track(conn, track.book_id).await? else {
        return Ok(entries);
    };
    let mut statement = chapters::Entity::find()
        .filter(chapters::Column::TrackId.eq(main.id))
        .filter(chapters::Column::Status.eq(ChapterStatus::Official.as_str()))
        .order_by_asc(chapters::Column::Ord);
    if track.anchor_kind == "after_chapter" {
        let Some(up_to) = anchor_ord(conn, track).await? else {
            return Ok(entries);
        };
        statement = statement.filter(chapters::Column::Ord.lte(up_to));
    }
    let mut main_rows = statement.all(conn).await?;
    let missing = wanted.saturating_sub(entries.len());
    let skip = main_rows.len().saturating_sub(missing);
    main_rows.drain(..skip);
    let mut topped: Vec<ChapterEntry> = main_rows
        .into_iter()
        .map(|row| ChapterEntry {
            label: label_of(prompts, &main, &row),
            chapter: row,
        })
        .collect();
    topped.extend(entries);
    Ok(topped)
}

/// The chapter ids of the mid-term window W of a target position (spec/06 §4.1, §4.3): the last
/// M chapters of P, topped up from the main story for a side track. The retrieval tools use it
/// too, so `get_entity`'s "count in the mid-term window" means the window the memory block shows.
pub async fn midterm_window_chapter_ids(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    track: &tracks::Model,
    chapter: &chapters::Model,
) -> Result<Vec<i32>, AppError> {
    let settings = book_settings(conn, chapter.book_id).await?;
    let prior = prior_chapters(conn, prompts, track, chapter, &settings).await?;
    let mid_start = prior
        .len()
        .saturating_sub(settings.midterm_window.max(0) as usize);
    Ok(prior[mid_start..]
        .iter()
        .map(|entry| entry.chapter.id)
        .collect())
}

/// The official chapters of a track with a smaller `ord`, oldest first.
async fn official_before(
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

/// The book's main track (spec/02 §5 (1)).
async fn main_track(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Option<tracks::Model>, AppError> {
    Ok(tracks::Entity::find()
        .filter(tracks::Column::BookId.eq(book_id))
        .filter(tracks::Column::Kind.eq("main"))
        .one(conn)
        .await?)
}

/// The `ord` of a side track's `after_chapter` anchor (spec/08 §6).
async fn anchor_ord(
    conn: &impl ConnectionTrait,
    track: &tracks::Model,
) -> Result<Option<i32>, AppError> {
    let Some(chapter_id) = track.anchor_chapter_id else {
        return Ok(None);
    };
    Ok(chapters::Entity::find_by_id(chapter_id)
        .one(conn)
        .await?
        .map(|row| row.ord))
}

/// A chapter's label, rendered by `shared.chapter_label` (spec/09 §1).
///
/// The memory templates render the label and the title side by side
/// (`{{ label }}《{{ title }}》`), so the label never carries the title itself.
fn label_of(prompts: &PromptSet, track: &tracks::Model, chapter: &chapters::Model) -> String {
    prompts.chapter_label(
        &track.kind,
        &track.title,
        i64::from(chapter.ord),
        &chapter.title,
        false,
    )
}

/// The target chapter's brief (spec/06 §4.2 `chapter`).
fn chapter_brief(
    prompts: &PromptSet,
    track: &tracks::Model,
    chapter: &chapters::Model,
    settings: &BookSettings,
) -> ChapterBrief {
    ChapterBrief {
        id: chapter.id,
        number: chapter.ord,
        label: label_of(prompts, track, chapter),
        title: chapter.title.clone(),
        guidance: chapter.guidance.clone(),
        has_outline: !chapter.outline.trim().is_empty(),
        outline: chapter.outline.clone(),
        target_min: settings.target_words_min,
        target_max: settings.target_words_max,
    }
}

// ───────────────────────────── §4.2 short-term ─────────────────────────────

/// The protagonist the writer of the target chapter is told about (spec/06 §4.2).
///
/// A side track's `focus_entity_id` wins (a story about another character); otherwise the visible
/// entity with `is_protagonist = 1`. In a side track whose anchor lies before the newest change of
/// the state, `after_anchor` is set so the model knows the state may not apply yet (D-038).
async fn protagonist(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    track: &tracks::Model,
    chapter: &chapters::Model,
) -> Result<Option<EntityStateItem>, AppError> {
    let visibility = super::canon::visibility_of_track(chapter.book_id, Some(track));
    let row = match track.focus_entity_id {
        Some(focus_id) => entities::Entity::find_by_id(focus_id).one(conn).await?,
        None => {
            entities::Entity::find()
                .filter(entities::Column::BookId.eq(track.book_id))
                .filter(entities::Column::IsProtagonist.eq(1))
                .filter(visibility.visible_condition(entities::Column::TrackId))
                .one(conn)
                .await?
        }
    };
    let Some(row) = row else {
        return Ok(None);
    };
    let attributes: BTreeMap<String, String> =
        crate::db::json::from_json_or_default(&row.attributes);
    Ok(Some(EntityStateItem {
        id: row.id,
        name: row.name,
        status_label: prompts.label("entity_status", &row.status),
        status: row.status,
        state: row.state,
        after_anchor: state_after_anchor(conn, track, row.id).await?,
        attributes: attributes.into_iter().collect(),
    }))
}

/// True when the newest change of an entity came from a main-track chapter after the anchor
/// (spec/06 §4.2, D-038).
pub async fn state_after_anchor(
    conn: &impl ConnectionTrait,
    track: &tracks::Model,
    entity_id: i32,
) -> Result<bool, AppError> {
    if track.kind != "side" || track.anchor_kind != "after_chapter" {
        return Ok(false);
    }
    let Some(anchor_id) = track.anchor_chapter_id else {
        return Ok(false);
    };
    let Some(anchor) = chapters::Entity::find_by_id(anchor_id).one(conn).await? else {
        return Ok(false);
    };
    let last = change_log::Entity::find()
        .filter(change_log::Column::TargetType.eq("entity"))
        .filter(change_log::Column::TargetId.eq(entity_id))
        .filter(change_log::Column::RevertedById.is_null())
        .order_by_desc(change_log::Column::Id)
        .one(conn)
        .await?;
    let Some(change) = last else {
        return Ok(false);
    };
    let Some(chapter_id) = change.chapter_id else {
        return Ok(false);
    };
    let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(conn).await? else {
        return Ok(false);
    };
    Ok(chapter.track_id == anchor.track_id && chapter.ord > anchor.ord)
}

// ───────────────────────────── §4.3 mid-term ─────────────────────────────

/// Visible, non-reverted entity/relation change notes inside the window (spec/06 §4.3).
async fn recent_changes(
    conn: &impl ConnectionTrait,
    _prompts: &PromptSet,
    chapter: &chapters::Model,
    visibility: &Visibility,
    window: &[i32],
    labels: &HashMap<i32, String>,
) -> Result<Vec<ChangeItem>, AppError> {
    if window.is_empty() {
        return Ok(Vec::new());
    }
    let rows = change_log::Entity::find()
        .filter(change_log::Column::BookId.eq(chapter.book_id))
        .filter(visibility.visible_condition(change_log::Column::TrackId))
        .filter(change_log::Column::RevertedById.is_null())
        .filter(change_log::Column::TargetType.is_in(["entity", "relation"]))
        .filter(change_log::Column::ChapterId.is_in(window.to_vec()))
        .filter(change_log::Column::Note.ne(""))
        .all(conn)
        .await?;
    // "Ordered by chapter, then id; at most 40 (the newest are kept)" (spec/06 §4.3). The window
    // is already in story order (P: topped-up main chapters then the track's own), which is the
    // right order for a side track too — its chapter numbers restart at 1, so sorting by `ord`
    // alone would put side chapter 1 before the main chapters it follows.
    let chapter_position: HashMap<i32, usize> = window
        .iter()
        .enumerate()
        .map(|(position, chapter_id)| (*chapter_id, position))
        .collect();
    let mut ordered: Vec<&change_log::Model> = rows.iter().collect();
    ordered.sort_by_key(|row| {
        (
            row.chapter_id
                .and_then(|id| chapter_position.get(&id).copied())
                .unwrap_or(usize::MAX),
            row.id,
        )
    });
    let keep = ordered.len().saturating_sub(RECENT_CHANGES_LIMIT);
    let mut items = Vec::new();
    for row in &ordered[keep..] {
        let target = change_target(conn, row).await?;
        if target.is_empty() {
            continue;
        }
        items.push(ChangeItem {
            chapter_id: row.chapter_id,
            chapter_label: row
                .chapter_id
                .and_then(|id| labels.get(&id).cloned())
                .unwrap_or_default(),
            target,
            note: row.note.clone(),
        });
    }
    Ok(items)
}

/// The stored target label of a change (spec/05 §1.5: entities render as `名字(#id)`).
async fn change_target(
    conn: &impl ConnectionTrait,
    change: &change_log::Model,
) -> Result<String, AppError> {
    match change.target_type.as_str() {
        "entity" => {
            let name = entities::Entity::find_by_id(change.target_id)
                .one(conn)
                .await?
                .map(|row| row.name)
                .unwrap_or_default();
            if name.is_empty() {
                return Ok(String::new());
            }
            Ok(format!("{name}(#{})", change.target_id))
        }
        "relation" => {
            let Some(relation) =
                crate::db::entities::relations::Entity::find_by_id(change.target_id)
                    .one(conn)
                    .await?
            else {
                return Ok(String::new());
            };
            let source = entities::Entity::find_by_id(relation.source_id)
                .one(conn)
                .await?;
            let target = entities::Entity::find_by_id(relation.target_id)
                .one(conn)
                .await?;
            match (source, target) {
                (Some(source), Some(target)) => Ok(format!(
                    "{}(#{}) → {}(#{}) ({})",
                    source.name, source.id, target.name, target.id, relation.kind
                )),
                _ => Ok(String::new()),
            }
        }
        _ => Ok(String::new()),
    }
}

/// Characters with appearances in the window, sorted by count desc, at most 30 (spec/06 §4.3).
async fn appearance_counts(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    chapter: &chapters::Model,
    visibility: &Visibility,
    window: &[i32],
    labels: &HashMap<i32, String>,
) -> Result<Vec<AppearanceItem>, AppError> {
    if window.is_empty() {
        return Ok(Vec::new());
    }
    let rows = appearances::Entity::find()
        .filter(appearances::Column::ChapterId.is_in(window.to_vec()))
        .all(conn)
        .await?;
    let entity_ids: Vec<i32> = {
        let mut ids: Vec<i32> = rows.iter().map(|row| row.entity_id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    if entity_ids.is_empty() {
        return Ok(Vec::new());
    }
    let characters = entities::Entity::find()
        .filter(entities::Column::Id.is_in(entity_ids))
        .filter(entities::Column::BookId.eq(chapter.book_id))
        .filter(entities::Column::Kind.eq(EntityKind::Character.as_str()))
        .filter(visibility.visible_condition(entities::Column::TrackId))
        .all(conn)
        .await?;
    let names: HashMap<i32, String> = characters
        .iter()
        .map(|row| (row.id, row.name.clone()))
        .collect();
    let mut by_entity: HashMap<i32, Vec<AppearanceChapterItem>> = HashMap::new();
    for row in &rows {
        let Some(_) = names.get(&row.entity_id) else {
            continue;
        };
        by_entity
            .entry(row.entity_id)
            .or_default()
            .push(AppearanceChapterItem {
                chapter_id: row.chapter_id,
                label: labels.get(&row.chapter_id).cloned().unwrap_or_default(),
                role: crate::db::json::AppearanceRole::from_str_opt(&row.role)
                    .unwrap_or(crate::db::json::AppearanceRole::Mentioned),
                role_label: prompts.label("appearance_role", &row.role),
            });
    }
    let mut items: Vec<AppearanceItem> = by_entity
        .into_iter()
        .map(|(id, chapters)| AppearanceItem {
            id,
            name: names.get(&id).cloned().unwrap_or_default(),
            count: chapters.len() as i32,
            chapters,
        })
        .collect();
    items.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.id.cmp(&b.id)));
    items.truncate(APPEARANCES_LIMIT);
    Ok(items)
}

/// Active important characters that appear in P but in no chapter of the window (spec/06 §4.3).
async fn absent_important(
    conn: &impl ConnectionTrait,
    chapter: &chapters::Model,
    visibility: &Visibility,
    prior: &[ChapterEntry],
    window: &[i32],
    labels: &HashMap<i32, String>,
) -> Result<Vec<AbsenceItem>, AppError> {
    let window: HashSet<i32> = window.iter().copied().collect();
    let prior_ids: Vec<i32> = prior.iter().map(|entry| entry.chapter.id).collect();
    if prior_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = appearances::Entity::find()
        .filter(appearances::Column::ChapterId.is_in(prior_ids))
        .all(conn)
        .await?;
    // The latest appearance of each entity in P (P is oldest first, so the last write wins).
    let mut latest: HashMap<i32, i32> = HashMap::new();
    let mut seen_in_window: HashSet<i32> = HashSet::new();
    for row in &rows {
        if window.contains(&row.chapter_id) {
            seen_in_window.insert(row.entity_id);
            continue;
        }
        latest.insert(row.entity_id, row.chapter_id);
    }
    let candidates: Vec<i32> = latest
        .keys()
        .copied()
        .filter(|id| !seen_in_window.contains(id))
        .collect();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let rows = entities::Entity::find()
        .filter(entities::Column::Id.is_in(candidates))
        .filter(entities::Column::BookId.eq(chapter.book_id))
        .filter(entities::Column::Kind.eq(EntityKind::Character.as_str()))
        .filter(entities::Column::Status.eq(EntityStatus::Active.as_str()))
        .filter(entities::Column::Importance.gte(IMPORTANT))
        .filter(visibility.visible_condition(entities::Column::TrackId))
        .all(conn)
        .await?;
    let mut items: Vec<AbsenceItem> = rows
        .into_iter()
        .map(|row| {
            let last_chapter_id = latest.get(&row.id).copied().unwrap_or_default();
            AbsenceItem {
                id: row.id,
                name: row.name,
                last_chapter_id,
                last_label: labels.get(&last_chapter_id).cloned().unwrap_or_default(),
            }
        })
        .collect();
    items.sort_by_key(|item| item.id);
    items.truncate(ABSENT_LIMIT);
    Ok(items)
}

// ───────────────────────────── §4.4 long-term index ─────────────────────────────

/// Active facts and world rules with importance ≥ 4, at most 15 (spec/06 §4.4).
async fn key_notes(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    visibility: &Visibility,
) -> Result<Vec<NoteItem>, AppError> {
    let mut rows = memory_notes::Entity::find()
        .filter(memory_notes::Column::BookId.eq(visibility.book_id))
        .filter(visibility.visible_condition(memory_notes::Column::TrackId))
        .filter(memory_notes::Column::Status.eq(NoteStatus::Active.as_str()))
        .filter(
            memory_notes::Column::Kind
                .is_in([NoteKind::Fact.as_str(), NoteKind::WorldRule.as_str()]),
        )
        .filter(memory_notes::Column::Importance.gte(IMPORTANT))
        .order_by_desc(memory_notes::Column::Importance)
        .order_by_asc(memory_notes::Column::Id)
        .all(conn)
        .await?;
    rows.truncate(KEY_NOTES_LIMIT);
    Ok(rows
        .into_iter()
        .map(|row| NoteItem {
            id: row.id,
            kind: NoteKind::from_str_opt(&row.kind).unwrap_or(NoteKind::Other),
            kind_label: prompts.label("note_kind", &row.kind),
            content: row.content,
        })
        .collect())
}

/// Open items sorted by importance desc, then planted chapter asc, at most 20 (spec/06 §4.4).
///
/// Every open item is listed — unlike `key_notes`, this part has no importance threshold.
async fn open_foreshadowing(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    visibility: &Visibility,
    labels: &HashMap<i32, String>,
) -> Result<Vec<ForeshadowItem>, AppError> {
    let mut rows = foreshadowing::Entity::find()
        .filter(foreshadowing::Column::BookId.eq(visibility.book_id))
        .filter(visibility.visible_condition(foreshadowing::Column::TrackId))
        .filter(foreshadowing::Column::Status.eq(ForeshadowingStatus::Open.as_str()))
        .all(conn)
        .await?;
    rows.sort_by(|a, b| {
        b.importance
            .cmp(&a.importance)
            .then_with(|| {
                a.planted_chapter_id
                    .unwrap_or(i32::MAX)
                    .cmp(&b.planted_chapter_id.unwrap_or(i32::MAX))
            })
            .then_with(|| a.id.cmp(&b.id))
    });
    rows.truncate(OPEN_FORESHADOWING_LIMIT);
    let _ = prompts;
    Ok(rows
        .into_iter()
        .map(|row| ForeshadowItem {
            id: row.id,
            title: row.title,
            importance: row.importance,
            planted_chapter_id: row.planted_chapter_id,
            planted_label: row
                .planted_chapter_id
                .and_then(|id| labels.get(&id).cloned())
                .unwrap_or_default(),
        })
        .collect())
}

/// Visible entities sorted by importance desc, then last appearance desc, at most 40
/// (spec/06 §4.4).
async fn cast(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    visibility: &Visibility,
) -> Result<Vec<CastItem>, AppError> {
    let mut rows = entities::Entity::find()
        .filter(entities::Column::BookId.eq(visibility.book_id))
        .filter(visibility.visible_condition(entities::Column::TrackId))
        .all(conn)
        .await?;
    rows.sort_by(|a, b| {
        b.importance
            .cmp(&a.importance)
            .then_with(|| b.last_chapter_id.cmp(&a.last_chapter_id))
            .then_with(|| a.id.cmp(&b.id))
    });
    rows.truncate(CAST_LIMIT);
    Ok(rows
        .into_iter()
        .map(|row| CastItem {
            id: row.id,
            name: row.name,
            kind_label: prompts.label("entity_kind", &row.kind),
            kind: EntityKind::from_str_opt(&row.kind).unwrap_or(EntityKind::Other),
            summary: row.summary,
            importance: row.importance,
            last_chapter_id: row.last_chapter_id,
        })
        .collect())
}

/// Entities per kind, timeline events, active notes per kind, open foreshadowing (spec/06 §4.4).
async fn counts(
    conn: &impl ConnectionTrait,
    visibility: &Visibility,
) -> Result<CanonCounts, AppError> {
    let mut counts = CanonCounts::default();
    let entity_rows = entities::Entity::find()
        .filter(entities::Column::BookId.eq(visibility.book_id))
        .filter(visibility.visible_condition(entities::Column::TrackId))
        .all(conn)
        .await?;
    for row in &entity_rows {
        match row.kind.as_str() {
            "character" => counts.character += 1,
            "organization" => counts.organization += 1,
            "location" => counts.location += 1,
            "item" => counts.item += 1,
            _ => counts.other += 1,
        }
    }
    counts.event = timeline_events::Entity::find()
        .filter(timeline_events::Column::BookId.eq(visibility.book_id))
        .filter(visibility.visible_condition(timeline_events::Column::TrackId))
        .count(conn)
        .await? as i32;
    let note_rows = memory_notes::Entity::find()
        .filter(memory_notes::Column::BookId.eq(visibility.book_id))
        .filter(visibility.visible_condition(memory_notes::Column::TrackId))
        .filter(memory_notes::Column::Status.eq(NoteStatus::Active.as_str()))
        .all(conn)
        .await?;
    for row in &note_rows {
        match row.kind.as_str() {
            "fact" => counts.fact += 1,
            "world_rule" => counts.world_rule += 1,
            _ => counts.note_other += 1,
        }
    }
    counts.foreshadowing_open = foreshadowing::Entity::find()
        .filter(foreshadowing::Column::BookId.eq(visibility.book_id))
        .filter(visibility.visible_condition(foreshadowing::Column::TrackId))
        .filter(foreshadowing::Column::Status.eq(ForeshadowingStatus::Open.as_str()))
        .count(conn)
        .await? as i32;
    Ok(counts)
}

/// A side track's anchor (spec/06 §2, §4.2); `None` for the main track.
async fn anchor(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    track: &tracks::Model,
) -> Result<Option<AnchorInfo>, AppError> {
    if track.kind == "main" || track.anchor_kind == "none" {
        return Ok(None);
    }
    let (chapter_id, label) = match track.anchor_chapter_id {
        Some(chapter_id) => match chapters::Entity::find_by_id(chapter_id).one(conn).await? {
            Some(chapter) => {
                let main = main_track(conn, track.book_id).await?;
                let label = main
                    .as_ref()
                    .map(|main| label_of(prompts, main, &chapter))
                    .unwrap_or_default();
                (Some(chapter_id), label)
            }
            None => (Some(chapter_id), String::new()),
        },
        None => (None, String::new()),
    };
    Ok(Some(AnchorInfo {
        kind: track.anchor_kind.clone(),
        chapter_id,
        label,
    }))
}

// ───────────────────────────── helpers ─────────────────────────────

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

/// A chapter's stored content.
pub fn content_of(chapter: &chapters::Model) -> Result<ChapterContent, AppError> {
    crate::db::json::from_json(&chapter.content)
        .map_err(|err| AppError::internal(format!("corrupt chapters.content: {err}")))
}

// ───────────────────────────── the Memory page API (spec/10 §2.4) ─────────────────────────────

/// `GET /api/books/:id/memory?track_id&chapter_id` (spec/10 §2.4): the snapshot the writer of the
/// target chapter receives, plus the exact `block.memory` text (spec/06 §4.5).
///
/// Without `chapter_id` the target is the next chapter of the main track (the position the author
/// would write next); the track defaults to the main track.
pub async fn view(
    state: &crate::state::AppState,
    book_id: i32,
    query: &crate::api::dto::canon::MemoryQuery,
) -> Result<crate::api::dto::canon::MemoryViewDto, AppError> {
    let prompts = state.prompts().for_book(state.db(), Some(book_id)).await?;
    let (track, chapter) = target_position(state.db().conn(), book_id, query).await?;
    let snapshot = snapshot(state.db().conn(), &prompts, &track, &chapter).await?;
    Ok(crate::api::dto::canon::MemoryViewDto {
        rendered: render_block_memory(&prompts, &snapshot)?,
        snapshot: snapshot_dto(&snapshot),
    })
}

/// The `(track, chapter)` a memory target position names (spec/10 §2.4).
///
/// A `chapter_id` names a position, so its own track is the target track; `track_id` is only
/// needed for "the next chapter of this track". Both together must agree.
async fn target_position(
    conn: &impl ConnectionTrait,
    book_id: i32,
    query: &crate::api::dto::canon::MemoryQuery,
) -> Result<(tracks::Model, chapters::Model), AppError> {
    let chapter = match query.chapter_id {
        Some(chapter_id) => Some(
            chapters::Entity::find_by_id(chapter_id)
                .filter(chapters::Column::BookId.eq(book_id))
                .one(conn)
                .await?
                .ok_or_else(|| AppError::not_found(format!("chapter #{chapter_id}")))?,
        ),
        None => None,
    };
    let track = match (query.track_id, &chapter) {
        (Some(track_id), Some(chapter)) => {
            let track = tracks::Entity::find_by_id(track_id)
                .filter(tracks::Column::BookId.eq(book_id))
                .one(conn)
                .await?
                .ok_or_else(|| AppError::not_found(format!("track #{track_id}")))?;
            if chapter.track_id != track.id {
                return Err(AppError::validation(format!(
                    "chapter #{} is not in track #{track_id}",
                    chapter.id
                )));
            }
            track
        }
        (Some(track_id), None) => tracks::Entity::find_by_id(track_id)
            .filter(tracks::Column::BookId.eq(book_id))
            .one(conn)
            .await?
            .ok_or_else(|| AppError::not_found(format!("track #{track_id}")))?,
        (None, Some(chapter)) => tracks::Entity::find_by_id(chapter.track_id)
            .one(conn)
            .await?
            .ok_or_else(|| AppError::not_found(format!("track #{}", chapter.track_id)))?,
        (None, None) => main_track(conn, book_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("the main track of book #{book_id}")))?,
    };
    // "The next chapter" of the track: the last one, so the page shows what writing it would see
    // (and a book with no chapters renders the empty states).
    let chapter = match chapter {
        Some(chapter) => chapter,
        None => next_chapter(conn, book_id, &track).await?,
    };
    Ok((track, chapter))
}

/// The chapter the Memory page defaults to (spec/10 §2.4 "the next chapter of the main track").
async fn next_chapter(
    conn: &impl ConnectionTrait,
    book_id: i32,
    track: &tracks::Model,
) -> Result<chapters::Model, AppError> {
    let last = chapters::Entity::find()
        .filter(chapters::Column::TrackId.eq(track.id))
        .order_by_desc(chapters::Column::Ord)
        .one(conn)
        .await?;
    match last {
        Some(chapter) => Ok(chapter),
        // A book with no chapters still gets a position, so the page renders the empty states of
        // a first chapter instead of a 404.
        None => Ok(crate::db::entities::chapters::Model {
            id: 0,
            book_id,
            track_id: track.id,
            ord: 1,
            title: String::new(),
            status: ChapterStatus::Planned.as_str().to_owned(),
            guidance: String::new(),
            outline: String::new(),
            reference_notes: String::new(),
            summary: String::new(),
            content: String::from("{\"paragraphs\":[],\"next_pid\":1,\"tail_open\":false}"),
            content_version: 0,
            word_count: 0,
            official_version_id: None,
            canon_status: "none".to_owned(),
            synced_version_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }),
    }
}

/// Renders `block.memory` exactly as a writer kickoff would (spec/06 §4.5).
///
/// The Memory page shows every part, so the outline is always included here: unlike a kickoff,
/// the page has no `block.chapter_draft` that would render it instead (spec/06 §4.2).
fn render_block_memory(prompts: &PromptSet, snapshot: &MemorySnapshot) -> Result<String, AppError> {
    let chapter = crate::prompts::context::ChapterContext {
        id: snapshot.chapter.id,
        number: snapshot.chapter.number,
        label: snapshot.chapter.label.clone(),
        title: snapshot.chapter.title.clone(),
        guidance: snapshot.chapter.guidance.clone(),
        outline: snapshot.chapter.outline.clone(),
        reference_notes: String::new(),
        target_min: snapshot.chapter.target_min,
        target_max: snapshot.chapter.target_max,
        word_count: 0,
        has_draft: false,
    };
    let rendered = prompts
        .render_value(
            "block.memory",
            crate::prompts::template_value(&serde_json::json!({
                "memory": memory_context(snapshot),
                "chapter": chapter,
            })),
        )
        .map_err(|err| err.into_app_error())?;
    Ok(rendered.text)
}

/// The `memory` context of `block.memory` (spec/09 §2 `vars.memory`).
pub fn memory_context(snapshot: &MemorySnapshot) -> crate::prompts::context::MemoryContext {
    use crate::prompts::context as ctx;
    ctx::MemoryContext {
        is_first: snapshot.is_first,
        window_mid: snapshot.window_mid,
        recent_summaries: snapshot
            .recent_summaries
            .iter()
            .map(|item| ctx::ChapterSummaryContext {
                label: item.label.clone(),
                title: item.title.clone(),
                summary: item.summary.clone(),
            })
            .collect(),
        previous_tail: snapshot
            .previous_tail
            .as_ref()
            .map(|tail| ctx::PreviousTailContext {
                label: tail.label.clone(),
                text: tail.text.clone(),
            }),
        protagonist: snapshot
            .protagonist
            .as_ref()
            .map(|item| ctx::ProtagonistStateContext {
                id: item.id,
                name: item.name.clone(),
                status: item.status.clone(),
                status_label: item.status_label.clone(),
                state: item.state.clone(),
                after_anchor: item.after_anchor,
                attributes: item
                    .attributes
                    .iter()
                    .map(|(key, value)| ctx::AttributeContext {
                        key: key.clone(),
                        value: value.clone(),
                    })
                    .collect(),
            }),
        recent_changes: snapshot
            .recent_changes
            .iter()
            .map(|item| ctx::RecentChangeContext {
                chapter_label: item.chapter_label.clone(),
                target: item.target.clone(),
                note: item.note.clone(),
            })
            .collect(),
        appearances: snapshot
            .appearances
            .iter()
            .map(|item| ctx::AppearanceCountContext {
                id: item.id,
                name: item.name.clone(),
                count: item.count,
                chapters: item
                    .chapters
                    .iter()
                    .map(|chapter| ctx::AppearanceChapterContext {
                        label: chapter.label.clone(),
                        role_label: chapter.role_label.clone(),
                    })
                    .collect(),
            })
            .collect(),
        absent_important: snapshot
            .absent_important
            .iter()
            .map(|item| ctx::AbsentEntityContext {
                id: item.id,
                name: item.name.clone(),
                last_label: item.last_label.clone(),
            })
            .collect(),
        key_notes: snapshot
            .key_notes
            .iter()
            .map(|item| ctx::KeyNoteContext {
                id: item.id,
                kind_label: item.kind_label.clone(),
                content: item.content.clone(),
            })
            .collect(),
        open_foreshadowing: snapshot
            .open_foreshadowing
            .iter()
            .map(|item| ctx::OpenForeshadowingContext {
                id: item.id,
                title: item.title.clone(),
                planted_label: item.planted_label.clone(),
            })
            .collect(),
        cast: snapshot
            .cast
            .iter()
            .map(|item| ctx::CastSummaryContext {
                id: item.id,
                name: item.name.clone(),
                kind_label: item.kind_label.clone(),
                summary: item.summary.clone(),
            })
            .collect(),
        counts: ctx::MemoryCountsContext {
            character: snapshot.counts.character,
            organization: snapshot.counts.organization,
            location: snapshot.counts.location,
            item: snapshot.counts.item,
            other: snapshot.counts.other,
            event: snapshot.counts.event,
            fact: snapshot.counts.fact,
            world_rule: snapshot.counts.world_rule,
            foreshadowing_open: snapshot.counts.foreshadowing_open,
        },
    }
}

/// Maps the snapshot to its DTO (spec/10 §2.4).
pub fn snapshot_dto(snapshot: &MemorySnapshot) -> crate::api::dto::canon::MemorySnapshotDto {
    use crate::api::dto::canon as dto;
    dto::MemorySnapshotDto {
        chapter: dto::MemoryChapterDto {
            id: snapshot.chapter.id,
            number: snapshot.chapter.number,
            label: snapshot.chapter.label.clone(),
            title: snapshot.chapter.title.clone(),
            guidance: snapshot.chapter.guidance.clone(),
            outline: snapshot.chapter.outline.clone(),
            target_min: snapshot.chapter.target_min,
            target_max: snapshot.chapter.target_max,
            has_outline: snapshot.chapter.has_outline,
        },
        is_first: snapshot.is_first,
        window_mid: snapshot.window_mid,
        recent_summaries: snapshot
            .recent_summaries
            .iter()
            .map(|item| dto::MemorySummaryDto {
                chapter_id: item.chapter_id,
                label: item.label.clone(),
                title: item.title.clone(),
                summary: item.summary.clone(),
            })
            .collect(),
        previous_tail: snapshot
            .previous_tail
            .as_ref()
            .map(|tail| dto::MemoryTailDto {
                chapter_id: tail.chapter_id,
                label: tail.label.clone(),
                text: tail.text.clone(),
            }),
        protagonist: snapshot
            .protagonist
            .as_ref()
            .map(|item| dto::MemoryProtagonistDto {
                id: item.id,
                name: item.name.clone(),
                status: EntityStatus::from_str_opt(&item.status).unwrap_or(EntityStatus::Active),
                state: item.state.clone(),
                after_anchor: item.after_anchor,
                attributes: item.attributes.iter().cloned().collect(),
            }),
        recent_changes: snapshot
            .recent_changes
            .iter()
            .map(|item| dto::MemoryChangeDto {
                chapter_id: item.chapter_id,
                chapter_label: item.chapter_label.clone(),
                target: item.target.clone(),
                note: item.note.clone(),
            })
            .collect(),
        appearances: snapshot
            .appearances
            .iter()
            .map(|item| dto::MemoryAppearanceDto {
                id: item.id,
                name: item.name.clone(),
                count: item.count,
                chapters: item
                    .chapters
                    .iter()
                    .map(|chapter| dto::MemoryAppearanceChapterDto {
                        chapter_id: chapter.chapter_id,
                        label: chapter.label.clone(),
                        // The role label is rendered; the DTO carries the enum value.
                        role: chapter.role,
                    })
                    .collect(),
            })
            .collect(),
        absent_important: snapshot
            .absent_important
            .iter()
            .map(|item| dto::MemoryAbsenceDto {
                id: item.id,
                name: item.name.clone(),
                last_chapter_id: item.last_chapter_id,
                last_label: item.last_label.clone(),
            })
            .collect(),
        key_notes: snapshot
            .key_notes
            .iter()
            .map(|item| dto::MemoryNoteItemDto {
                id: item.id,
                kind: item.kind,
                content: item.content.clone(),
            })
            .collect(),
        open_foreshadowing: snapshot
            .open_foreshadowing
            .iter()
            .map(|item| dto::MemoryForeshadowItemDto {
                id: item.id,
                title: item.title.clone(),
                importance: item.importance,
                planted_chapter_id: item.planted_chapter_id,
                planted_label: item.planted_label.clone(),
            })
            .collect(),
        cast: snapshot
            .cast
            .iter()
            .map(|item| dto::MemoryCastItemDto {
                id: item.id,
                name: item.name.clone(),
                kind: item.kind,
                summary: item.summary.clone(),
                importance: item.importance,
                last_chapter_id: item.last_chapter_id,
            })
            .collect(),
        counts: dto::MemoryCountsDto {
            character: snapshot.counts.character,
            organization: snapshot.counts.organization,
            location: snapshot.counts.location,
            item: snapshot.counts.item,
            other: snapshot.counts.other,
            event: snapshot.counts.event,
            fact: snapshot.counts.fact,
            world_rule: snapshot.counts.world_rule,
            note_other: snapshot.counts.note_other,
            foreshadowing_open: snapshot.counts.foreshadowing_open,
        },
        anchor: snapshot.anchor.as_ref().map(|anchor| dto::MemoryAnchorDto {
            kind: crate::db::json::AnchorKind::from_str_opt(&anchor.kind)
                .unwrap_or(crate::db::json::AnchorKind::None),
            chapter_id: anchor.chapter_id,
            label: anchor.label.clone(),
        }),
    }
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
