//! The book-agent's new action tools (plan §3.3 tools 22–42, §5.4, D-169).
//!
//! Previews compute the same before/after as `execute` without writing. The approval path runs
//! `prepare` again before `execute`, so chapter locks are checked against current data.

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::dto::chapters::{ChapterContentTextPut, ChapterInput, ChapterPatch};
use crate::api::dto::goals::{GoalNodeInput, GoalNodePatch};
use crate::api::dto::knowledge::EntryInput;
use crate::api::dto::research::{ResearchTaskCreate, ResearchTaskPatch};
use crate::db::entities::{agent_sessions, chapters, model_profiles, providers, research_tasks};
use crate::db::json::{
    AgentRole, CanonStatus, ChapterContent, ChapterStatus, GoalCondition, GoalStatus,
    ResearchTaskStatus, SessionKind, SessionStatus,
};
use crate::services::canon::{CanonWrite, Visibility as CanonVisibility};
use crate::services::maintenance::{last_archivist_kind, open_archivist_session};
use crate::services::research::Transition;
use crate::services::{
    chapters as chapter_service, goals as goal_service, knowledge, research, versions,
};

use super::{
    PrepareCtx, Prepared, ToolCtx, ToolError, ToolErrorKind, ToolHandler, ToolKind, ToolOutcome,
};

fn decode<T: for<'de> Deserialize<'de>>(input: Value) -> Result<T, ToolError> {
    serde_json::from_value(input).map_err(|error| {
        ToolError::new(ToolErrorKind::Validation).with("errors", vec![error.to_string()])
    })
}

fn internal() -> ToolError {
    ToolError::new(ToolErrorKind::Internal)
}
fn validation(message: &str) -> ToolError {
    ToolError::new(ToolErrorKind::Validation).with("errors", vec![message.to_owned()])
}
fn book_id(ctx: &ToolCtx<'_>) -> Result<i32, ToolError> {
    ctx.book.map(|book| book.id).ok_or_else(internal)
}

