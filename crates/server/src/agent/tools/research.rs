//! The researcher's write tools (spec/05 §8, spec/07 §8): `save_knowledge_entry`,
//! `update_knowledge_entry` and `finish_research`.
//!
//! All three are restricted to the session's **target** scope: a researcher writes into the scope
//! of its own research task and nowhere else (spec/05 §1.4). They run in `execute`, inside the
//! transaction that seals the call, so an entry and its chunks commit with the tool result.

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::dto::knowledge::{EntryInput, EntryPatch};
use crate::db::entities::{knowledge_entries, knowledge_scopes, research_tasks};
use crate::db::json::CompletionKind;
use crate::services::knowledge;

use crate::db::json::ToolEffects;

use super::{
    PrepareCtx, Prepared, ToolCtx, ToolError, ToolErrorKind, ToolHandler, ToolKind, ToolOutcome,
};

/// The budget a researcher session is told about (spec/05 §8): 40 tool calls and 25 entries,
/// both soft limits that a notice reports past rather than a hard refusal.
pub const MAX_TOOL_CALLS: i32 = 40;
pub const MAX_ENTRIES: i32 = 25;

fn decode<T: for<'de> Deserialize<'de>>(input: Value) -> Result<T, ToolError> {
    serde_json::from_value(input).map_err(|err| validation(vec![err.to_string()]))
}

fn internal() -> ToolError {
    ToolError::new(ToolErrorKind::Internal)
}

/// Every tool error this module builds carries **exactly** the fields its `result.error` branch
/// reads. The renderer is strict (`UndefinedBehavior::Strict`): a branch that reaches a missing
/// variable throws, and `ToolOutcome::error` then returns an empty text — an error the model cannot
/// read. The `every_tool_error_renders` test in `agent/tools/mod.rs` guards the class; these
/// helpers keep the call sites honest.
fn not_found(prompts: &crate::prompts::PromptSet, what: &str, id: i32) -> ToolError {
    // `not_found`: "找不到{{ what_label }} #{{ id }}。"
    ToolError::new(ToolErrorKind::NotFound)
        .with("what_label", prompts.label("target_type", what))
        .with("id", id)
}

fn entry_exists(title: &str, id: i32) -> ToolError {
    // `entry_exists`: "已有同名条目《{{ title }}》(#{{ id }})".
    ToolError::new(ToolErrorKind::EntryExists)
        .with("title", title.to_owned())
        .with("id", id)
}

fn scope_not_found(name: &str, available: Vec<String>) -> ToolError {
    // `scope_not_found`: "没有名为「{{ name }}」…可以检索的范围：{{ available | join }}".
    ToolError::new(ToolErrorKind::ScopeNotFound)
        .with("name", name.to_owned())
        .with("available", available)
}

fn find_failed(prompts: &crate::prompts::PromptSet, find: &str, count: usize) -> ToolError {
    // `find_failed`: "在{{ doc_label }}中…{{ count }}…「{{ find }}」".
    ToolError::new(ToolErrorKind::FindFailed)
        .with("doc_label", prompts.label("target_type", "knowledge_entry"))
        .with("find", find.to_owned())
        .with("count", count as i64)
}

fn validation(errors: Vec<String>) -> ToolError {
    // `validation`: "{% for e in errors %}{{ e }}…" — a list, not a string.
    ToolError::new(ToolErrorKind::Validation).with("errors", errors)
}

fn mutually_exclusive(fields: Vec<&str>) -> ToolError {
    // `mutually_exclusive`: "{{ fields | join("、") }} 不能同时提供".
    ToolError::new(ToolErrorKind::MutuallyExclusive).with("fields", fields)
}

