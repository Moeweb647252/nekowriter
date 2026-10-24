//! Context assembly and request building (spec/03 §10–§11).
//!
//! Ordering principle: **stable first**. System prompt (static per role) → setting materials
//! (stable per book revision) → memory (changes per chapter) → task (changes per session).
//!
//! The kickoff is a list of blocks per session kind (§10 table); each block is one `text` block of
//! the kickoff user message. `build_request` then turns the persisted session into a
//! [`nekowriter_llm::ChatRequest`] **from persisted state only**, so building it twice yields the
//! same bytes (spec/03 §15 test 14: append-only history proves a previous step's request is a
//! byte-identical prefix of the current one).

use std::collections::BTreeMap;

use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};

use crate::db::entities::{
    agent_messages, agent_sessions, agent_tool_calls, books, chapter_versions, chapters,
    character_designs, entities, research_tasks, setting_docs, tracks,
};
use crate::db::json::{
    self, AgentRole, ChapterContent, CompletionKind, MessageKind, MessageRole, Phase, Protocol,
    RequestMeta, SessionKind, StoredBlock, ToolCallStatus, VersionKind,
};
use crate::error::{AppError, ErrorCode};
use crate::events::ServerEvent;
use crate::prompts::PromptSet;
use crate::prompts::context::{
    BookContext, CastEntryContext, ChapterContext, CharacterContext, DraftContext, FeedbackContext,
    FeedbackIssueContext, GranularityContext, MemoryContext, MemoryCountsContext, ParagraphContext,
    ScopeContext, SettingsContext, TrackContext,
};
use crate::text;

use super::session;

/// The kickoff blocks of every session kind, in order (spec/03 §10 table).
///
/// `block.settings` carries a cache breakpoint (`cache: true`).
pub fn kickoff_blocks(kind: SessionKind, redraft: bool) -> Vec<&'static str> {
    match kind {
        SessionKind::Conception => vec!["conception.kickoff"],
        SessionKind::Research => vec!["block.book_brief", "researcher.kickoff"],
        SessionKind::ChapterWrite => {
            if redraft {
                vec!["block.settings", "block.memory", "writer.kickoff.redraft"]
            } else {
                vec!["block.settings", "block.memory", "writer.kickoff.chapter"]
            }
        }
        SessionKind::ChapterRevise => vec![
            "block.settings",
            "block.memory",
            "block.chapter_draft",
            "writer.kickoff.revise",
        ],
        SessionKind::ChapterReview => vec![
            "block.settings",
            "block.memory",
            "block.chapter_draft",
            "reviewer.kickoff",
        ],
        SessionKind::ChapterMaintenance => vec![
            "block.settings",
            "block.memory",
            "block.cast_index",
            "block.chapter_draft",
            "archivist.kickoff.maintenance",
        ],
        SessionKind::CanonSeed => vec!["block.settings", "archivist.kickoff.seed"],
        SessionKind::ChapterResync => vec![
            "block.settings",
            "block.memory",
            "block.cast_index",
            "block.chapter_diff",
            "archivist.kickoff.resync",
        ],
    }
}

/// True when the kickoff block of that key carries a cache breakpoint (spec/03 §10: the settings
/// block, shared by every session of the book).
pub fn block_is_cached(key: &str) -> bool {
    key == "block.settings"
}

/// What a kickoff renders (spec/03 §10).
///
/// Two templates include other blocks through MiniJinja: `block.anchor` (every writer, reviewer
/// and archivist task of a side-track chapter) and `block.chapter_draft` (`writer.kickoff.chapter`
/// when the chapter already has text). `block.memory` needs to know whether the outline is
/// already rendered elsewhere so it never appears twice (spec/06 §4.2), which is why the plan
/// travels with the session instead of being re-derived per block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KickoffPlan {
    pub kind: SessionKind,
    pub redraft: bool,
}

impl KickoffPlan {
    pub fn new(kind: SessionKind, redraft: bool) -> Self {
        Self { kind, redraft }
    }

    /// The top-level kickoff blocks of spec/03 §10.
    pub fn blocks(self) -> Vec<&'static str> {
        kickoff_blocks(self.kind, self.redraft)
    }

    /// True when a `block.chapter_draft` renders the chapter's outline somewhere in the kickoff
    /// (as a top-level block, or through `writer.kickoff.chapter`'s include when the chapter has
    /// text). `block.memory` then only names the chapter (spec/06 §4.2).
    pub fn renders_outline_elsewhere(self, has_draft: bool) -> bool {
        match self.kind {
            // `writer.kickoff.chapter` includes `block.chapter_draft` when the chapter has text,
            // and `writer.kickoff.redraft` shows the old outline itself (spec/06 §4.2).
            SessionKind::ChapterWrite => has_draft || self.redraft,
            SessionKind::ChapterRevise
            | SessionKind::ChapterReview
            | SessionKind::ChapterMaintenance => true,
            _ => false,
        }
    }
}

/// The rendered kickoff message of a session kind: one `text` block per kickoff block
/// (spec/03 §10).
pub async fn assemble_kickoff(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    session: &agent_sessions::Model,
    redraft: bool,
) -> Result<Vec<StoredBlock>, AppError> {
    let plan = KickoffPlan::new(session::kind_of(session), redraft);
    let mut blocks = Vec::new();
    for key in plan.blocks() {
        let context = block_context(conn, prompts, session, plan, key).await?;
        let rendered = prompts
            .render_value(key, crate::prompts::template_value(&context))
            .map_err(|err| err.into_app_error())?;
        if rendered.text.trim().is_empty() {
            continue;
        }
        // Only the settings block places a cache breakpoint (spec/03 §10); the others store no
        // flag, so `StoredBlock::cache` stays absent.
        blocks.push(StoredBlock::Text {
            text: rendered.text,
            cache: if block_is_cached(key) {
                Some(true)
            } else {
                None
            },
        });
    }
    Ok(blocks)
}

