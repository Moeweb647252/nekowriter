//! Archivist tools (spec/05 §6): every canon write that maintenance performs.
//!
//! All writes go through `services::canon`, so they record `change_log` rows (`actor: agent`,
//! `chapter_id` = the session chapter, `session_id`), emit `canon.changed` and follow the
//! visibility rule of spec/06 §2: new rows carry the isolated track's id, shared rows are
//! read-only there (`error.isolated_readonly`), and `propose_setting_change` (proposals.rs)
//! refuses entirely in an isolated track.
//!
//! `finish_maintenance` validates the required steps (appearances and summary) and the
//! protagonist-state confirmation of spec/06 §6; its `complete: maintenance` effect is applied by
//! the runtime's completion handler.

use std::collections::BTreeMap;

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::dto::canon::{
    AppearanceInput, EntityInput, EntityPatch, ForeshadowingInput, MemoryNoteInput,
    TimelineEventInput, TimelineEventPatch,
};
use crate::db::entities::{agent_tool_calls, appearances, change_log, chapters, entities};
use crate::db::json::{
    AppearanceRole, ChangeActor, ChangeOp, ChangeTargetType, CompletionKind, EntityKind,
    EntityStatus, ForeshadowingStatus, NoteKind, Snapshot, ToolCallStatus, ToolEffects,
};
use crate::events::ServerEvent;
use crate::services::canon::{
    self, CanonError, CanonWrite,
    relation::{RelationDetails, RelationKey},
};
use crate::services::canon::{appearance, entity, foreshadow, note, timeline};

use super::resolve::{EntityResolver, VisibleEntity};
use super::{Prepared, ToolCtx, ToolError, ToolErrorKind, ToolHandler, ToolKind, ToolOutcome};

// ───────────────────────────── shared helpers ─────────────────────────────

fn decode<T: for<'de> Deserialize<'de>>(input: Value) -> Result<T, ToolError> {
    serde_json::from_value(input).map_err(|err| {
        ToolError::new(ToolErrorKind::Validation).with("errors", vec![err.to_string()])
    })
}

fn internal() -> ToolError {
    ToolError::new(ToolErrorKind::Internal)
}

/// The book of the session; every canon tool needs one.
fn book_id(ctx: &ToolCtx<'_>) -> Result<i32, ToolError> {
    ctx.book.map(|book| book.id).ok_or_else(internal)
}

/// The agent write handle of the session: its visibility, session and chapter (spec/06 §3).
fn write_of(ctx: &ToolCtx<'_>) -> Result<CanonWrite, ToolError> {
    let book_id = book_id(ctx)?;
    let visibility = canon::Visibility::of_session(book_id, ctx.visibility);
    Ok(CanonWrite::agent(
        visibility,
        ctx.session.id,
        ctx.chapter.map(|chapter| chapter.id),
    ))
}

/// The session chapter, where the tool needs one (`record_appearances`, `set_chapter_summary`).
fn chapter_of<'a>(ctx: &'a ToolCtx<'_>) -> Result<&'a chapters::Model, ToolError> {
    ctx.chapter.ok_or_else(|| {
        ToolError::new(ToolErrorKind::Validation)
            .with("errors", vec!["this tool needs a chapter".to_owned()])
    })
}

/// The resolver over the session's visible entities (spec/05 §1.7).
async fn resolver(ctx: &ToolCtx<'_>) -> Result<EntityResolver, ToolError> {
    let book_id = book_id(ctx)?;
    let visibility = canon::Visibility::of_session(book_id, ctx.visibility);
    let rows = canon::visible_entities(ctx.tx, visibility)
        .await
        .map_err(|_| internal())?;
    Ok(EntityResolver::new(
        rows.into_iter()
            .map(|row| VisibleEntity {
                id: row.id,
                name: row.name,
                aliases: row.aliases.clone().into_inner(),
                kind_label: ctx.prompts.label("entity_kind", row.kind.as_str()),
            })
            .collect(),
    ))
}

/// Resolves one entity reference and loads its row.
async fn resolve_entity(ctx: &ToolCtx<'_>, reference: &str) -> Result<entities::Model, ToolError> {
    let found = resolver(ctx).await?.resolve(reference)?.clone();
    canon::load_entity(ctx.tx, found.id)
        .await
        .map_err(|error| map_canon(error, None, None))
}