/// The target scope of the session (spec/05 §1.4).
///
/// A researcher always has one: `run` resolves the scope before the session starts. A missing
/// scope is therefore a state defect, reported with the tool error the prompts already word.
async fn target_scope(ctx: &ToolCtx<'_>, tool: &str) -> Result<(i32, String), ToolError> {
    use crate::db::json::SessionKind;
    if crate::agent::session::kind_of(ctx.session) != SessionKind::Research {
        // `notice.tool_not_allowed` reads tool, phase_label and allowed.
        let phase = ctx.session.phase;
        return Err(ToolError::tool_not_allowed(
            tool,
            crate::agent::tools::registry::phase_label(ctx.prompts, phase),
            serde_json::from_str::<Vec<serde_json::Value>>(&ctx.session.tools)
                .ok()
                .map(|tools| {
                    tools
                        .iter()
                        .filter_map(|tool| {
                            tool.get("name")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_owned)
                        })
                        .collect()
                })
                .unwrap_or_default(),
        ));
    }
    let Some(task_id) = ctx.session.research_task_id else {
        return Err(ToolError::new(ToolErrorKind::NoKnowledgeScope));
    };
    let task = research_tasks::Entity::find_by_id(task_id)
        .one(ctx.tx)
        .await
        .map_err(|_| internal())?
        .ok_or_else(|| ToolError::new(ToolErrorKind::NoKnowledgeScope))?;
    let Some(scope_id) = task.scope_id else {
        return Err(ToolError::new(ToolErrorKind::NoKnowledgeScope));
    };
    let scope = knowledge_scopes::Entity::find_by_id(scope_id)
        .one(ctx.tx)
        .await
        .map_err(|_| internal())?
        .ok_or_else(|| scope_not_found(&task.scope_name, Vec::new()))?;
    Ok((scope.id, scope.name))
}

/// Maps a knowledge-service error onto the tool error of spec/05 §8.
///
/// `title` is the title the call tried to use, so `entry_exists` can name it.
fn knowledge_error(ctx: &ToolCtx<'_>, error: crate::error::AppError, title: &str) -> ToolError {
    use crate::error::ErrorCode;
    match error.code {
        ErrorCode::ConflictExists => {
            let id = error
                .details
                .as_ref()
                .and_then(|details| details.get("id"))
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_default();
            entry_exists(title, i32::try_from(id).unwrap_or_default())
        }
        ErrorCode::NotFound => not_found(ctx.prompts, "knowledge_entry", 0),
        ErrorCode::ValidationFailed => {
            // `details.fields` is an object of field → message; the template wants a list.
            let errors = error
                .details
                .as_ref()
                .and_then(|details| details.get("fields"))
                .and_then(serde_json::Value::as_object)
                .map(|fields| {
                    fields
                        .iter()
                        .map(|(field, message)| {
                            format!("{field}: {}", message.as_str().unwrap_or_default())
                        })
                        .collect::<Vec<String>>()
                })
                .unwrap_or_else(|| vec![error.message.clone()]);
            validation(errors)
        }
        _ => internal(),
    }
}

// ───────────────────────────── save_knowledge_entry ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveEntryInput {
    title: String,
    content: String,
    #[serde(default)]
    source_urls: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
}

/// `save_knowledge_entry` (spec/05 §8, spec/07 §8 step 4).
pub struct SaveKnowledgeEntryTool;