/// The template context of one kickoff block (spec/03 §10, spec/09 §2).
async fn block_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    session: &agent_sessions::Model,
    plan: KickoffPlan,
    key: &str,
) -> Result<Value, AppError> {
    match key {
        "block.settings" => Ok(json!({
            "book": book_context(conn, prompts, session.book_id).await?,
            "settings": settings_context(conn, session.book_id).await?,
        })),
        "block.memory" => {
            let chapter =
                chapter_context(conn, prompts, session.chapter_id, session.book_id).await?;
            let chapter = memory_chapter_view(chapter, plan);
            Ok(json!({
                "memory": memory_context(conn, prompts, session).await?,
                "chapter": chapter,
            }))
        }
        "block.chapter_draft" => Ok(json!({
            "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
            "draft": draft_context(conn, session.chapter_id).await?,
        })),
        "block.cast_index" => Ok(json!({
            "cast_index": cast_index_context(conn, prompts, session).await?,
        })),
        "block.chapter_diff" => Ok(json!({
            "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
            "diff": json!({ "changes": [] }),
        })),
        "block.anchor" => Ok(json!({
            "track": track_context(conn, prompts, session.track_id).await?,
        })),
        "block.book_brief" => Ok(json!({
            "book": book_context(conn, prompts, session.book_id).await?,
        })),
        "conception.kickoff" => Ok(json!({
            "conception": conception_context(conn, prompts, session.book_id).await?,
        })),
        "researcher.kickoff" => Ok(json!({
            "research": research_context(conn, prompts, session.research_task_id).await?,
        })),
        "writer.kickoff.chapter" => Ok(json!({
            "book": book_context(conn, prompts, session.book_id).await?,
            "track": track_context(conn, prompts, session.track_id).await?,
            "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
            "draft": draft_context(conn, session.chapter_id).await?,
        })),
        "writer.kickoff.redraft" => Ok(json!({
            "book": book_context(conn, prompts, session.book_id).await?,
            "track": track_context(conn, prompts, session.track_id).await?,
            "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
            "feedback": feedback_context(conn, prompts, session.chapter_id).await?,
            "previous_outline": previous_outline(conn, session.chapter_id).await?,
        })),
        "writer.kickoff.revise" => Ok(json!({
            "book": book_context(conn, prompts, session.book_id).await?,
            "track": track_context(conn, prompts, session.track_id).await?,
            "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
            "feedback": feedback_context(conn, prompts, session.chapter_id).await?,
        })),
        "writer.kickoff.rollover" => rollover_context(conn, prompts, session).await,
        "reviewer.kickoff" => Ok(json!({
            "book": book_context(conn, prompts, session.book_id).await?,
            "track": track_context(conn, prompts, session.track_id).await?,
            "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
            "prior": Value::Null,
        })),
        "archivist.kickoff.maintenance" => Ok(json!({
            "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
        })),
        "archivist.kickoff.seed" => Ok(json!({
            "book": book_context(conn, prompts, session.book_id).await?,
            "settings": settings_context(conn, session.book_id).await?,
        })),
        "archivist.kickoff.resync" => Ok(json!({
            "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
        })),
        other => Err(AppError::internal(format!(
            "no kickoff context builder for {other}"
        ))),
    }
}

/// `block.book_brief` and every task: the book (spec/09 §2 `vars.book`).
async fn book_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    book_id: Option<i32>,
) -> Result<BookContext, AppError> {
    let Some(book_id) = book_id else {
        return Ok(BookContext {
            title: String::new(),
            genre: String::new(),
            idea: String::new(),
            writing_language: "en".to_owned(),
            writing_language_name: prompts.label("language", "en"),
            unspaced: false,
        });
    };
    let book = books::Entity::find_by_id(book_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?;
    Ok(BookContext {
        title: book.title,
        genre: book.genre,
        idea: book.idea,
        writing_language_name: prompts.label("language", &book.writing_language),
        unspaced: text::is_unspaced(&book.writing_language),
        writing_language: book.writing_language,
    })
}

/// `block.anchor` and the kickoffs: the track (spec/09 §2 `vars.track`).
async fn track_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    track_id: Option<i32>,
) -> Result<TrackContext, AppError> {
    let Some(track_id) = track_id else {
        return Ok(empty_track_context());
    };
    let track = tracks::Entity::find_by_id(track_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("track {track_id}")))?;
    let anchor_label = match track.anchor_chapter_id {
        Some(chapter_id) => {
            let chapter = chapters::Entity::find_by_id(chapter_id)
                .one(conn)
                .await?
                .ok_or_else(|| AppError::not_found(format!("chapter {chapter_id}")))?;
            prompts.chapter_label(
                &track.kind,
                &track.title,
                i64::from(chapter.ord),
                &chapter.title,
                !chapter.title.is_empty(),
            )
        }
        None => String::new(),
    };
    Ok(TrackContext {
        is_main: track.kind == "main",
        title: if track.kind == "main" {
            String::new()
        } else {
            track.title.clone()
        },
        kind: track.kind,
        synopsis: track.synopsis,
        canon_mode: track.canon_mode,
        anchor_kind: track.anchor_kind,
        anchor_label,
    })
}