/// Maps a canon service failure to the tool error the model sees (spec/05 §6, spec/09 §2).
///
/// `not_found` names the target type and id for the `error.not_found` rendering; `conflict_name`
/// turns a unique-name collision into `error.entity_exists`.
fn map_canon(
    error: CanonError,
    not_found: Option<(&str, i32)>,
    conflict_name: Option<&str>,
) -> ToolError {
    match error {
        CanonError::IsolatedReadonly { target } => {
            ToolError::new(ToolErrorKind::IsolatedReadonly).with("target", target)
        }
        CanonError::App(error) => {
            if error.code == crate::error::ErrorCode::NotFound
                && let Some((what_label, id)) = not_found
            {
                return ToolError::new(ToolErrorKind::NotFound)
                    .with("what_label", what_label.to_owned())
                    .with("id", id);
            }
            if error.code == crate::error::ErrorCode::ConflictExists
                && let Some(name) = conflict_name
            {
                let id = error
                    .details
                    .as_ref()
                    .and_then(|details| details.get("id"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                return ToolError::new(ToolErrorKind::EntityExists)
                    .with("name", name.to_owned())
                    .with("id", id);
            }
            ToolError::new(ToolErrorKind::Validation).with("errors", vec![error.message])
        }
    }
}

/// The change ids of the events a canon write returned.
fn change_ids(events: &[ServerEvent]) -> Vec<i32> {
    events
        .iter()
        .filter_map(|event| match event {
            ServerEvent::CanonChanged { change_id, .. } => Some(*change_id),
            _ => None,
        })
        .collect()
}

/// Renders `result.canon_write`, publishes the canon writes' events and builds the structured
/// `ui` payload (spec/05 §1.6).
///
/// The canon services hand back the `canon.changed` event of every write (spec/06 §3); a tool must
/// forward it so the UI invalidates its canon queries — and, since M7, so the search index follows
/// the same transaction (spec/07 §2). An event dropped here is invisible to both.
fn canon_outcome(
    ctx: &mut ToolCtx<'_>,
    action: &str,
    target: String,
    events: &[ServerEvent],
    target_type: &str,
    target_id: i32,
    name: &str,
) -> Result<ToolOutcome, ToolError> {
    canon_outcome_with(
        ctx,
        action,
        target,
        events,
        target_type,
        target_id,
        name,
        json!({}),
    )
}

// The shared renderer of every `result.canon_write` outcome: the 8 arguments are the template's
// variables (action, target, target type/id, name and the event list), which the prompt pack reads
// by name; a struct would obscure that the template is the contract (spec/09 §2).
#[allow(clippy::too_many_arguments)]
fn canon_outcome_with(
    ctx: &mut ToolCtx<'_>,
    action: &str,
    target: String,
    events: &[ServerEvent],
    target_type: &str,
    target_id: i32,
    name: &str,
    extra: Value,
) -> Result<ToolOutcome, ToolError> {
    for event in events {
        ctx.emit(event.clone());
    }
    // Every variable of `result.canon_write` is provided: the template tests `moved` and the
    // position fields for every action, and strict-undefined rendering rejects a missing one.
    let mut fields = json!({
        "action": action,
        "target": target,
        "count": Value::Null,
        "prev_event": Value::Null,
        "next_event": Value::Null,
        "moved": false,
    });
    if let Some(object) = extra.as_object() {
        for (key, value) in object {
            fields[key] = value.clone();
        }
    }
    let text = ctx
        .prompts
        .render_value(
            "result.canon_write",
            crate::prompts::template_value(&fields),
        )
        .map_err(|_| internal())?
        .text;
    Ok(ToolOutcome::text(text).with_ui(json!({
        "change_ids": change_ids(events),
        "target_type": target_type,
        "target_id": target_id,
        "name": name,
    })))
}

/// Rejects an empty note on an update (spec/06 §3: agents must give notes for updates).
fn require_note(note: &str) -> Result<String, ToolError> {
    let note = note.trim().to_owned();
    if note.is_empty() {
        return Err(ToolError::new(ToolErrorKind::Validation)
            .with("errors", vec!["note must not be empty".to_owned()]));
    }
    Ok(note)
}

// ───────────────────────────── entity tools ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateEntityInput {
    kind: EntityKind,
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    attributes: BTreeMap<String, String>,
    #[serde(default = "default_importance")]
    importance: i32,
    #[serde(default)]
    status: Option<EntityStatus>,
    #[serde(default)]
    is_protagonist: Option<bool>,
    #[serde(default)]
    note: Option<String>,
}

fn default_importance() -> i32 {
    2
}

/// `create_entity` (spec/05 §6).
pub struct CreateEntityTool;

#[async_trait::async_trait]
impl ToolHandler for CreateEntityTool {
    fn name(&self) -> &'static str {
        "create_entity"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: CreateEntityInput = decode(input)?;
        // `is_protagonist` exists only for the initial seed (spec/05 §6: "canon_seed only,
        // otherwise error.seed_only"). Passing it at all is the violation, `false` included.
        if input.is_protagonist.is_some() && ctx.kind() != crate::db::json::SessionKind::CanonSeed {
            return Err(ToolError::new(ToolErrorKind::SeedOnly)
                .with("tool", "create_entity")
                .with("field", "is_protagonist"));
        }
        let book_id = book_id(ctx)?;
        let write = write_of(ctx)?;
        let name = input.name.trim().to_owned();
        let (dto, events) = entity::create_inner(
            ctx.tx,
            &write,
            book_id,
            EntityInput {
                kind: input.kind,
                name: name.clone(),
                aliases: input.aliases,
                summary: input.summary,
                description: input.description,
                state: input.state,
                attributes: input.attributes,
                importance: input.importance,
                status: input.status.unwrap_or(EntityStatus::Active),
                is_protagonist: input.is_protagonist.unwrap_or(false),
                track_id: None,
                note: input.note,
            },
        )
        .await
        .map_err(|error| map_canon(error, None, Some(&name)))?;
        canon_outcome(
            ctx,
            "create_entity",
            dto.name.clone(),
            &events,
            "entity",
            dto.id,
            &dto.name,
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateEntityInput {
    entity: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    aliases_add: Vec<String>,
    #[serde(default)]
    aliases_remove: Vec<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    description_append: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    attributes_set: Option<BTreeMap<String, String>>,
    #[serde(default)]
    attributes_remove: Vec<String>,
    #[serde(default)]
    importance: Option<i32>,
    #[serde(default)]
    status: Option<EntityStatus>,
}

/// `update_entity` (spec/05 §6).
pub struct UpdateEntityTool;

#[async_trait::async_trait]
impl ToolHandler for UpdateEntityTool {
    fn name(&self) -> &'static str {
        "update_entity"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }

    fn preview_field(&self) -> Option<&'static str> {
        Some("state")
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: UpdateEntityInput = decode(input)?;
        let note = require_note(&input.note)?;
        if input.description.is_some() && input.description_append.is_some() {
            return Err(ToolError::new(ToolErrorKind::MutuallyExclusive)
                .with("fields", vec!["description", "description_append"]));
        }
        let entity = resolve_entity(ctx, &input.entity).await?;
        let mut patch = EntityPatch {
            name: input.name,
            summary: input.summary,
            description: None,
            state: input.state,
            importance: input.importance,
            status: input.status,
            note: Some(note),
            ..EntityPatch::default()
        };
        // Aliases are additive here; the patch replaces the whole list.
        if !input.aliases_add.is_empty() || !input.aliases_remove.is_empty() {
            let mut aliases =
                canon::aliases_of(&entity).map_err(|error| map_canon(error, None, None))?;
            for add in &input.aliases_add {
                let add = add.trim();
                if !add.is_empty() && !aliases.iter().any(|alias| alias == add) {
                    aliases.push(add.to_owned());
                }
            }
            aliases.retain(|alias| !input.aliases_remove.iter().any(|remove| remove == alias));
            patch.aliases = Some(canon::clean_aliases(&aliases));
        }
        if let Some(text) = &input.description {
            patch.description = Some(text.clone());
        } else if let Some(append) = &input.description_append {
            let current = entity.description.trim_end();
            patch.description = Some(if current.is_empty() {
                append.clone()
            } else {
                format!("{current}\n\n{append}")
            });
        }
        if input.attributes_set.is_some() || !input.attributes_remove.is_empty() {
            let mut attributes = canon::entity::attributes_of(&entity)
                .map_err(|error| map_canon(error, None, None))?;
            if let Some(set) = &input.attributes_set {
                for (key, value) in set {
                    attributes.insert(key.clone(), value.clone());
                }
            }
            for key in &input.attributes_remove {
                attributes.remove(key);
            }
            patch.attributes = Some(attributes);
        }
        let confict_name = patch.name.clone();
        // `what_label` is model-facing text, so it comes from `shared.label` (spec/09 §1).
        let entity_label = ctx.prompts.label("target_type", "entity");
        let (dto, events) = entity::update_inner(ctx.tx, &write_of(ctx)?, entity.id, patch)
            .await
            .map_err(|error| {
                map_canon(
                    error,
                    Some((entity_label.as_str(), entity.id)),
                    confict_name.as_deref(),
                )
            })?;
        canon_outcome(
            ctx,
            "update_entity",
            dto.name.clone(),
            &events,
            "entity",
            dto.id,
            &dto.name,
        )
    }
}

// ───────────────────────────── relation tools ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetRelationInput {
    source: String,
    target: String,
    kind: String,
    #[serde(default)]
    inverse_kind: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    note: String,
}