fn map_chapter_error(error: crate::error::AppError) -> ToolError {
    if error.code == crate::error::ErrorCode::ChapterLocked {
        let session_id = error
            .details
            .as_ref()
            .and_then(|details| details.get("session_id"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        ToolError::new(ToolErrorKind::ChapterLocked).with("session_id", session_id)
    } else {
        internal()
    }
}
fn write_context(ctx: &ToolCtx<'_>) -> CanonWrite {
    let book_id = ctx.book.map(|book| book.id).unwrap_or_default();
    CanonWrite::agent(
        CanonVisibility::of_session(book_id, ctx.visibility),
        ctx.session.id,
        ctx.session.chapter_id,
    )
}
fn text_outcome(
    ctx: &mut ToolCtx<'_>,
    action: &str,
    summary: String,
    details: Value,
) -> Result<ToolOutcome, ToolError> {
    let rendered = ctx
        .prompts
        .render_value(
            "result.book_agent_write",
            crate::prompts::template_value(&json!({
                "action": action, "summary": summary, "details": details,
            })),
        )
        .map_err(|_| internal())?;
    Ok(ToolOutcome::text(rendered.text).with_ui(json!({"action": action})))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateChapterInput {
    #[serde(default)]
    track: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    guidance: String,
}

pub struct CreateChapterTool;
#[async_trait::async_trait]
impl ToolHandler for CreateChapterTool {
    fn name(&self) -> &'static str {
        "create_chapter"
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
        let input: CreateChapterInput = decode(input)?;
        let book_id = book_id(ctx)?;
        let track_id =
            resolve_track(ctx, book_id, input.track.as_deref().unwrap_or("main")).await?;
        let (dto, events) = chapter_service::create_inner(
            ctx.tx,
            track_id,
            ChapterInput {
                title: input.title,
                guidance: input.guidance,
            },
        )
        .await
        .map_err(|_| internal())?;
        ctx.events.extend(events);
        text_outcome(
            ctx,
            "create_chapter",
            format!("创建章节《{}》", dto.list_item.title),
            json!({"chapter_id": dto.list_item.id, "ord": dto.list_item.number}),
        )
    }
    async fn preview(&self, _ctx: &mut ToolCtx<'_>, input: Value) -> Result<Value, ToolError> {
        let input: CreateChapterInput = decode(input)?;
        Ok(
            json!({"kind":"fields","label":"create_chapter","rows":[{"field":"track","after":input.track.unwrap_or_else(|| "main".to_owned())},{"field":"title","after":input.title},{"field":"guidance","after":input.guidance}]}),
        )
    }
}

async fn resolve_track(ctx: &ToolCtx<'_>, book_id: i32, raw: &str) -> Result<i32, ToolError> {
    if let Some(id) = raw.strip_prefix('#').and_then(|v| v.parse::<i32>().ok()) {
        let track = crate::db::entities::tracks::Entity::find_by_id(id)
            .one(ctx.tx)
            .await
            .map_err(|_| internal())?
            .filter(|t| t.book_id == book_id);
        return track.map(|t| t.id).ok_or_else(|| {
            ToolError::new(ToolErrorKind::NotFound)
                .with("what_label", "track".to_owned())
                .with("id", id)
        });
    }
    if raw != "main" {
        return Err(validation("track must be main or #id"));
    }
    crate::db::entities::tracks::Entity::find()
        .filter(crate::db::entities::tracks::COLUMN.book_id.eq(book_id))
        .filter(
            crate::db::entities::tracks::COLUMN
                .kind
                .eq(crate::db::json::TrackKind::Main),
        )
        .one(ctx.tx)
        .await
        .map_err(|_| internal())?
        .map(|t| t.id)
        .ok_or_else(internal)
}

#[derive(Debug, Deserialize)]
struct EditChapterInput {
    chapter: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    guidance: Option<String>,
    #[serde(default)]
    outline: Option<String>,
    #[serde(default)]
    outline_edits: Option<Vec<FindReplace>>,
    #[serde(default)]
    reference_notes: Option<String>,
    #[serde(default)]
    notes_edits: Option<Vec<FindReplace>>,
}
#[derive(Debug, Deserialize)]
struct FindReplace {
    find: String,
    replace: String,
}

pub struct EditChapterTool;
#[async_trait::async_trait]
impl ToolHandler for EditChapterTool {
    fn name(&self) -> &'static str {
        "edit_chapter"
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
        let input: EditChapterInput = decode(input)?;
        let chapter = resolve_chapter(ctx, book_id(ctx)?, &input.chapter).await?;
        if input.outline.is_some() && input.outline_edits.is_some() {
            return Err(validation(
                "outline and outline_edits are mutually exclusive",
            ));
        }
        if input.reference_notes.is_some() && input.notes_edits.is_some() {
            return Err(validation(
                "reference_notes and notes_edits are mutually exclusive",
            ));
        }
        let outline = match (input.outline, input.outline_edits) {
            (Some(value), _) => Some(value),
            (None, Some(edits)) => Some(apply_edits(&chapter.outline, &edits, ctx)?),
            _ => None,
        };
        let notes = match (input.reference_notes, input.notes_edits) {
            (Some(value), _) => Some(value),
            (None, Some(edits)) => Some(apply_edits(&chapter.reference_notes, &edits, ctx)?),
            _ => None,
        };
        let (dto, events) = chapter_service::update_inner(
            ctx.tx,
            chapter.id,
            ChapterPatch {
                title: input.title,
                guidance: input.guidance,
                outline,
                reference_notes: notes,
                summary: None,
            },
        )
        .await
        .map_err(map_chapter_error)?;
        ctx.events.extend(events);
        text_outcome(
            ctx,
            "edit_chapter",
            format!("修改章节《{}》", dto.list_item.title),
            json!({"chapter_id": dto.list_item.id}),
        )
    }
    async fn preview(&self, ctx: &mut ToolCtx<'_>, input: Value) -> Result<Value, ToolError> {
        let input: EditChapterInput = decode(input)?;
        let chapter = resolve_chapter(ctx, book_id(ctx)?, &input.chapter).await?;
        if input.outline.is_some() && input.outline_edits.is_some() {
            return Err(validation(
                "outline and outline_edits are mutually exclusive",
            ));
        }
        if input.reference_notes.is_some() && input.notes_edits.is_some() {
            return Err(validation(
                "reference_notes and notes_edits are mutually exclusive",
            ));
        }
        let mut rows = Vec::new();
        if let Some(v) = input.title {
            rows.push(json!({"field":"title","before":chapter.title,"after":v}));
        }
        if let Some(v) = input.guidance {
            rows.push(json!({"field":"guidance","before":chapter.guidance,"after":v}));
        }
        if let Some(v) = input.outline {
            rows.push(json!({"field":"outline","before":chapter.outline,"after":v}));
        } else if let Some(edits) = input.outline_edits {
            let after = apply_edits(&chapter.outline, &edits, ctx)?;
            rows.push(json!({"field":"outline","before":chapter.outline,"after":after}));
        }
        if let Some(v) = input.reference_notes {
            rows.push(
                json!({"field":"reference_notes","before":chapter.reference_notes,"after":v}),
            );
        } else if let Some(edits) = input.notes_edits {
            let after = apply_edits(&chapter.reference_notes, &edits, ctx)?;
            rows.push(
                json!({"field":"reference_notes","before":chapter.reference_notes,"after":after}),
            );
        }
        Ok(json!({"kind":"fields","label":chapter.title,"rows":rows}))
    }
}

#[derive(Debug, Deserialize)]
struct EditChapterTextInput {
    chapter: String,
    edits: Vec<FindReplace>,
}

pub struct EditChapterTextTool;
#[async_trait::async_trait]
impl ToolHandler for EditChapterTextTool {
    fn name(&self) -> &'static str {
        "edit_chapter_text"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }
    async fn prepare(&self, ctx: &PrepareCtx<'_>, input: &Value) -> Result<Prepared, ToolError> {
        let input: EditChapterTextInput =
            serde_json::from_value(input.clone()).map_err(|e| validation(&e.to_string()))?;
        let chapter = resolve_chapter_conn(
            ctx.db,
            ctx.session.book_id.ok_or_else(internal)?,
            &input.chapter,
        )
        .await?;
        chapter_service::ensure_editable(ctx.db, chapter.id)
            .await
            .map_err(map_chapter_error)?;
        Ok(Prepared::EditChapterText)
    }
    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: EditChapterTextInput = decode(input)?;
        if !matches!(prepared, Prepared::EditChapterText) {
            return Err(ToolError::new(ToolErrorKind::ChapterLocked).with("session_id", 0));
        }
        let chapter = resolve_chapter(ctx, book_id(ctx)?, &input.chapter).await?;
        let (new_text, before) = apply_text_edits(&chapter, &input.edits, ctx)?;
        let (result, events) = chapter_service::put_content_text_inner(
            ctx.tx,
            chapter.id,
            ChapterContentTextPut {
                base_version: chapter.content_version,
                text: new_text,
            },
        )
        .await
        .map_err(map_chapter_error)?;
        // The backup is written only after the text service accepted the edit. If the write is
        // rejected (lock or stale version), the transaction has no backup row either.
        let has_backup = crate::db::entities::chapter_versions::Entity::find()
            .filter(
                crate::db::entities::chapter_versions::COLUMN
                    .chapter_id
                    .eq(chapter.id),
            )
            .filter(
                crate::db::entities::chapter_versions::COLUMN
                    .kind
                    .eq(crate::db::json::VersionKind::Backup),
            )
            .filter(
                crate::db::entities::chapter_versions::COLUMN
                    .session_id
                    .eq(ctx.session.id),
            )
            .one(ctx.tx)
            .await
            .map_err(|_| internal())?
            .is_some();
        if !has_backup {
            versions::create_version(
                ctx.tx,
                versions::NewVersion {
                    chapter_id: chapter.id,
                    kind: crate::db::json::VersionKind::Backup,
                    title: &chapter.title,
                    content: &before,
                    word_count: chapter.word_count,
                    note: "book agent edit",
                    session_id: Some(ctx.session.id),
                },
            )
            .await
            .map_err(|_| internal())?;
        }
        ctx.events.extend(events);
        text_outcome(
            ctx,
            "edit_chapter_text",
            format!("修改章节《{}》正文", chapter.title),
            json!({"chapter_id": chapter.id, "version": result.version, "word_count": result.word_count}),
        )
    }
    async fn preview(&self, ctx: &mut ToolCtx<'_>, input: Value) -> Result<Value, ToolError> {
        let input: EditChapterTextInput = decode(input)?;
        let chapter = resolve_chapter(ctx, book_id(ctx)?, &input.chapter).await?;
        let (_, before) = apply_text_edits(&chapter, &input.edits, ctx)?;
        let (after, _) = apply_text_edits(&chapter, &input.edits, ctx)?;
        Ok(json!({"kind":"diff","label":chapter.title,"before":render_text(&before),"after":after}))
    }
}