fn empty_track_context() -> TrackContext {
    TrackContext {
        is_main: true,
        kind: "main".to_owned(),
        title: String::new(),
        synopsis: String::new(),
        canon_mode: "canon".to_owned(),
        anchor_kind: "none".to_owned(),
        anchor_label: String::new(),
    }
}

/// The chapter of a task (spec/09 §2 `vars.chapter`).
async fn chapter_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    chapter_id: Option<i32>,
    book_id: Option<i32>,
) -> Result<ChapterContext, AppError> {
    let Some(chapter_id) = chapter_id else {
        return Ok(ChapterContext {
            id: 0,
            number: 0,
            label: String::new(),
            title: String::new(),
            guidance: String::new(),
            outline: String::new(),
            reference_notes: String::new(),
            target_min: 0,
            target_max: 0,
            word_count: 0,
            has_draft: false,
        });
    };
    let chapter = chapters::Entity::find_by_id(chapter_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("chapter {chapter_id}")))?;
    let track = tracks::Entity::find_by_id(chapter.track_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("track {}", chapter.track_id)))?;
    let settings = book_settings(conn, book_id.or(Some(chapter.book_id))).await?;
    let content: ChapterContent = json::from_json_or_default(&chapter.content);
    Ok(ChapterContext {
        id: chapter.id,
        number: chapter.ord,
        // The templates append `《title》` themselves when a title exists, so the label carries
        // none (spec/09 §1: `with_title` is for callers that need the whole reference in one
        // string, such as the memory window).
        label: prompts.chapter_label(
            &track.kind,
            &track.title,
            i64::from(chapter.ord),
            &chapter.title,
            false,
        ),
        title: chapter.title,
        guidance: chapter.guidance,
        outline: chapter.outline,
        reference_notes: chapter.reference_notes,
        target_min: settings.target_words_min,
        target_max: settings.target_words_max,
        word_count: chapter.word_count,
        has_draft: !content.paragraphs.is_empty(),
    })
}

/// The book's settings (defaults when there is no book).
async fn book_settings(
    conn: &impl ConnectionTrait,
    book_id: Option<i32>,
) -> Result<json::BookSettings, AppError> {
    let Some(book_id) = book_id else {
        return Ok(json::BookSettings::default());
    };
    let book = books::Entity::find_by_id(book_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?;
    Ok(json::from_json_or_default(&book.settings))
}

/// `block.settings`: the three documents, the character designs and the attached scopes.
async fn settings_context(
    conn: &impl ConnectionTrait,
    book_id: Option<i32>,
) -> Result<SettingsContext, AppError> {
    let Some(book_id) = book_id else {
        return Ok(SettingsContext {
            outline: String::new(),
            world: String::new(),
            protagonist: String::new(),
            characters: Vec::new(),
            scopes: Vec::new(),
        });
    };
    let docs = setting_docs::Entity::find()
        .filter(setting_docs::Column::BookId.eq(book_id))
        .all(conn)
        .await?;
    let mut outline = String::new();
    let mut world = String::new();
    let mut protagonist = String::new();
    for doc in docs {
        match doc.kind.as_str() {
            "outline" => outline = doc.content,
            "world" => world = doc.content,
            "protagonist" => protagonist = doc.content,
            _ => {}
        }
    }
    let characters = character_designs::Entity::find()
        .filter(character_designs::Column::BookId.eq(book_id))
        .order_by_asc(character_designs::Column::Rank)
        .all(conn)
        .await?
        .into_iter()
        .map(|row| CharacterContext {
            rank: row.rank,
            name: row.name,
            role_label: row.role_label,
            summary: row.summary,
            profile: row.profile,
        })
        .collect();
    Ok(SettingsContext {
        outline,
        world,
        protagonist,
        characters,
        // The knowledge scopes of a book arrive with M7 (spec/07 §1).
        scopes: Vec::<ScopeContext>::new(),
    })
}

/// `block.memory`: the tiered memory snapshot of the target chapter (spec/06 §4).
async fn memory_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    session: &agent_sessions::Model,
) -> Result<MemoryContext, AppError> {
    let Some(chapter_id) = session.chapter_id else {
        return Ok(empty_memory_context());
    };
    let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(conn).await? else {
        return Ok(empty_memory_context());
    };
    let Some(track) = tracks::Entity::find_by_id(chapter.track_id)
        .one(conn)
        .await?
    else {
        return Ok(empty_memory_context());
    };
    // The whole tiered snapshot of spec/06 §4, rendered by `services/memory.rs`.
    let snapshot = crate::services::memory::snapshot(conn, prompts, &track, &chapter).await?;
    Ok(crate::services::memory::memory_context(&snapshot))
}

/// The chapter view `block.memory` renders: label and title only, and the outline only when the
/// kickoff has no `block.chapter_draft` and the task does not show the old outline itself
/// (spec/06 §4.2, D-050).
fn memory_chapter_view(chapter: ChapterContext, plan: KickoffPlan) -> ChapterContext {
    let renders_outline = plan.renders_outline_elsewhere(chapter.has_draft);
    ChapterContext {
        label: chapter.label,
        title: chapter.title,
        // The template renders neither of these; blanking them documents that the memory block
        // does not restate what the task already says.
        guidance: String::new(),
        target_min: 0,
        target_max: 0,
        outline: if renders_outline {
            String::new()
        } else {
            chapter.outline
        },
        reference_notes: String::new(),
        word_count: chapter.word_count,
        has_draft: chapter.has_draft,
        id: chapter.id,
        number: chapter.number,
    }
}

