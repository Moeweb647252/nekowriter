//! Conception tools (spec/05 §7): `ask_user`, the setting-material writes, the character
//! tools, `set_research_topics` and `request_review`.
//!
//! `ask_user` and `request_review` set `await_input`, so the call records what it asked in its
//! `ui` payload and `input` at execute time, inside the call's transaction; the answer arrives
//! later through `POST /sessions/:id/answer` and is validated against what was recorded
//! (spec/05 §7, spec/03 §6) by [`validate_ask_user_answer`] and
//! [`apply_request_review_answer`].
//!
//! Every write records a `change_log` row (`actor: agent`) through the setting-material
//! services and emits `settings.changed` with the matching kind (research topics are tasks, not
//! canon, so they have no change row).

use std::collections::BTreeMap;

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::db::entities::{
    book_scope_exclusions, books, character_designs, knowledge_scopes, research_tasks,
    setting_docs::ActiveModel as SettingDocActive,
};
use crate::db::json::{
    AwaitInputKind, ConceptionState, ResearchTaskKind, ResearchTaskOrigin, ResearchTaskStatus,
    SettingDocKind,
};
use crate::error::AppError;
use crate::events::ServerEvent;
use crate::services::setting_docs as setting_docs_service;

use super::{Prepared, ToolCtx, ToolError, ToolErrorKind, ToolHandler, ToolKind, ToolOutcome};

fn decode<T: for<'de> Deserialize<'de>>(input: Value) -> Result<T, ToolError> {
    serde_json::from_value(input).map_err(|err| validation(vec![err.to_string()]))
}

fn internal() -> ToolError {
    ToolError::new(ToolErrorKind::Internal)
}

/// Every tool error this module builds carries **exactly** the fields its `result.error` branch
/// reads. The renderer is strict (`UndefinedBehavior::Strict`): a branch that reaches a missing
/// variable throws, and [`ToolOutcome::error`] then returns an empty text — an error the model
/// cannot read. `every_tool_error_renders_a_non_empty_text` in `agent/tools/mod.rs` guards the
/// class; these helpers keep the call sites honest, and `conception_tools.rs` asserts the actual
/// call sites render.
fn validation(errors: Vec<String>) -> ToolError {
    // `validation`: "{% for e in errors %}{{ e }}…" — a list, never a string.
    ToolError::new(ToolErrorKind::Validation).with("errors", errors)
}

/// `find_failed`: "在{{ doc_label }}中…出现了 {{ count }} 次…「{{ find }}」" — the label is
/// rendered by the prompt pack, never an English literal.
fn find_failed(doc_label: String, find: &str, count: usize) -> ToolError {
    ToolError::new(ToolErrorKind::FindFailed)
        .with("doc_label", doc_label)
        .with("find", find.to_owned())
        .with("count", count as i64)
}

/// `character_not_found`: "人物设定中没有「{{ name }}」。"
fn character_not_found(name: &str) -> ToolError {
    ToolError::new(ToolErrorKind::CharacterNotFound).with("name", name.to_owned())
}

/// `character_exists`: "人物设定中已有「{{ name }}」…"
fn character_exists(name: &str) -> ToolError {
    ToolError::new(ToolErrorKind::CharacterExists).with("name", name.to_owned())
}

/// `reorder_mismatch`: "排序列表必须完整包含全部人物且不能重复：{{ expected | join }}。"
fn reorder_mismatch(expected: Vec<String>) -> ToolError {
    ToolError::new(ToolErrorKind::ReorderMismatch).with("expected", expected)
}

/// The book of the session; every conception tool needs one.
fn book_id(ctx: &ToolCtx<'_>) -> Result<i32, ToolError> {
    ctx.book.map(|book| book.id).ok_or_else(internal)
}

/// Renders `result.conception_write` and returns the same fields as the `ui` payload
/// (spec/05 §1.5–§1.6). Every template variable is provided because the template branches on
/// `action` and strict-undefined rendering rejects a missing one.
fn conception_outcome(ctx: &ToolCtx<'_>, mut fields: Value) -> Result<ToolOutcome, ToolError> {
    if let Some(object) = fields.as_object_mut() {
        object.entry("kept").or_insert_with(|| json!([]));
    }
    let text = ctx
        .prompts
        .render_value(
            "result.conception_write",
            crate::prompts::template_value(&fields),
        )
        .map_err(|_| internal())?
        .text;
    Ok(ToolOutcome::text(text).with_ui(fields))
}

