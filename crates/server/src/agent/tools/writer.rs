//! Writer tools (spec/05 §4): planning, drafting and finish (PLAN M4.2).
//!
//! The central rule is the granularity matrix of spec/03 §8. **Which rule applies** is decided by
//! the granularity stored in the assistant message's `request_meta` — the mode in effect when the
//! model generated the call — not by the session's current value, so switching granularity never
//! rejects a call that was already being generated.
//!
//! Every text-changing tool **stages** its chapter write instead of performing it: the new content
//! travels on the outcome and is applied by the transaction that finalizes the call (D-139), so a
//! held call's text is not in the chapter until the author approves it, and feedback on the held
//! call discards it. Model-facing text is rendered from `prompts/results.toml`; nothing English
//! reaches the model.

use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::dto::ContentOp;
use crate::db::entities::chapters;
use crate::db::json::{
    ChapterContent, CompletionKind, Granularity, Paragraph, PendingContent, PendingOutline,
    SessionKind, ToolEffects,
};
use crate::events::ServerEvent;
use crate::text;

use super::{
    Prepared, ToolCtx, ToolError, ToolErrorKind, ToolHandler, ToolKind, ToolOutcome,
    resolve::EntityResolver,
};

/// The maximum length of one sentence under `sentence` granularity (spec/03 §8).
pub const SENTENCE_LIMIT: usize = 400;

/// The maximum length of one paragraph under `sentence`/`paragraph` granularity (spec/03 §8).
pub const PARAGRAPH_LIMIT: usize = 3_000;

/// The maximum length of one segment under `free`/`chapter` granularity (spec/03 §8).
pub const FREE_LIMIT: usize = 60_000;

/// A completion below this fraction of `target_words_min` is withheld once (spec/03 §8, D-029).
pub const SHORT_COMPLETION_RATIO: f64 = 0.6;

/// The `read_draft` default cap (spec/05 §4).
pub const READ_DRAFT_DEFAULT: usize = 12_000;

