//! Retrieval tools (spec/05 §3, PLAN M5.4): `get_entity`, `list_entities`, `get_relations`,
//! `query_timeline`, `list_foreshadowing`, `read_chapter`.
//!
//! Every tool reads canon through the services, so the visibility rules of spec/06 §2 apply
//! uniformly (`track_id IS NULL OR track_id = T`), and every result is rendered from
//! `prompts/results.toml`: no model-facing text lives here (spec/05 §1.1).
//!
//! Entity parameters are resolved by [`EntityResolver`] (spec/05 §1.7): `#12` or `12` → id, exact
//! name, exact alias, case-insensitive name or alias, unique substring; no match is
//! `error.entity_not_found` with up to 5 suggestions, several matches are
//! `error.entity_ambiguous` with the candidates.
//!
//! A side track with an anchor marks items from main-story chapters after it with the "after the
//! anchor" marker (spec/06 §2, spec/08 §6). The marker helper lives here because these tools need
//! it; `block.anchor` itself is M10.

use std::collections::BTreeMap;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::db::entities::{
    appearances, change_log, chapters, character_designs, entities, foreshadowing, relations,
    timeline_event_entities, timeline_events, tracks,
};
use crate::db::json::{
    AnchorKind, CanonMode, ChangeTargetType, ChapterStatus, EntityKind, EntityStatus,
    ForeshadowingStatus, RelationStatus, TimelineKind, TimelineStatus, TrackKind,
};
use crate::prompts::context::{
    AppearanceSummaryContext, ChangeLineContext, DesignLinkContext, RelationLineContext,
};
use crate::services::canon::{self, Visibility};

use super::resolve::{EntityResolver, VisibleEntity};
use super::{Prepared, ToolCtx, ToolError, ToolErrorKind, ToolHandler, ToolKind, ToolOutcome};

/// `list_entities` default page size (spec/05 §3).
const LIST_DEFAULT: usize = 30;
/// `list_entities` maximum (spec/05 §3).
const LIST_MAX: usize = 100;
/// `query_timeline` default page size (spec/05 §3).
const TIMELINE_DEFAULT: usize = 20;
/// `query_timeline` maximum (spec/05 §3).
const TIMELINE_MAX: usize = 50;
/// `query_timeline` `around_event` window: 5 before and after (spec/05 §3).
const AROUND: usize = 5;
/// `list_foreshadowing` default page size (spec/05 §3).
const FORESHADOW_DEFAULT: usize = 20;
/// `list_foreshadowing` maximum (spec/05 §3).
const FORESHADOW_MAX: usize = 50;
/// `read_chapter` default cap (spec/05 §3).
const READ_CHAPTER_DEFAULT: usize = 8_000;
/// `read_chapter` maximum (spec/05 §3).
const READ_CHAPTER_MAX: usize = 20_000;
/// The latest change notes `get_entity` shows (spec/05 §3).
const ENTITY_CHANGES: usize = 10;