#[async_trait::async_trait]
impl ToolHandler for SaveKnowledgeEntryTool {
    fn name(&self) -> &'static str {
        "save_knowledge_entry"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Knowledge
    }

    async fn prepare(&self, _ctx: &PrepareCtx<'_>, _input: &Value) -> Result<Prepared, ToolError> {
        // Everything this tool needs is in the transaction; there is nothing to do outside it.
        Ok(Prepared::None)
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: SaveEntryInput = decode(input)?;
        let (scope_id, scope_name) = target_scope(ctx, "save_knowledge_entry").await?;
        let title = input.title.trim().to_owned();
        if title.is_empty() {
            return Err(validation(vec!["title must not be empty".to_owned()]));
        }
        if input.content.trim().is_empty() {
            return Err(validation(vec!["content must not be empty".to_owned()]));
        }
        // `source_urls` may be empty only for general knowledge (spec/05 §8). The model states
        // that in the content, which this tool cannot judge; what it can enforce is that a citation
        // list, when given, is well formed.
        let source_urls: Vec<String> = input
            .source_urls
            .iter()
            .map(|url| url.trim().to_owned())
            .filter(|url| !url.is_empty())
            .collect();
        for url in &source_urls {
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err(validation(vec![format!("source url {url} is not http(s)")]));
            }
        }
        let (entry, events) = knowledge::create_entry_inner(
            ctx.tx,
            scope_id,
            EntryInput {
                title: title.clone(),
                content: input.content,
                source_urls,
                tags: input.tags,
            },
        )
        .await
        .map_err(|error| knowledge_error(ctx, error, &title))?;
        // `knowledge.changed` so an open Knowledge page refreshes and the scope's entry count is
        // not stale (spec/10 §3).
        ctx.events.extend(events);
        // The entry is attributed to this research task, so `entries_created` can count the task's
        // entries across every run (spec/07 §8 step 5).
        if let Some(task_id) = ctx.session.research_task_id {
            knowledge_entries::ActiveModel {
                id: Set(entry.id),
                research_task_id: Set(Some(task_id)),
                source: Set(crate::db::json::EntrySource::Research),
                ..Default::default()
            }
            .update(ctx.tx)
            .await
            .map_err(|_| internal())?;
        }
        let text = ctx
            .prompts
            .render(
                "result.knowledge_write",
                &json!({ "action": "created", "id": entry.id, "title": title, "scope": scope_name }),
            )
            .map_err(|_| internal())?
            .text;
        Ok(ToolOutcome::text(text).with_ui(json!({
            "action": "created",
            "id": entry.id,
            "title": title,
        })))
    }
}

// ───────────────────────────── update_knowledge_entry ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateEntryInput {
    entry_id: i32,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    source_urls_add: Vec<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    edits: Option<Vec<Edit>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    find: String,
    replace: String,
}

/// `update_knowledge_entry` (spec/05 §8).
pub struct UpdateKnowledgeEntryTool;

#[async_trait::async_trait]
impl ToolHandler for UpdateKnowledgeEntryTool {
    fn name(&self) -> &'static str {
        "update_knowledge_entry"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Knowledge
    }

    async fn prepare(&self, _ctx: &PrepareCtx<'_>, _input: &Value) -> Result<Prepared, ToolError> {
        Ok(Prepared::None)
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: UpdateEntryInput = decode(input)?;
        let (scope_id, scope_name) = target_scope(ctx, "update_knowledge_entry").await?;
        // The target scope only: an entry of another scope is invisible, not merely read-only.
        let existing = knowledge_entries::Entity::find_by_id(input.entry_id)
            .filter(knowledge_entries::Column::ScopeId.eq(scope_id))
            .one(ctx.tx)
            .await
            .map_err(|_| internal())?
            .ok_or_else(|| not_found(ctx.prompts, "knowledge_entry", input.entry_id))?;

        // `content` and `edits` are mutually exclusive (spec/05 §8).
        if input.content.is_some() && input.edits.is_some() {
            return Err(mutually_exclusive(vec!["content", "edits"]));
        }
        let mut content = input.content;
        if let Some(edits) = &input.edits {
            // Every `find` must occur exactly once, and all edits apply or none do (spec/05 §8).
            let mut working = existing.content.clone();
            for edit in edits {
                if edit.find.is_empty() {
                    return Err(validation(vec!["find must not be empty".to_owned()]));
                }
                let occurrences = working.matches(&edit.find).count();
                if occurrences != 1 {
                    return Err(find_failed(ctx.prompts, &edit.find, occurrences));
                }
                working = working.replacen(&edit.find, &edit.replace, 1);
            }
            content = Some(working);
        }
        let source_urls = if input.source_urls_add.is_empty() {
            None
        } else {
            let mut urls: Vec<String> =
                serde_json::from_str(&existing.source_urls).unwrap_or_default();
            for url in &input.source_urls_add {
                let url = url.trim().to_owned();
                if !url.is_empty() && !urls.contains(&url) {
                    urls.push(url);
                }
            }
            Some(urls)
        };
        let title = input.title.map(|title| title.trim().to_owned());
        // `entry_exists` names the title, so the error is built with the title this call tried to
        // use — the new one when given, otherwise the stored one.
        let error_title = title.clone().unwrap_or_else(|| existing.title.clone());
        let (entry, events) = knowledge::update_entry_inner(
            ctx.tx,
            input.entry_id,
            EntryPatch {
                title: title.clone(),
                content,
                source_urls,
                tags: input.tags,
            },
        )
        .await
        .map_err(|error| knowledge_error(ctx, error, &error_title))?;
        // `knowledge.changed` for the scope's open pages (spec/10 §3).
        ctx.events.extend(events);
        let text = ctx
            .prompts
            .render(
                "result.knowledge_write",
                &json!({
                    "action": "updated",
                    "id": entry.id,
                    "title": title.unwrap_or(entry.title),
                    "scope": scope_name,
                }),
            )
            .map_err(|_| internal())?
            .text;
        Ok(ToolOutcome::text(text).with_ui(json!({
            "action": "updated",
            "id": entry.id,
        })))
    }
}