// ───────────────────────────── input types ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitOutlineInput {
    outline: String,
    #[serde(default)]
    reference_notes: String,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadDraftInput {
    #[serde(default)]
    from_pid: Option<String>,
    #[serde(default)]
    to_pid: Option<String>,
    #[serde(default)]
    max_chars: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InsertTextInput {
    text: String,
    #[serde(default)]
    paragraph_end: Option<bool>,
    #[serde(default)]
    chapter_end: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InsertParagraphsInput {
    after: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplaceTextInput {
    pid: String,
    old: String,
    new: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RewriteParagraphInput {
    pid: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteParagraphsInput {
    pids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FinishDraftInput {
    #[serde(default)]
    note: Option<String>,
}

fn decode<T: for<'de> Deserialize<'de>>(input: Value) -> Result<T, ToolError> {
    serde_json::from_value(input).map_err(|err| {
        ToolError::new(ToolErrorKind::Validation).with("errors", vec![err.to_string()])
    })
}

// ───────────────────────────── granularity ─────────────────────────────

/// The text rules a call must obey (spec/03 §8). `mode` comes from the assistant message's
/// `request_meta`, never from the session row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRules {
    pub mode: Granularity,
    /// True in the revision phases (`chapter_end` is ignored there).
    pub revising: bool,
}

impl TextRules {
    /// The rules of the call's `request_meta` (spec/03 §8 "which rule applies").
    pub fn of(ctx: &ToolCtx<'_>) -> Self {
        Self {
            mode: ctx
                .request_meta
                .granularity
                .unwrap_or(Granularity::Paragraph),
            revising: matches!(
                ctx.request_meta.phase,
                crate::db::json::Phase::RevisionPlanning | crate::db::json::Phase::Revising
            ),
        }
    }

    /// The rule of `insert_text` for this mode (spec/03 §8 table).
    pub fn segment(&self) -> SegmentRule {
        match self.mode {
            Granularity::Sentence => SegmentRule {
                no_line_breaks: true,
                limit: Some(SENTENCE_LIMIT),
                reason_label: SegmentLimit::Sentence,
            },
            Granularity::Paragraph => SegmentRule {
                no_line_breaks: true,
                limit: Some(PARAGRAPH_LIMIT),
                reason_label: SegmentLimit::Paragraph,
            },
            Granularity::Free | Granularity::Chapter => SegmentRule {
                no_line_breaks: false,
                limit: Some(FREE_LIMIT),
                reason_label: SegmentLimit::Segment,
            },
        }
    }

    /// The rule of `insert_paragraphs` and `rewrite_paragraph`: a single paragraph under
    /// `sentence`/`paragraph`, several under `free`/`chapter` (spec/03 §8).
    pub fn paragraph_edit(&self) -> SegmentRule {
        match self.mode {
            Granularity::Sentence | Granularity::Paragraph => SegmentRule {
                no_line_breaks: true,
                limit: Some(PARAGRAPH_LIMIT),
                reason_label: SegmentLimit::Paragraph,
            },
            Granularity::Free | Granularity::Chapter => SegmentRule {
                no_line_breaks: false,
                limit: Some(FREE_LIMIT),
                reason_label: SegmentLimit::Segment,
            },
        }
    }

    /// True when a completion in this mode happens through `insert_text` (spec/03 §8:
    /// `chapter_end` or `chapter` mode).
    pub fn completes_on_insert(&self) -> bool {
        matches!(self.mode, Granularity::Chapter)
    }
}

/// What a segment rule protects (the `reason` and the unit named in `error.granularity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentLimit {
    /// One sentence (`insert_text` under `sentence`).
    Sentence,
    /// One paragraph.
    Paragraph,
    /// A free segment (several paragraphs allowed).
    Segment,
}

/// The text shape one call may write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentRule {
    pub no_line_breaks: bool,
    pub limit: Option<usize>,
    pub reason_label: SegmentLimit,
}

/// Checks one call's text against its granularity rule (spec/03 §8).
///
/// Returns the normalized paragraphs the call writes, or the `error.granularity` the model reads.
/// Empty text is an error: every text-adding tool must add something.
pub fn validate_text(
    prompts: &crate::prompts::PromptSet,
    mode: Granularity,
    rule: SegmentRule,
    tool: &str,
    text: &str,
) -> Result<Vec<String>, ToolError> {
    let normalized = text::normalize_segment(text);
    let length = normalized.chars().count();
    let reason = if normalized.is_empty() {
        Some("empty")
    } else if rule.no_line_breaks && normalized.contains('\n') {
        Some("newline")
    } else if rule.limit.is_some_and(|limit| length > limit) {
        Some("too_long")
    } else {
        None
    };
    let Some(reason) = reason else {
        return Ok(text::split_paragraphs(text));
    };
    let limit = rule.limit.unwrap_or(0);
    Err(ToolError::new(ToolErrorKind::Granularity)
        .with("mode", mode.as_str())
        .with("mode_label", prompts.label("granularity", mode.as_str()))
        .with("tool", tool.to_owned())
        .with("reason", reason)
        .with("length", length)
        .with("limit", limit))
}

// ───────────────────────────── shared helpers ─────────────────────────────

/// The session's chapter, or an internal error (a writer tool always has one).
fn chapter_of<'a>(ctx: &'a ToolCtx<'_>) -> Result<&'a chapters::Model, ToolError> {
    ctx.chapter
        .ok_or_else(|| ToolError::new(ToolErrorKind::Internal))
}

/// The chapter's stored content.
fn content_of(chapter: &chapters::Model) -> ChapterContent {
    crate::db::json::from_json_or_default(&chapter.content)
}

/// The book's target length, defaulting to the schema defaults when the book is absent.
fn targets(ctx: &ToolCtx<'_>) -> (i32, i32) {
    let settings: crate::db::json::BookSettings = ctx
        .book
        .as_ref()
        .map(|book| crate::db::json::from_json_or_default(&book.settings))
        .unwrap_or_default();
    (settings.target_words_min, settings.target_words_max)
}

/// Stages a new content state for the transaction that finalizes the call (spec/03 §5.2, §5.4).
///
/// Nothing is written here. The returned [`PendingContent`] travels on the outcome and the
/// finalizing transaction applies it, so `chapters.content`, `content_version`, `word_count` and the
/// `chapter.content` event all move together with the call's status — and only if the write is
/// accepted. A call that holds for approval therefore leaves the chapter byte-identical (D-139).
pub fn stage_content(
    ctx: &ToolCtx<'_>,
    content: ChapterContent,
) -> Result<PendingContent, ToolError> {
    let chapter = chapter_of(ctx)?;
    Ok(PendingContent {
        content,
        expected_version: chapter.content_version,
    })
}

/// True when a completion should be withheld as short (spec/03 §8, D-029).
///
/// Only `drafting` completions of a chapter draft are checked: revisions finish with
/// `finish_revision` (spec/03 §8).
pub fn is_short_completion(ctx: &ToolCtx<'_>, word_count: i32) -> bool {
    if ctx.request_meta.phase != crate::db::json::Phase::Drafting {
        return false;
    }
    let (target_min, _) = targets(ctx);
    if target_min <= 0 {
        return false;
    }
    let threshold = (f64::from(target_min) * SHORT_COMPLETION_RATIO) as i32;
    word_count < threshold
}

/// The counter flag of the one withheld completion (spec/03 §8: exactly once).
fn finish_warned(ctx: &ToolCtx<'_>) -> bool {
    crate::agent::session::counters_of(ctx.session).finish_warned
}

/// Resolves a pid to its index, or `error.pid_not_found`.
fn pid_index(content: &ChapterContent, pid: &str) -> Result<usize, ToolError> {
    content
        .paragraphs
        .iter()
        .position(|paragraph| paragraph.pid == pid)
        .ok_or_else(|| {
            ToolError::new(ToolErrorKind::PidNotFound)
                .with("pids", vec![pid.to_owned()])
                .with("tool", "")
        })
}

// ───────────────────────────── submit_outline ─────────────────────────────

/// `submit_outline` (spec/05 §4): save the outline and notes, enter drafting.
pub struct SubmitOutlineTool;

#[async_trait::async_trait]
impl ToolHandler for SubmitOutlineTool {
    fn name(&self) -> &'static str {
        "submit_outline"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Draft
    }

    fn preview_field(&self) -> Option<&'static str> {
        Some("outline")
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: SubmitOutlineInput = decode(input)?;
        // D-146: the outline session stages its proposal; a legacy `chapter_write`/`planning`
        // session (started before the split) keeps the immediate write and the phase change.
        if crate::agent::session::kind_of(ctx.session) == SessionKind::ChapterOutline {
            return stage_outline(ctx, input).await;
        }
        let chapter = chapter_of(ctx)?.clone();
        // The title is only set when the author has not set one (spec/05 §4).
        let set_title = chapter.title.trim().is_empty()
            && input
                .title
                .as_deref()
                .map(str::trim)
                .is_some_and(|title| !title.is_empty());
        let title = if set_title {
            input
                .title
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .to_owned()
        } else {
            chapter.title.clone()
        };
        chapters::ActiveModel {
            id: Set(chapter.id),
            outline: Set(input.outline.clone()),
            reference_notes: Set(input.reference_notes.clone()),
            title: Set(title.clone()),
            updated_at: Set(chrono::Utc::now()),
            ..Default::default()
        }
        .update(ctx.tx)
        .await
        .map_err(|_| ToolError::new(ToolErrorKind::Internal))?;
        let row = chapters::Entity::find_by_id(chapter.id)
            .one(ctx.tx)
            .await
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .ok_or_else(|| ToolError::new(ToolErrorKind::Internal))?;
        ctx.emit(ServerEvent::ChapterUpdated {
            chapter: crate::api::dto::ChapterListItemDto::from(row),
            outline_changed: true,
        });
        let (target_min, target_max) = targets(ctx);
        let text = ctx
            .prompts
            .render_value(
                "result.writer_control",
                crate::prompts::template_value(&json!({
                    "action": "submit_outline",
                    "word_count": chapter.word_count,
                    "target_min": target_min,
                    "target_max": target_max,
                    "confirm_needed": false,
                    "title_set": set_title,
                    "title": title,
                    "staged": false,
                })),
            )
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        Ok(ToolOutcome::text(text)
            .with_ui(json!({
                "outline_chars": input.outline.chars().count(),
                "notes_chars": input.reference_notes.chars().count(),
            }))
            .with_effects(ToolEffects {
                phase_to: Some(crate::db::json::Phase::Drafting),
                ..ToolEffects::default()
            }))
    }
}

/// Stages an outline-session `submit_outline` proposal (spec/03 §5.2, D-146).
///
/// Nothing is written: `chapters.outline` changes only when the finalizing transaction calls
/// `apply_pending_outline`, so a held or rejected proposal never reaches the chapter. The title is
/// staged too and applied only while the chapter's title is still empty.
async fn stage_outline(
    ctx: &mut ToolCtx<'_>,
    input: SubmitOutlineInput,
) -> Result<ToolOutcome, ToolError> {
    let chapter = chapter_of(ctx)?;
    let title = input
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned);
    let (target_min, target_max) = targets(ctx);
    let text = ctx
        .prompts
        .render_value(
            "result.writer_control",
            crate::prompts::template_value(&json!({
                "action": "submit_outline",
                "word_count": chapter.word_count,
                "target_min": target_min,
                "target_max": target_max,
                "confirm_needed": false,
                "title_set": title.is_some(),
                "title": title.clone().unwrap_or_default(),
                "staged": true,
            })),
        )
        .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
        .text;
    Ok(ToolOutcome::text(text)
        .with_ui(json!({
            "outline_chars": input.outline.chars().count(),
            "notes_chars": input.reference_notes.chars().count(),
        }))
        .with_effects(ToolEffects {
            complete: Some(CompletionKind::Outline),
            pending_outline: Some(PendingOutline {
                outline: input.outline,
                reference_notes: input.reference_notes,
                title,
            }),
            ..ToolEffects::default()
        }))
}