// ───────────────────────────── ask_user ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AskUserInput {
    questions: Vec<AskQuestion>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AskQuestion {
    id: String,
    question: String,
    #[serde(default)]
    options: Vec<AskOption>,
    #[serde(default)]
    multi_select: bool,
    #[serde(default = "default_true")]
    allow_other: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AskOption {
    label: String,
    #[serde(default)]
    description: String,
}

fn default_true() -> bool {
    true
}

/// `ask_user` (spec/05 §7): records the questions and awaits the author's answer.
///
/// The questions travel in the `ui` payload (the form is built from it); the answer is
/// validated against the same questions by [`validate_ask_user_answer`], which reads them back
/// from the stored call input.
pub struct AskUserTool;

#[async_trait::async_trait]
impl ToolHandler for AskUserTool {
    fn name(&self) -> &'static str {
        "ask_user"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Interactive
    }

    fn preview_field(&self) -> Option<&'static str> {
        Some("questions")
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: AskUserInput = decode(input)?;
        let questions: Vec<Value> = input
            .questions
            .iter()
            .map(|question| {
                json!({
                    "id": question.id,
                    "question": question.question,
                    "options": question.options.iter().map(|option| json!({
                        "label": option.label,
                        "description": option.description,
                    })).collect::<Vec<_>>(),
                    "multi_select": question.multi_select,
                    "allow_other": question.allow_other,
                })
            })
            .collect();
        let text = ctx
            .prompts
            .render_value(
                "result.ask_user",
                crate::prompts::template_value(&json!({ "answers": [] })),
            )
            .map_err(|_| internal())?
            .text;
        Ok(ToolOutcome::text(text)
            .with_ui(json!({ "questions": questions }))
            .with_await_input(AwaitInputKind::AskUser))
    }
}

/// Validates an `ask_user` answer payload against the questions the call recorded (spec/05 §7):
/// every answer id must be one of the question ids, every selected label must be one of that
/// question's options, several selections need `multi_select`, and `other` needs `allow_other`.
///
/// Returns one entry per asked question in the questions' order (an unanswered one renders as
/// such), each carrying its question text, so `result.ask_user` reads in the order the author
/// saw the form.
pub fn validate_ask_user_answer(
    input_text: &str,
    payload: &Value,
) -> Result<Vec<Value>, ToolError> {
    let call_input: AskUserInput = serde_json::from_str(input_text).map_err(|_| {
        ToolError::new(ToolErrorKind::Internal).with("errors", vec!["unreadable ask_user input"])
    })?;
    let questions: BTreeMap<String, &AskQuestion> = call_input
        .questions
        .iter()
        .map(|question| (question.id.clone(), question))
        .collect();
    let answers_raw = payload
        .get("answers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
    for answer in answers_raw {
        let Some(id) = answer.get("id").and_then(Value::as_str).map(str::to_owned) else {
            return Err(validation(vec!["an answer is missing its id".to_owned()]));
        };
        let Some(question) = questions.get(&id) else {
            return Err(validation(vec![format!("unknown question id {id:?}")]));
        };
        let labels: Vec<String> = question
            .options
            .iter()
            .map(|option| option.label.clone())
            .collect();
        let selected: Vec<String> = answer
            .get("selected")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect();
        for label in &selected {
            if !labels.contains(label) {
                return Err(validation(vec![format!(
                    "answer {id:?} selects {label:?}, which is not one of its options"
                )]));
            }
        }
        if selected.len() > 1 && !question.multi_select {
            return Err(validation(vec![format!(
                "answer {id:?} selects several options but the question is single-choice"
            )]));
        }
        let other = answer
            .get("other")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_default();
        if !other.is_empty() && !question.allow_other {
            return Err(validation(vec![format!(
                "answer {id:?} has free text but the question offers no Other"
            )]));
        }
        by_id.insert(
            id,
            json!({
                "id": question.id,
                "question": question.question,
                "selected": selected,
                "other": other,
            }),
        );
    }
    Ok(call_input
        .questions
        .iter()
        .map(|question| {
            by_id.remove(&question.id).unwrap_or_else(|| {
                json!({
                    "id": question.id,
                    "question": question.question,
                    "selected": [],
                    "other": "",
                })
            })
        })
        .collect())
}

// ───────────────────────────── setting documents ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteSettingDocInput {
    doc: SettingDocKind,
    content: String,
}

/// `write_setting_doc` (spec/05 §7): replaces the document (revision + 1).
pub struct WriteSettingDocTool;