// ───────────────────────────── input types ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetEntityInput {
    entity: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListEntitiesInput {
    #[serde(default)]
    kind: Option<EntityKind>,
    #[serde(default)]
    status: Option<EntityStatus>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetRelationsInput {
    entity: String,
    #[serde(default)]
    other: Option<String>,
    #[serde(default)]
    include_ended: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryTimelineInput {
    #[serde(default)]
    entity: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    around_event: Option<i32>,
    #[serde(default)]
    under: Option<i32>,
    #[serde(default)]
    from_chapter: Option<i32>,
    #[serde(default)]
    to_chapter: Option<i32>,
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    order: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListForeshadowingInput {
    #[serde(default)]
    status: Option<ForeshadowingStatus>,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadChapterInput {
    chapter: String,
    #[serde(default)]
    from_pid: Option<String>,
    #[serde(default)]
    max_chars: Option<i64>,
}

fn decode<T: for<'de> Deserialize<'de>>(input: Value) -> Result<T, ToolError> {
    serde_json::from_value(input).map_err(|err| {
        ToolError::new(ToolErrorKind::Validation).with("errors", vec![err.to_string()])
    })
}

// ───────────────────────────── shared helpers ─────────────────────────────

/// The canon visibility of the session (spec/06 §2).
fn visibility_of(ctx: &ToolCtx<'_>) -> Visibility {
    let book_id = ctx
        .book
        .map(|book| book.id)
        .or(ctx.session.book_id)
        .unwrap_or_default();
    Visibility::of_session(book_id, ctx.visibility)
}

/// Maps an internal canon failure to the tool error the model sees.
fn canon_error(ctx: &ToolCtx<'_>, error: canon::CanonError) -> ToolError {
    match error {
        canon::CanonError::App(error) => map_app_error(ctx, &error),
        canon::CanonError::IsolatedReadonly { target } => {
            ToolError::new(ToolErrorKind::IsolatedReadonly).with("target", target)
        }
    }
}

/// Maps a service `AppError` to the tool error whose `result.error` branch fits it.
fn map_app_error(_ctx: &ToolCtx<'_>, error: &crate::error::AppError) -> ToolError {
    match error.code {
        crate::error::ErrorCode::ConflictExists => {
            let id = error
                .details
                .as_ref()
                .and_then(|details| details.get("id"))
                .and_then(Value::as_i64)
                .unwrap_or_default();
            ToolError::new(ToolErrorKind::EntityExists)
                .with("id", id)
                .with("name", error.message.clone())
        }
        crate::error::ErrorCode::ChangeNotLatest => {
            let later = error
                .details
                .as_ref()
                .and_then(|details| details.get("later_change_ids"))
                .cloned()
                .unwrap_or(Value::Array(Vec::new()));
            ToolError::new(ToolErrorKind::NotFound)
                .with("what_label", "change")
                .with("id", later)
        }
        _ => ToolError::new(ToolErrorKind::Validation).with("errors", vec![error.message.clone()]),
    }
}

/// Loads the visible entities as the resolver's view (spec/05 §1.7).
async fn resolver(ctx: &ToolCtx<'_>) -> Result<EntityResolver, ToolError> {
    let visibility = visibility_of(ctx);
    let rows = canon::visible_entities(ctx.tx, visibility)
        .await
        .map_err(|error| canon_error(ctx, error))?;
    let mut visible = Vec::with_capacity(rows.len());
    for row in &rows {
        visible.push(VisibleEntity {
            id: row.id,
            name: row.name.clone(),
            aliases: canon::aliases_of(row).map_err(|error| canon_error(ctx, error))?,
            kind_label: ctx.prompts.label("entity_kind", row.kind.as_str()),
        });
    }
    Ok(EntityResolver::new(visible))
}

/// The time anchor of the session's side track (spec/06 §2, spec/08 §6).
#[derive(Debug, Clone, Copy, Default)]
struct Anchor {
    kind: AnchorKindState,
    /// The anchor chapter's `ord` on the main track, for `after_chapter`.
    main_ord: Option<i32>,
    main_track: Option<i32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum AnchorKindState {
    #[default]
    None,
    BeforeStart,
    AfterChapter,
    AfterEnd,
}

/// Maps the stored typed anchor kind to the retrieval marker; exhaustive by construction.
pub(super) fn anchor_kind_state(kind: AnchorKind) -> AnchorKindState {
    match kind {
        AnchorKind::None => AnchorKindState::None,
        AnchorKind::BeforeStart => AnchorKindState::BeforeStart,
        AnchorKind::AfterChapter => AnchorKindState::AfterChapter,
        AnchorKind::AfterEnd => AnchorKindState::AfterEnd,
    }
}

/// Reads the session track's anchor (nothing for the main track).
async fn anchor_of(ctx: &ToolCtx<'_>) -> Anchor {
    let Some(track) = ctx.track else {
        return Anchor::default();
    };
    if track.kind != TrackKind::Side {
        return Anchor::default();
    }
    let kind = anchor_kind_state(track.anchor_kind);
    let mut anchor = Anchor {
        kind,
        main_ord: None,
        main_track: None,
    };
    if kind == AnchorKindState::None {
        return anchor;
    }
    // Every marker compares a chapter with the **main** track (spec/06 §2: "a main-track chapter
    // with ord > anchor.ord"), so the main track id is needed for `before_start` too — not only
    // for the `after_chapter` position.
    anchor.main_track = main_track_id(ctx).await;
    if kind == AnchorKindState::AfterChapter
        && let Some(chapter_id) = track.anchor_chapter_id
        && let Ok(Some(chapter)) = chapters::Entity::find_by_id(chapter_id).one(ctx.tx).await
    {
        anchor.main_ord = Some(chapter.ord);
        anchor.main_track = Some(chapter.track_id);
    }
    anchor
}

/// True when a chapter's item must be marked "after the anchor" (spec/06 §2).
///
/// A main-track chapter with `ord > anchor.ord` is marked; for `before_start` every main-track
/// item is; for `after_end` nothing is.
fn is_after_anchor(anchor: Anchor, chapter: Option<&chapters::Model>) -> bool {
    let Some(chapter) = chapter else {
        return false;
    };
    match anchor.kind {
        AnchorKindState::None | AnchorKindState::AfterEnd => false,
        AnchorKindState::BeforeStart => Some(chapter.track_id) == anchor.main_track,
        AnchorKindState::AfterChapter => {
            Some(chapter.track_id) == anchor.main_track && Some(chapter.ord) > anchor.main_ord
        }
    }
}

/// Loads the entities a caller names by id, dropping the ones that no longer exist.
async fn entities_by_id(
    ctx: &ToolCtx<'_>,
    ids: &[i32],
) -> Result<BTreeMap<i32, entities::Model>, ToolError> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let rows = entities::Entity::find()
        .filter(entities::COLUMN.id.is_in(ids.to_vec()))
        .all(ctx.tx)
        .await
        .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?;
    Ok(rows.into_iter().map(|row| (row.id, row)).collect())
}

/// Loads the chapters a caller names by id.
async fn chapters_by_id(
    ctx: &ToolCtx<'_>,
    ids: &[i32],
) -> Result<BTreeMap<i32, chapters::Model>, ToolError> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let rows = chapters::Entity::find()
        .filter(chapters::COLUMN.id.is_in(ids.to_vec()))
        .all(ctx.tx)
        .await
        .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?;
    Ok(rows.into_iter().map(|row| (row.id, row)).collect())
}

/// A chapter's label, rendered by `shared.chapter_label` (spec/05 §1.5).
async fn label_of_chapter(
    ctx: &ToolCtx<'_>,
    chapter_id: Option<i32>,
    cache: &mut BTreeMap<i32, String>,
) -> Result<String, ToolError> {
    let Some(chapter_id) = chapter_id else {
        return Ok(String::new());
    };
    if let Some(label) = cache.get(&chapter_id) {
        return Ok(label.clone());
    }
    let label = canon::chapter_label(ctx.tx, ctx.prompts, Some(chapter_id))
        .await
        .map_err(|error| canon_error(ctx, error))?;
    cache.insert(chapter_id, label.clone());
    Ok(label)
}

/// `Name(#id)`, the form every result uses for an entity (spec/05 §1.5).
fn entity_ref(row: &entities::Model) -> String {
    format!("{}(#{})", row.name, row.id)
}

/// The `ui` payload's kind value: the stored enum (spec/05 §1.6, D-054).
fn kind_of(row: &entities::Model) -> EntityKind {
    row.kind
}

/// The `ui` payload's status value.
fn status_of(row: &entities::Model) -> EntityStatus {
    row.status
}

/// The resolved entity a parameter names (spec/05 §1.7).
async fn resolve_entity(ctx: &ToolCtx<'_>, reference: &str) -> Result<entities::Model, ToolError> {
    let resolver = resolver(ctx).await?;
    let found = resolver.resolve(reference)?;
    entities::Entity::find_by_id(found.id)
        .one(ctx.tx)
        .await
        .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?
        .ok_or_else(|| {
            ToolError::new(ToolErrorKind::EntityNotFound).with("query", reference.to_owned())
        })
}

// ───────────────────────────── get_entity ─────────────────────────────

/// `get_entity` (spec/05 §3): the entity card.
pub struct GetEntityTool;

#[async_trait::async_trait]
impl ToolHandler for GetEntityTool {
    fn name(&self) -> &'static str {
        "get_entity"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Retrieval
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: GetEntityInput = decode(input)?;
        let entity = resolve_entity(ctx, &input.entity).await?;
        let visibility = visibility_of(ctx);
        let anchor = anchor_of(ctx).await;
        let mut labels = BTreeMap::new();

        // Relations in both directions, phrased from this entity's perspective (spec/06 §1.2),
        // and only the ones this session may see: a shared entity never leaks another isolated
        // track's private relation (spec/06 §2).
        let relation_rows = relations::Entity::find()
            .filter(relations::COLUMN.book_id.eq(entity.book_id))
            .filter(visibility.visible_condition(relations::Column::TrackId))
            .filter(
                sea_orm::Condition::any()
                    .add(relations::COLUMN.source_id.eq(entity.id))
                    .add(relations::COLUMN.target_id.eq(entity.id)),
            )
            .order_by_asc(relations::COLUMN.id)
            .all(ctx.tx)
            .await
            .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?;
        let mut other_ids: Vec<i32> = Vec::new();
        let mut since_ids: Vec<i32> = Vec::new();
        for row in &relation_rows {
            other_ids.push(if row.source_id == entity.id {
                row.target_id
            } else {
                row.source_id
            });
            since_ids.extend(
                [row.since_chapter_id, row.until_chapter_id]
                    .into_iter()
                    .flatten(),
            );
        }
        let others = entities_by_id(ctx, &other_ids).await?;
        let mut relation_lines = Vec::new();
        for row in &relation_rows {
            let (subject, object, kind) = if row.source_id == entity.id {
                (
                    entity_ref(&entity),
                    others
                        .get(&row.target_id)
                        .map(entity_ref)
                        .unwrap_or_default(),
                    row.kind.clone(),
                )
            } else {
                (
                    others
                        .get(&row.source_id)
                        .map(entity_ref)
                        .unwrap_or_default(),
                    entity_ref(&entity),
                    if row.inverse_kind.is_empty() {
                        // The forward kind is the fallback phrasing when no inverse wording
                        // exists (spec/06 §1.2).
                        row.kind.clone()
                    } else {
                        row.inverse_kind.clone()
                    },
                )
            };
            if subject.is_empty() || object.is_empty() {
                continue;
            }
            relation_lines.push(RelationLineContext {
                subject,
                object,
                kind,
                ended: row.status == RelationStatus::Ended,
                until_label: label_of_chapter(ctx, row.until_chapter_id, &mut labels).await?,
                description: row.description.clone(),
                since_label: label_of_chapter(ctx, row.since_chapter_id, &mut labels).await?,
            });
        }

        // The latest change notes, with chapter labels and anchor marks (spec/05 §3).
        let change_rows = change_log::Entity::find()
            .filter(change_log::COLUMN.target_type.eq(ChangeTargetType::Entity))
            .filter(change_log::COLUMN.target_id.eq(entity.id))
            .filter(visibility.visible_condition(change_log::Column::TrackId))
            .filter(change_log::COLUMN.reverted_by_id.is_null())
            .filter(change_log::COLUMN.note.ne(""))
            .order_by_desc(change_log::COLUMN.id)
            .limit(ENTITY_CHANGES as u64)
            .all(ctx.tx)
            .await
            .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?;
        let change_chapter_ids: Vec<i32> = change_rows
            .iter()
            .filter_map(|row| row.chapter_id)
            .collect();
        let change_chapters = chapters_by_id(ctx, &change_chapter_ids).await?;
        let mut changes = Vec::new();
        let mut state_after_anchor = false;
        for row in &change_rows {
            let chapter = row.chapter_id.and_then(|id| change_chapters.get(&id));
            let after_anchor = is_after_anchor(anchor, chapter);
            // The state is flagged when its newest change came from after the anchor (§4.2).
            if changes.is_empty() {
                state_after_anchor = after_anchor;
            }
            changes.push(ChangeLineContext {
                chapter_label: label_of_chapter(ctx, row.chapter_id, &mut labels).await?,
                after_anchor,
                note: row.note.clone(),
            });
        }
        // A change outside the recent 10 still decides the flag: check the newest one.
        if let Some(newest) = change_rows.first()
            && newest.chapter_id != change_chapter_ids.first().copied()
        {
            state_after_anchor = false;
        }
        if change_rows.is_empty() {
            state_after_anchor = false;
        }

        // Appearances: first, last and the mid-term window count (spec/05 §3).
        let appearance_rows = appearances::Entity::find()
            .filter(appearances::COLUMN.entity_id.eq(entity.id))
            .all(ctx.tx)
            .await
            .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?;
        let window_size = ctx
            .book
            .map(|book| book.settings.clone().into_inner().midterm_window)
            .unwrap_or(10)
            .max(0);
        // The mid-term window of the target position, shared with `block.memory` (spec/06 §4.1):
        // the last M chapters of P before the session chapter, topped up from the main story for
        // a side track. The session chapter itself is not part of it.
        let window_chapters = match (ctx.track, ctx.chapter) {
            (Some(track), Some(chapter)) => crate::services::memory::midterm_window_chapter_ids(
                ctx.tx,
                ctx.prompts,
                track,
                chapter,
            )
            .await
            .map_err(|error| map_app_error(ctx, &error))?,
            _ => Vec::new(),
        };
        let window_count = appearance_rows
            .iter()
            .filter(|row| window_chapters.contains(&row.chapter_id))
            .count() as i32;
        let first_label = label_of_chapter(ctx, entity.first_chapter_id, &mut labels).await?;
        let last_label = label_of_chapter(ctx, entity.last_chapter_id, &mut labels).await?;

        let design = character_designs::Entity::find()
            .filter(character_designs::COLUMN.entity_id.eq(entity.id))
            .order_by_asc(character_designs::COLUMN.rank)
            .one(ctx.tx)
            .await
            .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?
            .map(|row| DesignLinkContext {
                role_label: row.role_label,
                summary: row.summary,
            });
        let attributes: BTreeMap<String, String> = entity.attributes.clone().into_inner();
        let aliases = canon::aliases_of(&entity).map_err(|error| canon_error(ctx, error))?;
        let value = json!({
            "e": {
                "id": entity.id,
                "name": entity.name,
                "kind": entity.kind,
                "kind_label": ctx.prompts.label("entity_kind", entity.kind.as_str()),
                "is_protagonist": entity.is_protagonist,
                "importance": entity.importance,
                "status_label": ctx.prompts.label("entity_status", entity.status.as_str()),
                "aliases": aliases,
                "summary": entity.summary,
                "state": entity.state,
                "state_after_anchor": state_after_anchor,
                "attributes": attributes.iter().map(|(key, value)| json!({
                    "key": key,
                    "value": value,
                })).collect::<Vec<_>>(),
                "description": entity.description,
            },
            "relations": relation_lines,
            "changes": changes,
            "appearances": AppearanceSummaryContext {
                first_label,
                last_label,
                window_size,
                window_count,
            },
            "design": design,
        });
        let text = ctx
            .prompts
            .render_value("result.entity_card", crate::prompts::template_value(&value))
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        Ok(ToolOutcome::text(text).with_ui(json!({
            "entity_id": entity.id,
            "name": entity.name,
            "kind": kind_of(&entity),
        })))
    }
}

// ───────────────────────────── list_entities ─────────────────────────────

/// `list_entities` (spec/05 §3): one line per entity.
pub struct ListEntitiesTool;

#[async_trait::async_trait]
impl ToolHandler for ListEntitiesTool {
    fn name(&self) -> &'static str {
        "list_entities"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Retrieval
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: ListEntitiesInput = decode(input)?;
        let visibility = visibility_of(ctx);
        let limit = input
            .limit
            .map(|value| value.clamp(1, LIST_MAX as i64) as usize)
            .unwrap_or(LIST_DEFAULT);
        let needle = input
            .query
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_lowercase);
        let rows = canon::visible_entities(ctx.tx, visibility)
            .await
            .map_err(|error| canon_error(ctx, error))?;
        let mut items = Vec::new();
        for row in &rows {
            if let Some(kind) = input.kind
                && row.kind != kind
            {
                continue;
            }
            if let Some(status) = input.status
                && row.status != status
            {
                continue;
            }
            let aliases = canon::aliases_of(row).map_err(|error| canon_error(ctx, error))?;
            if let Some(needle) = &needle
                && !row.name.to_lowercase().contains(needle)
                && !row.summary.to_lowercase().contains(needle)
                && !aliases
                    .iter()
                    .any(|alias| alias.to_lowercase().contains(needle))
            {
                continue;
            }
            items.push(json!({
                "id": row.id,
                "name": row.name,
                "kind_label": ctx.prompts.label("entity_kind", row.kind.as_str()),
                "aliases": aliases,
                "summary": row.summary,
                "status": status_of(row),
                "status_label": ctx.prompts.label("entity_status", row.status.as_str()),
            }));
        }
        // Sorted by importance desc, then last appearance desc (spec/05 §3).
        let mut ordered: Vec<(i32, Option<i32>, Value)> = rows
            .iter()
            .filter(|row| items.iter().any(|item| item["id"] == json!(row.id)))
            .map(|row| (row.importance, row.last_chapter_id, Value::Null))
            .collect();
        let _ = &mut ordered;
        items.sort_by(|a, b| {
            let a_row = rows
                .iter()
                .find(|row| json!(row.id) == a["id"])
                .map(|r| (r.importance, r.last_chapter_id));
            let b_row = rows
                .iter()
                .find(|row| json!(row.id) == b["id"])
                .map(|r| (r.importance, r.last_chapter_id));
            match (a_row, b_row) {
                (Some((a_importance, a_last)), Some((b_importance, b_last))) => b_importance
                    .cmp(&a_importance)
                    .then_with(|| b_last.cmp(&a_last)),
                _ => std::cmp::Ordering::Equal,
            }
        });
        let total = items.len();
        items.truncate(limit);
        let shown = items.len();
        let text = ctx
            .prompts
            .render_value(
                "result.entity_list",
                crate::prompts::template_value(&json!({
                    "items": items,
                    "total": total,
                    "shown": shown,
                })),
            )
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        Ok(ToolOutcome::text(text).with_ui(json!({ "count": shown })))
    }
}