// ───────────────────────────── read_draft ─────────────────────────────

/// `read_draft` (spec/05 §4): the current text with pids, the tail marker and the target.
pub struct ReadDraftTool;

#[async_trait::async_trait]
impl ToolHandler for ReadDraftTool {
    fn name(&self) -> &'static str {
        "read_draft"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Draft
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: ReadDraftInput = decode(input)?;
        let content = content_of(chapter_of(ctx)?);
        let start = match &input.from_pid {
            Some(pid) => content
                .paragraphs
                .iter()
                .position(|paragraph| &paragraph.pid == pid)
                .ok_or_else(|| {
                    ToolError::new(ToolErrorKind::PidNotFound)
                        .with("pids", vec![pid.clone()])
                        .with("tool", "read_draft")
                })?,
            None => 0,
        };
        let end = match &input.to_pid {
            Some(pid) => content
                .paragraphs
                .iter()
                .position(|paragraph| &paragraph.pid == pid)
                .map(|index| index + 1)
                .ok_or_else(|| {
                    ToolError::new(ToolErrorKind::PidNotFound)
                        .with("pids", vec![pid.clone()])
                        .with("tool", "read_draft")
                })?,
            None => content.paragraphs.len(),
        };
        let limit = input
            .max_chars
            .unwrap_or(READ_DRAFT_DEFAULT)
            .clamp(500, 30_000);
        let mut used = 0usize;
        let mut shown: Vec<(String, String)> = Vec::new();
        let mut truncated_at: Option<String> = None;
        for paragraph in &content.paragraphs[start..end.max(start)] {
            let length = paragraph.text.chars().count() + paragraph.pid.chars().count();
            if !shown.is_empty() && used + length > limit {
                truncated_at = Some(paragraph.pid.clone());
                break;
            }
            used += length;
            shown.push((paragraph.pid.clone(), paragraph.text.clone()));
        }
        let (target_min, target_max) = targets(ctx);
        let total = content.paragraphs.len();
        // The tail marker belongs to the last paragraph of the whole draft, which is only shown
        // when the range reaches the end and nothing was cut.
        let tail_open = content.tail_open && truncated_at.is_none() && end >= total;
        let text = ctx
            .prompts
            .render_value(
                "result.draft_text",
                crate::prompts::template_value(&json!({
                    "paragraphs": shown.iter().map(|(pid, text)| json!({ "pid": pid, "text": text })).collect::<Vec<_>>(),
                    "tail_open": tail_open,
                    "word_count": text::content_word_count(&content),
                    "target_min": target_min,
                    "target_max": target_max,
                    "next_pid": truncated_at,
                    "total_paragraphs": total,
                })),
            )
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        Ok(ToolOutcome::text(text).with_ui(json!({ "paragraphs": shown.len() })))
    }
}