fn empty_memory_context() -> MemoryContext {
    MemoryContext {
        is_first: true,
        window_mid: 10,
        recent_summaries: Vec::new(),
        previous_tail: None,
        protagonist: None,
        recent_changes: Vec::new(),
        appearances: Vec::new(),
        absent_important: Vec::new(),
        key_notes: Vec::new(),
        open_foreshadowing: Vec::new(),
        cast: Vec::new(),
        counts: MemoryCountsContext {
            character: 0,
            organization: 0,
            location: 0,
            item: 0,
            other: 0,
            event: 0,
            fact: 0,
            world_rule: 0,
            foreshadowing_open: 0,
        },
    }
}

/// `block.cast_index`: the visible entities the archivist matches against (spec/06 §4.5, M5).
async fn cast_index_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    session: &agent_sessions::Model,
) -> Result<Vec<CastEntryContext>, AppError> {
    let _ = (conn, prompts, session);
    Ok(Vec::new())
}

/// `block.chapter_draft` and the writer tasks: the current text with pids (spec/03 §10).
async fn draft_context(
    conn: &impl ConnectionTrait,
    chapter_id: Option<i32>,
) -> Result<DraftContext, AppError> {
    let Some(chapter_id) = chapter_id else {
        return Ok(DraftContext {
            paragraphs: Vec::new(),
            tail_open: false,
            word_count: 0,
            paragraph_count: 0,
        });
    };
    let chapter = chapters::Entity::find_by_id(chapter_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("chapter {chapter_id}")))?;
    let content: ChapterContent = json::from_json_or_default(&chapter.content);
    let paragraphs: Vec<ParagraphContext> = content
        .paragraphs
        .iter()
        .map(|paragraph| ParagraphContext {
            pid: paragraph.pid.clone(),
            text: paragraph.text.clone(),
        })
        .collect();
    Ok(DraftContext {
        paragraph_count: paragraphs.len() as i32,
        paragraphs,
        tail_open: content.tail_open,
        word_count: chapter.word_count,
    })
}

/// `feedback` (`vars.feedback`): the author's most recent feedback for the chapter, with the
/// issues they ticked (spec/08 §1.1 send-back).
///
/// The last `chapter_feedback` row of the chapter is the one a redraft or a revision addresses;
/// the `issues` are copied into `chapter_feedback.issues` at send-back time (M6.2), so this only
/// renders stored data. A chapter without feedback (a plain write) renders the empty feedback.
async fn feedback_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    chapter_id: Option<i32>,
) -> Result<FeedbackContext, AppError> {
    let Some(chapter_id) = chapter_id else {
        return Ok(FeedbackContext {
            text: String::new(),
            issues: Vec::new(),
        });
    };
    let feedback = crate::db::entities::chapter_feedback::Entity::find()
        .filter(crate::db::entities::chapter_feedback::Column::ChapterId.eq(chapter_id))
        .order_by_desc(crate::db::entities::chapter_feedback::Column::Id)
        .one(conn)
        .await?;
    let Some(feedback) = feedback else {
        return Ok(FeedbackContext {
            text: String::new(),
            issues: Vec::new(),
        });
    };
    Ok(FeedbackContext {
        text: feedback.text,
        issues: render_issues(prompts, &feedback.issues),
    })
}

/// Renders the stored `ReviewIssue[]` of a feedback row for `vars.feedback.issues`.
pub fn render_issues(prompts: &PromptSet, raw: &str) -> Vec<FeedbackIssueContext> {
    let issues: Vec<json::ReviewIssue> = json::from_json_or_default(raw);
    issues
        .into_iter()
        .map(|issue| FeedbackIssueContext {
            severity_label: prompts.label("severity", issue.severity.as_str()),
            category_label: prompts.label("category", issue.category.as_str()),
            pids: issue.pids,
            quote: issue.quote,
            problem: issue.problem,
            suggestion: issue.suggestion,
        })
        .collect()
}

/// `previous_outline` for `writer.kickoff.redraft`: the outline of the backed-up version.
///
/// The redraft's creation side effect saves the previous text as a `backup` version before it
/// clears the content; the outline itself is not part of a version, so it comes from the newest
/// `backup` version's sibling state. M6.2 records the old outline on the feedback row's report;
/// until then the chapter's current outline is the reference (it is what the author saw).
async fn previous_outline(
    conn: &impl ConnectionTrait,
    chapter_id: Option<i32>,
) -> Result<String, AppError> {
    let Some(chapter_id) = chapter_id else {
        return Ok(String::new());
    };
    let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(conn).await? else {
        return Ok(String::new());
    };
    Ok(chapter.outline)
}

/// `writer.kickoff.rollover` (spec/03 §9): the continuation task of a superseded session.
///
/// The accepted revision plan travels here rather than through the block list, because the
/// rollover blocks are fixed (`block.settings`, `block.memory`, `block.chapter_draft`, this task)
/// and the plan only exists for a revising session.
async fn rollover_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    session: &agent_sessions::Model,
) -> Result<Value, AppError> {
    let phase = session::phase_of(session);
    let revising = matches!(phase, Phase::RevisionPlanning | Phase::Revising);
    let content = chapter_content(conn, session.chapter_id).await?;
    let granularity = granularity_context(
        prompts,
        session::granularity_of(session),
        session::phase_of(session),
        &content,
    );
    Ok(json!({
        "book": book_context(conn, prompts, session.book_id).await?,
        "track": track_context(conn, prompts, session.track_id).await?,
        "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
        "feedback": feedback_context(conn, prompts, session.chapter_id).await?,
        "revision_plan": accepted_revision_plan(conn, session).await?,
        "author_notes": author_notes(conn, session).await?,
        "granularity": granularity,
        "phase": if revising { "revising" } else { "drafting" },
    }))
}

