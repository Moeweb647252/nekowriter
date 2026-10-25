//! The four search tools (spec/05 §3): `search_canon`, `search_knowledge`, `read_knowledge_entry`
//! and `search_chapters`.
//!
//! Every tool runs the hybrid query of spec/07 §5: the query vector is computed in `prepare`
//! (outside any transaction, spec/05 §1.2) and the fused results are rendered from
//! `prompts/results.toml`. When the embedder is not ready the tools answer from the FTS list and
//! the result says that only keyword matching ran.
//!
//! Visibility: book searches only see shared documents plus the session's isolated track's own
//! (spec/06 §2); knowledge searches only see the book's attached scopes, or the research
//! session's target scope (spec/07 §1, spec/05 §8).

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::agent::tools::{ToolError, ToolErrorKind, Visibility};
use crate::db::entities::{chapters, knowledge_entries, knowledge_scopes, research_tasks, tracks};
use crate::db::json::SessionKind;
use crate::search::{BookCorpus, BookSearch, hybrid_book, hybrid_knowledge};
use crate::services::canon;

use super::{PrepareCtx, Prepared, ToolCtx, ToolHandler, ToolKind, ToolOutcome};

/// `search_canon` default and maximum (spec/05 §3).
const CANON_DEFAULT: usize = 8;
const CANON_MAX: usize = 20;
/// `search_knowledge` and `search_chapters` default and maximum (spec/05 §3).
const KNOWLEDGE_DEFAULT: usize = 6;
const KNOWLEDGE_MAX: usize = 15;
/// `read_knowledge_entry` page size (spec/05 §3).
pub const ENTRY_READ_CHARS: usize = 6_000;
/// A knowledge snippet in a tool result (spec/05 §3).
pub const KNOWLEDGE_SNIPPET_CHARS: usize = 400;

/// The five kinds `search_canon` may return (spec/05 §3).
const CANON_KINDS: [&str; 5] = ["entity", "note", "foreshadowing", "event", "summary"];