// ───────────────────────────── get_relations ─────────────────────────────

/// `get_relations` (spec/05 §3): relation lines with descriptions and chapter labels.
pub struct GetRelationsTool;

#[async_trait::async_trait]
impl ToolHandler for GetRelationsTool {
    fn name(&self) -> &'static str {
        "get_relations"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Retrieval
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: GetRelationsInput = decode(input)?;
        let entity = resolve_entity(ctx, &input.entity).await?;
        let other = match &input.other {
            Some(reference) => Some(resolve_entity(ctx, reference).await?),
            None => None,
        };
        let include_ended = input.include_ended.unwrap_or(false);
        let visibility = visibility_of(ctx);
        let rows = relations::Entity::find()
            .filter(relations::COLUMN.book_id.eq(entity.book_id))
            .filter(visibility.visible_condition(relations::Column::TrackId))
            .filter(
                sea_orm::Condition::any()
                    .add(relations::COLUMN.source_id.eq(entity.id))
                    .add(relations::COLUMN.target_id.eq(entity.id)),
            )
            .order_by_asc(relations::COLUMN.id)
            .all(ctx.tx)
            .await
            .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?;
        let mut other_ids = Vec::new();
        for row in &rows {
            other_ids.push(if row.source_id == entity.id {
                row.target_id
            } else {
                row.source_id
            });
        }
        let others = entities_by_id(ctx, &other_ids).await?;
        let mut labels = BTreeMap::new();
        let mut items = Vec::new();
        for row in &rows {
            if row.status == RelationStatus::Ended && !include_ended {
                continue;
            }
            let counterpart = if row.source_id == entity.id {
                row.target_id
            } else {
                row.source_id
            };
            if let Some(other) = &other
                && counterpart != other.id
            {
                continue;
            }
            let Some(counterpart_row) = others.get(&counterpart) else {
                continue;
            };
            let (subject, object, kind) = if row.source_id == entity.id {
                (
                    entity_ref(&entity),
                    entity_ref(counterpart_row),
                    row.kind.clone(),
                )
            } else {
                (
                    entity_ref(counterpart_row),
                    entity_ref(&entity),
                    if row.inverse_kind.is_empty() {
                        row.kind.clone()
                    } else {
                        row.inverse_kind.clone()
                    },
                )
            };
            items.push(RelationLineContext {
                subject,
                object,
                kind,
                ended: row.status == RelationStatus::Ended,
                until_label: label_of_chapter(ctx, row.until_chapter_id, &mut labels).await?,
                description: row.description.clone(),
                since_label: label_of_chapter(ctx, row.since_chapter_id, &mut labels).await?,
            });
        }
        let count = items.len();
        let text = ctx
            .prompts
            .render_value(
                "result.relations",
                crate::prompts::template_value(&json!({
                    "entity": entity_ref(&entity),
                    "items": items,
                })),
            )
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        Ok(ToolOutcome::text(text).with_ui(json!({
            "entity_id": entity.id,
            "count": count,
        })))
    }
}