// ───────────────────────────── insert_text ─────────────────────────────

/// `insert_text` (spec/05 §4, spec/03 §8): append per granularity, continuing an open tail.
pub struct InsertTextTool;

/// What a text change touched, for `result.draft_edit` and the call's `ui` (spec/05 §4).
#[derive(Debug, Default, Clone)]
struct EditParts {
    inserted: Vec<String>,
    appended_to: Option<String>,
    deleted: Vec<String>,
}

/// Reads an `insert`/`update`/`delete` op list into [`EditParts`].
fn edit_parts(ops: &[ContentOp]) -> EditParts {
    let mut parts = EditParts::default();
    for op in ops {
        match op {
            ContentOp::Insert { pid, .. } => parts.inserted.push(pid.clone()),
            ContentOp::Update { pid, .. } => parts.appended_to = Some(pid.clone()),
            ContentOp::Delete { pid } => parts.deleted.push(pid.clone()),
            ContentOp::Tail { .. } => {}
        }
    }
    parts
}

/// The completion flags of one `insert_text` call (spec/03 §8).
#[derive(Debug, Clone, Copy, Default)]
struct CompletionFlags {
    chapter_end: bool,
    short_of_target: bool,
    ignored_chapter_end: bool,
}

/// Builds the full `result.draft_edit` context (spec/09 §2 `vars`).
fn draft_edit_value(
    ctx: &ToolCtx<'_>,
    content: &ChapterContent,
    action: &str,
    parts: &EditParts,
    after: Option<&str>,
    completion: CompletionFlags,
) -> Value {
    let (target_min, target_max) = targets(ctx);
    json!({
        "action": action,
        "inserted": parts.inserted,
        "appended_to": parts.appended_to,
        "pid": if action == "insert_text" { Value::Null } else { json!(parts.appended_to.clone().or_else(|| parts.inserted.first().cloned())) },
        "deleted": parts.deleted,
        "after": after,
        "tail_open": content.tail_open,
        "last_pid": content.paragraphs.last().map(|p| p.pid.clone()).unwrap_or_default(),
        "word_count": text::content_word_count(content),
        "target_min": target_min,
        "target_max": target_max,
        "chapter_end": completion.chapter_end,
        "short_of_target": completion.short_of_target,
        "ignored_chapter_end": completion.ignored_chapter_end,
    })
}