/// The accepted revision plan of a revising session (spec/03 §9): the `submit_revision_plan`
/// result the session's chain recorded.
async fn accepted_revision_plan(
    conn: &impl ConnectionTrait,
    session: &agent_sessions::Model,
) -> Result<String, AppError> {
    let calls = agent_tool_calls::Entity::find()
        .filter(agent_tool_calls::Column::SessionId.eq(session.id))
        .filter(agent_tool_calls::Column::Name.eq("submit_revision_plan"))
        .order_by_desc(agent_tool_calls::Column::Id)
        .one(conn)
        .await?;
    let Some(call) = calls else {
        return Ok(String::new());
    };
    let input: Value = serde_json::from_str(&call.input).unwrap_or(Value::Null);
    Ok(input
        .get("plan")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned())
}

/// The author's earlier feedback and steering messages of the session chain (spec/03 §9).
///
/// Sources: every `user_feedback` appendix (sealed or not) and every consumed `user_message`
/// inbox item, newest last, capped at 20 items / 6,000 characters keeping the newest.
async fn author_notes(
    conn: &impl ConnectionTrait,
    session: &agent_sessions::Model,
) -> Result<Vec<String>, AppError> {
    const MAX_ITEMS: usize = 20;
    const MAX_CHARS: usize = 6_000;

    // 1. Walk the chain from the root to this session so the notes are in chronological order.
    let mut chain: Vec<agent_sessions::Model> = vec![session.clone()];
    let mut cursor = session.parent_session_id;
    let mut guard = 0;
    while let Some(parent_id) = cursor {
        guard += 1;
        if guard > 64 {
            break;
        }
        let Some(parent) = agent_sessions::Entity::find_by_id(parent_id)
            .one(conn)
            .await?
        else {
            break;
        };
        cursor = parent.parent_session_id;
        chain.push(parent);
    }
    chain.reverse();

    let mut notes: Vec<String> = Vec::new();
    for ancestor in &chain {
        // Feedback given on a held call is stored in that call's appendices.
        let calls = agent_tool_calls::Entity::find()
            .filter(agent_tool_calls::Column::SessionId.eq(ancestor.id))
            .order_by_asc(agent_tool_calls::Column::Id)
            .all(conn)
            .await?;
        for call in calls {
            let appendices: Vec<json::Appendix> = json::from_json_or_default(&call.appendices);
            for appendix in appendices {
                if appendix.kind == json::AppendixKind::UserFeedback
                    && !appendix.text.trim().is_empty()
                {
                    notes.push(appendix.text.trim().to_owned());
                }
            }
        }
        // Steering messages are consumed inbox rows.
        let items = crate::db::entities::agent_inbox::Entity::find()
            .filter(crate::db::entities::agent_inbox::Column::SessionId.eq(ancestor.id))
            .filter(crate::db::entities::agent_inbox::Column::Kind.eq("user_message"))
            .filter(crate::db::entities::agent_inbox::Column::ConsumedAt.is_not_null())
            .order_by_asc(crate::db::entities::agent_inbox::Column::Id)
            .all(conn)
            .await?;
        for item in items {
            if !item.content.trim().is_empty() {
                notes.push(item.content.trim().to_owned());
            }
        }
    }

    // 2. Keep the newest items that fit both caps.
    let mut kept: Vec<String> = Vec::new();
    let mut total = 0usize;
    for note in notes.iter().rev() {
        let length = note.chars().count();
        if kept.len() >= MAX_ITEMS || total + length > MAX_CHARS {
            break;
        }
        total += length;
        kept.push(note.clone());
    }
    kept.reverse();
    Ok(kept)
}

/// The stored content of a chapter (empty when the session has none).
async fn chapter_content(
    conn: &impl ConnectionTrait,
    chapter_id: Option<i32>,
) -> Result<ChapterContent, AppError> {
    let Some(chapter_id) = chapter_id else {
        return Ok(ChapterContent::default());
    };
    let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(conn).await? else {
        return Ok(ChapterContent::default());
    };
    Ok(json::from_json_or_default(&chapter.content))
}

/// `conception.kickoff` (spec/03 §10).
async fn conception_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    book_id: Option<i32>,
) -> Result<Value, AppError> {
    let Some(book_id) = book_id else {
        return Ok(json!({
            "title": "",
            "idea": "",
            "genre": "",
            "writing_language_name": prompts.label("language", "en"),
        }));
    };
    let book = books::Entity::find_by_id(book_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book {book_id}")))?;
    Ok(json!({
        "title": book.title,
        "idea": book.idea,
        "genre": book.genre,
        "writing_language_name": prompts.label("language", &book.writing_language),
    }))
}

/// `researcher.kickoff` (spec/07 §8).
async fn research_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    research_task_id: Option<i32>,
) -> Result<Value, AppError> {
    // The language label comes from the registry when the task has one.
    if research_task_id.is_none() {
        let _ = prompts;
    }
    let Some(task_id) = research_task_id else {
        return Ok(json!({
            "topic": "",
            "requirements": "",
            "scope_name": "",
            "scope_description": "",
            "language_name": prompts.label("language", "en"),
            "existing_titles": [],
            "max_tool_calls": 40,
            "max_entries": 25,
        }));
    };
    let task = research_tasks::Entity::find_by_id(task_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("research task {task_id}")))?;
    Ok(json!({
        "topic": task.topic,
        "requirements": task.requirements,
        "scope_name": task.scope_name,
        "scope_description": "",
        "language_name": prompts.label("language", &task.language),
        "existing_titles": [],
        "max_tool_calls": 40,
        "max_entries": 25,
    }))
}