#[async_trait::async_trait]
impl ToolHandler for WriteSettingDocTool {
    fn name(&self) -> &'static str {
        "write_setting_doc"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Control
    }

    fn preview_field(&self) -> Option<&'static str> {
        Some("content")
    }

    async fn preview(&self, ctx: &mut ToolCtx<'_>, input: Value) -> Result<Value, ToolError> {
        let input: WriteSettingDocInput = decode(input)?;
        let after = input.content.trim_end().to_owned();
        if after.trim().is_empty() {
            return Err(validation(vec!["content must not be empty".to_owned()]));
        }
        let book_id = book_id(ctx)?;
        let before = setting_docs_service::load(ctx.tx, book_id, input.doc)
            .await
            .map_err(|_| internal())?
            .content;
        Ok(json!({
            "kind": "diff",
            "label": ctx.prompts.label("setting_doc", input.doc.as_str()),
            "before": before,
            "after": after,
        }))
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: WriteSettingDocInput = decode(input)?;
        let content = input.content.trim_end().to_owned();
        if content.trim().is_empty() {
            return Err(validation(vec!["content must not be empty".to_owned()]));
        }
        let book_id = book_id(ctx)?;
        let dto = setting_docs_service::write_from_agent(
            ctx.tx,
            book_id,
            input.doc,
            &content,
            "",
            Some(ctx.session.id),
        )
        .await
        .map_err(|_| internal())?;
        ctx.emit(ServerEvent::SettingsChanged {
            book_id,
            kind: crate::events::SettingsKind::SettingDoc,
            doc_kind: Some(input.doc),
            revision: Some(dto.revision),
        });
        let doc_label = ctx.prompts.label("setting_doc", input.doc.as_str());
        conception_outcome(
            ctx,
            json!({
                "action": "write_setting_doc",
                "doc_label": doc_label,
                "revision": dto.revision,
                "chars": content.chars().count() as i64,
                "name": "",
                "rank": 0,
                "order": [],
                "count": 0,
                "created": false,
            }),
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditSettingDocInput {
    doc: SettingDocKind,
    edits: Vec<FindReplace>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FindReplace {
    find: String,
    replace: String,
}

/// `edit_setting_doc` (spec/05 §7): applies every find/replace atomically, or none.
pub struct EditSettingDocTool;

#[async_trait::async_trait]
impl ToolHandler for EditSettingDocTool {
    fn name(&self) -> &'static str {
        "edit_setting_doc"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Control
    }

    async fn preview(&self, ctx: &mut ToolCtx<'_>, input: Value) -> Result<Value, ToolError> {
        let input: EditSettingDocInput = decode(input)?;
        if input.edits.is_empty() {
            return Err(validation(vec!["edits must not be empty".to_owned()]));
        }
        let book_id = book_id(ctx)?;
        let doc = setting_docs_service::load(ctx.tx, book_id, input.doc)
            .await
            .map_err(|_| internal())?;
        let mut after = doc.content.clone();
        for edit in &input.edits {
            let occurrences = after.matches(&edit.find).count();
            if edit.find.is_empty() || occurrences != 1 {
                return Err(find_failed(
                    ctx.prompts.label("setting_doc", input.doc.as_str()),
                    &edit.find,
                    occurrences,
                ));
            }
            after = after.replacen(&edit.find, &edit.replace, 1);
        }
        Ok(json!({
            "kind": "diff",
            "label": ctx.prompts.label("setting_doc", input.doc.as_str()),
            "before": doc.content,
            "after": after,
        }))
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: EditSettingDocInput = decode(input)?;
        if input.edits.is_empty() {
            return Err(validation(vec!["edits must not be empty".to_owned()]));
        }
        let book_id = book_id(ctx)?;
        let doc = setting_docs_service::load(ctx.tx, book_id, input.doc)
            .await
            .map_err(|_| internal())?;
        // Each `find` must match exactly once in the current text; all-or-none (spec/05 §7). The
        // loop builds the new text first, so a failed edit changes nothing.
        let mut content = doc.content.clone();
        for edit in &input.edits {
            let occurrences = content.matches(&edit.find).count();
            if edit.find.is_empty() || occurrences != 1 {
                return Err(find_failed(
                    ctx.prompts.label("setting_doc", input.doc.as_str()),
                    &edit.find,
                    occurrences,
                ));
            }
            content = content.replacen(&edit.find, &edit.replace, 1);
        }
        let dto = setting_docs_service::write_from_agent(
            ctx.tx,
            book_id,
            input.doc,
            &content,
            "",
            Some(ctx.session.id),
        )
        .await
        .map_err(|_| internal())?;
        ctx.emit(ServerEvent::SettingsChanged {
            book_id,
            kind: crate::events::SettingsKind::SettingDoc,
            doc_kind: Some(input.doc),
            revision: Some(dto.revision),
        });
        let doc_label = ctx.prompts.label("setting_doc", input.doc.as_str());
        conception_outcome(
            ctx,
            json!({
                "action": "edit_setting_doc",
                "doc_label": doc_label,
                "revision": dto.revision,
                "chars": 0,
                "name": "",
                "rank": 0,
                "order": [],
                "count": 0,
                "created": false,
            }),
        )
    }
}

// ───────────────────────────── characters ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpsertCharacterInput {
    name: String,
    #[serde(default)]
    new_name: Option<String>,
    #[serde(default)]
    role_label: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    rank: Option<i32>,
}

/// `upsert_character` (spec/05 §7): creates or updates a design by name; `rank` moves it.
///
/// The writes go through the character service's agent paths, so every change carries its
/// `change_log` row (`actor: agent`) and the ranking stays contiguous `1..n`.
pub struct UpsertCharacterTool;

#[async_trait::async_trait]
impl ToolHandler for UpsertCharacterTool {
    fn name(&self) -> &'static str {
        "upsert_character"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Control
    }

    fn preview_field(&self) -> Option<&'static str> {
        Some("profile")
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: UpsertCharacterInput = decode(input)?;
        let name = input.name.trim().to_owned();
        if name.is_empty() {
            return Err(validation(vec!["name must not be empty".to_owned()]));
        }
        let book_id = book_id(ctx)?;
        let existing = find_design(ctx, book_id, &name).await?;
        let (dto, created) = match existing {
            None => {
                let (dto, events) = crate::services::characters::create_from_agent(
                    ctx.tx,
                    book_id,
                    ctx.session.id,
                    crate::api::dto::settings_materials::CharacterInput {
                        name: name.clone(),
                        role_label: input.role_label.clone().unwrap_or_default(),
                        summary: input.summary.clone().unwrap_or_default(),
                        profile: input.profile.clone().unwrap_or_default(),
                        note: String::new(),
                    },
                )
                .await
                .map_err(|error| map_character_error(error, &name))?;
                forward_settings_events(ctx, &events);
                (dto, true)
            }
            Some(existing) => {
                let new_name = input
                    .new_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .unwrap_or(&name)
                    .to_owned();
                let (dto, events) = crate::services::characters::update_from_agent(
                    ctx.tx,
                    existing.id,
                    ctx.session.id,
                    crate::api::dto::settings_materials::CharacterPatch {
                        name: Some(new_name.clone()),
                        role_label: input.role_label,
                        summary: input.summary,
                        profile: input.profile,
                        entity_id: None,
                        note: String::new(),
                    },
                )
                .await
                .map_err(|error| map_character_error(error, &new_name))?;
                forward_settings_events(ctx, &events);
                (dto, false)
            }
        };
        bump_characters_revision(ctx.tx, book_id)
            .await
            .map_err(|_| internal())?;
        // A rank moves the design within the ranking (spec/05 §7).
        let dto = match input.rank.filter(|rank| *rank >= 1) {
            Some(rank) if rank != dto.rank => {
                let (rank, events) =
                    move_design_to_rank(ctx, book_id, dto.id, &dto.name, rank).await?;
                forward_settings_events(ctx, &events);
                let mut dto = load_design_dto(ctx, dto.id, &dto.name.clone()).await?;
                dto.rank = rank;
                dto
            }
            _ => dto,
        };
        conception_outcome(
            ctx,
            json!({
                "action": "upsert_character",
                "doc_label": "",
                "revision": 0,
                "chars": 0,
                "name": dto.name,
                "rank": dto.rank,
                "order": [],
                "count": 0,
                "created": created,
            }),
        )
    }
}