/// Renders `result.draft_edit` with `action` and the edit's parts.
fn render_draft_edit(
    ctx: &ToolCtx<'_>,
    content: &ChapterContent,
    action: &str,
    parts: &EditParts,
    after: Option<&str>,
) -> Result<String, ToolError> {
    let value = draft_edit_value(
        ctx,
        content,
        action,
        parts,
        after,
        CompletionFlags::default(),
    );
    ctx.prompts
        .render_value("result.draft_edit", crate::prompts::template_value(&value))
        .map(|rendered| rendered.text)
        .map_err(|_| ToolError::new(ToolErrorKind::Internal))
}

#[async_trait::async_trait]
impl ToolHandler for InsertTextTool {
    fn name(&self) -> &'static str {
        "insert_text"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Draft
    }

    fn preview_field(&self) -> Option<&'static str> {
        Some("text")
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: InsertTextInput = decode(input)?;
        let rules = TextRules::of(ctx);
        // Validate against the granularity the call was generated under (spec/03 §8).
        validate_text(
            ctx.prompts,
            rules.mode,
            rules.segment(),
            "insert_text",
            &input.text,
        )?;
        let language = ctx.writing_language();
        let mut content = content_of(chapter_of(ctx)?);
        let tail_before = content.tail_open;
        let mut ops =
            text::append_segment(&mut content, &input.text, &language).map_err(map_text_error)?;

        // The tail state after the segment (spec/03 §8 table):
        // `sentence`/`free` follow `paragraph_end`; `paragraph`/`chapter` are always closed.
        let paragraph_end = input.paragraph_end.unwrap_or(true);
        content.tail_open = match rules.mode {
            Granularity::Sentence | Granularity::Free => !paragraph_end,
            Granularity::Paragraph | Granularity::Chapter => false,
        };
        if tail_before != content.tail_open {
            ops.push(ContentOp::Tail {
                open: content.tail_open,
            });
        }
        let parts = edit_parts(&ops);

        let word_count = text::content_word_count(&content);
        // `chapter_end` completes only outside the revision phases (spec/03 §8); `chapter` mode
        // implies it.
        let chapter_end_requested = input.chapter_end.unwrap_or(false);
        // `chapter` mode implies `chapter_end` (spec/03 §8), so the implied completion is ignored
        // in `revising` exactly like an explicit one — and says so, or the model is left waiting
        // for a completion the session will never reach.
        let ignored_chapter_end =
            (chapter_end_requested || rules.completes_on_insert()) && rules.revising;
        let completes = (chapter_end_requested || rules.completes_on_insert()) && !rules.revising;
        // A short completion is withheld once (spec/03 §8, D-029).
        let withheld = completes && !finish_warned(ctx) && is_short_completion(ctx, word_count);
        let chapter_end = completes && !withheld;

        let pending = stage_content(ctx, content.clone())?;

        let text = ctx
            .prompts
            .render_value(
                "result.draft_edit",
                crate::prompts::template_value(&draft_edit_value(
                    ctx,
                    &content,
                    "insert_text",
                    &parts,
                    None,
                    CompletionFlags {
                        chapter_end,
                        short_of_target: withheld,
                        ignored_chapter_end,
                    },
                )),
            )
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        let effects = if chapter_end {
            ToolEffects {
                complete: Some(CompletionKind::Draft),
                ..ToolEffects::default()
            }
        } else {
            ToolEffects::default()
        };
        let mut outcome = ToolOutcome::text(text)
            .with_pending_content(pending)
            .with_ui(json!({
                "inserted": parts.inserted,
                "appended_to": parts.appended_to,
                "tail_open": content.tail_open,
                "word_count": word_count,
            }))
            .with_effects(effects);
        if withheld {
            outcome = outcome.with_finish_warned();
        }
        Ok(outcome)
    }
}

// ───────────────────────────── insert_paragraphs ─────────────────────────────

/// `insert_paragraphs` (spec/05 §4): insert complete paragraphs after a pid (or at the start).
pub struct InsertParagraphsTool;