fn apply_edits(text: &str, edits: &[FindReplace], _ctx: &ToolCtx<'_>) -> Result<String, ToolError> {
    let mut value = text.to_owned();
    for edit in edits {
        let count = value.matches(&edit.find).count();
        if edit.find.is_empty() || count != 1 {
            return Err(ToolError::new(ToolErrorKind::FindFailed)
                .with("doc_label", "text".to_owned())
                .with("find", edit.find.clone())
                .with("count", count as i64));
        }
        value = value.replacen(&edit.find, &edit.replace, 1);
    }
    Ok(value)
}
fn render_text(content: &ChapterContent) -> String {
    content
        .paragraphs
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}
fn apply_text_edits(
    chapter: &chapters::Model,
    edits: &[FindReplace],
    _ctx: &ToolCtx<'_>,
) -> Result<(String, ChapterContent), ToolError> {
    let before = chapter.content.clone().into_inner();
    let mut text = render_text(&before);
    for edit in edits {
        let count = text.matches(&edit.find).count();
        if edit.find.is_empty() || count != 1 {
            return Err(ToolError::new(ToolErrorKind::FindFailed)
                .with("doc_label", "text".to_owned())
                .with("find", edit.find.clone())
                .with("count", count as i64));
        }
        text = text.replacen(&edit.find, &edit.replace, 1);
    }
    Ok((text, before))
}
async fn resolve_chapter(
    ctx: &ToolCtx<'_>,
    book_id: i32,
    raw: &str,
) -> Result<chapters::Model, ToolError> {
    resolve_chapter_conn(ctx.tx, book_id, raw).await
}
async fn resolve_chapter_conn(
    conn: &impl sea_orm::ConnectionTrait,
    book_id: i32,
    raw: &str,
) -> Result<chapters::Model, ToolError> {
    let chapter = if let Some(id) = raw.strip_prefix('#').and_then(|v| v.parse::<i32>().ok()) {
        chapters::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(|_| internal())?
    } else {
        let number = raw
            .parse::<i32>()
            .map_err(|_| validation("chapter must be a number or #id"))?;
        let track = crate::db::entities::tracks::Entity::find()
            .filter(crate::db::entities::tracks::COLUMN.book_id.eq(book_id))
            .filter(
                crate::db::entities::tracks::COLUMN
                    .kind
                    .eq(crate::db::json::TrackKind::Main),
            )
            .one(conn)
            .await
            .map_err(|_| internal())?;
        match track {
            Some(track) => chapters::Entity::find()
                .filter(chapters::COLUMN.book_id.eq(book_id))
                .filter(chapters::COLUMN.track_id.eq(track.id))
                .filter(chapters::COLUMN.ord.eq(number))
                .one(conn)
                .await
                .map_err(|_| internal())?,
            None => None,
        }
    };
    chapter.filter(|c| c.book_id == book_id).ok_or_else(|| {
        ToolError::new(ToolErrorKind::ChapterNotFound).with("chapter", raw.to_owned())
    })
}

#[derive(Debug, Deserialize)]
struct DeleteCanonInput {
    target_type: String,
    id: i32,
    note: String,
}

pub struct DeleteCanonRecordTool;
#[async_trait::async_trait]
impl ToolHandler for DeleteCanonRecordTool {
    fn name(&self) -> &'static str {
        "delete_canon_record"
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
        let input: DeleteCanonInput = decode(input)?;
        let systems = crate::agent::session::canon_systems_of(ctx.session);
        let on = match input.target_type.as_str() {
            "entity" | "relation" => systems.entities,
            "timeline_event" => systems.timeline,
            "memory_note" => systems.memory,
            "foreshadowing" => systems.foreshadowing,
            _ => return Err(validation("unknown target_type")),
        };
        if !on {
            return Err(ToolError::new(ToolErrorKind::SystemOff).with("system", input.target_type));
        }
        let mut write = write_context(ctx);
        write.note = Some(input.note.clone());
        let events = match input.target_type.as_str() {
            "entity" => crate::services::canon::entity::delete_inner(ctx.tx, &write, input.id)
                .await
                .map_err(|_| internal())?,
            "relation" => crate::services::canon::relation::delete_inner(ctx.tx, &write, input.id)
                .await
                .map_err(|_| internal())?,
            "timeline_event" => {
                crate::services::canon::timeline::delete_inner(ctx.tx, &write, input.id)
                    .await
                    .map_err(|_| internal())?
            }
            "memory_note" => crate::services::canon::note::delete_inner(ctx.tx, &write, input.id)
                .await
                .map_err(|_| internal())?,
            "foreshadowing" => {
                crate::services::canon::foreshadow::delete_inner(ctx.tx, &write, input.id)
                    .await
                    .map_err(|_| internal())?
            }
            _ => unreachable!(),
        };
        ctx.events.extend(events);
        text_outcome(
            ctx,
            "delete_canon_record",
            format!("删除{} #{}", input.target_type, input.id),
            json!({"note": input.note}),
        )
    }
    async fn preview(&self, ctx: &mut ToolCtx<'_>, input: Value) -> Result<Value, ToolError> {
        let input: DeleteCanonInput = decode(input)?;
        let target = if input.target_type == "entity" {
            crate::services::canon::load_entity(ctx.tx, input.id)
                .await
                .map(|entity| entity.name)
                .unwrap_or_else(|_| format!("#{}", input.id))
        } else {
            format!("#{}", input.id)
        };
        Ok(
            json!({"kind":"action","label":"delete_canon_record","text":format!("删除{} {}：{}", input.target_type, target, input.note)}),
        )
    }
}

