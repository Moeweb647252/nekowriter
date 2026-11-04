//! Book-agent read tools (plan §3.3, §5.3; D-169).
//!
//! These are read-only tools. The book agent has no track, so `Visibility::of` is `Shared`:
//! main-story canon is visible and an isolated side track's private rows are not. Chapter reads,
//! by contrast, deliberately reach every track.

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::db::entities::{
    agent_sessions, chapters, character_designs, research_tasks, setting_docs, tracks,
};
use crate::db::json::{GoalLevel, GoalStatus, ResearchTaskStatus, SettingDocKind};
use crate::services::canon;

use super::{Prepared, ToolCtx, ToolError, ToolErrorKind, ToolHandler, ToolKind, ToolOutcome};

const SETTING_DOC_CHARS: usize = 12_000;

fn book_id(ctx: &ToolCtx<'_>) -> Result<i32, ToolError> {
    ctx.book
        .map(|book| book.id)
        .ok_or_else(|| ToolError::new(ToolErrorKind::Internal))
}

fn internal() -> ToolError {
    ToolError::new(ToolErrorKind::Internal)
}

fn validation(message: &str) -> ToolError {
    ToolError::new(ToolErrorKind::Validation).with("errors", vec![message.to_owned()])
}

pub struct BookOverviewTool;

#[async_trait::async_trait]
impl ToolHandler for BookOverviewTool {
    fn name(&self) -> &'static str {
        "book_overview"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Retrieval
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        _input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let index = crate::agent::context::book_index_context(ctx.tx, ctx.session)
            .await
            .map_err(|_| internal())?;
        let rendered = ctx
            .prompts
            .render_value(
                "block.book_index",
                crate::prompts::template_value(&json!({ "index": index })),
            )
            .map_err(|_| internal())?;
        Ok(ToolOutcome::text(rendered.text).with_ui(json!({ "index": index })))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenChapterInput {
    chapter: String,
    #[serde(default)]
    part: Option<String>,
    #[serde(default)]
    from_pid: Option<String>,
    #[serde(default)]
    max_chars: Option<i64>,
}

pub struct OpenChapterTool;

#[async_trait::async_trait]
impl ToolHandler for OpenChapterTool {
    fn name(&self) -> &'static str {
        "open_chapter"
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
        let input: OpenChapterInput =
            serde_json::from_value(input).map_err(|error| validation(&error.to_string()))?;
        let book_id = book_id(ctx)?;
        let chapter = resolve_chapter(ctx, book_id, &input.chapter).await?;
        let part = input.part.as_deref().unwrap_or("text");
        let max_chars = input.max_chars.unwrap_or(8_000).clamp(1, 20_000) as usize;
        if !matches!(
            part,
            "text" | "outline" | "reference_notes" | "guidance" | "summary"
        ) {
            return Err(validation(
                "part must be text, outline, reference_notes, guidance or summary",
            ));
        }
        let content = chapter.content.clone().into_inner();
        let mut paragraphs = Vec::new();
        if part == "text" {
            let start = input
                .from_pid
                .as_deref()
                .and_then(|pid| content.paragraphs.iter().position(|p| p.pid == pid));
            if input.from_pid.is_some() && start.is_none() {
                return Err(ToolError::new(ToolErrorKind::PidNotFound)
                    .with("pids", vec![input.from_pid.unwrap_or_default()]));
            }
            paragraphs = content
                .paragraphs
                .iter()
                .skip(start.unwrap_or(0))
                .cloned()
                .collect();
        }
        let mut text = String::new();
        let value = match part {
            "outline" => chapter.outline.clone(),
            "reference_notes" => chapter.reference_notes.clone(),
            "guidance" => chapter.guidance.clone(),
            "summary" => chapter.summary.clone(),
            _ => {
                for paragraph in &paragraphs {
                    text.push_str(&format!("[{}] {}\n", paragraph.pid, paragraph.text));
                }
                text
            }
        };
        let cut = value
            .char_indices()
            .nth(max_chars)
            .map(|(index, _)| index)
            .unwrap_or(value.len());
        let body = &value[..cut];
        let next_pid = if part == "text" && cut < value.len() {
            // The cut may land inside a paragraph. Name that partially displayed paragraph so the
            // continuation can re-read it in full; only a cut exactly on a paragraph boundary
            // advances to the next pid.
            let mut offset = 0;
            let mut partial = None;
            for paragraph in &paragraphs {
                let line = format!("[{}] {}\n", paragraph.pid, paragraph.text);
                if cut < offset + line.len() {
                    partial = Some(paragraph.pid.clone());
                    break;
                }
                offset += line.len();
            }
            partial
        } else {
            None
        };
        let open_sessions = agent_sessions::Entity::find()
            .filter(agent_sessions::COLUMN.chapter_id.eq(chapter.id))
            .filter(
                agent_sessions::COLUMN
                    .status
                    .is_in(crate::services::sessions::open_statuses()),
            )
            .all(ctx.tx)
            .await
            .map_err(|_| internal())?
            .into_iter()
            .map(|row| json!({"id": row.id, "kind": row.kind, "status": row.status}))
            .collect::<Vec<_>>();
        let label = canon::chapter_label(ctx.tx, ctx.prompts, Some(chapter.id))
            .await
            .map_err(|_| internal())?;
        let rendered = ctx
            .prompts
            .render_value(
                "result.chapter_view",
                crate::prompts::template_value(&json!({
                    "label": label,
                    "part": part,
                    "body": body,
                    "next_pid": next_pid,
                    "status": chapter.status,
                    "content_version": chapter.content_version,
                    "canon_status": chapter.canon_status,
                    "words": chapter.word_count,
                    "open_sessions": open_sessions,
                })),
            )
            .map_err(|_| internal())?;
        Ok(ToolOutcome::text(rendered.text).with_ui(json!({
            "chapter_id": chapter.id, "part": part, "status": chapter.status,
            "content_version": chapter.content_version, "canon_status": chapter.canon_status,
            "words": chapter.word_count, "open_sessions": open_sessions, "next_pid": next_pid,
        })))
    }
}