// ───────────────────────────── finish_research ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FinishInput {
    report: String,
}

/// `finish_research` (spec/05 §8, spec/07 §8 step 5).
pub struct FinishResearchTool;

#[async_trait::async_trait]
impl ToolHandler for FinishResearchTool {
    fn name(&self) -> &'static str {
        "finish_research"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Knowledge
    }

    async fn prepare(&self, _ctx: &PrepareCtx<'_>, _input: &Value) -> Result<Prepared, ToolError> {
        Ok(Prepared::None)
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: FinishInput = decode(input)?;
        let report = input.report.trim().to_owned();
        if report.is_empty() {
            return Err(validation(vec!["report must not be empty".to_owned()]));
        }
        // The task's own scope must exist before a task can be reported done (spec/05 §1.4).
        let (scope_id, _scope_name) = target_scope(ctx, "finish_research").await?;
        // The entries **this session** saved or updated (spec/05 §8). An entry counts when the
        // task owns it and its `updated_at` is not older than this session: saves create it inside
        // the session and updates touch it, while entries left over from an earlier run are older
        // than the session and are not claimed by it. The task's stored `entries_created` still
        // accumulates across every run (spec/07 §8 step 5, `services::research::finish`).
        let entries = match ctx.session.research_task_id {
            Some(task_id) => knowledge_entries::Entity::find()
                .filter(knowledge_entries::Column::ResearchTaskId.eq(task_id))
                .filter(knowledge_entries::Column::ScopeId.eq(scope_id))
                .filter(knowledge_entries::Column::UpdatedAt.gte(ctx.session.created_at))
                .count(ctx.tx)
                .await
                .map_err(|_| internal())?,
            None => 0,
        };
        let text = ctx
            .prompts
            .render("result.finish_research", &json!({ "entries": entries }))
            .map_err(|_| internal())?
            .text;
        // `complete: research` runs the handler in the transaction that seals this call
        // (spec/03 §5.4): the task moves to `done`, its report and `entries_created` land, and the
        // session ends.
        Ok(ToolOutcome::text(text)
            .with_ui(json!({ "entries": entries }))
            .with_effects(ToolEffects {
                complete: Some(CompletionKind::Research),
                ..ToolEffects::default()
            }))
    }
}

// ───────────────────────────── registration ─────────────────────────────

/// The researcher's write tools (spec/05 §8).
pub fn all() -> Vec<std::sync::Arc<dyn ToolHandler>> {
    vec![
        std::sync::Arc::new(SaveKnowledgeEntryTool),
        std::sync::Arc::new(UpdateKnowledgeEntryTool),
        std::sync::Arc::new(FinishResearchTool),
    ]
}