#[derive(Debug, Deserialize)]
struct GoalInput {
    level: crate::db::json::GoalLevel,
    title: String,
    purpose: String,
    #[serde(default)]
    parent_id: Option<i32>,
    #[serde(default)]
    status: Option<GoalStatus>,
    #[serde(default)]
    chapter_from: Option<i32>,
    #[serde(default)]
    chapter_to: Option<i32>,
    #[serde(default)]
    conditions: Option<Vec<Value>>,
    #[serde(default)]
    entity_ids: Option<Vec<i32>>,
    #[serde(default)]
    event_ids: Option<Vec<i32>>,
    #[serde(default)]
    foreshadowing_ids: Option<Vec<i32>>,
    #[serde(default)]
    note: String,
}
pub struct CreateGoalTool;
#[async_trait::async_trait]
impl ToolHandler for CreateGoalTool {
    fn name(&self) -> &'static str {
        "create_goal"
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
        let input: GoalInput = decode(input)?;
        let book_id = book_id(ctx)?;
        let conditions: Option<Vec<GoalCondition>> = input
            .conditions
            .map(|v| {
                serde_json::from_value(Value::Array(v))
                    .map_err(|_| validation("invalid conditions"))
            })
            .transpose()?;
        let (dto, events) = goal_service::create_inner_with_note(
            ctx.tx,
            &write_context(ctx),
            book_id,
            GoalNodeInput {
                track_id: None,
                level: input.level,
                parent_id: input.parent_id,
                title: input.title,
                purpose: input.purpose,
                status: input.status,
                chapter_from: input.chapter_from,
                chapter_to: input.chapter_to,
                conditions,
                entity_ids: input.entity_ids,
                event_ids: input.event_ids,
                foreshadowing_ids: input.foreshadowing_ids,
            },
            &input.note,
        )
        .await
        .map_err(|_| internal())?;
        ctx.events.extend(events);
        text_outcome(
            ctx,
            "create_goal",
            format!("创建目标 #{}", dto.id),
            json!({"node": dto.id}),
        )
    }
    async fn preview(&self, _ctx: &mut ToolCtx<'_>, input: Value) -> Result<Value, ToolError> {
        let input: GoalInput = decode(input)?;
        Ok(json!({"kind":"fields","label":"create_goal","rows":[
            {"field":"title","after":input.title}, {"field":"purpose","after":input.purpose}, {"field":"level","after":input.level},
            {"field":"parent_id","after":input.parent_id}, {"field":"status","after":input.status},
            {"field":"chapter_from","after":input.chapter_from}, {"field":"chapter_to","after":input.chapter_to},
            {"field":"conditions","after":input.conditions}, {"field":"entity_ids","after":input.entity_ids},
            {"field":"event_ids","after":input.event_ids}, {"field":"foreshadowing_ids","after":input.foreshadowing_ids}
        ]}))
    }
}

#[derive(Debug, Deserialize)]
struct UpdateGoalInput {
    node: i32,
    note: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    status: Option<GoalStatus>,
    #[serde(default)]
    confirm_open_children: Option<bool>,
}
pub struct UpdateGoalTool;
#[async_trait::async_trait]
impl ToolHandler for UpdateGoalTool {
    fn name(&self) -> &'static str {
        "update_goal"
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
        let input: UpdateGoalInput = decode(input)?;
        let patch = GoalNodePatch {
            title: input.title,
            purpose: input.purpose,
            status: input.status,
            confirm_open_children: input.confirm_open_children,
            ..Default::default()
        };
        let (dto, events) =
            goal_service::update_inner(ctx.tx, &write_context(ctx), input.node, patch)
                .await
                .map_err(|error| match error {
                    crate::services::canon::CanonError::App(app)
                        if app.code == crate::error::ErrorCode::GoalOpenChildren =>
                    {
                        ToolError::new(ToolErrorKind::GoalOpenChildren).with("node", input.node)
                    }
                    _ => internal(),
                })?;
        ctx.events.extend(events);
        text_outcome(
            ctx,
            "update_goal",
            format!("更新目标 #{}", dto.id),
            json!({"node": dto.id, "note": input.note}),
        )
    }
    async fn preview(&self, ctx: &mut ToolCtx<'_>, input: Value) -> Result<Value, ToolError> {
        let input: UpdateGoalInput = decode(input)?;
        let book_id = book_id(ctx)?;
        let nodes = goal_service::list(
            ctx.tx,
            book_id,
            &crate::api::dto::goals::GoalQuery {
                track_id: None,
                ..Default::default()
            },
        )
        .await
        .map_err(|_| internal())?;
        let current = nodes
            .into_iter()
            .find(|node| node.id == input.node)
            .ok_or_else(internal)?;
        let mut rows = Vec::new();
        if let Some(value) = input.title {
            rows.push(json!({"field":"title","before":current.title,"after":value}));
        }
        if let Some(value) = input.purpose {
            rows.push(json!({"field":"purpose","before":current.purpose,"after":value}));
        }
        if let Some(value) = input.status {
            rows.push(json!({"field":"status","before":current.status,"after":value}));
        }
        Ok(json!({"kind":"fields","label":current.title,"rows":rows}))
    }
}