// ───────────────────────────── query_timeline ─────────────────────────────

/// `query_timeline` (spec/05 §3): events in story order (or newest first).
pub struct QueryTimelineTool;

#[async_trait::async_trait]
impl ToolHandler for QueryTimelineTool {
    fn name(&self) -> &'static str {
        "query_timeline"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Retrieval
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: QueryTimelineInput = decode(input)?;
        let visibility = visibility_of(ctx);
        let book_id = visibility.book_id;
        let limit = input
            .limit
            .map(|value| value.clamp(1, TIMELINE_MAX as i64) as usize)
            .unwrap_or(TIMELINE_DEFAULT);
        let entity = match &input.entity {
            Some(reference) => Some(resolve_entity(ctx, reference).await?),
            None => None,
        };
        let needle = input
            .query
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_lowercase);
        let mut rows = timeline_events::Entity::find()
            .filter(timeline_events::COLUMN.book_id.eq(book_id))
            .filter(
                timeline_events::COLUMN
                    .kind
                    .eq(TimelineKind::Entry.as_str()),
            )
            .filter(visibility.visible_condition(timeline_events::Column::TrackId))
            .order_by_asc(timeline_events::COLUMN.position)
            .order_by_asc(timeline_events::COLUMN.id)
            .all(ctx.tx)
            .await
            .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?;
        let mut links = timeline_links(ctx, &rows).await?;
        // The tree prunes abandoned subtrees; `under` keeps one visible entry's descendants, and
        // a visible entry under a hidden parent is shown at the top level (spec/06 §1.4, T8).
        let tree = crate::services::canon::timeline_tree::tree_rows(ctx.tx, book_id)
            .await
            .map_err(|error| canon_error(ctx, error))?;
        let by_id: BTreeMap<i32, timeline_events::Model> =
            tree.into_iter().map(|row| (row.id, row)).collect();
        let mut visible_ids: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();
        for row in by_id.values() {
            if row.status == TimelineStatus::Abandoned {
                continue;
            }
            visible_ids.insert(row.id);
        }
        if let Some(under_id) = input.under {
            let Some(under) = by_id.get(&under_id) else {
                return Err(ToolError::new(ToolErrorKind::NotFound)
                    .with(
                        "what_label",
                        ctx.prompts.label("target_type", "timeline_event"),
                    )
                    .with("id", under_id));
            };
            if under.status == TimelineStatus::Abandoned || !visible_ids.contains(&under_id) {
                return Err(ToolError::new(ToolErrorKind::NotFound)
                    .with(
                        "what_label",
                        ctx.prompts.label("target_type", "timeline_event"),
                    )
                    .with("id", under_id));
            }
        }
        rows.retain(|row| {
            if !visible_ids.contains(&row.id) {
                return false;
            }
            match input.under {
                None => true,
                Some(under_id) => {
                    crate::services::canon::timeline_tree::ancestor_chain(&by_id, row.id)
                        .iter()
                        .any(|ancestor| ancestor.id == under_id)
                }
            }
        });
        if let Some(entity) = &entity {
            rows.retain(|row| {
                links
                    .get(&row.id)
                    .is_some_and(|ids| ids.contains(&entity.id))
            });
        }
        // `from_chapter`/`to_chapter` are numbers in the main track (spec/05 §3).
        let range = chapter_ord_range(ctx, input.from_chapter, input.to_chapter).await?;
        if let Some((from, to)) = range {
            let mut chapter_ids: Vec<i32> = rows.iter().filter_map(|row| row.chapter_id).collect();
            chapter_ids.sort_unstable();
            chapter_ids.dedup();
            let chapters = chapters_by_id(ctx, &chapter_ids).await?;
            let main_id = main_track_id(ctx).await.unwrap_or_default();
            rows.retain(|row| {
                let Some(chapter_id) = row.chapter_id else {
                    return false;
                };
                let Some(chapter) = chapters.get(&chapter_id) else {
                    return false;
                };
                chapter.track_id == main_id
                    && from.is_none_or(|from| chapter.ord >= from)
                    && to.is_none_or(|to| chapter.ord <= to)
            });
        }
        if let Some(needle) = &needle {
            rows.retain(|row| {
                row.title.to_lowercase().contains(needle)
                    || row.description.to_lowercase().contains(needle)
            });
        }
        // `around_event`: the 5 events before and after the named one (spec/05 §3).
        let around = match input.around_event {
            Some(event_id) => {
                let all_ids: Vec<i32> = rows.iter().map(|row| row.id).collect();
                let position = all_ids
                    .iter()
                    .position(|id| *id == event_id)
                    .ok_or_else(|| {
                        ToolError::new(ToolErrorKind::NotFound)
                            .with(
                                "what_label",
                                ctx.prompts.label("target_type", "timeline_event"),
                            )
                            .with("id", event_id)
                    })?;
                let start = position.saturating_sub(AROUND);
                let end = (position + AROUND + 1).min(rows.len());
                rows[start..end].to_vec()
            }
            None => rows,
        };
        let mode = input.order.as_deref().unwrap_or("chronological");
        let mut ordered = around;
        if mode == "recent_first" {
            ordered.reverse();
        }
        let total = ordered.len();
        ordered.truncate(limit);
        let shown = ordered.len();
        let planned_count = ordered
            .iter()
            .filter(|row| row.status == TimelineStatus::Planned)
            .count();

        let chapter_ids: Vec<i32> = ordered.iter().filter_map(|row| row.chapter_id).collect();
        let chapters = chapters_by_id(ctx, &chapter_ids).await?;
        let all_entity_ids: Vec<i32> = {
            let mut ids: Vec<i32> = Vec::new();
            for row in &ordered {
                ids.extend(links.get(&row.id).cloned().unwrap_or_default());
            }
            ids.sort_unstable();
            ids.dedup();
            ids
        };
        let names = entities_by_id(ctx, &all_entity_ids).await?;
        let anchor = anchor_of(ctx).await;
        let mut labels = BTreeMap::new();
        let mut items = Vec::new();
        for row in &ordered {
            let chapter = row.chapter_id.and_then(|id| chapters.get(&id));
            let parent = row
                .parent_id
                .and_then(|parent_id| by_id.get(&parent_id))
                .filter(|parent| visible_ids.contains(&parent.id));
            let parent_label = parent.map(|parent| format!("#{} {}", parent.id, parent.title));
            items.push(json!({
                "id": row.id,
                "story_time": row.story_time,
                "title": row.title,
                "description": row.description,
                "planned": row.status == TimelineStatus::Planned,
                "parent_id": parent.map(|parent| parent.id),
                "parent": parent_label,
                "chapter_label": label_of_chapter(ctx, row.chapter_id, &mut labels).await?,
                "entities": links
                    .get(&row.id)
                    .map(|ids| {
                        ids.iter()
                            .filter_map(|id| names.get(id))
                            .map(entity_ref)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
                "after_anchor": is_after_anchor(anchor, chapter),
            }));
        }
        let text = ctx
            .prompts
            .render_value(
                "result.timeline",
                crate::prompts::template_value(&json!({
                    "items": items,
                    "total": total,
                    "shown": shown,
                    "mode": mode,
                    "planned_count": planned_count,
                })),
            )
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        let _ = &mut links;
        Ok(ToolOutcome::text(text).with_ui(json!({ "count": shown })))
    }
}