/// `set_relation` (spec/05 §6).
pub struct SetRelationTool;

#[async_trait::async_trait]
impl ToolHandler for SetRelationTool {
    fn name(&self) -> &'static str {
        "set_relation"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: SetRelationInput = decode(input)?;
        let source = resolve_entity(ctx, &input.source).await?;
        let target = resolve_entity(ctx, &input.target).await?;
        let book_id = book_id(ctx)?;
        let (dto, events) = canon::relation::set_relation_inner(
            ctx.tx,
            &write_of(ctx)?,
            book_id,
            RelationKey {
                source_id: source.id,
                target_id: target.id,
                kind: &input.kind,
            },
            RelationDetails {
                inverse_kind: &input.inverse_kind,
                description: &input.description,
                note: &input.note,
            },
        )
        .await
        .map_err(|error| map_canon(error, None, None))?;
        canon_outcome(
            ctx,
            "set_relation",
            format!("{} → {}", source.name, target.name),
            &events,
            "relation",
            dto.id,
            &dto.kind,
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EndRelationInput {
    source: String,
    target: String,
    kind: String,
    #[serde(default)]
    note: String,
}

/// `end_relation` (spec/05 §6).
pub struct EndRelationTool;

#[async_trait::async_trait]
impl ToolHandler for EndRelationTool {
    fn name(&self) -> &'static str {
        "end_relation"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: EndRelationInput = decode(input)?;
        let note = require_note(&input.note)?;
        let source = resolve_entity(ctx, &input.source).await?;
        let target = resolve_entity(ctx, &input.target).await?;
        let book_id = book_id(ctx)?;
        let (dto, events) = canon::relation::end_relation_inner(
            ctx.tx,
            &write_of(ctx)?,
            book_id,
            RelationKey {
                source_id: source.id,
                target_id: target.id,
                kind: &input.kind,
            },
            &note,
        )
        .await
        .map_err(|error| {
            // The table names `error.relation_not_found` for a missing active relation.
            match &error {
                CanonError::App(app) if app.code == crate::error::ErrorCode::NotFound => {
                    ToolError::new(ToolErrorKind::RelationNotFound)
                        .with("source", source.name.clone())
                        .with("target", target.name.clone())
                        .with("kind", input.kind.clone())
                }
                _ => map_canon(error, None, None),
            }
        })?;
        canon_outcome(
            ctx,
            "end_relation",
            format!("{} → {}", source.name, target.name),
            &events,
            "relation",
            dto.id,
            &dto.kind,
        )
    }
}

// ───────────────────────────── appearances ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordAppearancesInput {
    appearances: Vec<AppearanceEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppearanceEntry {
    entity: String,
    #[serde(default)]
    role: Option<AppearanceRole>,
    #[serde(default)]
    note: String,
}

/// `record_appearances` (spec/05 §6): replaces this chapter's appearance set.
pub struct RecordAppearancesTool;

#[async_trait::async_trait]
impl ToolHandler for RecordAppearancesTool {
    fn name(&self) -> &'static str {
        "record_appearances"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: RecordAppearancesInput = decode(input)?;
        let chapter = chapter_of(ctx)?.clone();
        let mut entries = Vec::with_capacity(input.appearances.len());
        for entry in &input.appearances {
            let entity = resolve_entity(ctx, &entry.entity).await?;
            entries.push(AppearanceInput {
                entity_id: entity.id,
                role: entry.role.unwrap_or(AppearanceRole::Minor),
                note: entry.note.clone(),
            });
        }
        let (rows, events) =
            appearance::record_set_inner(ctx.tx, &write_of(ctx)?, chapter.id, &entries)
                .await
                .map_err(|error| map_canon(error, None, None))?;
        canon_outcome_with(
            ctx,
            "record_appearances",
            String::new(),
            &events,
            "appearance_set",
            chapter.id,
            "",
            json!({ "count": rows.len() }),
        )
    }
}