// ───────────────────────────── input types ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchCanonInput {
    query: String,
    #[serde(default)]
    kinds: Option<Vec<String>>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchKnowledgeInput {
    query: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadKnowledgeEntryInput {
    entry_id: i32,
    #[serde(default)]
    offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchChaptersInput {
    query: String,
    #[serde(default)]
    limit: Option<i64>,
}

fn decode<T: for<'de> Deserialize<'de>>(input: Value) -> Result<T, ToolError> {
    serde_json::from_value(input).map_err(|err| {
        ToolError::new(ToolErrorKind::Validation).with("errors", vec![err.to_string()])
    })
}

// ───────────────────────────── shared helpers ─────────────────────────────

/// The book a tool result belongs to (spec/05 §1.2).
fn book_id_of(ctx: &ToolCtx<'_>) -> Option<i32> {
    ctx.book.map(|book| book.id).or(ctx.session.book_id)
}

/// The isolated track whose private documents are visible (spec/06 §2).
fn isolated_track_of(ctx: &ToolCtx<'_>) -> Option<i32> {
    match ctx.visibility {
        Visibility::Isolated { track_id } => Some(track_id),
        Visibility::Shared => None,
    }
}

fn internal() -> ToolError {
    ToolError::new(ToolErrorKind::Internal)
}

/// Renders `result.search_hits` (spec/09 §2).
fn render_hits(
    ctx: &ToolCtx<'_>,
    corpus: &str,
    query: &str,
    hits: Value,
    vectors_available: bool,
) -> Result<ToolOutcome, ToolError> {
    let text = ctx
        .prompts
        .render_value(
            "result.search_hits",
            crate::prompts::template_value(&json!({
                "corpus": corpus,
                "query": query,
                "hits": hits,
                "vector_available": vectors_available,
            })),
        )
        .map_err(|_| internal())?
        .text;
    Ok(ToolOutcome::text(text))
}

// ───────────────────────────── search_canon ─────────────────────────────

/// `search_canon` (spec/05 §3): hybrid search over the visible book archive, passages excluded.
pub struct SearchCanonTool;

#[async_trait::async_trait]
impl ToolHandler for SearchCanonTool {
    fn name(&self) -> &'static str {
        "search_canon"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Retrieval
    }

    async fn prepare(&self, ctx: &PrepareCtx<'_>, input: &Value) -> Result<Prepared, ToolError> {
        prepare_query_vector(ctx, input).await
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: SearchCanonInput = decode(input)?;
        let query = input.query.trim();
        if query.is_empty() {
            return Err(ToolError::new(ToolErrorKind::Validation)
                .with("errors", vec!["query must not be empty".to_owned()]));
        }
        let limit = input
            .limit
            .map(|value| value.clamp(1, CANON_MAX as i64) as usize)
            .unwrap_or(CANON_DEFAULT);
        let kinds = match input.kinds {
            Some(kinds) => {
                let mut cleaned = Vec::new();
                for kind in kinds {
                    if !CANON_KINDS.contains(&kind.as_str()) {
                        return Err(ToolError::new(ToolErrorKind::Validation)
                            .with("errors", vec![format!("unknown search kind {kind}")]));
                    }
                    cleaned.push(kind);
                }
                Some(cleaned)
            }
            None => None,
        };
        let Some(book_id) = book_id_of(ctx) else {
            return Err(internal());
        };
        let vector = query_vector(prepared);
        let hits = hybrid_book(
            ctx.tx,
            query,
            vector.as_deref(),
            &BookSearch {
                book_id,
                corpus: BookCorpus::Canon,
                kinds: kinds.as_deref(),
                isolated_track: isolated_track_of(ctx),
                limit,
            },
        )
        .await
        .map_err(|_| internal())?;
        let rendered: Vec<Value> = hits
            .hits
            .iter()
            .map(|hit| {
                json!({
                    "ref": hit.ref_id,
                    "title": hit.title,
                    "kind_label": ctx.prompts.label("search_kind", &hit.kind),
                    "snippet": hit.snippet,
                    "after_anchor": false,
                })
            })
            .collect();
        let count = rendered.len();
        let outcome = render_hits(
            ctx,
            "canon",
            query,
            Value::Array(rendered),
            hits.vectors_available,
        )?;
        Ok(outcome.with_ui(json!({ "query": query, "count": count })))
    }
}

// ───────────────────────────── search_knowledge ─────────────────────────────

/// `search_knowledge` (spec/05 §3, §8): hybrid search over the visible scopes, or the target scope
/// of a research session.
pub struct SearchKnowledgeTool;

#[async_trait::async_trait]
impl ToolHandler for SearchKnowledgeTool {
    fn name(&self) -> &'static str {
        "search_knowledge"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Knowledge
    }

    async fn prepare(&self, ctx: &PrepareCtx<'_>, input: &Value) -> Result<Prepared, ToolError> {
        prepare_query_vector(ctx, input).await
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: SearchKnowledgeInput = decode(input)?;
        let query = input.query.trim();
        if query.is_empty() {
            return Err(ToolError::new(ToolErrorKind::Validation)
                .with("errors", vec!["query must not be empty".to_owned()]));
        }
        let limit = input
            .limit
            .map(|value| value.clamp(1, KNOWLEDGE_MAX as i64) as usize)
            .unwrap_or(KNOWLEDGE_DEFAULT);
        let visible = visible_scopes(ctx).await?;
        let available: Vec<String> = visible.iter().map(|scope| scope.name.clone()).collect();
        let selected: Vec<i32> = match input.scope.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => {
                let found = visible
                    .iter()
                    .find(|scope| scope.name == name)
                    .or_else(|| {
                        visible
                            .iter()
                            .find(|scope| scope.name.eq_ignore_ascii_case(name))
                    })
                    .ok_or_else(|| {
                        ToolError::new(ToolErrorKind::ScopeNotFound)
                            .with("name", name.to_owned())
                            .with("available", available.clone())
                    })?;
                vec![found.id]
            }
            _ => visible.iter().map(|scope| scope.id).collect(),
        };
        let vector = query_vector(prepared);
        let hits = hybrid_knowledge(ctx.tx, query, vector.as_deref(), &selected, limit)
            .await
            .map_err(|_| internal())?;
        let rendered: Vec<Value> = hits
            .hits
            .iter()
            .map(|hit| {
                json!({
                    "ref": hit.entry_id,
                    "title": hit.title,
                    "location": hit.scope_name,
                    "snippet": truncate_chars(&hit.snippet, KNOWLEDGE_SNIPPET_CHARS),
                    "after_anchor": false,
                })
            })
            .collect();
        let count = rendered.len();
        let outcome = render_hits(
            ctx,
            "knowledge",
            query,
            Value::Array(rendered),
            hits.vectors_available,
        )?;
        Ok(outcome.with_ui(json!({ "query": query, "count": count })))
    }
}

// ───────────────────────────── read_knowledge_entry ─────────────────────────────

/// `read_knowledge_entry` (spec/05 §3): the entry text from `offset`, 6,000 characters at a time.
pub struct ReadKnowledgeEntryTool;