#[derive(Debug, Deserialize)]
struct CreateResearchInput {
    topic: String,
    requirements: String,
    scope: String,
}
pub struct CreateResearchTaskTool;
#[async_trait::async_trait]
impl ToolHandler for CreateResearchTaskTool {
    fn name(&self) -> &'static str {
        "create_research_task"
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
        let input: CreateResearchInput = decode(input)?;
        let (dto, events, _) = research::create_inner(
            ctx.tx,
            ResearchTaskCreate {
                book_id: Some(book_id(ctx)?),
                check_id: None,
                scope_name: input.scope,
                topic: input.topic,
                requirements: input.requirements,
                language: None,
            },
        )
        .await
        .map_err(|_| internal())?;
        ctx.events.extend(events);
        text_outcome(
            ctx,
            "create_research_task",
            format!("创建资料任务 #{}", dto.id),
            json!({"task": dto.id}),
        )
    }
    async fn preview(&self, _ctx: &mut ToolCtx<'_>, input: Value) -> Result<Value, ToolError> {
        let input: CreateResearchInput = decode(input)?;
        Ok(
            json!({"kind":"fields","label":"create_research_task","rows":[{"field":"topic","after":input.topic},{"field":"requirements","after":input.requirements},{"field":"scope","after":input.scope}]}),
        )
    }
}

#[derive(Debug, Deserialize)]
struct UpdateResearchInput {
    task: i32,
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    requirements: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    status: Option<String>,
}
pub struct UpdateResearchTaskTool;
#[async_trait::async_trait]
impl ToolHandler for UpdateResearchTaskTool {
    fn name(&self) -> &'static str {
        "update_research_task"
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
        let input: UpdateResearchInput = decode(input)?;
        let status = input
            .status
            .as_deref()
            .map(|s| match s {
                "ready" => Ok(ResearchTaskStatus::Ready),
                "deferred" => Ok(ResearchTaskStatus::Deferred),
                _ => Err(validation("status must be ready or deferred")),
            })
            .transpose()?;
        if input
            .topic
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(validation("a research task needs a topic"));
        }
        if input
            .scope
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(validation("a research task needs a scope name"));
        }
        let fields = ResearchTaskPatch {
            topic: input.topic,
            requirements: input.requirements,
            scope_name: input.scope,
            status: None,
        };
        let (dto, events) = if status == Some(ResearchTaskStatus::Deferred) {
            let (_, mut transition_events) =
                research::transition_inner(ctx.tx, input.task, Transition::Defer)
                    .await
                    .map_err(|_| internal())?;
            let (dto, mut patch_events) = research::patch_inner(ctx.tx, input.task, fields)
                .await
                .map_err(|_| internal())?;
            transition_events.append(&mut patch_events);
            Ok((dto, transition_events))
        } else if status == Some(ResearchTaskStatus::Ready) {
            let current = research::get(ctx.tx, input.task)
                .await
                .map_err(|_| internal())?;
            if current.status == ResearchTaskStatus::Deferred {
                let (_, mut transition_events) =
                    research::transition_inner(ctx.tx, input.task, Transition::Resume)
                        .await
                        .map_err(|_| internal())?;
                let (dto, mut patch_events) = research::patch_inner(ctx.tx, input.task, fields)
                    .await
                    .map_err(|_| internal())?;
                transition_events.append(&mut patch_events);
                Ok((dto, transition_events))
            } else {
                research::patch_inner(
                    ctx.tx,
                    input.task,
                    ResearchTaskPatch {
                        status: Some(ResearchTaskStatus::Ready),
                        ..fields
                    },
                )
                .await
                .map_err(|_| internal())
            }
        } else {
            research::patch_inner(ctx.tx, input.task, fields)
                .await
                .map_err(|_| internal())
        }?;
        ctx.events.extend(events);
        text_outcome(
            ctx,
            "update_research_task",
            format!("更新资料任务 #{}", dto.id),
            json!({"task": dto.id}),
        )
    }
    async fn preview(&self, ctx: &mut ToolCtx<'_>, input: Value) -> Result<Value, ToolError> {
        let input: UpdateResearchInput = decode(input)?;
        let current = research::get(ctx.tx, input.task)
            .await
            .map_err(|_| internal())?;
        let mut rows = Vec::new();
        if let Some(value) = input.topic {
            rows.push(json!({"field":"topic","before":current.topic,"after":value}));
        }
        if let Some(value) = input.requirements {
            rows.push(json!({"field":"requirements","before":current.requirements,"after":value}));
        }
        if let Some(value) = input.scope {
            rows.push(json!({"field":"scope","before":current.scope_name,"after":value}));
        }
        if let Some(value) = input.status {
            rows.push(json!({"field":"status","before":current.status,"after":value}));
        }
        Ok(json!({"kind":"fields","label":current.topic,"rows":rows}))
    }
}