// ───────────────────────────── timeline ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AddTimelineEventInput {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    story_time: String,
    #[serde(default)]
    after_event: Option<i32>,
    #[serde(default)]
    before_event: Option<i32>,
    #[serde(default)]
    entities: Vec<String>,
    #[serde(default = "default_importance")]
    importance: i32,
}

/// `add_timeline_event` (spec/05 §6).
pub struct AddTimelineEventTool;

#[async_trait::async_trait]
impl ToolHandler for AddTimelineEventTool {
    fn name(&self) -> &'static str {
        "add_timeline_event"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: AddTimelineEventInput = decode(input)?;
        if input.after_event.is_some() && input.before_event.is_some() {
            return Err(ToolError::new(ToolErrorKind::MutuallyExclusive)
                .with("fields", vec!["after_event", "before_event"]));
        }
        let book_id = book_id(ctx)?;
        let mut entity_ids = Vec::with_capacity(input.entities.len());
        for reference in &input.entities {
            entity_ids.push(resolve_entity(ctx, reference).await?.id);
        }
        let (dto, events) = timeline::create_inner(
            ctx.tx,
            &write_of(ctx)?,
            book_id,
            TimelineEventInput {
                title: input.title,
                description: input.description,
                story_time: input.story_time,
                chapter_id: None,
                entity_ids,
                importance: input.importance,
                after_id: input.after_event,
                before_id: input.before_event,
                note: None,
            },
        )
        .await
        .map_err(|error| map_canon(error, None, None))?;
        let (prev, next) = timeline_neighbors(ctx.tx, book_id, dto.id).await?;
        canon_outcome_with(
            ctx,
            "add_timeline_event",
            dto.title.clone(),
            &events,
            "timeline_event",
            dto.id,
            &dto.title,
            json!({ "prev_event": prev, "next_event": next, "moved": false }),
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateTimelineEventInput {
    event_id: i32,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    story_time: Option<String>,
    #[serde(default)]
    entities: Option<Vec<String>>,
    #[serde(default)]
    importance: Option<i32>,
    #[serde(default)]
    move_after: Option<i32>,
    #[serde(default)]
    move_before: Option<i32>,
}

/// `update_timeline_event` (spec/05 §6).
pub struct UpdateTimelineEventTool;

#[async_trait::async_trait]
impl ToolHandler for UpdateTimelineEventTool {
    fn name(&self) -> &'static str {
        "update_timeline_event"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: UpdateTimelineEventInput = decode(input)?;
        if input.move_after.is_some() && input.move_before.is_some() {
            return Err(ToolError::new(ToolErrorKind::MutuallyExclusive)
                .with("fields", vec!["move_after", "move_before"]));
        }
        let mut entity_ids = None;
        if let Some(entities) = &input.entities {
            let mut ids = Vec::with_capacity(entities.len());
            for reference in entities {
                ids.push(resolve_entity(ctx, reference).await?.id);
            }
            entity_ids = Some(ids);
        }
        let moved = input.move_after.is_some() || input.move_before.is_some();
        let book_id = book_id(ctx)?;
        let (dto, events) = timeline::update_inner(
            ctx.tx,
            &write_of(ctx)?,
            input.event_id,
            TimelineEventPatch {
                title: input.title,
                description: input.description,
                story_time: input.story_time,
                entity_ids,
                importance: input.importance,
                after_id: input.move_after,
                before_id: input.move_before,
                ..TimelineEventPatch::default()
            },
        )
        .await
        .map_err(|error| {
            let label = ctx.prompts.label("target_type", "timeline_event");
            map_canon(error, Some((label.as_str(), input.event_id)), None)
        })?;
        let (prev, next) = timeline_neighbors(ctx.tx, book_id, dto.id).await?;
        canon_outcome_with(
            ctx,
            "update_timeline_event",
            dto.title.clone(),
            &events,
            "timeline_event",
            dto.id,
            &dto.title,
            json!({ "prev_event": prev, "next_event": next, "moved": moved }),
        )
    }
}

/// The event before and after `event_id` in story order (for the result's position note).
async fn timeline_neighbors(
    conn: &impl ConnectionTrait,
    book_id: i32,
    event_id: i32,
) -> Result<(Option<i32>, Option<i32>), ToolError> {
    let rows = timeline::ordered(conn, book_id, None)
        .await
        .map_err(|error| map_canon(error, None, None))?;
    let index = rows.iter().position(|row| row.id == event_id);
    match index {
        Some(index) => Ok((
            index
                .checked_sub(1)
                .and_then(|previous| rows.get(previous))
                .map(|row| row.id),
            rows.get(index + 1).map(|row| row.id),
        )),
        None => Ok((None, None)),
    }
}

// ───────────────────────────── notes and foreshadowing ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AddMemoryNoteInput {
    kind: NoteKind,
    content: String,
    #[serde(default)]
    entities: Vec<String>,
    #[serde(default = "default_importance")]
    importance: i32,
    #[serde(default)]
    note: Option<String>,
}