/// Maps a character-service failure to the tool error the model sees (spec/05 §1.4: a name
/// collision is `error.character_exists`, an unknown name `error.character_not_found`).
fn map_character_error(error: AppError, name: &str) -> ToolError {
    if error.code == crate::error::ErrorCode::ConflictExists {
        return character_exists(name);
    }
    if error.code == crate::error::ErrorCode::NotFound {
        return character_not_found(name);
    }
    validation(vec![error.message])
}

/// Re-emits the settings events a service call returned through the tool context.
fn forward_settings_events(ctx: &mut ToolCtx<'_>, events: &[ServerEvent]) {
    for event in events {
        ctx.emit(event.clone());
    }
}

/// Loads one design row as its DTO.
async fn load_design_dto(
    ctx: &ToolCtx<'_>,
    character_id: i32,
    name: &str,
) -> Result<crate::api::dto::settings_materials::CharacterDto, ToolError> {
    let row = character_designs::Entity::find_by_id(character_id)
        .one(ctx.tx)
        .await
        .map_err(|_| internal())?
        .ok_or_else(|| character_not_found(name))?;
    crate::services::characters::to_dto(&row).map_err(|_| internal())
}

/// Finds a design by exact name.
async fn find_design(
    ctx: &ToolCtx<'_>,
    book_id: i32,
    name: &str,
) -> Result<Option<character_designs::Model>, ToolError> {
    character_designs::Entity::find_by_book_name((book_id, name.to_owned()))
        .one(ctx.tx)
        .await
        .map_err(|_| internal())
}

/// Moves a design to `rank` (1-based, clamped to the list) and renumbers the rest.
///
/// The recorded change row describes the move, the same bookkeeping the author's drag and drop
/// produces through `characters::reorder_inner` (which keys on ids; the tool keys on names).
async fn move_design_to_rank(
    ctx: &mut ToolCtx<'_>,
    book_id: i32,
    character_id: i32,
    name: &str,
    rank: i32,
) -> Result<(i32, Vec<ServerEvent>), ToolError> {
    let mut rows = ranked_designs(ctx, book_id).await?;
    let position = rows
        .iter()
        .position(|design| design.id == character_id)
        .ok_or_else(|| character_not_found(name))?;
    let before = crate::services::change_log::character_snapshot(&rows[position]);
    let target = (rank as usize).saturating_sub(1).min(rows.len() - 1);
    let moved = rows.remove(position);
    let final_rank = target as i32 + 1;
    rows.insert(target, moved);
    let timestamp = chrono::Utc::now();
    let mut change_id = None;
    for (index, design) in rows.iter().enumerate() {
        let next_rank = index as i32 + 1;
        if design.rank != next_rank {
            character_designs::ActiveModel {
                id: Set(design.id),
                rank: Set(next_rank),
                updated_at: Set(timestamp),
                ..Default::default()
            }
            .update(ctx.tx)
            .await
            .map_err(|_| internal())?;
            if design.id == character_id {
                let after = crate::services::change_log::character_snapshot(design);
                let mut after = after;
                if let crate::db::json::Snapshot::CharacterDesign { rank, .. } = &mut after {
                    *rank = next_rank;
                }
                change_id = Some(
                    crate::services::change_log::record(
                        ctx.tx,
                        crate::services::change_log::NewChange {
                            book_id,
                            track_id: None,
                            chapter_id: None,
                            session_id: Some(ctx.session.id),
                            actor: crate::db::json::ChangeActor::Agent,
                            target_type: crate::db::json::ChangeTargetType::CharacterDesign,
                            target_id: design.id,
                            op: crate::db::json::ChangeOp::Update,
                            before: Some(&before),
                            after: Some(&after),
                            note: "",
                            reverts_id: None,
                        },
                    )
                    .await
                    .map_err(|_| internal())?,
                );
            }
        }
    }
    let _ = change_id;
    Ok((
        final_rank,
        vec![ServerEvent::SettingsChanged {
            book_id,
            kind: crate::events::SettingsKind::Characters,
            doc_kind: None,
            revision: None,
        }],
    ))
}