/// The entity links of a set of timeline events, in one query.
async fn timeline_links(
    ctx: &ToolCtx<'_>,
    rows: &[timeline_events::Model],
) -> Result<BTreeMap<i32, Vec<i32>>, ToolError> {
    let mut links: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
    if rows.is_empty() {
        return Ok(links);
    }
    let ids: Vec<i32> = rows.iter().map(|row| row.id).collect();
    let join_rows = timeline_event_entities::Entity::find()
        .filter(timeline_event_entities::COLUMN.event_id.is_in(ids))
        .order_by_asc(timeline_event_entities::COLUMN.entity_id)
        .all(ctx.tx)
        .await
        .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?;
    for row in join_rows {
        links.entry(row.event_id).or_default().push(row.entity_id);
    }
    Ok(links)
}

/// The main track of the session's book (for `from_chapter`/`to_chapter`).
async fn main_track_id(ctx: &ToolCtx<'_>) -> Option<i32> {
    let visibility = visibility_of(ctx);
    tracks::Entity::find()
        .filter(tracks::COLUMN.book_id.eq(visibility.book_id))
        .filter(tracks::COLUMN.kind.eq(TrackKind::Main))
        .one(ctx.tx)
        .await
        .ok()
        .flatten()
        .map(|row| row.id)
}