#[async_trait::async_trait]
impl ToolHandler for ReadKnowledgeEntryTool {
    fn name(&self) -> &'static str {
        "read_knowledge_entry"
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
        let input: ReadKnowledgeEntryInput = decode(input)?;
        let offset = input.offset.unwrap_or(0).max(0) as usize;
        let visible = visible_scopes(ctx).await?;
        let available: Vec<String> = visible.iter().map(|scope| scope.name.clone()).collect();
        let entry = knowledge_entries::Entity::find_by_id(input.entry_id)
            .one(ctx.tx)
            .await
            .map_err(|_| internal())?
            .ok_or_else(|| {
                ToolError::new(ToolErrorKind::NotFound)
                    .with(
                        "what_label",
                        ctx.prompts.label("target_type", "knowledge_entry"),
                    )
                    .with("id", input.entry_id)
            })?;
        let Some(scope) = visible.iter().find(|scope| scope.id == entry.scope_id) else {
            let scope_name = knowledge_scopes::Entity::find_by_id(entry.scope_id)
                .one(ctx.tx)
                .await
                .map_err(|_| internal())?
                .map(|row| row.name)
                .unwrap_or_default();
            return Err(ToolError::new(ToolErrorKind::ScopeNotFound)
                .with("name", scope_name)
                .with("available", available));
        };
        let total = entry.content.chars().count();
        let start = offset.min(total);
        let content: String = entry
            .content
            .chars()
            .skip(start)
            .take(ENTRY_READ_CHARS)
            .collect();
        let end = start + content.chars().count();
        let next_offset = if end < total { Some(end) } else { None };
        let sources: Vec<String> = crate::db::json::from_json_or_default(&entry.source_urls);
        let text = ctx
            .prompts
            .render_value(
                "result.knowledge_entry",
                crate::prompts::template_value(&json!({
                    "id": entry.id,
                    "title": entry.title,
                    "scope": scope.name,
                    "sources": sources,
                    "content": content,
                    "offset": start,
                    "end": end,
                    "total": total,
                    "next_offset": next_offset,
                })),
            )
            .map_err(|_| internal())?
            .text;
        Ok(ToolOutcome::text(text).with_ui(json!({
            "entry_id": entry.id,
            "title": entry.title,
        })))
    }
}

// ───────────────────────────── search_chapters ─────────────────────────────

/// `search_chapters` (spec/05 §3): hybrid search over official passages of the visible tracks.
pub struct SearchChaptersTool;

#[async_trait::async_trait]
impl ToolHandler for SearchChaptersTool {
    fn name(&self) -> &'static str {
        "search_chapters"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Retrieval
    }

    async fn prepare(&self, ctx: &PrepareCtx<'_>, input: &Value) -> Result<Prepared, ToolError> {
        prepare_query_vector(ctx, input).await
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: SearchChaptersInput = decode(input)?;
        let query = input.query.trim();
        if query.is_empty() {
            return Err(ToolError::new(ToolErrorKind::Validation)
                .with("errors", vec!["query must not be empty".to_owned()]));
        }
        let limit = input
            .limit
            .map(|value| value.clamp(1, KNOWLEDGE_MAX as i64) as usize)
            .unwrap_or(KNOWLEDGE_DEFAULT);
        let Some(book_id) = book_id_of(ctx) else {
            return Err(internal());
        };
        let vector = query_vector(prepared);
        let hits = hybrid_book(
            ctx.tx,
            query,
            vector.as_deref(),
            &BookSearch {
                book_id,
                corpus: BookCorpus::Passage,
                kinds: None,
                isolated_track: isolated_track_of(ctx),
                limit,
            },
        )
        .await
        .map_err(|_| internal())?;
        let anchor = anchor_of(ctx).await?;
        let mut rendered = Vec::with_capacity(hits.hits.len());
        for hit in &hits.hits {
            let label = canon::chapter_label(ctx.tx, ctx.prompts, Some(hit.ref_id))
                .await
                .map_err(|_| internal())?;
            let pids: Vec<String> = parse_pids(&hit.meta);
            let after_anchor = anchor.marks(hit.ref_id, ctx.tx).await?;
            rendered.push(json!({
                "ref": hit.ref_id,
                "title": label,
                "pids": pids,
                "snippet": hit.snippet,
                "after_anchor": after_anchor,
            }));
        }
        let count = rendered.len();
        let outcome = render_hits(
            ctx,
            "chapters",
            query,
            Value::Array(rendered),
            hits.vectors_available,
        )?;
        Ok(outcome.with_ui(json!({ "query": query, "count": count })))
    }
}

// ───────────────────────────── helpers ─────────────────────────────

/// Computes the query vector in the `prepare` phase (spec/05 §1.2, spec/07 §5). It never errors:
/// no embedder means keyword-only search.
async fn prepare_query_vector(ctx: &PrepareCtx<'_>, input: &Value) -> Result<Prepared, ToolError> {
    let Some(query) = input.get("query").and_then(Value::as_str) else {
        return Ok(Prepared::None);
    };
    if query.trim().is_empty() {
        return Ok(Prepared::None);
    }
    match ctx.state.search().query_vector(query).await {
        Some(vector) => Ok(Prepared::QueryVector(vector)),
        None => Ok(Prepared::None),
    }
}