/// The book's designs, most important first.
async fn ranked_designs(
    ctx: &ToolCtx<'_>,
    book_id: i32,
) -> Result<Vec<character_designs::Model>, ToolError> {
    character_designs::Entity::find()
        .filter(character_designs::COLUMN.book_id.eq(book_id))
        .order_by_asc(character_designs::COLUMN.rank)
        .all(ctx.tx)
        .await
        .map_err(|_| internal())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveCharacterInput {
    name: String,
}

/// `remove_character` (spec/05 §7).
pub struct RemoveCharacterTool;

#[async_trait::async_trait]
impl ToolHandler for RemoveCharacterTool {
    fn name(&self) -> &'static str {
        "remove_character"
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
        let input: RemoveCharacterInput = decode(input)?;
        let name = input.name.trim();
        let book_id = book_id(ctx)?;
        let design = find_design(ctx, book_id, name)
            .await?
            .ok_or_else(|| character_not_found(name))?;
        let events =
            crate::services::characters::delete_from_agent(ctx.tx, design.id, ctx.session.id)
                .await
                .map_err(|error| map_character_error(error, name))?;
        forward_settings_events(ctx, &events);
        bump_characters_revision(ctx.tx, book_id)
            .await
            .map_err(|_| internal())?;
        conception_outcome(
            ctx,
            json!({
                "action": "remove_character",
                "doc_label": "",
                "revision": 0,
                "chars": 0,
                "name": design.name,
                "rank": 0,
                "order": [],
                "count": 0,
                "created": false,
            }),
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReorderCharactersInput {
    names: Vec<String>,
}

/// `reorder_characters` (spec/05 §7): the complete order, a permutation of all names.
pub struct ReorderCharactersTool;

#[async_trait::async_trait]
impl ToolHandler for ReorderCharactersTool {
    fn name(&self) -> &'static str {
        "reorder_characters"
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
        let input: ReorderCharactersInput = decode(input)?;
        let book_id = book_id(ctx)?;
        let rows = ranked_designs(ctx, book_id).await?;
        let by_name: BTreeMap<&str, i32> =
            rows.iter().map(|row| (row.name.as_str(), row.id)).collect();
        let mut seen = std::collections::BTreeSet::new();
        let mut ids = Vec::with_capacity(input.names.len());
        for name in &input.names {
            let Some(id) = by_name.get(name.trim()) else {
                return Err(reorder_mismatch(
                    rows.iter().map(|row| row.name.clone()).collect(),
                ));
            };
            if !seen.insert(*id) {
                return Err(reorder_mismatch(
                    rows.iter().map(|row| row.name.clone()).collect(),
                ));
            }
            ids.push(*id);
        }
        if ids.len() != rows.len() {
            return Err(reorder_mismatch(
                rows.iter().map(|row| row.name.clone()).collect(),
            ));
        }
        let timestamp = chrono::Utc::now();
        for (index, id) in ids.iter().enumerate() {
            character_designs::ActiveModel {
                id: Set(*id),
                rank: Set(index as i32 + 1),
                updated_at: Set(timestamp),
                ..Default::default()
            }
            .update(ctx.tx)
            .await
            .map_err(|_| internal())?;
        }
        // Every conception write records `change_log` rows with `actor: agent` (spec/05 §7): a
        // reorder is an update of each design whose rank moved, so each records its before/after
        // (the author's drag and drop is reported through `notice.setting_edits` instead).
        for (position, id) in ids.iter().enumerate() {
            let Some(before_row) = rows.iter().find(|row| row.id == *id) else {
                continue;
            };
            let rank = position as i32 + 1;
            if before_row.rank == rank {
                continue;
            }
            let before = crate::services::change_log::character_snapshot(before_row);
            let mut after = before.clone();
            if let crate::db::json::Snapshot::CharacterDesign { rank: slot, .. } = &mut after {
                *slot = rank;
            }
            crate::services::change_log::record(
                ctx.tx,
                crate::services::change_log::NewChange {
                    book_id,
                    track_id: None,
                    chapter_id: None,
                    session_id: Some(ctx.session.id),
                    actor: crate::db::json::ChangeActor::Agent,
                    target_type: crate::db::json::ChangeTargetType::CharacterDesign,
                    target_id: *id,
                    op: crate::db::json::ChangeOp::Update,
                    before: Some(&before),
                    after: Some(&after),
                    note: "",
                    reverts_id: None,
                },
            )
            .await
            .map_err(|_| internal())?;
        }
        ctx.emit(ServerEvent::SettingsChanged {
            book_id,
            kind: crate::events::SettingsKind::Characters,
            doc_kind: None,
            revision: None,
        });
        bump_characters_revision(ctx.tx, book_id)
            .await
            .map_err(|_| internal())?;
        conception_outcome(
            ctx,
            json!({
                "action": "reorder_characters",
                "doc_label": "",
                "revision": 0,
                "chars": 0,
                "name": "",
                "rank": 0,
                "order": input.names,
                "count": 0,
                "created": false,
            }),
        )
    }
}

// ───────────────────────────── research topics ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetResearchTopicsInput {
    topics: Vec<TopicInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TopicInput {
    topic: String,
    requirements: String,
    scope: String,
}

/// `set_research_topics` (spec/05 §7): replaces the book's `draft` research tasks.
///
/// Research topics are tasks, not canon, so there is no change row; the `research.updated`
/// event of every removed and created task keeps the studio's tab live.
pub struct SetResearchTopicsTool;

#[async_trait::async_trait]
impl ToolHandler for SetResearchTopicsTool {
    fn name(&self) -> &'static str {
        "set_research_topics"
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
        let input: SetResearchTopicsInput = decode(input)?;
        for topic in &input.topics {
            if topic.topic.trim().is_empty() || topic.scope.trim().is_empty() {
                return Err(validation(vec![
                    "a topic needs its text and a scope name".to_owned(),
                ]));
            }
        }
        // A check plans: it replaces only its own waiting topics and never touches the book's
        // conception drafts (spec/07 §8.1, D-168, R2).
        let check_task = match ctx.session.research_task_id {
            Some(task_id) => research_tasks::Entity::find_by_id(task_id)
                .one(ctx.tx)
                .await
                .map_err(|_| internal())?,
            None => None,
        }
        .filter(|task| task.kind == ResearchTaskKind::Preparation);
        if let Some(check) = check_task {
            if input.topics.len() > crate::services::research::MAX_CHECK_TOPICS {
                return Err(validation(vec![format!(
                    "at most {} research topics",
                    crate::services::research::MAX_CHECK_TOPICS
                )]));
            }
            let book_id = check.book_id.ok_or_else(internal)?;
            for topic in &input.topics {
                let name = topic.scope.trim();
                if let Some(scope) = knowledge_scopes::Entity::find()
                    .filter(knowledge_scopes::COLUMN.name.eq(name))
                    .one(ctx.tx)
                    .await
                    .map_err(|_| internal())?
                {
                    let excluded = book_scope_exclusions::Entity::find()
                        .filter(book_scope_exclusions::COLUMN.book_id.eq(book_id))
                        .filter(book_scope_exclusions::COLUMN.scope_id.eq(scope.id))
                        .one(ctx.tx)
                        .await
                        .map_err(|_| internal())?
                        .is_some();
                    if excluded {
                        return Err(ToolError::new(ToolErrorKind::ScopeExcluded)
                            .with("name", name.to_owned()));
                    }
                }
            }
            let waiting = research_tasks::Entity::find()
                .filter(research_tasks::COLUMN.check_id.eq(check.id))
                .filter(research_tasks::COLUMN.kind.eq(ResearchTaskKind::Topic))
                .filter(
                    research_tasks::COLUMN
                        .origin
                        .eq(ResearchTaskOrigin::Conception),
                )
                .filter(research_tasks::COLUMN.status.eq(ResearchTaskStatus::Ready))
                .filter(research_tasks::COLUMN.session_id.is_null())
                .order_by_asc(research_tasks::COLUMN.id)
                .all(ctx.tx)
                .await
                .map_err(|_| internal())?;
            for task in &waiting {
                research_tasks::Entity::delete_by_id(task.id)
                    .exec(ctx.tx)
                    .await
                    .map_err(|_| internal())?;
            }
            let kept: Vec<Value> = research_tasks::Entity::find()
                .filter(research_tasks::COLUMN.check_id.eq(check.id))
                .order_by_asc(research_tasks::COLUMN.id)
                .all(ctx.tx)
                .await
                .map_err(|_| internal())?
                .into_iter()
                .map(|task| {
                    json!({
                        "topic": task.topic,
                        "requirements": task.requirements,
                        "scope_name": task.scope_name,
                        "status": task.status.as_str(),
                    })
                })
                .collect();
            let now = chrono::Utc::now();
            let language = ctx.writing_language();
            for topic in &input.topics {
                let inserted = research_tasks::Entity::insert(research_tasks::ActiveModel {
                    book_id: Set(Some(book_id)),
                    kind: Set(ResearchTaskKind::Topic),
                    origin: Set(ResearchTaskOrigin::Conception),
                    scope_name: Set(topic.scope.trim().to_owned()),
                    topic: Set(topic.topic.trim().to_owned()),
                    requirements: Set(topic.requirements.trim().to_owned()),
                    language: Set(language.clone()),
                    status: Set(ResearchTaskStatus::Ready),
                    check_id: Set(Some(check.id)),
                    created_at: Set(now),
                    updated_at: Set(now),
                    ..Default::default()
                })
                .exec_with_returning(ctx.tx)
                .await
                .map_err(|_| internal())?;
                ctx.emit(ServerEvent::ResearchUpdated {
                    task: crate::services::research::to_dto(inserted),
                });
            }
            for task in &waiting {
                ctx.emit(ServerEvent::ResearchUpdated {
                    task: crate::services::research::to_dto(task.clone()),
                });
            }
            return conception_outcome(
                ctx,
                json!({
                    "action": "set_research_topics",
                    "doc_label": "",
                    "revision": 0,
                    "chars": 0,
                    "name": "",
                    "rank": 0,
                    "order": [],
                    "count": input.topics.len(),
                    "created": false,
                    "kept": kept,
                }),
            );
        }
        let book_id = book_id(ctx)?;
        let all_topics = research_tasks::Entity::find()
            .filter(research_tasks::COLUMN.book_id.eq(book_id))
            .filter(research_tasks::COLUMN.kind.eq(ResearchTaskKind::Topic))
            .order_by_asc(research_tasks::COLUMN.id)
            .all(ctx.tx)
            .await
            .map_err(|_| internal())?;
        let mut drafts = Vec::new();
        let mut kept = Vec::new();
        for task in all_topics {
            if task.check_id.is_some() {
                continue;
            }
            if task.origin == ResearchTaskOrigin::Conception
                && task.status == ResearchTaskStatus::Draft
            {
                drafts.push(task);
            } else if task.origin == ResearchTaskOrigin::Author
                && task.status == ResearchTaskStatus::Draft
            {
                // The result names the author's drafts left in place (spec/05 §7); the kickoff
                // separately lists every protected topic task.
                kept.push(task);
            }
        }
        let now = chrono::Utc::now();
        for task in &drafts {
            research_tasks::Entity::delete_by_id(task.id)
                .exec(ctx.tx)
                .await
                .map_err(|_| internal())?;
        }
        // A task carries the book's writing language (spec/07 §8 step 2).
        let language = ctx
            .book
            .map(|book| book.writing_language.clone())
            .unwrap_or_else(|| "en".to_owned());
        for topic in &input.topics {
            let row = research_tasks::ActiveModel {
                book_id: Set(Some(book_id)),
                kind: Set(ResearchTaskKind::Topic),
                origin: Set(ResearchTaskOrigin::Conception),
                scope_name: Set(topic.scope.trim().to_owned()),
                topic: Set(topic.topic.trim().to_owned()),
                requirements: Set(topic.requirements.trim().to_owned()),
                language: Set(language.clone()),
                // The book is conceiving, so the task starts as a draft (spec/10 §2.5); finalize
                // turns the drafts `ready`.
                status: Set(ResearchTaskStatus::Draft),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            };
            let inserted = research_tasks::Entity::insert(row)
                .exec_with_returning(ctx.tx)
                .await
                .map_err(|_| internal())?;
            ctx.emit(ServerEvent::ResearchUpdated {
                task: crate::services::research::to_dto(inserted),
            });
        }
        for task in &drafts {
            ctx.emit(ServerEvent::ResearchUpdated {
                task: crate::services::research::to_dto(task.clone()),
            });
        }
        ctx.emit(ServerEvent::SettingsChanged {
            book_id,
            kind: crate::events::SettingsKind::ResearchTopics,
            doc_kind: None,
            revision: None,
        });
        bump_topics_revision(ctx.tx, book_id)
            .await
            .map_err(|_| internal())?;
        let kept: Vec<Value> = kept
            .iter()
            .map(|task| {
                json!({
                    "topic": task.topic,
                    "requirements": task.requirements,
                    "scope_name": task.scope_name,
                    "status": task.status.as_str(),
                })
            })
            .collect();
        conception_outcome(
            ctx,
            json!({
                "action": "set_research_topics",
                "doc_label": "",
                "revision": 0,
                "chars": 0,
                "name": "",
                "rank": 0,
                "order": [],
                "count": input.topics.len(),
                "created": false,
                "kept": kept,
            }),
        )
    }
}

// ───────────────────────────── request_review ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestReviewInput {
    message: String,
}