/// `add_memory_note` (spec/05 §6).
pub struct AddMemoryNoteTool;

#[async_trait::async_trait]
impl ToolHandler for AddMemoryNoteTool {
    fn name(&self) -> &'static str {
        "add_memory_note"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: AddMemoryNoteInput = decode(input)?;
        let book_id = book_id(ctx)?;
        let mut entity_ids = Vec::with_capacity(input.entities.len());
        for reference in &input.entities {
            entity_ids.push(resolve_entity(ctx, reference).await?.id);
        }
        let content = input.content.trim().to_owned();
        let (dto, events) = note::create_inner(
            ctx.tx,
            &write_of(ctx)?,
            book_id,
            MemoryNoteInput {
                kind: input.kind,
                content: content.clone(),
                chapter_id: None,
                importance: input.importance,
                entity_ids,
                note: input.note,
            },
        )
        .await
        .map_err(|error| map_canon(error, None, None))?;
        let name: String = content.chars().take(40).collect();
        canon_outcome(
            ctx,
            "add_memory_note",
            name.clone(),
            &events,
            "memory_note",
            dto.id,
            &name,
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetireMemoryNoteInput {
    note_id: i32,
    reason: String,
}

/// `retire_memory_note` (spec/05 §6).
pub struct RetireMemoryNoteTool;

#[async_trait::async_trait]
impl ToolHandler for RetireMemoryNoteTool {
    fn name(&self) -> &'static str {
        "retire_memory_note"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: RetireMemoryNoteInput = decode(input)?;
        let reason = require_note(&input.reason)?;
        let (dto, events) = note::retire_inner(ctx.tx, &write_of(ctx)?, input.note_id, &reason)
            .await
            .map_err(|error| {
                let label = ctx.prompts.label("target_type", "memory_note");
                map_canon(error, Some((label.as_str(), input.note_id)), None)
            })?;
        let name: String = dto.content.chars().take(40).collect();
        canon_outcome(
            ctx,
            "retire_memory_note",
            name.clone(),
            &events,
            "memory_note",
            dto.id,
            &name,
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AddForeshadowingInput {
    title: String,
    description: String,
    #[serde(default)]
    planned_payoff: String,
    #[serde(default)]
    entities: Vec<String>,
    #[serde(default = "default_importance")]
    importance: i32,
}

/// `add_foreshadowing` (spec/05 §6).
pub struct AddForeshadowingTool;

#[async_trait::async_trait]
impl ToolHandler for AddForeshadowingTool {
    fn name(&self) -> &'static str {
        "add_foreshadowing"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: AddForeshadowingInput = decode(input)?;
        let book_id = book_id(ctx)?;
        let mut entity_ids = Vec::with_capacity(input.entities.len());
        for reference in &input.entities {
            entity_ids.push(resolve_entity(ctx, reference).await?.id);
        }
        let (dto, events) = foreshadow::create_inner(
            ctx.tx,
            &write_of(ctx)?,
            book_id,
            ForeshadowingInput {
                title: input.title,
                description: input.description,
                planned_payoff: input.planned_payoff,
                planted_chapter_id: None,
                importance: input.importance,
                entity_ids,
                note: None,
            },
        )
        .await
        .map_err(|error| map_canon(error, None, None))?;
        canon_outcome(
            ctx,
            "add_foreshadowing",
            dto.title.clone(),
            &events,
            "foreshadowing",
            dto.id,
            &dto.title,
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveForeshadowingInput {
    foreshadowing_id: i32,
    status: ForeshadowingStatus,
    resolution: String,
}

/// `resolve_foreshadowing` (spec/05 §6).
pub struct ResolveForeshadowingTool;

#[async_trait::async_trait]
impl ToolHandler for ResolveForeshadowingTool {
    fn name(&self) -> &'static str {
        "resolve_foreshadowing"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: ResolveForeshadowingInput = decode(input)?;
        if !matches!(
            input.status,
            ForeshadowingStatus::Resolved | ForeshadowingStatus::Abandoned
        ) {
            return Err(ToolError::new(ToolErrorKind::Validation).with(
                "errors",
                vec!["status must be resolved or abandoned".to_owned()],
            ));
        }
        let resolution = require_note(&input.resolution)?;
        let (dto, events) = foreshadow::resolve_inner(
            ctx.tx,
            &write_of(ctx)?,
            input.foreshadowing_id,
            input.status,
            &resolution,
        )
        .await
        .map_err(|error| {
            let label = ctx.prompts.label("target_type", "foreshadowing");
            map_canon(error, Some((label.as_str(), input.foreshadowing_id)), None)
        })?;
        canon_outcome(
            ctx,
            "resolve_foreshadowing",
            dto.title.clone(),
            &events,
            "foreshadowing",
            dto.id,
            &dto.title,
        )
    }
}

// ───────────────────────────── chapter summary ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetChapterSummaryInput {
    summary: String,
}

/// `set_chapter_summary` (spec/05 §6): the summary is short-term memory for later chapters.
pub struct SetChapterSummaryTool;

#[async_trait::async_trait]
impl ToolHandler for SetChapterSummaryTool {
    fn name(&self) -> &'static str {
        "set_chapter_summary"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }

    fn preview_field(&self) -> Option<&'static str> {
        Some("summary")
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: SetChapterSummaryInput = decode(input)?;
        let summary = input.summary.trim().to_owned();
        if summary.is_empty() {
            return Err(ToolError::new(ToolErrorKind::Validation)
                .with("errors", vec!["summary must not be empty".to_owned()]));
        }
        // 150–400 characters are recommended; the hard limit is 2,000 (spec/05 §6).
        let chars = summary.chars().count();
        if chars > 2_000 {
            return Err(ToolError::new(ToolErrorKind::Validation).with(
                "errors",
                vec![format!("summary is {chars} characters; the limit is 2000")],
            ));
        }
        let chapter = chapter_of(ctx)?.clone();
        if chapter.summary == summary {
            return canon_outcome_with(
                ctx,
                "set_chapter_summary",
                summary.clone(),
                &[],
                "chapter_summary",
                chapter.id,
                "",
                json!({}),
            );
        }
        let before = Snapshot::ChapterSummary {
            summary: chapter.summary.clone(),
        };
        let after = Snapshot::ChapterSummary {
            summary: summary.clone(),
        };
        chapters::ActiveModel {
            id: sea_orm::Set(chapter.id),
            summary: sea_orm::Set(summary.clone()),
            updated_at: sea_orm::Set(chrono::Utc::now()),
            ..Default::default()
        }
        .update(ctx.tx)
        .await
        .map_err(|_| internal())?;
        let change_id = crate::services::change_log::record(
            ctx.tx,
            crate::services::change_log::NewChange {
                book_id: chapter.book_id,
                track_id: Some(chapter.track_id),
                chapter_id: Some(chapter.id),
                session_id: Some(ctx.session.id),
                actor: ChangeActor::Agent,
                target_type: ChangeTargetType::ChapterSummary,
                target_id: chapter.id,
                op: ChangeOp::Update,
                before: Some(&before),
                after: Some(&after),
                note: "",
                reverts_id: None,
            },
        )
        .await
        .map_err(|_| internal())?;
        let events = vec![ServerEvent::CanonChanged {
            book_id: chapter.book_id,
            target_type: ChangeTargetType::ChapterSummary,
            target_id: chapter.id,
            op: ChangeOp::Update,
            change_id,
        }];
        canon_outcome(
            ctx,
            "set_chapter_summary",
            summary,
            &events,
            "chapter_summary",
            chapter.id,
            "",
        )
    }
}

// ───────────────────────────── finish_maintenance ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FinishMaintenanceInput {
    #[serde(default)]
    report: String,
}

/// `finish_maintenance` (spec/05 §6, spec/06 §6).
pub struct FinishMaintenanceTool;

#[async_trait::async_trait]
impl ToolHandler for FinishMaintenanceTool {
    fn name(&self) -> &'static str {
        "finish_maintenance"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Control
    }