#[derive(Debug, Deserialize)]
struct DeleteResearchInput {
    task: i32,
}
pub struct DeleteResearchTaskTool;
#[async_trait::async_trait]
impl ToolHandler for DeleteResearchTaskTool {
    fn name(&self) -> &'static str {
        "delete_research_task"
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
        let input: DeleteResearchInput = decode(input)?;
        let events = research::delete_inner(ctx.tx, input.task)
            .await
            .map_err(|_| internal())?;
        ctx.events.extend(events);
        text_outcome(
            ctx,
            "delete_research_task",
            format!("删除资料任务 #{}", input.task),
            json!({}),
        )
    }
    async fn preview(&self, ctx: &mut ToolCtx<'_>, input: Value) -> Result<Value, ToolError> {
        let input: DeleteResearchInput = decode(input)?;
        let current = research::get(ctx.tx, input.task)
            .await
            .map_err(|_| internal())?;
        Ok(
            json!({"kind":"action","label":"delete_research_task","text":format!("删除资料任务 #{}《{}》（{}）", input.task, current.topic, current.status)}),
        )
    }
}

#[derive(Debug, Deserialize)]
struct AddKnowledgeInput {
    scope: String,
    title: String,
    content: String,
    #[serde(default)]
    source_urls: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
}
pub struct AddKnowledgeEntryTool;
#[async_trait::async_trait]
impl ToolHandler for AddKnowledgeEntryTool {
    fn name(&self) -> &'static str {
        "add_knowledge_entry"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Knowledge
    }
    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: AddKnowledgeInput = decode(input)?;
        let book_id = book_id(ctx)?;
        let scopes = knowledge::book_scopes(ctx.tx, book_id)
            .await
            .map_err(|_| internal())?;
        let scope = scopes
            .into_iter()
            .find(|scope| scope.name == input.scope)
            .ok_or_else(|| {
                ToolError::new(ToolErrorKind::ScopeNotFound)
                    .with("name", input.scope.clone())
                    .with("available", Vec::<String>::new())
            })?;
        let (dto, events) = knowledge::create_entry_inner(
            ctx.tx,
            scope.id,
            EntryInput {
                title: input.title,
                content: input.content,
                source_urls: input.source_urls,
                tags: input.tags,
            },
        )
        .await
        .map_err(|_| internal())?;
        ctx.events.extend(events);
        text_outcome(
            ctx,
            "add_knowledge_entry",
            format!("添加知识条目《{}》", dto.title),
            json!({"entry": dto.id, "scope": scope.name, "shared": true}),
        )
    }
    async fn preview(&self, _ctx: &mut ToolCtx<'_>, input: Value) -> Result<Value, ToolError> {
        let input: AddKnowledgeInput = decode(input)?;
        Ok(
            json!({"kind":"fields","label":"add_knowledge_entry","shared":true,"rows":[
                {"field":"scope","after":input.scope}, {"field":"title","after":input.title}, {"field":"content","after":input.content},
                {"field":"source_urls","after":input.source_urls}, {"field":"tags","after":input.tags}
            ]}),
        )
    }
}

#[derive(Debug, Deserialize)]
struct StartSessionInput {
    action: String,
    #[serde(default)]
    chapter: Option<String>,
    #[serde(default)]
    task: Option<i32>,
    #[serde(default)]
    guidance: Option<String>,
}

pub struct StartSessionTool;

#[async_trait::async_trait]
impl ToolHandler for StartSessionTool {
    fn name(&self) -> &'static str {
        "start_session"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Control
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: StartSessionInput = decode(input)?;
        let book_id = book_id(ctx)?;
        let chapter = input
            .chapter
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        if input.chapter.is_some()
            && matches!(input.action.as_str(), "research" | "knowledge_check")
        {
            return Err(validation("chapter is not valid for this action"));
        }
        if input.task.is_some() && input.action != "research" {
            return Err(validation("task is only valid for research"));
        }
        if input.guidance.is_some()
            && !matches!(input.action.as_str(), "outline" | "knowledge_check")
        {
            return Err(validation(
                "guidance is only valid for outline and knowledge_check",
            ));
        }
        match input.action.as_str() {
            "outline" | "maintain" | "resync" => {
                let chapter =
                    chapter.ok_or_else(|| validation("chapter is required for this action"))?;
                let chapter = resolve_chapter(ctx, book_id, chapter).await?;
                match input.action.as_str() {
                    "outline" => start_outline(ctx, &chapter, input.guidance).await,
                    "maintain" => start_maintain(ctx, &chapter).await,
                    "resync" => start_resync(ctx, &chapter).await,
                    _ => unreachable!(),
                }
            }
            "research" => {
                let task = input
                    .task
                    .ok_or_else(|| validation("task is required for research"))?;
                start_research(ctx, task).await
            }
            "knowledge_check" => {
                let focus = input.guidance.unwrap_or_default().trim().to_owned();
                if focus.is_empty() {
                    return Err(validation("guidance is required for knowledge_check"));
                }
                ensure_model_available(ctx, AgentRole::Researcher).await?;
                let (task, session_id, warnings, events) =
                    research::start_knowledge_check(ctx.state, ctx.tx, book_id, focus)
                        .await
                        .map_err(map_start_error)?;
                ctx.events.extend(events);
                session_start_outcome(
                    ctx,
                    session_id,
                    SessionKind::Research,
                    false,
                    warnings,
                    Some(task.id),
                )
            }
            other => Err(validation(&format!("unsupported action {other}"))),
        }
    }

    async fn preview(&self, _ctx: &mut ToolCtx<'_>, input: Value) -> Result<Value, ToolError> {
        let input: StartSessionInput = decode(input)?;
        let target = input
            .chapter
            .as_deref()
            .or(if input.task.is_some() {
                Some("task")
            } else {
                None
            })
            .unwrap_or("");
        Ok(
            json!({"kind":"action","label":"start_session","text":format!("启动 {} {}", input.action, target)}),
        )
    }
}