/// `request_review` (spec/05 §7): awaits the author's approvals.
///
/// The approvals are recorded when the answer arrives, by [`apply_request_review_answer`]: an
/// approved document records its `revision` in `setting_docs.approved_revision`, and the
/// characters and topics revisions in `ConceptionState`.
pub struct RequestReviewTool;

#[async_trait::async_trait]
impl ToolHandler for RequestReviewTool {
    fn name(&self) -> &'static str {
        "request_review"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Interactive
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: RequestReviewInput = decode(input)?;
        let message = input.message.trim().to_owned();
        if message.is_empty() {
            return Err(validation(vec!["message must not be empty".to_owned()]));
        }
        let text = ctx
            .prompts
            .render_value(
                "result.request_review",
                crate::prompts::template_value(&json!({
                    "items": [],
                    "general": "",
                    "all_approved": false,
                })),
            )
            .map_err(|_| internal())?
            .text;
        Ok(ToolOutcome::text(text)
            .with_ui(json!({ "message": message }))
            .with_await_input(AwaitInputKind::RequestReview))
    }
}

/// One item of a validated review answer, as `result.request_review` renders it.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewAnswerItem {
    /// The prompt-pack label of the item (`shared.label setting_doc`), rendered for the model.
    pub doc_label: String,
    pub approved: bool,
    pub feedback: String,
}