    fn preview_field(&self) -> Option<&'static str> {
        Some("report")
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: FinishMaintenanceInput = decode(input)?;
        let report = input.report.trim().to_owned();
        if report.is_empty() {
            return Err(ToolError::new(ToolErrorKind::Validation)
                .with("errors", vec!["report must not be empty".to_owned()]));
        }
        let book_id = book_id(ctx)?;

        // `chapter_maintenance` requires recorded appearances and a summary (spec/05 §6).
        if ctx.kind() == crate::db::json::SessionKind::ChapterMaintenance {
            let mut missing = Vec::new();
            if !session_called(ctx, "record_appearances").await? {
                missing.push("appearances");
            }
            if !session_called(ctx, "set_chapter_summary").await? {
                missing.push("summary");
            }
            if !missing.is_empty() {
                return Err(
                    ToolError::new(ToolErrorKind::MaintenanceIncomplete).with("missing", missing)
                );
            }
        }

        // Protagonist-state confirmation (spec/06 §6, D-060): when the protagonist appeared in
        // the maintained chapter and this session did not change their `state`, the first call
        // withholds completion once.
        let mut confirm_needed = false;
        if ctx.kind() == crate::db::json::SessionKind::ChapterMaintenance {
            let chapter = chapter_of(ctx)?;
            if let Some(protagonist) = protagonist_of(ctx).await? {
                // An isolated track cannot write a shared protagonist: their changes become a
                // track-local note and no confirmation is asked (D-060).
                let shared_in_isolated =
                    !ctx.visibility.can_write_shared() && protagonist.track_id.is_none();
                let appeared = appearances::Entity::find()
                    .filter(appearances::COLUMN.chapter_id.eq(chapter.id))
                    .filter(appearances::COLUMN.entity_id.eq(protagonist.id))
                    .one(ctx.tx)
                    .await
                    .map_err(|_| internal())?
                    .is_some();
                let state_touched = session_changed_state(ctx, protagonist.id).await?;
                let already_warned = crate::agent::session::counters_of(ctx.session).finish_warned;
                if appeared && !shared_in_isolated && !state_touched && !already_warned {
                    confirm_needed = true;
                }
            }
        }