/// `(from, to)` chapter numbers of the main track, when either was given.
async fn chapter_ord_range(
    ctx: &ToolCtx<'_>,
    from: Option<i32>,
    to: Option<i32>,
) -> Result<Option<(Option<i32>, Option<i32>)>, ToolError> {
    if from.is_none() && to.is_none() {
        return Ok(None);
    }
    let _ = ctx;
    Ok(Some((from, to)))
}

// ───────────────────────────── list_foreshadowing ─────────────────────────────

/// `list_foreshadowing` (spec/05 §3).
pub struct ListForeshadowingTool;

#[async_trait::async_trait]
impl ToolHandler for ListForeshadowingTool {
    fn name(&self) -> &'static str {
        "list_foreshadowing"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Retrieval
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: ListForeshadowingInput = decode(input)?;
        let visibility = visibility_of(ctx);
        let limit = input
            .limit
            .map(|value| value.clamp(1, FORESHADOW_MAX as i64) as usize)
            .unwrap_or(FORESHADOW_DEFAULT);
        let status = input.status.unwrap_or(ForeshadowingStatus::Open);
        let entity = match &input.entity {
            Some(reference) => Some(resolve_entity(ctx, reference).await?),
            None => None,
        };
        let mut rows = foreshadowing::Entity::find()
            .filter(foreshadowing::COLUMN.book_id.eq(visibility.book_id))
            .filter(visibility.visible_condition(foreshadowing::Column::TrackId))
            .filter(foreshadowing::COLUMN.status.eq(status.as_str()))
            .all(ctx.tx)
            .await
            .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?;
        if let Some(entity) = &entity {
            let mut links: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
            let ids: Vec<i32> = rows.iter().map(|row| row.id).collect();
            if !ids.is_empty() {
                let join_rows = crate::db::entities::foreshadowing_entities::Entity::find()
                    .filter(
                        crate::db::entities::foreshadowing_entities::COLUMN
                            .foreshadowing_id
                            .is_in(ids),
                    )
                    .all(ctx.tx)
                    .await
                    .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?;
                for row in join_rows {
                    links
                        .entry(row.foreshadowing_id)
                        .or_default()
                        .push(row.entity_id);
                }
            }
            rows.retain(|row| {
                links
                    .get(&row.id)
                    .is_some_and(|ids| ids.contains(&entity.id))
            });
        }
        // Sorted by importance desc, then planted chapter asc (the memory order, spec/06 §4.4).
        rows.sort_by(|a, b| {
            b.importance.cmp(&a.importance).then_with(|| {
                a.planted_chapter_id
                    .unwrap_or(i32::MAX)
                    .cmp(&b.planted_chapter_id.unwrap_or(i32::MAX))
            })
        });
        rows.truncate(limit);
        let anchor_ids: Vec<i32> = rows.iter().filter_map(|row| row.payoff_event_id).collect();
        let anchors: BTreeMap<i32, timeline_events::Model> = if anchor_ids.is_empty() {
            BTreeMap::new()
        } else {
            timeline_events::Entity::find()
                .filter(timeline_events::COLUMN.id.is_in(anchor_ids))
                .all(ctx.tx)
                .await
                .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?
                .into_iter()
                .map(|row| (row.id, row))
                .collect()
        };
        let timeline_on = crate::agent::session::canon_systems_of(ctx.session).timeline;
        let mut labels = BTreeMap::new();
        let mut items = Vec::new();
        for row in &rows {
            let anchor_row = if timeline_on {
                row.payoff_event_id.and_then(|id| anchors.get(&id))
            } else {
                None
            };
            let anchor_abandoned =
                anchor_row.is_some_and(|anchor| anchor.status == TimelineStatus::Abandoned);
            let anchor = anchor_row.filter(|anchor| anchor.status != TimelineStatus::Abandoned);
            items.push(json!({
                "id": row.id,
                "title": row.title,
                "status_label": ctx.prompts.label("foreshadowing_status", row.status.as_str()),
                "description": row.description,
                "planted_label": label_of_chapter(ctx, row.planted_chapter_id, &mut labels).await?,
                "anchor": anchor.map(|anchor| json!({
                    "id": anchor.id,
                    "title": anchor.title,
                    "position_label": row.payoff_position
                        .map(|position| ctx.prompts.label("payoff_position", position.as_str()))
                        .unwrap_or_default(),
                    "planned": anchor.status == TimelineStatus::Planned,
                })),
                "anchor_abandoned": anchor_abandoned,
                "planned_payoff": row.planned_payoff,
                "resolution": row.resolution,
                "resolved_label": label_of_chapter(ctx, row.resolved_chapter_id, &mut labels).await?,
            }));
        }
        let text = ctx
            .prompts
            .render_value(
                "result.foreshadowing_list",
                crate::prompts::template_value(&json!({ "items": items })),
            )
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        Ok(ToolOutcome::text(text).with_ui(json!({ "count": rows.len() })))
    }
}