/// Validates a `request_review` answer payload (spec/05 §7), records the approvals and returns
/// the rendered item list.
///
/// The payload is `{docs: {outline?, world?, protagonist?, characters?, research_topics?}, general?}`,
/// where each present item is `{approved: bool, feedback?}`. An approved document records its
/// current `revision` in `setting_docs.approved_revision`; the characters and topics approvals
/// record their revisions in `ConceptionState` (a revision counter that changes on every write,
/// so a later write clears the approval).
pub async fn apply_request_review_answer(
    tx: &impl ConnectionTrait,
    prompts: &crate::prompts::PromptSet,
    book_id: i32,
    payload: &Value,
) -> Result<(Vec<ReviewAnswerItem>, String, bool), AppError> {
    let docs = payload.get("docs").cloned().unwrap_or(Value::Null);
    let item_of = |key: &str| -> Option<(bool, String)> {
        let item = docs.get(key)?;
        let approved = item.get("approved")?.as_bool()?;
        let feedback = item
            .get("feedback")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Some((approved, feedback))
    };
    let general = payload
        .get("general")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    let mut state = load_conception_state(tx, book_id).await?;
    let mut items = Vec::new();
    let mut all_approved = true;
    // The three documents, then the characters and the topics (the studio's tab order).
    for key in ["outline", "world", "protagonist"] {
        let Some((approved, feedback)) = item_of(key) else {
            continue;
        };
        let kind = SettingDocKind::from_str_opt(key)
            .ok_or_else(|| AppError::internal(format!("unknown setting doc {key}")))?;
        let doc = setting_docs_service::load(tx, book_id, kind).await?;
        // Approval is a revision reference; a rejection clears it, so the rejected revision is
        // not approved by a *previous* review of the same text (D-105: the studio's Finalize
        // warning and the Demo's revise plan both read `revision == approved_revision`).
        SettingDocActive {
            id: Set(doc.id),
            approved_revision: Set(approved.then_some(doc.revision)),
            updated_at: Set(chrono::Utc::now()),
            ..Default::default()
        }
        .update(tx)
        .await?;
        all_approved &= approved;
        items.push(ReviewAnswerItem {
            doc_label: prompts.label("setting_doc", key),
            approved,
            feedback,
        });
    }
    if let Some((approved, feedback)) = item_of("characters") {
        state.characters_approved_revision = approved.then_some(state.characters_revision);
        all_approved &= approved;
        items.push(ReviewAnswerItem {
            doc_label: prompts.label("setting_doc", "characters"),
            approved,
            feedback,
        });
    }
    if let Some((approved, feedback)) = item_of("research_topics") {
        state.topics_approved_revision = approved.then_some(state.topics_revision);
        all_approved &= approved;
        items.push(ReviewAnswerItem {
            doc_label: prompts.label("setting_doc", "research_topics"),
            approved,
            feedback,
        });
    }
    store_conception_state(tx, book_id, &state).await?;
    // "Everything approved" needs something to have been approved: an empty review payload must
    // not tell the model the author passed the materials (spec/05 §7).
    let all_approved = all_approved && !items.is_empty();
    Ok((items, general, all_approved))
}