fn query_vector(prepared: Prepared) -> Option<Vec<f32>> {
    match prepared {
        Prepared::QueryVector(vector) => Some(vector),
        _ => None,
    }
}

/// The scopes the session may search (spec/07 §1): the book's attached scopes, or the research
/// session's target scope.
async fn visible_scopes(ctx: &ToolCtx<'_>) -> Result<Vec<knowledge_scopes::Model>, ToolError> {
    let ids: Vec<i32> = if ctx.kind() == SessionKind::Research {
        let Some(task_id) = ctx.session.research_task_id else {
            return Err(ToolError::new(ToolErrorKind::NoKnowledgeScope));
        };
        let task = research_tasks::Entity::find_by_id(task_id)
            .one(ctx.tx)
            .await
            .map_err(|_| internal())?
            .ok_or_else(|| ToolError::new(ToolErrorKind::NoKnowledgeScope))?;
        match task.scope_id {
            Some(scope_id) => vec![scope_id],
            None => return Err(ToolError::new(ToolErrorKind::NoKnowledgeScope)),
        }
    } else {
        let Some(book_id) = book_id_of(ctx) else {
            return Err(ToolError::new(ToolErrorKind::NoKnowledgeScope));
        };
        crate::services::knowledge::visible_scope_ids(ctx.tx, book_id)
            .await
            .map_err(|_| internal())?
    };
    if ids.is_empty() {
        return Err(ToolError::new(ToolErrorKind::NoKnowledgeScope));
    }
    knowledge_scopes::Entity::find()
        .filter(knowledge_scopes::Column::Id.is_in(ids))
        .all(ctx.tx)
        .await
        .map_err(|_| internal())
}

/// Truncates a string to `max` characters.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    text.chars().take(max).collect()
}

/// The `meta.pids` of a passage document (spec/07 §2).
fn parse_pids(meta: &str) -> Vec<String> {
    let value: Value = serde_json::from_str(meta).unwrap_or(Value::Null);
    value
        .get("pids")
        .and_then(Value::as_array)
        .map(|pids| {
            pids.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

// ───────────────────────────── anchor marking (spec/06 §2, spec/08 §6) ─────────────────────────────

/// The session track's anchor (`none` for the main track).
struct Anchor {
    kind: String,
    /// The anchor chapter's main-track `ord` for `after_chapter`.
    ord: Option<i32>,
    /// The book's main track id.
    main_track: Option<i32>,
}

impl Anchor {
    /// True when a chapter of the main story comes after this side story's anchor and must be
    /// marked (spec/06 §2, spec/08 §6).
    async fn marks(
        &self,
        chapter_id: i32,
        conn: &impl sea_orm::ConnectionTrait,
    ) -> Result<bool, ToolError> {
        if self.kind == "none" {
            return Ok(false);
        }
        let Some(chapter) = chapters::Entity::find_by_id(chapter_id)
            .one(conn)
            .await
            .map_err(|_| internal())?
        else {
            return Ok(false);
        };
        if Some(chapter.track_id) != self.main_track {
            return Ok(false);
        }
        match self.kind.as_str() {
            "before_start" => Ok(true),
            "after_chapter" => Ok(self.ord.is_some_and(|ord| chapter.ord > ord)),
            _ => Ok(false),
        }
    }
}

/// Reads the session track's anchor.
async fn anchor_of(ctx: &ToolCtx<'_>) -> Result<Anchor, ToolError> {
    let mut anchor = Anchor {
        kind: "none".to_owned(),
        ord: None,
        main_track: None,
    };
    let Some(track) = ctx.track else {
        return Ok(anchor);
    };
    if track.kind != "side" || track.anchor_kind == "none" {
        return Ok(anchor);
    }
    let book_id = track.book_id;
    anchor.main_track = tracks::Entity::find()
        .filter(tracks::Column::BookId.eq(book_id))
        .filter(tracks::Column::Kind.eq("main"))
        .one(ctx.tx)
        .await
        .map_err(|_| internal())?
        .map(|row| row.id);
    anchor.kind = track.anchor_kind.clone();
    if let Some(chapter_id) = track.anchor_chapter_id
        && let Some(chapter) = chapters::Entity::find_by_id(chapter_id)
            .one(ctx.tx)
            .await
            .map_err(|_| internal())?
    {
        anchor.ord = Some(chapter.ord);
    }
    Ok(anchor)
}

// ───────────────────────────── registration ─────────────────────────────

/// Every search tool of this milestone (spec/05 §3).
pub fn all() -> Vec<std::sync::Arc<dyn ToolHandler>> {
    vec![
        std::sync::Arc::new(SearchCanonTool),
        std::sync::Arc::new(SearchKnowledgeTool),
        std::sync::Arc::new(ReadKnowledgeEntryTool),
        std::sync::Arc::new(SearchChaptersTool),
    ]
}