async fn resolve_chapter(
    ctx: &ToolCtx<'_>,
    book_id: i32,
    raw: &str,
) -> Result<chapters::Model, ToolError> {
    let chapter = if let Some(id) = raw
        .strip_prefix('#')
        .and_then(|value| value.parse::<i32>().ok())
    {
        chapters::Entity::find_by_id(id)
            .one(ctx.tx)
            .await
            .map_err(|_| internal())?
    } else {
        let number = raw
            .parse::<i32>()
            .map_err(|_| validation("chapter must be a number or #id"))?;
        let track = tracks::Entity::find()
            .filter(tracks::COLUMN.book_id.eq(book_id))
            .filter(tracks::COLUMN.kind.eq(crate::db::json::TrackKind::Main))
            .one(ctx.tx)
            .await
            .map_err(|_| internal())?;
        match track {
            Some(track) => chapters::Entity::find()
                .filter(chapters::COLUMN.book_id.eq(book_id))
                .filter(chapters::COLUMN.track_id.eq(track.id))
                .filter(chapters::COLUMN.ord.eq(number))
                .one(ctx.tx)
                .await
                .map_err(|_| internal())?,
            None => None,
        }
    };
    chapter
        .filter(|chapter| chapter.book_id == book_id)
        .ok_or_else(|| {
            ToolError::new(ToolErrorKind::ChapterNotFound).with("chapter", raw.to_owned())
        })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadSettingDocInput {
    doc: SettingDocKind,
    #[serde(default)]
    offset: Option<i64>,
}

pub struct ReadSettingDocTool;

#[async_trait::async_trait]
impl ToolHandler for ReadSettingDocTool {
    fn name(&self) -> &'static str {
        "read_setting_doc"
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
        let input: ReadSettingDocInput =
            serde_json::from_value(input).map_err(|error| validation(&error.to_string()))?;
        let book_id = book_id(ctx)?;
        let doc = setting_docs::Entity::find()
            .filter(setting_docs::COLUMN.book_id.eq(book_id))
            .filter(setting_docs::COLUMN.kind.eq(input.doc))
            .one(ctx.tx)
            .await
            .map_err(|_| internal())?
            .ok_or_else(|| {
                ToolError::new(ToolErrorKind::NotFound)
                    .with("what_label", "setting document".to_owned())
            })?;
        let offset = input.offset.unwrap_or(0).max(0) as usize;
        let total = doc.content.chars().count();
        let start = offset.min(total);
        let end = (start + SETTING_DOC_CHARS).min(total);
        let content: String = doc.content.chars().skip(start).take(end - start).collect();
        let next_offset = (end < total).then_some(end as i64);
        let label = ctx.prompts.label("setting_doc", input.doc.as_str());
        let rendered = ctx
            .prompts
            .render_value(
                "result.setting_doc_view",
                crate::prompts::template_value(&json!({
                    "label": label, "content": content, "offset": start, "end": end, "total": total,
                    "next_offset": next_offset, "revision": doc.revision,
                })),
            )
            .map_err(|_| internal())?;
        Ok(ToolOutcome::text(rendered.text).with_ui(json!({"doc": input.doc, "offset": start, "end": end, "total": total, "next_offset": next_offset, "revision": doc.revision})))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadCharactersInput {
    #[serde(default)]
    name: Option<String>,
}

pub struct ReadCharactersTool;

#[async_trait::async_trait]
impl ToolHandler for ReadCharactersTool {
    fn name(&self) -> &'static str {
        "read_characters"
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
        let input: ReadCharactersInput =
            serde_json::from_value(input).map_err(|error| validation(&error.to_string()))?;
        let book_id = book_id(ctx)?;
        let mut query = character_designs::Entity::find()
            .filter(character_designs::COLUMN.book_id.eq(book_id))
            .order_by_asc(character_designs::COLUMN.rank);
        if let Some(name) = input.name.as_deref() {
            query = query.filter(character_designs::COLUMN.name.eq(name));
        }
        let designs = query.all(ctx.tx).await.map_err(|_| internal())?;
        if let Some(name) = input
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            && designs.is_empty()
        {
            return Err(
                ToolError::new(ToolErrorKind::CharacterNotFound).with("name", name.to_owned())
            );
        }
        let rendered = ctx.prompts.render_value("result.characters_view", crate::prompts::template_value(&json!({
            "designs": designs.iter().map(|design| json!({"rank": design.rank, "name": design.name, "role_label": design.role_label, "summary": design.summary, "profile": design.profile, "entity_id": design.entity_id})).collect::<Vec<_>>(),
        }))).map_err(|_| internal())?;
        Ok(ToolOutcome::text(rendered.text).with_ui(json!({"count": designs.len()})))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListGoalsInput {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    level: Option<GoalLevel>,
}

pub struct ListGoalsTool;

#[async_trait::async_trait]
impl ToolHandler for ListGoalsTool {
    fn name(&self) -> &'static str {
        "list_goals"
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
        let input: ListGoalsInput =
            serde_json::from_value(input).map_err(|error| validation(&error.to_string()))?;
        let book_id = book_id(ctx)?;
        // Shared scope only (plan row 11); `open` is the default and means active + waiting.
        let mut nodes = crate::services::goals::list(
            ctx.tx,
            book_id,
            &crate::api::dto::goals::GoalQuery {
                track_id: None,
                ..Default::default()
            },
        )
        .await
        .map_err(|_| internal())?;
        let status = input.status.as_deref().unwrap_or("open");
        match status {
            "all" => {}
            "open" => {
                nodes.retain(|node| matches!(node.status, GoalStatus::Active | GoalStatus::Waiting))
            }
            other => {
                let wanted = GoalStatus::from_str_opt(other)
                    .ok_or_else(|| validation("unknown goal status"))?;
                nodes.retain(|node| node.status == wanted);
            }
        }
        if let Some(level) = input.level {
            nodes.retain(|node| node.level == level);
        }
        let rendered = ctx.prompts.render_value("result.goal_list", crate::prompts::template_value(&json!({
            "nodes": nodes.iter().map(|node| json!({"id": node.id, "level": node.level, "title": node.title, "purpose": node.purpose, "status": node.status, "chapter_from": node.chapter_from, "chapter_to": node.chapter_to, "ready": node.ready, "closable": node.closable})).collect::<Vec<_>>(),
        }))).map_err(|_| internal())?;
        Ok(ToolOutcome::text(rendered.text).with_ui(json!({"count": nodes.len()})))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListResearchInput {
    #[serde(default)]
    status: Option<ResearchTaskStatus>,
}

pub struct ListResearchTool;

#[async_trait::async_trait]
impl ToolHandler for ListResearchTool {
    fn name(&self) -> &'static str {
        "list_research"
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
        let input: ListResearchInput =
            serde_json::from_value(input).map_err(|error| validation(&error.to_string()))?;
        let book_id = book_id(ctx)?;
        let mut query = research_tasks::Entity::find()
            .filter(research_tasks::COLUMN.book_id.eq(book_id))
            .order_by_desc(research_tasks::COLUMN.id);
        if let Some(status) = input.status {
            query = query.filter(research_tasks::COLUMN.status.eq(status.as_str()));
        }
        let tasks = query.all(ctx.tx).await.map_err(|_| internal())?;
        let rendered = ctx.prompts.render_value("result.research_list", crate::prompts::template_value(&json!({
            "tasks": tasks.iter().map(|task| json!({"id": task.id, "kind": task.kind, "topic": task.topic, "scope": task.scope_name, "status": task.status, "entries": task.entries_created})).collect::<Vec<_>>(),
        }))).map_err(|_| internal())?;
        Ok(ToolOutcome::text(rendered.text).with_ui(json!({"count": tasks.len()})))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListSessionsInput {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    kind: Option<crate::db::json::SessionKind>,
}

pub struct ListSessionsTool;

#[async_trait::async_trait]
impl ToolHandler for ListSessionsTool {
    fn name(&self) -> &'static str {
        "list_sessions"
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
        let input: ListSessionsInput =
            serde_json::from_value(input).map_err(|error| validation(&error.to_string()))?;
        let book_id = book_id(ctx)?;
        let mut query = agent_sessions::Entity::find()
            .filter(agent_sessions::COLUMN.book_id.eq(book_id))
            .order_by_desc(agent_sessions::COLUMN.id);
        let status = input.status.as_deref().unwrap_or("open");
        if status == "open" {
            query = query.filter(
                agent_sessions::COLUMN
                    .status
                    .is_in(crate::services::sessions::open_statuses()),
            );
        } else if status != "all" {
            query = query.filter(agent_sessions::COLUMN.status.eq(status));
        }
        if let Some(kind) = input.kind {
            query = query.filter(agent_sessions::COLUMN.kind.eq(kind.as_str()));
        }
        let rows = query.all(ctx.tx).await.map_err(|_| internal())?;
        let mut sessions = Vec::with_capacity(rows.len());
        for row in rows {
            let chapter_label = match row.chapter_id {
                Some(id) => Some(
                    canon::chapter_label(ctx.tx, ctx.prompts, Some(id))
                        .await
                        .map_err(|_| internal())?,
                ),
                None => None,
            };
            sessions.push(json!({"id": row.id, "kind": row.kind, "chapter_id": row.chapter_id, "chapter_label": chapter_label, "status": row.status, "created_at": row.created_at}));
        }
        let rendered = ctx
            .prompts
            .render_value(
                "result.session_list",
                crate::prompts::template_value(&json!({
                    "sessions": sessions,
                })),
            )
            .map_err(|_| internal())?;
        Ok(ToolOutcome::text(rendered.text).with_ui(json!({"count": sessions.len()})))
    }
}

pub fn all() -> Vec<std::sync::Arc<dyn ToolHandler>> {
    vec![
        std::sync::Arc::new(BookOverviewTool),
        std::sync::Arc::new(OpenChapterTool),
        std::sync::Arc::new(ReadSettingDocTool),
        std::sync::Arc::new(ReadCharactersTool),
        std::sync::Arc::new(ListGoalsTool),
        std::sync::Arc::new(ListResearchTool),
        std::sync::Arc::new(ListSessionsTool),
    ]
}