/// Bumps `ConceptionState.characters_revision` (every character write invalidates an earlier
/// approval, spec/02 §4).
pub async fn bump_characters_revision(
    tx: &impl ConnectionTrait,
    book_id: i32,
) -> Result<(), AppError> {
    let mut state = load_conception_state(tx, book_id).await?;
    state.characters_revision += 1;
    store_conception_state(tx, book_id, &state).await
}

/// Bumps `ConceptionState.topics_revision` (spec/02 §4).
pub async fn bump_topics_revision(tx: &impl ConnectionTrait, book_id: i32) -> Result<(), AppError> {
    let mut state = load_conception_state(tx, book_id).await?;
    state.topics_revision += 1;
    store_conception_state(tx, book_id, &state).await
}

/// Reads the book's `ConceptionState`.
pub async fn load_conception_state(
    tx: &impl ConnectionTrait,
    book_id: i32,
) -> Result<ConceptionState, AppError> {
    let book = books::Entity::find_by_id(book_id)
        .one(tx)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?;
    Ok(book.conception.clone().into_inner())
}

/// Writes the book's `ConceptionState`.
pub async fn store_conception_state(
    tx: &impl ConnectionTrait,
    book_id: i32,
    state: &ConceptionState,
) -> Result<(), AppError> {
    let raw = crate::services::json_text(state.clone());
    books::ActiveModel {
        id: Set(book_id),
        conception: Set(raw),
        updated_at: Set(chrono::Utc::now()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    Ok(())
}

/// The conception role's handlers (spec/05 §2).
pub fn all() -> Vec<std::sync::Arc<dyn ToolHandler>> {
    vec![
        std::sync::Arc::new(AskUserTool),
        std::sync::Arc::new(WriteSettingDocTool),
        std::sync::Arc::new(EditSettingDocTool),
        std::sync::Arc::new(UpsertCharacterTool),
        std::sync::Arc::new(RemoveCharacterTool),
        std::sync::Arc::new(ReorderCharactersTool),
        std::sync::Arc::new(SetResearchTopicsTool),
        std::sync::Arc::new(RequestReviewTool),
    ]
}