// ───────────────────────────── request building ─────────────────────────────

/// Builds the provider request of one session **from persisted state only** (spec/03 §11).
///
/// Invariant checked before sending: every tool call of every assistant message has a sealed
/// result. A violation is an internal defect, never a user error.
pub async fn build_request(
    conn: &impl ConnectionTrait,
    session: &agent_sessions::Model,
    _prompts: &PromptSet,
) -> Result<nekowriter_llm::ChatRequest, AppError> {
    let snapshot: json::ModelSnapshot = match json::from_json(&session.model_snapshot) {
        Ok(snapshot) => snapshot,
        Err(err) => return Err(AppError::internal(format!("corrupt model_snapshot: {err}"))),
    };
    let options: nekowriter_llm::ModelOptions =
        serde_json::from_value(snapshot.options.clone()).unwrap_or_default();
    let tools: Vec<nekowriter_llm::ToolDef> = json::from_json_or_default(&session.tools);
    let role = session::role_of(session);

    let mut messages = Vec::new();
    for message in super::runner::messages_of(conn, session.id).await? {
        if message.role == MessageRole::User.as_str() {
            let blocks: Vec<StoredBlock> = json::from_json_or_default(&message.content);
            let blocks = blocks
                .into_iter()
                .filter_map(|block| match block {
                    StoredBlock::Text { text, cache } => Some(nekowriter_llm::TextBlock {
                        text,
                        cache: cache.unwrap_or(false),
                    }),
                    _ => None,
                })
                .collect();
            messages.push(nekowriter_llm::Message::User { blocks });
            continue;
        }
        let payload: json::ProviderPayload = message
            .provider_payload
            .as_deref()
            .and_then(|raw| json::from_json::<json::ProviderPayload>(raw).ok())
            .unwrap_or(json::ProviderPayload::Mock {
                content: Value::Null,
            });
        messages.push(nekowriter_llm::Message::Assistant {
            payload: to_llm_payload(payload),
        });
        let calls = super::runner::calls_of(conn, message.id).await?;
        if calls.is_empty() {
            continue;
        }
        let mut results = Vec::new();
        for call in calls {
            let Some(content) = call.result_text.clone() else {
                return Err(AppError::internal(format!(
                    "tool call {} has no sealed result; refusing to send an invalid history",
                    call.id
                )));
            };
            results.push(nekowriter_llm::ToolResult {
                call_id: call.provider_call_id.clone(),
                content,
                is_error: call.is_error != 0,
            });
        }
        messages.push(nekowriter_llm::Message::ToolResults { results });
    }

    let cache_ttl = match options.cache_ttl {
        nekowriter_llm::CacheTtlOption::OneHour => nekowriter_llm::CacheTtl::OneHour,
        _ => nekowriter_llm::CacheTtl::FiveMinutes,
    };
    Ok(nekowriter_llm::ChatRequest {
        model: snapshot.model.clone(),
        system: session.system_prompt.clone(),
        messages,
        tools,
        max_output_tokens: u32::try_from(snapshot.max_output_tokens).unwrap_or(4096),
        // Writer sessions make one call per round (spec/03 §11, D-022).
        parallel_tool_calls: role != AgentRole::Writer,
        cache: nekowriter_llm::CacheHints { ttl: cache_ttl },
        options: options.clone(),
        demo_hints: if snapshot.protocol == Protocol::Mock {
            Some(demo_hints(session, conn).await?)
        } else {
            None
        },
    })
}

/// The demo-provider hints of the current step (spec/03 §11, spec/04 §7.2).
///
/// `language` is the book's writing language so the Demo provider picks the corpus variant, and
/// `tools` are the session's frozen tool names so it can drive the test-only tools of the debug
/// session (spec/10 §2.7).
async fn demo_hints(
    session: &agent_sessions::Model,
    conn: &impl ConnectionTrait,
) -> Result<nekowriter_llm::DemoHints, AppError> {
    let counters = session::counters_of(session);
    let language = match session.book_id {
        Some(book_id) => books::Entity::find_by_id(book_id)
            .one(conn)
            .await?
            .map(|book| book.writing_language)
            .unwrap_or_else(|| "en".to_owned()),
        None => "en".to_owned(),
    };
    Ok(nekowriter_llm::DemoHints {
        kind: session.kind.clone(),
        phase: session.phase.clone(),
        granularity: session.granularity.clone(),
        step_in_phase: counters.steps.max(0) as u32,
        language,
        tools: frozen_tool_names(session),
        protagonist: protagonist_name(conn, session).await?,
    })
}

/// The protagonist's stored name, when the session's book has one (spec/04 §7.2).
///
/// The Demo provider looks the protagonist up in its first planning step, which is what the
/// manual check of M5 exercises (`get_entity` on a real canon row).
async fn protagonist_name(
    conn: &impl ConnectionTrait,
    session: &agent_sessions::Model,
) -> Result<Option<String>, AppError> {
    let Some(book_id) = session.book_id else {
        return Ok(None);
    };
    // A side track's focus character is its protagonist (spec/06 §4.2).
    let focus = match session.track_id {
        Some(track_id) => tracks::Entity::find_by_id(track_id)
            .one(conn)
            .await?
            .and_then(|track| track.focus_entity_id),
        None => None,
    };
    let row = match focus {
        Some(entity_id) => entities::Entity::find_by_id(entity_id).one(conn).await?,
        None => {
            entities::Entity::find()
                .filter(entities::Column::BookId.eq(book_id))
                .filter(entities::Column::IsProtagonist.eq(1))
                .one(conn)
                .await?
        }
    };
    Ok(row.map(|row| row.name))
}