// ───────────────────────────── read_chapter ─────────────────────────────

/// `read_chapter` (spec/05 §3): the official text with pids.
pub struct ReadChapterTool;

#[async_trait::async_trait]
impl ToolHandler for ReadChapterTool {
    fn name(&self) -> &'static str {
        "read_chapter"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Retrieval
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: ReadChapterInput = decode(input)?;
        let chapter = resolve_chapter(ctx, &input.chapter).await?;
        if chapter.status != ChapterStatus::Official {
            return Err(ToolError::new(ToolErrorKind::ChapterNotOfficial).with(
                "label",
                canon::chapter_label(ctx.tx, ctx.prompts, Some(chapter.id))
                    .await
                    .map_err(|error| canon_error(ctx, error))?,
            ));
        }
        let content: crate::db::json::ChapterContent = chapter.content.clone().into_inner();
        let limit = input
            .max_chars
            .map(|value| value.clamp(500, READ_CHAPTER_MAX as i64) as usize)
            .unwrap_or(READ_CHAPTER_DEFAULT);
        let start = match &input.from_pid {
            Some(pid) => content
                .paragraphs
                .iter()
                .position(|paragraph| &paragraph.pid == pid)
                .ok_or_else(|| {
                    ToolError::new(ToolErrorKind::PidNotFound)
                        .with("pids", vec![pid.clone()])
                        .with("tool", "read_chapter")
                })?,
            None => 0,
        };
        let mut used = 0usize;
        let mut shown = Vec::new();
        let mut next_pid: Option<String> = None;
        for paragraph in &content.paragraphs[start..] {
            let length = paragraph.text.chars().count() + paragraph.pid.chars().count();
            if !shown.is_empty() && used + length > limit {
                next_pid = Some(paragraph.pid.clone());
                break;
            }
            used += length;
            shown.push(json!({ "pid": paragraph.pid, "text": paragraph.text }));
        }
        let text = ctx
            .prompts
            .render_value(
                "result.chapter_text",
                crate::prompts::template_value(&json!({
                    "label": canon::chapter_label(ctx.tx, ctx.prompts, Some(chapter.id))
                        .await
                        .map_err(|error| canon_error(ctx, error))?,
                    "paragraphs": shown,
                    "next_pid": next_pid,
                    "total_paragraphs": content.paragraphs.len(),
                })),
            )
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        Ok(ToolOutcome::text(text).with_ui(json!({
            "chapter_id": chapter.id,
            "track_id": chapter.track_id,
            "number": chapter.ord,
        })))
    }
}