        let changes = change_log::Entity::find()
            .filter(change_log::COLUMN.session_id.eq(ctx.session.id))
            .all(ctx.tx)
            .await
            .map_err(|_| internal())?
            .len();
        let proposals = crate::db::entities::setting_proposals::Entity::find()
            .filter(
                crate::db::entities::setting_proposals::COLUMN
                    .session_id
                    .eq(ctx.session.id),
            )
            .all(ctx.tx)
            .await
            .map_err(|_| internal())?
            .len();
        let text = ctx
            .prompts
            .render_value(
                "result.finish_maintenance",
                crate::prompts::template_value(&json!({
                    "confirm_needed": confirm_needed,
                    "changes": changes,
                    "proposals": proposals,
                })),
            )
            .map_err(|_| internal())?
            .text;
        let mut outcome = ToolOutcome::text(text).with_ui(json!({
            "changes": changes,
            "proposals": proposals,
        }));
        if !confirm_needed {
            outcome = outcome.with_effects(ToolEffects {
                complete: Some(CompletionKind::Maintenance),
                ..ToolEffects::default()
            });
        } else {
            outcome = outcome.with_finish_warned();
        }
        let _ = book_id;
        Ok(outcome)
    }
}

/// True when this session has a resolved call of `name`.
async fn session_called(ctx: &ToolCtx<'_>, name: &str) -> Result<bool, ToolError> {
    Ok(agent_tool_calls::Entity::find()
        .filter(agent_tool_calls::COLUMN.session_id.eq(ctx.session.id))
        .filter(agent_tool_calls::COLUMN.name.eq(name))
        .filter(agent_tool_calls::COLUMN.status.eq(ToolCallStatus::Done))
        .one(ctx.tx)
        .await
        .map_err(|_| internal())?
        .is_some())
}