#[async_trait::async_trait]
impl ToolHandler for InsertParagraphsTool {
    fn name(&self) -> &'static str {
        "insert_paragraphs"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Draft
    }

    fn preview_field(&self) -> Option<&'static str> {
        Some("text")
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: InsertParagraphsInput = decode(input)?;
        let rules = TextRules::of(ctx);
        let parts_text = validate_text(
            ctx.prompts,
            rules.mode,
            rules.paragraph_edit(),
            "insert_paragraphs",
            &input.text,
        )?;
        let mut content = content_of(chapter_of(ctx)?);
        let at_start = input.after.trim() == "start";
        let anchor = if at_start {
            None
        } else {
            let pid = input.after.trim().to_owned();
            pid_index(&content, &pid)?;
            Some(pid)
        };
        // Inserting after an open tail closes it first (spec/05 §4).
        if content.tail_open
            && anchor.as_deref() != content.paragraphs.last().map(|p| p.pid.as_str())
        {
            content.tail_open = false;
        }
        let index = match &anchor {
            None => 0,
            Some(pid) => pid_index(&content, pid)? + 1,
        };
        let mut inserted = Vec::new();
        for (offset, part) in parts_text.iter().enumerate() {
            let pid = text::allocate_pid(&mut content);
            content.paragraphs.insert(
                index + offset,
                Paragraph {
                    pid: pid.clone(),
                    text: part.clone(),
                },
            );
            inserted.push(pid);
        }
        let pending = stage_content(ctx, content.clone())?;
        let text = render_draft_edit(
            ctx,
            &content,
            "insert_paragraphs",
            &EditParts {
                inserted: inserted.clone(),
                ..EditParts::default()
            },
            if at_start {
                None
            } else {
                Some(input.after.trim())
            },
        )?;
        Ok(ToolOutcome::text(text)
            .with_pending_content(pending)
            .with_ui(json!({
                "inserted": inserted,
                "after": if at_start { Value::Null } else { json!(input.after.trim()) },
            })))
    }
}

// ───────────────────────────── replace_text ─────────────────────────────

/// `replace_text` (spec/05 §4): replace one occurrence inside a paragraph.
pub struct ReplaceTextTool;

#[async_trait::async_trait]
impl ToolHandler for ReplaceTextTool {
    fn name(&self) -> &'static str {
        "replace_text"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Draft
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: ReplaceTextInput = decode(input)?;
        let mut content = content_of(chapter_of(ctx)?);
        let index = pid_index(&content, &input.pid)?;
        let paragraph = content.paragraphs[index].text.clone();
        let occurrences = if input.old.is_empty() {
            0
        } else {
            paragraph.matches(&input.old).count()
        };
        if occurrences == 0 {
            return Err(ToolError::new(ToolErrorKind::TextNotFound)
                .with("pid", input.pid)
                .with("text", paragraph));
        }
        if occurrences > 1 {
            return Err(ToolError::new(ToolErrorKind::TextAmbiguous)
                .with("pid", input.pid)
                .with("count", occurrences));
        }
        let updated = paragraph.replacen(&input.old, &input.new, 1);
        let mut parts = EditParts::default();
        if updated.trim().is_empty() {
            // An empty result deletes the paragraph (spec/05 §4).
            content.paragraphs.remove(index);
            parts.deleted.push(input.pid.clone());
            // Removing the open last paragraph closes the tail.
            if content.tail_open && index >= content.paragraphs.len() {
                content.tail_open = false;
            }
        } else {
            content.paragraphs[index].text = updated;
            parts.appended_to = Some(input.pid.clone());
        }
        let after_text = content
            .paragraphs
            .get(index)
            .map(|p| p.text.clone())
            .unwrap_or_default();
        let pending = stage_content(ctx, content.clone())?;
        // `replace_text` renders through its own action branch; the pid travels as `pid`.
        let (target_min, target_max) = targets(ctx);
        let text = ctx
            .prompts
            .render_value(
                "result.draft_edit",
                crate::prompts::template_value(&json!({
                    "action": "replace_text",
                    "pid": input.pid,
                    "deleted": !parts.deleted.is_empty(),
                    "inserted": parts.inserted,
                    "appended_to": parts.appended_to,
                    "after": Value::Null,
                    "tail_open": content.tail_open,
                    "last_pid": content.paragraphs.last().map(|p| p.pid.clone()).unwrap_or_default(),
                    "word_count": text::content_word_count(&content),
                    "target_min": target_min,
                    "target_max": target_max,
                    "chapter_end": false,
                    "short_of_target": false,
                    "ignored_chapter_end": false,
                })),
            )
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        Ok(ToolOutcome::text(text)
            .with_pending_content(pending)
            .with_ui(json!({
                "pid": input.pid,
                "before": paragraph,
                "after": after_text,
            })))
    }
}

// ───────────────────────────── rewrite_paragraph ─────────────────────────────

/// `rewrite_paragraph` (spec/05 §4): replace a whole paragraph; line breaks split the text.
pub struct RewriteParagraphTool;