/// Resolves a `chapter` parameter (spec/05 §3): `"12"` in the session's track, `"main 12"` on the
/// main track, `"#345"` by id. A chapter of another isolated track is invisible
/// (`error.chapter_not_found`).
async fn resolve_chapter(ctx: &ToolCtx<'_>, reference: &str) -> Result<chapters::Model, ToolError> {
    let trimmed = reference.trim();
    let visibility = visibility_of(ctx);
    let not_found =
        || ToolError::new(ToolErrorKind::ChapterNotFound).with("query", trimmed.to_owned());
    // `#345`: by id.
    if let Some(digits) = trimmed.strip_prefix('#') {
        let id = digits.parse::<i32>().map_err(|_| not_found())?;
        let row = chapters::Entity::find_by_id(id)
            .one(ctx.tx)
            .await
            .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?
            .ok_or_else(not_found)?;
        if row.book_id != visibility.book_id {
            return Err(not_found());
        }
        if !visible_track(ctx, &visibility, row.track_id).await? {
            return Err(not_found());
        }
        return Ok(row);
    }
    // `main 12`: the main track's number.
    let (track_id, number) = if let Some(rest) = trimmed.strip_prefix("main ") {
        let number = rest.trim().parse::<i32>().map_err(|_| not_found())?;
        let main = main_track_id(ctx).await.ok_or_else(not_found)?;
        (main, number)
    } else {
        let number = trimmed.parse::<i32>().map_err(|_| not_found())?;
        let track_id = ctx
            .track
            .map(|track| track.id)
            .or(ctx.chapter.map(|chapter| chapter.track_id))
            .ok_or_else(not_found)?;
        (track_id, number)
    };
    let row = chapters::Entity::find_by_track_ord((track_id, number))
        .one(ctx.tx)
        .await
        .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?
        .ok_or_else(not_found)?;
    if !visible_track(ctx, &visibility, row.track_id).await? {
        return Err(not_found());
    }
    Ok(row)
}

/// True when the session may see a chapter of that track (spec/06 §2).
async fn visible_track(
    ctx: &ToolCtx<'_>,
    visibility: &Visibility,
    track_id: i32,
) -> Result<bool, ToolError> {
    let Some(track) = tracks::Entity::find_by_id(track_id)
        .one(ctx.tx)
        .await
        .map_err(|error| canon_error(ctx, canon::CanonError::from(error)))?
    else {
        return Ok(false);
    };
    if track.kind == TrackKind::Main || track.canon_mode == CanonMode::Canon {
        return Ok(true);
    }
    Ok(visibility.sees(Some(track.id)))
}

// ───────────────────────────── registration ─────────────────────────────

/// Every retrieval tool of this milestone (spec/05 §3).
pub fn all() -> Vec<std::sync::Arc<dyn ToolHandler>> {
    vec![
        std::sync::Arc::new(GetEntityTool),
        std::sync::Arc::new(ListEntitiesTool),
        std::sync::Arc::new(GetRelationsTool),
        std::sync::Arc::new(QueryTimelineTool),
        std::sync::Arc::new(ListForeshadowingTool),
        std::sync::Arc::new(ReadChapterTool),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_kind_state_covers_every_variant() {
        assert_eq!(anchor_kind_state(AnchorKind::None), AnchorKindState::None);
        assert_eq!(
            anchor_kind_state(AnchorKind::BeforeStart),
            AnchorKindState::BeforeStart
        );
        assert_eq!(
            anchor_kind_state(AnchorKind::AfterChapter),
            AnchorKindState::AfterChapter
        );
        assert_eq!(
            anchor_kind_state(AnchorKind::AfterEnd),
            AnchorKindState::AfterEnd
        );
    }

    /// Restoring the old `as_str()` + catch-all match must fail this source guard.
    #[test]
    fn anchor_kind_mapping_is_typed() {
        let source = include_str!("retrieval.rs");
        let start = source
            .find("fn anchor_kind_state")
            .expect("the mapping exists");
        let end = start + source[start..].find("\n}\n").expect("the mapping ends");
        assert!(
            !source[start..end].contains(".as_str()"),
            "anchor_kind_state must match the typed AnchorKind"
        );
    }
}