/// The protagonist: the side track's focus character, otherwise the visible `is_protagonist`.
async fn protagonist_of(ctx: &ToolCtx<'_>) -> Result<Option<entities::Model>, ToolError> {
    let book_id = book_id(ctx)?;
    let focus = match ctx.track {
        Some(track) => track.focus_entity_id,
        None => None,
    };
    if let Some(entity_id) = focus {
        return canon::load_visible_entity(
            ctx.tx,
            canon::Visibility::of_session(book_id, ctx.visibility),
            entity_id,
        )
        .await
        .map(Some)
        .map_err(|error| map_canon(error, None, None));
    }
    let visibility = canon::Visibility::of_session(book_id, ctx.visibility);
    let rows = canon::visible_entities(ctx.tx, visibility)
        .await
        .map_err(|error| map_canon(error, None, None))?;
    Ok(rows.into_iter().find(|row| row.is_protagonist))
}

/// True when a change of this session touched the entity's `state` (spec/06 §6).
async fn session_changed_state(ctx: &ToolCtx<'_>, entity_id: i32) -> Result<bool, ToolError> {
    let rows = change_log::Entity::find()
        .filter(change_log::COLUMN.session_id.eq(ctx.session.id))
        .filter(change_log::COLUMN.target_type.eq(ChangeTargetType::Entity))
        .filter(change_log::COLUMN.target_id.eq(entity_id))
        .order_by_asc(change_log::COLUMN.id)
        .all(ctx.tx)
        .await
        .map_err(|_| internal())?;
    for row in rows {
        let before = row.before.clone().map(|value| value.into_inner());
        let after = row.after.clone().map(|value| value.into_inner());
        let before_state = match &before {
            Some(Snapshot::Entity { state, .. }) => state.clone(),
            _ => String::new(),
        };
        let after_state = match &after {
            Some(Snapshot::Entity { state, .. }) => state.clone(),
            _ => String::new(),
        };
        if before_state != after_state {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The role's canon write handlers (spec/05 §2). The retrieval tools they share with the writer
/// live in `retrieval.rs`; `propose_setting_change` lives in `proposals.rs`.
pub fn all() -> Vec<std::sync::Arc<dyn ToolHandler>> {
    vec![
        std::sync::Arc::new(CreateEntityTool),
        std::sync::Arc::new(UpdateEntityTool),
        std::sync::Arc::new(SetRelationTool),
        std::sync::Arc::new(EndRelationTool),
        std::sync::Arc::new(RecordAppearancesTool),
        std::sync::Arc::new(AddTimelineEventTool),
        std::sync::Arc::new(UpdateTimelineEventTool),
        std::sync::Arc::new(AddMemoryNoteTool),
        std::sync::Arc::new(RetireMemoryNoteTool),
        std::sync::Arc::new(AddForeshadowingTool),
        std::sync::Arc::new(ResolveForeshadowingTool),
        std::sync::Arc::new(SetChapterSummaryTool),
        std::sync::Arc::new(FinishMaintenanceTool),
    ]
}