#[async_trait::async_trait]
impl ToolHandler for RewriteParagraphTool {
    fn name(&self) -> &'static str {
        "rewrite_paragraph"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Draft
    }

    fn preview_field(&self) -> Option<&'static str> {
        Some("text")
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: RewriteParagraphInput = decode(input)?;
        let rules = TextRules::of(ctx);
        let parts_text = validate_text(
            ctx.prompts,
            rules.mode,
            rules.paragraph_edit(),
            "rewrite_paragraph",
            &input.text,
        )?;
        let mut content = content_of(chapter_of(ctx)?);
        let index = pid_index(&content, &input.pid)?;
        let before = content.paragraphs[index].text.clone();
        // The first part keeps the pid, the rest get new pids after it (spec/05 §4).
        content.paragraphs[index].text = parts_text[0].clone();
        let mut after = input.pid.clone();
        let mut inserted = Vec::new();
        for part in parts_text.iter().skip(1) {
            let pid = text::allocate_pid(&mut content);
            let position = pid_index(&content, &after)? + 1;
            content.paragraphs.insert(
                position,
                Paragraph {
                    pid: pid.clone(),
                    text: part.clone(),
                },
            );
            after = pid.clone();
            inserted.push(pid);
        }
        let pending = stage_content(ctx, content.clone())?;
        let text = render_draft_edit(
            ctx,
            &content,
            "rewrite_paragraph",
            &EditParts {
                inserted: inserted.clone(),
                appended_to: Some(input.pid.clone()),
                ..EditParts::default()
            },
            None,
        )?;
        Ok(ToolOutcome::text(text)
            .with_pending_content(pending)
            .with_ui(json!({
                "pid": input.pid,
                "inserted": inserted,
                "before": before,
            })))
    }
}

// ───────────────────────────── delete_paragraphs ─────────────────────────────

/// `delete_paragraphs` (spec/05 §4): delete every named paragraph, or nothing at all.
pub struct DeleteParagraphsTool;

#[async_trait::async_trait]
impl ToolHandler for DeleteParagraphsTool {
    fn name(&self) -> &'static str {
        "delete_paragraphs"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Draft
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: DeleteParagraphsInput = decode(input)?;
        let mut content = content_of(chapter_of(ctx)?);
        // Unknown pids delete nothing at all (spec/05 §4).
        let missing: Vec<String> = input
            .pids
            .iter()
            .filter(|pid| !content.paragraphs.iter().any(|p| &p.pid == *pid))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(ToolError::new(ToolErrorKind::PidNotFound).with("pids", missing));
        }
        let open_last = content
            .paragraphs
            .last()
            .map(|p| p.pid.clone())
            .filter(|pid| input.pids.contains(pid));
        if open_last.is_some() && content.tail_open {
            content.tail_open = false;
        }
        for pid in &input.pids {
            content.paragraphs.retain(|paragraph| &paragraph.pid != pid);
        }
        let pending = stage_content(ctx, content.clone())?;
        let text = render_draft_edit(
            ctx,
            &content,
            "delete_paragraphs",
            &EditParts {
                deleted: input.pids.clone(),
                ..EditParts::default()
            },
            None,
        )?;
        Ok(ToolOutcome::text(text)
            .with_pending_content(pending)
            .with_ui(json!({ "deleted": input.pids })))
    }
}

// ───────────────────────────── finish_draft ─────────────────────────────

/// `finish_draft` (spec/05 §4): complete the draft, confirming a short one once.
pub struct FinishDraftTool;

#[async_trait::async_trait]
impl ToolHandler for FinishDraftTool {
    fn name(&self) -> &'static str {
        "finish_draft"
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
        let input: FinishDraftInput = decode(input)?;
        let mut content = content_of(chapter_of(ctx)?);
        if content.paragraphs.is_empty() {
            return Err(ToolError::new(ToolErrorKind::DraftEmpty));
        }
        // The draft must be non-empty and finishing closes an open tail (spec/05 §4). Closing the
        // tail is a real content change, so it is staged like any other write.
        let closing_tail = content.tail_open;
        if content.tail_open {
            content.tail_open = false;
        }
        let pending = if closing_tail {
            Some(stage_content(ctx, content.clone())?)
        } else {
            None
        };
        let word_count = text::content_word_count(&content);
        let confirm_needed = !finish_warned(ctx) && is_short_completion(ctx, word_count);
        let (target_min, target_max) = targets(ctx);
        let text = ctx
            .prompts
            .render_value(
                "result.writer_control",
                crate::prompts::template_value(&json!({
                    "action": "finish_draft",
                    "word_count": word_count,
                    "target_min": target_min,
                    "target_max": target_max,
                    "confirm_needed": confirm_needed,
                    "title_set": false,
                    "title": "",
                })),
            )
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        // `note` is read from the call's input by the completion handler that writes the `draft`
        // version (spec/08 §1.1), so the tool only needs to have it validated by `decode`.
        let mut outcome = ToolOutcome::text(text)
            .with_ui(json!({ "word_count": word_count, "confirm_needed": confirm_needed }));
        if confirm_needed {
            outcome = outcome.with_finish_warned();
        } else {
            outcome = outcome.with_effects(ToolEffects {
                complete: Some(CompletionKind::Draft),
                ..ToolEffects::default()
            });
        }
        // `with_effects` keeps the staged write, so the order of these two does not matter.
        if let Some(pending) = pending {
            outcome = outcome.with_pending_content(pending);
        }
        let _ = input.note;
        Ok(outcome)
    }
}