async fn ensure_model_available(ctx: &ToolCtx<'_>, role: AgentRole) -> Result<(), ToolError> {
    let Some(profile_id) = crate::services::sessions::try_resolve_model(ctx.tx, ctx.book, role)
        .await
        .map_err(|_| internal())?
    else {
        return Err(map_start_error(
            crate::error::AppError::new(
                crate::error::ErrorCode::ModelMissingRole,
                "no model configured for this role",
            )
            .with_details(json!({"role": role.as_str()})),
        ));
    };
    let profile = model_profiles::Entity::find_by_id(profile_id)
        .one(ctx.tx)
        .await
        .map_err(|_| internal())?;
    if profile.is_none()
        || (profile.is_some()
            && providers::Entity::find_by_id(profile.unwrap().provider_id)
                .one(ctx.tx)
                .await
                .map_err(|_| internal())?
                .is_none())
    {
        return Err(map_start_error(
            crate::error::AppError::new(
                crate::error::ErrorCode::ModelMissingRole,
                "the model profile or provider is missing",
            )
            .with_details(json!({"role": role.as_str()})),
        ));
    }
    Ok(())
}

fn map_start_error(error: crate::error::AppError) -> ToolError {
    if error.code == crate::error::ErrorCode::ChapterSessionExists {
        return ToolError::new(ToolErrorKind::SessionConflict).with(
            "session_id",
            error
                .details
                .as_ref()
                .and_then(|d| d.get("session_id"))
                .and_then(Value::as_i64)
                .unwrap_or(0),
        );
    }
    ToolError::new(ToolErrorKind::StartRefused)
        .with("reason", error.code.as_str())
        .with("details", error.details.unwrap_or(Value::Null))
}
fn map_conflict(session_id: i32) -> ToolError {
    ToolError::new(ToolErrorKind::SessionConflict).with("session_id", session_id)
}
fn session_start_outcome(
    ctx: &mut ToolCtx<'_>,
    session_id: i32,
    kind: SessionKind,
    existing: bool,
    warnings: Vec<String>,
    task_id: Option<i32>,
) -> Result<ToolOutcome, ToolError> {
    let rendered = ctx
        .prompts
        .render_value(
            "result.session_start",
            crate::prompts::template_value(&json!({
                "session_id": session_id, "kind": kind, "existing": existing, "warnings": warnings,
            })),
        )
        .map_err(|_| internal())?;
    let mut outcome = ToolOutcome::text(rendered.text).with_ui(json!({
        "session_id": session_id, "kind": kind, "existing": existing, "task_id": task_id,
    }));
    if !existing {
        outcome.spawn.push(session_id);
    }
    Ok(outcome)
}

async fn start_outline(
    ctx: &mut ToolCtx<'_>,
    chapter: &chapters::Model,
    guidance: Option<String>,
) -> Result<ToolOutcome, ToolError> {
    if chapter.status == ChapterStatus::Official {
        return Err(map_start_error(crate::error::AppError::new(
            crate::error::ErrorCode::ChapterInvalidState,
            "official chapter",
        )));
    }
    if let Some(id) = crate::services::open_planner_session(ctx.tx, chapter.id)
        .await
        .map_err(|_| internal())?
    {
        return Err(map_conflict(id));
    }
    if let Some(id) = crate::services::open_writer_session(ctx.tx, chapter.id)
        .await
        .map_err(|_| internal())?
    {
        return Err(map_conflict(id));
    }
    if let Some(details) = chapter_service::previous_chapter_blocker(ctx.tx, chapter)
        .await
        .map_err(|_| internal())?
    {
        return Err(ToolError::new(ToolErrorKind::StartRefused)
            .with("reason", "chapter.previous_not_ready")
            .with("details", details));
    }
    ensure_model_available(ctx, AgentRole::Planner).await?;
    let (session_id, events, warnings) = crate::services::sessions::create_inner(
        ctx.state,
        ctx.tx,
        crate::services::sessions::CreateSession {
            kind: SessionKind::ChapterOutline,
            book_id: Some(chapter.book_id),
            chapter_id: Some(chapter.id),
            guidance,
            chapter_guard: true,
            force_previous: false,
            ..Default::default()
        },
    )
    .await
    .map_err(map_start_error)?;
    ctx.events.extend(events);
    session_start_outcome(
        ctx,
        session_id,
        SessionKind::ChapterOutline,
        false,
        warnings,
        None,
    )
}

async fn start_maintain(
    ctx: &mut ToolCtx<'_>,
    chapter: &chapters::Model,
) -> Result<ToolOutcome, ToolError> {
    ensure_model_available(ctx, AgentRole::Archivist).await?;
    if chapter.status != ChapterStatus::Official {
        return Err(map_start_error(crate::error::AppError::new(
            crate::error::ErrorCode::ChapterInvalidState,
            "not official",
        )));
    }
    if !matches!(
        chapter.canon_status,
        CanonStatus::Pending | CanonStatus::Failed
    ) {
        return Err(map_start_error(crate::error::AppError::new(
            crate::error::ErrorCode::ChapterInvalidState,
            "canon not maintainable",
        )));
    }
    if let Some(open) = open_archivist_session(ctx.tx, chapter.id)
        .await
        .map_err(|_| internal())?
    {
        return Err(map_conflict(open.id));
    }
    let kind = last_archivist_kind(ctx.tx, chapter.id)
        .await
        .map_err(|_| internal())?
        .unwrap_or(SessionKind::ChapterMaintenance);
    let (session_id, events, warnings) = crate::services::sessions::create_inner(
        ctx.state,
        ctx.tx,
        crate::services::sessions::CreateSession {
            kind,
            book_id: Some(chapter.book_id),
            chapter_id: Some(chapter.id),
            ..Default::default()
        },
    )
    .await
    .map_err(map_start_error)?;
    ctx.events.extend(events);
    session_start_outcome(ctx, session_id, kind, false, warnings, None)
}