/// Converts the stored replay payload into the neutral one.
fn to_llm_payload(payload: json::ProviderPayload) -> nekowriter_llm::ProviderPayload {
    match payload {
        json::ProviderPayload::Openai {
            content,
            reasoning_content,
            tool_calls,
        } => nekowriter_llm::ProviderPayload::Openai {
            content,
            reasoning_content,
            tool_calls,
        },
        json::ProviderPayload::Anthropic { content } => {
            nekowriter_llm::ProviderPayload::Anthropic { content }
        }
        json::ProviderPayload::Mock { content } => {
            nekowriter_llm::ProviderPayload::Mock { content }
        }
    }
}

/// The `granularity` context of a session, for the notices and the writer rules (spec/03 §8).
pub fn granularity_context(
    prompts: &PromptSet,
    granularity: Option<json::Granularity>,
    phase: Phase,
    content: &ChapterContent,
) -> GranularityContext {
    let key = granularity
        .map(|value| value.as_str())
        .unwrap_or(json::Granularity::Paragraph.as_str())
        .to_owned();
    GranularityContext {
        label: prompts.label("granularity", &key),
        key,
        revising: matches!(phase, Phase::RevisionPlanning | Phase::Revising),
        tail_open: content.tail_open,
        last_pid: content
            .paragraphs
            .last()
            .map(|paragraph| paragraph.pid.clone())
            .unwrap_or_default(),
    }
}

// ───────────────────────────── rollover ─────────────────────────────

/// Creates the child session of a rollover and supersedes the parent (spec/03 §9).
///
/// The mechanics belong to M3 (session creation, inbox move, supersession); the child's kickoff
/// uses the `rollover` block list, whose `author_notes`/draft content is filled by the writer
/// milestones.
pub async fn rollover(
    state: &crate::state::AppState,
    parent: &agent_sessions::Model,
) -> Result<(), AppError> {
    let parent = parent.clone();
    let state_clone = state.clone();
    let (child_id, events) = crate::services::write(state.db(), move |tx| {
        let state = state_clone.clone();
        Box::pin(async move {
            let kind = session::kind_of(&parent);
            let prompts = state.prompts().for_book(state.db(), parent.book_id).await?;
            // 1. Create the child session (same kind, role, chapter, phase, granularity, flags,
            //    model snapshot; parent_session_id = old).
            let mut blocks = Vec::new();
            let plan = KickoffPlan {
                kind,
                redraft: false,
            };
            for key in [
                "block.settings",
                "block.memory",
                "block.chapter_draft",
                "writer.kickoff.rollover",
            ] {
                let context = block_context(tx, &prompts, &parent, plan, key).await?;
                let rendered = prompts
                    .render_value(key, crate::prompts::template_value(&context))
                    .map_err(|err| err.into_app_error())?;
                if !rendered.text.trim().is_empty() {
                    blocks.push(StoredBlock::Text {
                        text: rendered.text,
                        cache: Some(block_is_cached(key)),
                    });
                }
            }
            let now = chrono::Utc::now();
            let child = agent_sessions::ActiveModel {
                book_id: sea_orm::Set(parent.book_id),
                role: sea_orm::Set(parent.role.clone()),
                kind: sea_orm::Set(kind.as_str().to_owned()),
                chapter_id: sea_orm::Set(parent.chapter_id),
                track_id: sea_orm::Set(parent.track_id),
                research_task_id: sea_orm::Set(None),
                model_profile_id: sea_orm::Set(parent.model_profile_id),
                model_snapshot: sea_orm::Set(parent.model_snapshot.clone()),
                system_prompt: sea_orm::Set(parent.system_prompt.clone()),
                tools: sea_orm::Set(parent.tools.clone()),
                status: sea_orm::Set(json::SessionStatus::Running.as_str().to_owned()),
                phase: sea_orm::Set(parent.phase.clone()),
                granularity: sea_orm::Set(parent.granularity.clone()),
                auto_pause: sea_orm::Set(parent.auto_pause),
                pause_after_outline: sea_orm::Set(parent.pause_after_outline),
                pause_requested: sea_orm::Set(0),
                held_tool_call_id: sea_orm::Set(None),
                base_content_version: sea_orm::Set(parent.base_content_version),
                pending_comment: sea_orm::Set(parent.pending_comment.clone()),
                view_snapshot: sea_orm::Set(None),
                counters: sea_orm::Set("{}".to_owned()),
                usage: sea_orm::Set("{}".to_owned()),
                error: sea_orm::Set(None),
                parent_session_id: sea_orm::Set(Some(parent.id)),
                created_at: sea_orm::Set(now),
                updated_at: sea_orm::Set(now),
                finished_at: sea_orm::Set(None),
                ..Default::default()
            };
            let child = sea_orm::ActiveModelTrait::insert(child, tx).await?;
            super::runner::insert_kickoff(tx, child.id, blocks).await?;
            // 2. Move the unconsumed inbox items to the child (spec/03 §9 step 2).
            let pending = super::runner::pending_inbox_rows(tx, parent.id).await?;
            for item in pending {
                sea_orm::ActiveModelTrait::update(
                    crate::db::entities::agent_inbox::ActiveModel {
                        id: sea_orm::Set(item.id),
                        session_id: sea_orm::Set(child.id),
                        ..Default::default()
                    },
                    tx,
                )
                .await?;
            }
            // 3. The parent is superseded; the child runs.
            let events = session::transition(
                tx,
                &parent,
                json::SessionStatus::Superseded,
                session::TransitionCause::Rollover,
            )
            .await?;
            Ok((child.id, events))
        })
    })
    .await?;
    state.events().publish_all(events);
    state.manager().spawn(state.clone(), child_id);
    Ok(())
}