// ───────────────────────────── revision tools ─────────────────────────────

/// `submit_revision_plan` (spec/05 §4): record the plan and enter revising.
pub struct SubmitRevisionPlanTool;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitRevisionPlanInput {
    plan: String,
}

#[async_trait::async_trait]
impl ToolHandler for SubmitRevisionPlanTool {
    fn name(&self) -> &'static str {
        "submit_revision_plan"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Control
    }

    fn preview_field(&self) -> Option<&'static str> {
        Some("plan")
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: SubmitRevisionPlanInput = decode(input)?;
        let text = ctx
            .prompts
            .render_value(
                "result.writer_control",
                crate::prompts::template_value(&json!({ "action": "submit_revision_plan" })),
            )
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        Ok(ToolOutcome::text(text)
            .with_ui(json!({ "plan_chars": input.plan.chars().count() }))
            .with_effects(ToolEffects {
                phase_to: Some(crate::db::json::Phase::Revising),
                ..ToolEffects::default()
            }))
    }
}

/// `finish_revision` (spec/05 §4): complete the revision. M6.2 owns its handler.
pub struct FinishRevisionTool;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FinishRevisionInput {
    #[serde(default)]
    summary: String,
}

#[async_trait::async_trait]
impl ToolHandler for FinishRevisionTool {
    fn name(&self) -> &'static str {
        "finish_revision"
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
        let input: FinishRevisionInput = decode(input)?;
        let content = content_of(chapter_of(ctx)?);
        if content.paragraphs.is_empty() {
            return Err(ToolError::new(ToolErrorKind::DraftEmpty));
        }
        let word_count = text::content_word_count(&content);
        let text = ctx
            .prompts
            .render_value(
                "result.writer_control",
                crate::prompts::template_value(&json!({
                    "action": "finish_revision",
                    "word_count": word_count,
                })),
            )
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        let _ = input.summary;
        Ok(ToolOutcome::text(text)
            .with_ui(json!({ "word_count": word_count }))
            .with_effects(ToolEffects {
                complete: Some(CompletionKind::Revision),
                ..ToolEffects::default()
            }))
    }
}

// ───────────────────────────── registration ─────────────────────────────

/// Every writer tool of this milestone.
pub fn all() -> Vec<std::sync::Arc<dyn ToolHandler>> {
    vec![
        std::sync::Arc::new(SubmitOutlineTool),
        std::sync::Arc::new(SubmitRevisionPlanTool),
        std::sync::Arc::new(ReadDraftTool),
        std::sync::Arc::new(InsertTextTool),
        std::sync::Arc::new(InsertParagraphsTool),
        std::sync::Arc::new(ReplaceTextTool),
        std::sync::Arc::new(RewriteParagraphTool),
        std::sync::Arc::new(DeleteParagraphsTool),
        std::sync::Arc::new(FinishDraftTool),
        std::sync::Arc::new(FinishRevisionTool),
    ]
}

/// Maps a text-layer failure to the model-facing tool error.
fn map_text_error(error: text::TextError) -> ToolError {
    match error {
        text::TextError::PidNotFound(pid) => {
            ToolError::new(ToolErrorKind::PidNotFound).with("pids", vec![pid])
        }
        text::TextError::InvalidPid(pid) => {
            ToolError::new(ToolErrorKind::PidNotFound).with("pids", vec![pid])
        }
        // `validate_text` rejects empty segments before they reach the text layer.
        text::TextError::EmptyText(_) => ToolError::new(ToolErrorKind::Granularity)
            .with("mode", Granularity::Paragraph.as_str())
            .with("reason", "empty"),
        text::TextError::PidExhausted => ToolError::new(ToolErrorKind::Internal),
    }
}

/// The error the model gets for a granularity violation (spec/03 §8 `error.granularity`).
pub fn granularity_error(
    prompts: &crate::prompts::PromptSet,
    mode: Granularity,
    reason: &str,
    tool: &str,
) -> ToolError {
    ToolError::new(ToolErrorKind::Granularity)
        .with("mode", mode.as_str())
        .with("mode_label", prompts.label("granularity", mode.as_str()))
        .with("tool", tool.to_owned())
        .with("reason", reason.to_owned())
}

/// Builds a resolver over the book's visible entities (used by `read_chapter`, M5).
pub fn resolver_for(entities: Vec<super::resolve::VisibleEntity>) -> EntityResolver {
    EntityResolver::new(entities)
}

/// Re-export so the tests can build a content row the same way the tools do.
pub use crate::db::json::ChapterContent as Content;