async fn start_resync(
    ctx: &mut ToolCtx<'_>,
    chapter: &chapters::Model,
) -> Result<ToolOutcome, ToolError> {
    ensure_model_available(ctx, AgentRole::Archivist).await?;
    if chapter.status != ChapterStatus::Official || chapter.canon_status != CanonStatus::Stale {
        return Err(map_start_error(crate::error::AppError::new(
            crate::error::ErrorCode::ChapterInvalidState,
            "chapter is not stale official",
        )));
    }
    if let Some(open) = open_archivist_session(ctx.tx, chapter.id)
        .await
        .map_err(|_| internal())?
    {
        return Err(map_conflict(open.id));
    }
    let content: ChapterContent = chapter.content.clone().into_inner();
    let existing = crate::db::entities::chapter_versions::Entity::find()
        .filter(
            crate::db::entities::chapter_versions::COLUMN
                .chapter_id
                .eq(chapter.id),
        )
        .filter(
            crate::db::entities::chapter_versions::COLUMN
                .kind
                .eq(crate::db::json::VersionKind::Official),
        )
        .all(ctx.tx)
        .await
        .map_err(|_| internal())?
        .into_iter()
        .find(|version| version.content.clone().into_inner() == content);
    let version_id = if let Some(version) = existing {
        version.id
    } else {
        let word_count = crate::text::content_word_count(&content);
        versions::create_version(
            ctx.tx,
            versions::NewVersion {
                chapter_id: chapter.id,
                kind: crate::db::json::VersionKind::Official,
                title: &chapter.title,
                content: &content,
                word_count,
                note: "",
                session_id: None,
            },
        )
        .await
        .map_err(|_| internal())?
        .id
    };
    chapters::ActiveModel {
        id: sea_orm::Set(chapter.id),
        official_version_id: sea_orm::Set(Some(version_id)),
        updated_at: sea_orm::Set(crate::services::now()),
        ..Default::default()
    }
    .update(ctx.tx)
    .await
    .map_err(|_| internal())?;
    let (session_id, events, warnings) = crate::services::sessions::create_inner(
        ctx.state,
        ctx.tx,
        crate::services::sessions::CreateSession {
            kind: SessionKind::ChapterResync,
            book_id: Some(chapter.book_id),
            chapter_id: Some(chapter.id),
            ..Default::default()
        },
    )
    .await
    .map_err(map_start_error)?;
    ctx.events.extend(events);
    session_start_outcome(
        ctx,
        session_id,
        SessionKind::ChapterResync,
        false,
        warnings,
        None,
    )
}

async fn start_research(ctx: &mut ToolCtx<'_>, task_id: i32) -> Result<ToolOutcome, ToolError> {
    ensure_model_available(ctx, AgentRole::Researcher).await?;
    let task = research::get(ctx.tx, task_id)
        .await
        .map_err(|_| internal())?;
    if task.book_id != Some(book_id(ctx)?) {
        return Err(ToolError::new(ToolErrorKind::NotFound)
            .with("what_label", "research task".to_owned())
            .with("id", task_id));
    }
    if task.status == ResearchTaskStatus::Running {
        return Err(map_conflict(task.session_id.unwrap_or(0)));
    }
    if task.status == ResearchTaskStatus::Failed {
        if let Some(session_id) = task.session_id {
            agent_sessions::ActiveModel {
                id: sea_orm::Set(session_id),
                status: sea_orm::Set(SessionStatus::Running),
                error: sea_orm::Set(None),
                ..Default::default()
            }
            .update(ctx.tx)
            .await
            .map_err(|_| internal())?;
            research_tasks::ActiveModel {
                id: sea_orm::Set(task_id),
                status: sea_orm::Set(ResearchTaskStatus::Running),
                ..Default::default()
            }
            .update(ctx.tx)
            .await
            .map_err(|_| internal())?;
            let mut outcome = session_start_outcome(
                ctx,
                session_id,
                SessionKind::Research,
                true,
                Vec::new(),
                Some(task_id),
            )?;
            outcome.spawn.push(session_id);
            return Ok(outcome);
        }
        return Err(map_start_error(crate::error::AppError::new(
            crate::error::ErrorCode::ResearchInvalidState,
            "failed task has no session",
        )));
    }
    if matches!(
        task.status,
        ResearchTaskStatus::Draft
            | ResearchTaskStatus::Ready
            | ResearchTaskStatus::Deferred
            | ResearchTaskStatus::Done
            | ResearchTaskStatus::Cancelled
    ) {
        let scope_id = research::prepare_scope(ctx.tx, task_id)
            .await
            .map_err(map_start_error)?;
        let (session_id, events, warnings) = crate::services::sessions::create_inner(
            ctx.state,
            ctx.tx,
            crate::services::sessions::CreateSession {
                kind: SessionKind::Research,
                book_id: task.book_id,
                research_task_id: Some(task_id),
                kickoff_extra: scope_id.map(|id| json!({"scope_id": id})),
                ..Default::default()
            },
        )
        .await
        .map_err(map_start_error)?;
        ctx.events.extend(events);
        return session_start_outcome(
            ctx,
            session_id,
            SessionKind::Research,
            false,
            warnings,
            Some(task_id),
        );
    }
    Err(map_start_error(crate::error::AppError::new(
        crate::error::ErrorCode::ResearchInvalidState,
        "research task cannot run",
    )))
}

pub fn all() -> Vec<std::sync::Arc<dyn ToolHandler>> {
    vec![
        std::sync::Arc::new(CreateChapterTool),
        std::sync::Arc::new(EditChapterTool),
        std::sync::Arc::new(EditChapterTextTool),
        std::sync::Arc::new(DeleteCanonRecordTool),
        std::sync::Arc::new(CreateGoalTool),
        std::sync::Arc::new(UpdateGoalTool),
        std::sync::Arc::new(CreateResearchTaskTool),
        std::sync::Arc::new(UpdateResearchTaskTool),
        std::sync::Arc::new(DeleteResearchTaskTool),
        std::sync::Arc::new(AddKnowledgeEntryTool),
        std::sync::Arc::new(StartSessionTool),
    ]
}