// ───────────────────────────── completion ─────────────────────────────

/// Domain effects of a completing call (spec/03 §5.4, spec/08).
///
/// M3 provides the framework and the session-level effects. The per-kind handlers (a `draft`
/// version and `in_review`, a review report row, `canon_status` synced/stale, a research task
/// done) arrive with their milestones, which own the tools that complete those sessions.
pub async fn apply_completion(
    tx: &sea_orm::DatabaseTransaction,
    session: &agent_sessions::Model,
    call: &agent_tool_calls::Model,
    completion: CompletionKind,
) -> Result<Vec<ServerEvent>, AppError> {
    let mut events = Vec::new();
    match completion {
        // `finish_draft` / a chapter-completing `insert_text` (M4.2).
        CompletionKind::Draft => {
            if let Some(chapter_id) = session.chapter_id {
                events
                    .extend(super::completion::finish_draft(tx, session, call, chapter_id).await?);
            }
        }
        // M6.2 owns the revision completion.
        CompletionKind::Revision => {}
        // M6.1 owns the review report.
        CompletionKind::Review => {}
        // M6.3 owns the canon status.
        CompletionKind::Maintenance => {}
        // M8.3 owns the research task.
        CompletionKind::Research => {}
    }
    Ok(events)
}

/// The version kinds the completion handlers save (re-exported for the milestones).
pub fn completion_version_kind(completion: CompletionKind) -> Option<VersionKind> {
    match completion {
        CompletionKind::Draft => Some(VersionKind::Draft),
        CompletionKind::Revision => Some(VersionKind::Revision),
        _ => None,
    }
}

/// Stored `request_meta` of a message, when present.
pub fn request_meta(message: &agent_messages::Model) -> Option<RequestMeta> {
    message
        .request_meta
        .as_deref()
        .and_then(|raw| json::from_json::<RequestMeta>(raw).ok())
}

/// Reads the frozen tool names of a session (spec/03 §1.3).
pub fn frozen_tool_names(session: &agent_sessions::Model) -> Vec<String> {
    let tools: Vec<nekowriter_llm::ToolDef> = json::from_json_or_default(&session.tools);
    tools.into_iter().map(|tool| tool.name).collect()
}

/// True when the session's frozen tool list contains the tool (spec/03 §1.2).
pub fn freezes_tool(session: &agent_sessions::Model, name: &str) -> bool {
    frozen_tool_names(session).iter().any(|tool| tool == name)
}

/// The completion kinds that produce a version (spec/08 §1.1).
pub fn version_kind_label(kind: VersionKind) -> &'static str {
    kind.as_str()
}

/// A chapter version row's content, for the completion handlers.
pub async fn latest_version_kind(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
) -> Result<Option<VersionKind>, AppError> {
    let row = chapter_versions::Entity::find()
        .filter(chapter_versions::Column::ChapterId.eq(chapter_id))
        .order_by_desc(chapter_versions::Column::Id)
        .one(conn)
        .await?;
    Ok(row.and_then(|row| VersionKind::from_str_opt(&row.kind)))
}

/// The calls of a message that are still unresolved (used by tests and the debug endpoint).
pub async fn unresolved_calls(
    conn: &impl ConnectionTrait,
    message_id: i32,
) -> Result<Vec<agent_tool_calls::Model>, AppError> {
    Ok(agent_tool_calls::Entity::find()
        .filter(agent_tool_calls::Column::MessageId.eq(message_id))
        .filter(agent_tool_calls::Column::Status.is_in([
            ToolCallStatus::Pending.as_str(),
            ToolCallStatus::Executing.as_str(),
            ToolCallStatus::Held.as_str(),
            ToolCallStatus::AwaitingInput.as_str(),
        ]))
        .all(conn)
        .await?)
}

/// The number of pending inbox rows of a session (spec/10 §2.7 `inbox_pending`).
pub async fn inbox_pending(conn: &impl ConnectionTrait, session_id: i32) -> Result<i64, AppError> {
    use sea_orm::PaginatorTrait;
    Ok(crate::db::entities::agent_inbox::Entity::find()
        .filter(crate::db::entities::agent_inbox::Column::SessionId.eq(session_id))
        .filter(crate::db::entities::agent_inbox::Column::ConsumedAt.is_null())
        .count(conn)
        .await? as i64)
}

/// The message kinds that count as an author message (spec/03 §5.3).
pub fn is_author_kind(kind: &str) -> bool {
    kind == MessageKind::User.as_str() || kind == MessageKind::Kickoff.as_str()
}

/// A `BTreeMap` of the book's setting docs, keyed by kind (used by tests).
pub async fn setting_docs_map(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<BTreeMap<String, String>, AppError> {
    let docs = setting_docs::Entity::find()
        .filter(setting_docs::Column::BookId.eq(book_id))
        .all(conn)
        .await?;
    Ok(docs
        .into_iter()
        .map(|doc| (doc.kind, doc.content))
        .collect())
}

/// Error code helper for a missing model role (spec/03 §4 step 1).
pub fn missing_role(role: AgentRole) -> AppError {
    AppError::new(
        ErrorCode::ModelMissingRole,
        format!("no model is configured for the {} role", role.as_str()),
    )
    .with_details(json!({ "role": role.as_str() }))
}
