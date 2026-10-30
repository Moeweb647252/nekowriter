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

use crate::agent::tools::research::{
    MAX_ENTRIES as MAX_RESEARCH_ENTRIES, MAX_TOOL_CALLS as MAX_RESEARCH_TOOL_CALLS,
};
use crate::db::entities::{
    agent_messages, agent_sessions, agent_tool_calls, book_scopes, books, chapter_versions,
    chapters, character_designs, entities, knowledge_scopes, research_tasks, review_reports,
    setting_docs, tracks,
};
use crate::db::json::{
    self, AgentRole, CanonMode, CanonSystem, CanonSystems, ChapterContent, CompletionKind,
    InboxKind, JsonText, MessageKind, MessageRole, Phase, Protocol, RequestMeta, SessionCounters,
    SessionKind, StoredBlock, ToolCallStatus, TrackKind, Usage, VersionKind,
};
use crate::error::{AppError, ErrorCode};
use crate::events::ServerEvent;
use crate::prompts::PromptSet;
use crate::prompts::context::{
    BookContext, CastEntryContext, ChapterContext, CharacterContext, DiffChangeContext,
    DiffContext, DraftContext, FeedbackContext, FeedbackIssueContext, GoalsContext,
    GranularityContext, MemoryContext, MemoryCountsContext, ParagraphContext, PriorReportContext,
    RecentChapterContext, ScopeContext, SettingsContext, SystemsContext, TrackContext,
    WritingSkillContext,
};
use crate::text;

use super::session;

/// The kickoff blocks of every session kind, in order (spec/03 §10 table).
///
/// `block.settings` carries a cache breakpoint (`cache: true`).
pub fn kickoff_blocks(kind: SessionKind, _redraft: bool) -> Vec<&'static str> {
    match kind {
        SessionKind::Conception => vec!["conception.kickoff"],
        SessionKind::ChapterOutline => vec![
            "block.settings",
            "block.canon_systems",
            "block.memory",
            "block.recent_chapters",
            "block.goals",
            "planner.kickoff",
        ],
        SessionKind::Research => vec!["block.book_brief", "researcher.kickoff"],
        // A write session always drafts from the accepted outline; a redraft is the same kickoff
        // with its `feedback` branch filled (D-146). Writing skills sit right after settings
        // (stable-first ordering, design §6.1); recent chapters after memory.
        SessionKind::ChapterWrite => vec![
            "block.settings",
            "block.canon_systems",
            "block.writing_skills",
            "block.memory",
            "block.recent_chapters",
            "block.goals",
            "block.chapter_draft",
            "writer.kickoff.chapter",
        ],
        SessionKind::ChapterRevise => vec![
            "block.settings",
            "block.canon_systems",
            "block.writing_skills",
            "block.memory",
            "block.goals",
            "block.chapter_draft",
            "writer.kickoff.revise",
        ],
        SessionKind::ChapterReview => vec![
            "block.settings",
            "block.canon_systems",
            "block.writing_skills",
            "block.memory",
            "block.goals",
            "block.chapter_draft",
            "reviewer.kickoff",
        ],
        SessionKind::ChapterMaintenance => vec![
            "block.settings",
            "block.canon_systems",
            "block.memory",
            "block.cast_index",
            "block.goals",
            "block.chapter_draft",
            "archivist.kickoff.maintenance",
        ],
        SessionKind::CanonSeed => vec![
            "block.settings",
            "block.canon_systems",
            "archivist.kickoff.seed",
        ],
        SessionKind::ChapterResync => vec![
            "block.settings",
            "block.canon_systems",
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

/// The canon system that owns a whole kickoff block (plan §3.1), if any. `block.cast_index`
/// belongs to Entities; `block.goals` to Goals. A mixed block such as `block.memory` is not owned
/// by one system and renders its own fragments.
pub fn system_of_block(key: &str) -> Option<CanonSystem> {
    match key {
        "block.cast_index" => Some(CanonSystem::Entities),
        "block.goals" => Some(CanonSystem::Goals),
        _ => None,
    }
}

/// True when one system is on.
pub fn system_is_on(systems: &CanonSystems, system: CanonSystem) -> bool {
    match system {
        CanonSystem::Entities => systems.entities,
        CanonSystem::Timeline => systems.timeline,
        CanonSystem::Memory => systems.memory,
        CanonSystem::Foreshadowing => systems.foreshadowing,
        CanonSystem::Goals => systems.goals,
    }
}

/// The `systems` template variable of the session's frozen value.
fn systems_context(session: &agent_sessions::Model) -> SystemsContext {
    SystemsContext::from(&session::canon_systems_of(session))
}

/// `goal_node_entities` links, one batch, mapped back to the node order.
async fn goal_entity_links(
    conn: &impl ConnectionTrait,
    node_ids: &[i32],
) -> Result<std::collections::BTreeMap<i32, Vec<String>>, AppError> {
    if node_ids.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }
    let links = crate::db::entities::goal_node_entities::Entity::find()
        .filter(
            crate::db::entities::goal_node_entities::COLUMN
                .node_id
                .is_in(node_ids.to_vec()),
        )
        .order_by_asc(crate::db::entities::goal_node_entities::COLUMN.entity_id)
        .all(conn)
        .await?;
    let entity_ids: Vec<i32> = links.iter().map(|link| link.entity_id).collect();
    let names: std::collections::BTreeMap<i32, String> =
        crate::db::entities::entities::Entity::find()
            .filter(
                crate::db::entities::entities::COLUMN
                    .id
                    .is_in(entity_ids.clone()),
            )
            .all(conn)
            .await?
            .into_iter()
            .map(|entity| (entity.id, entity.name))
            .collect();
    let mut out: std::collections::BTreeMap<i32, Vec<String>> = std::collections::BTreeMap::new();
    for link in links {
        if let Some(name) = names.get(&link.entity_id) {
            out.entry(link.node_id)
                .or_default()
                .push(format!("{name}(#{})", link.entity_id));
        }
    }
    Ok(out)
}

/// `goal_node_events` links, one batch, mapped back to the node order.
async fn goal_event_links(
    conn: &impl ConnectionTrait,
    node_ids: &[i32],
) -> Result<std::collections::BTreeMap<i32, Vec<String>>, AppError> {
    if node_ids.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }
    let links = crate::db::entities::goal_node_events::Entity::find()
        .filter(
            crate::db::entities::goal_node_events::COLUMN
                .node_id
                .is_in(node_ids.to_vec()),
        )
        .order_by_asc(crate::db::entities::goal_node_events::COLUMN.event_id)
        .all(conn)
        .await?;
    let event_ids: Vec<i32> = links.iter().map(|link| link.event_id).collect();
    let titles: std::collections::BTreeMap<i32, String> =
        crate::db::entities::timeline_events::Entity::find()
            .filter(
                crate::db::entities::timeline_events::COLUMN
                    .id
                    .is_in(event_ids.clone()),
            )
            .all(conn)
            .await?
            .into_iter()
            .map(|event| (event.id, event.title))
            .collect();
    let mut out: std::collections::BTreeMap<i32, Vec<String>> = std::collections::BTreeMap::new();
    for link in links {
        if let Some(title) = titles.get(&link.event_id) {
            out.entry(link.node_id)
                .or_default()
                .push(format!("#{} {title}", link.event_id));
        }
    }
    Ok(out)
}

/// `goal_node_foreshadowing` links, one batch, mapped back to the node order.
async fn goal_foreshadowing_links(
    conn: &impl ConnectionTrait,
    node_ids: &[i32],
) -> Result<std::collections::BTreeMap<i32, Vec<String>>, AppError> {
    if node_ids.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }
    let links = crate::db::entities::goal_node_foreshadowing::Entity::find()
        .filter(
            crate::db::entities::goal_node_foreshadowing::COLUMN
                .node_id
                .is_in(node_ids.to_vec()),
        )
        .order_by_asc(crate::db::entities::goal_node_foreshadowing::COLUMN.foreshadowing_id)
        .all(conn)
        .await?;
    let hook_ids: Vec<i32> = links.iter().map(|link| link.foreshadowing_id).collect();
    let titles: std::collections::BTreeMap<i32, String> =
        crate::db::entities::foreshadowing::Entity::find()
            .filter(
                crate::db::entities::foreshadowing::COLUMN
                    .id
                    .is_in(hook_ids.clone()),
            )
            .all(conn)
            .await?
            .into_iter()
            .map(|hook| (hook.id, hook.title))
            .collect();
    let mut out: std::collections::BTreeMap<i32, Vec<String>> = std::collections::BTreeMap::new();
    for link in links {
        if let Some(title) = titles.get(&link.foreshadowing_id) {
            out.entry(link.node_id)
                .or_default()
                .push(format!("#{} {title}", link.foreshadowing_id));
        }
    }
    Ok(out)
}

/// `block.goals` and the maintenance task: the relevant goal tree of the session's chapter.
async fn goals_context(
    conn: &impl ConnectionTrait,
    session: &agent_sessions::Model,
) -> Result<GoalsContext, AppError> {
    let (Some(book_id), Some(track_id), Some(chapter_id)) =
        (session.book_id, session.track_id, session.chapter_id)
    else {
        return Ok(GoalsContext::default());
    };
    let Some(track) = tracks::Entity::find_by_id(track_id).one(conn).await? else {
        return Ok(GoalsContext::default());
    };
    let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(conn).await? else {
        return Ok(GoalsContext::default());
    };
    let selection = crate::services::goals::relevant_nodes(conn, book_id, &track, &chapter).await?;
    let systems = session::canon_systems_of(session);
    let ids: Vec<i32> = selection.nodes.iter().map(|node| node.id).collect();
    let entity_links = if systems.entities {
        goal_entity_links(conn, &ids).await?
    } else {
        std::collections::BTreeMap::new()
    };
    let event_links = if systems.timeline {
        goal_event_links(conn, &ids).await?
    } else {
        std::collections::BTreeMap::new()
    };
    let foreshadowing_links = if systems.foreshadowing {
        goal_foreshadowing_links(conn, &ids).await?
    } else {
        std::collections::BTreeMap::new()
    };
    let nodes = selection
        .nodes
        .into_iter()
        .map(|node| crate::prompts::context::GoalNodeContext {
            id: node.id,
            level: node.level.as_str().to_owned(),
            title: node.title,
            purpose: node.purpose,
            chapters: match (node.chapter_from, node.chapter_to) {
                (None, None) => String::new(),
                (Some(from), None) => format!("{from}-"),
                (None, Some(to)) => format!("-{to}"),
                (Some(from), Some(to)) => format!("{from}-{to}"),
            },
            overdue: node.chapter_to.is_some_and(|to| to < chapter.ord),
            entities: entity_links.get(&node.id).cloned().unwrap_or_default(),
            events: event_links.get(&node.id).cloned().unwrap_or_default(),
            foreshadowing: foreshadowing_links
                .get(&node.id)
                .cloned()
                .unwrap_or_default(),
        })
        .collect();
    Ok(GoalsContext {
        nodes,
        omitted: selection.omitted,
    })
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
    pub fn renders_outline_elsewhere(self, chapter_draft_renders: bool) -> bool {
        match self.kind {
            // The planner task includes the outline itself when one exists.
            SessionKind::ChapterOutline => true,
            // These kickoffs include `block.chapter_draft` only when that block actually renders;
            // an empty override must not suppress the outline from `block.memory` (IR-01).
            SessionKind::ChapterWrite
            | SessionKind::ChapterRevise
            | SessionKind::ChapterReview
            | SessionKind::ChapterMaintenance => chapter_draft_renders,
            _ => false,
        }
    }
}

/// Which include-capable blocks actually rendered non-empty after prompt overrides. `block.memory`
/// uses these facts to avoid suppressing content that no rendered block carries (IR-01).
#[derive(Debug, Clone, Copy, Default)]
struct RenderedBlocks {
    recent_chapters: bool,
    chapter_draft: bool,
}

/// The rendered kickoff message of a session kind: one `text` block per kickoff block
/// (spec/03 §10).
pub async fn assemble_kickoff(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    session: &agent_sessions::Model,
    redraft: bool,
    extra: Option<&Value>,
) -> Result<Vec<StoredBlock>, AppError> {
    let plan = KickoffPlan::new(session::kind_of(session), redraft);
    // The actual emitted block list, before `block.memory` renders: its dedupe decision must read
    // this list, not `plan.blocks()`, because a caller like `rollover` builds its own list that
    // deliberately leaves `block.recent_chapters` out (design §7, spec/03 §9).
    let mut emitted_blocks = plan.blocks();
    // A whole block owned by a disabled system is not rendered at all (plan §3.1). `block.memory`
    // is mixed and handles its fragments in the template.
    let frozen_systems = session::canon_systems_of(session);
    emitted_blocks.retain(|key| {
        system_of_block(key).is_none_or(|system| system_is_on(&frozen_systems, system))
    });
    // spec/07 §8 step 2: the researcher's kickoff carries the linked book's brief **only when the
    // task belongs to one**. A standalone research task has no book, and an empty brief block would
    // read as a blank book rather than "no book".
    if session::kind_of(session) == SessionKind::Research
        && session.book_id.is_none()
        && session.research_task_id.is_some()
    {
        emitted_blocks.retain(|key| *key != "block.book_brief");
    }
    let recent_chapters_renders = if emitted_blocks.contains(&"block.recent_chapters") {
        block_renders_non_empty(conn, prompts, session, plan, "block.recent_chapters", extra)
            .await?
    } else {
        false
    };
    let chapter_draft_renders = if emitted_blocks.contains(&"block.chapter_draft") {
        block_renders_non_empty(conn, prompts, session, plan, "block.chapter_draft", extra).await?
    } else {
        false
    };
    let mut blocks = Vec::new();
    for key in emitted_blocks.iter().copied() {
        let context = block_context(
            conn,
            prompts,
            session,
            plan,
            RenderedBlocks {
                recent_chapters: recent_chapters_renders,
                chapter_draft: chapter_draft_renders,
            },
            key,
            extra,
        )
        .await?;
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

/// Whether one kickoff block renders a non-empty text after its override is applied.
async fn block_renders_non_empty(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    session: &agent_sessions::Model,
    plan: KickoffPlan,
    key: &str,
    extra: Option<&Value>,
) -> Result<bool, AppError> {
    let context = block_context(
        conn,
        prompts,
        session,
        plan,
        RenderedBlocks::default(),
        key,
        extra,
    )
    .await?;
    let rendered = prompts
        .render_value(key, crate::prompts::template_value(&context))
        .map_err(|err| err.into_app_error())?;
    Ok(!rendered.text.trim().is_empty())
}

/// The template context of one kickoff block (spec/03 §10, spec/09 §2).
async fn block_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    session: &agent_sessions::Model,
    plan: KickoffPlan,
    rendered_blocks: RenderedBlocks,
    key: &str,
    extra: Option<&Value>,
) -> Result<Value, AppError> {
    match key {
        "block.settings" => Ok(json!({
            "book": book_context(conn, prompts, session.book_id).await?,
            "settings": settings_context(conn, session.book_id).await?,
        })),
        "block.memory" => {
            let chapter =
                chapter_context(conn, prompts, session.chapter_id, session.book_id).await?;
            let chapter = memory_chapter_view(chapter, plan, rendered_blocks.chapter_draft);
            Ok(json!({
                // For a write or outline kickoff the recent-chapters block repeats the previous
                // chapter in full; `previous_tail` is then a strict subset of it and is dropped
                // (design §7). Summaries stay: they are condensed accounts, not copies.
                "memory": memory_context(
                    conn,
                    prompts,
                    session,
                    rendered_blocks.recent_chapters,
                )
                .await?,
                "chapter": chapter,
                "systems": systems_context(session),
            }))
        }
        "block.canon_systems" => Ok(json!({
            "systems": systems_context(session),
        })),
        "block.goals" => Ok(json!({
            "goals": goals_context(conn, session).await?,
            "systems": systems_context(session),
        })),
        "block.writing_skills" => Ok(json!({
            "skills": writing_skills_context(conn, session).await?,
        })),
        "block.recent_chapters" => Ok(json!({
            "chapters": recent_chapters_context(conn, prompts, session).await?,
        })),
        "block.chapter_draft" => Ok(json!({
            "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
            "draft": draft_context(conn, session.chapter_id).await?,
        })),
        "block.cast_index" => Ok(json!({
            "cast_index": cast_index_context(conn, prompts, session).await?,
        })),
        "block.chapter_diff" => Ok(json!({
            "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
            "diff": chapter_diff_context(conn, session).await?,
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
        "planner.kickoff" => Ok(json!({
            "book": book_context(conn, prompts, session.book_id).await?,
            "track": track_context(conn, prompts, session.track_id).await?,
            "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
            "draft": draft_context(conn, session.chapter_id).await?,
        })),
        "writer.kickoff.chapter" => {
            let content = chapter_content(conn, session.chapter_id).await?;
            Ok(json!({
                "book": book_context(conn, prompts, session.book_id).await?,
                "track": track_context(conn, prompts, session.track_id).await?,
                "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
                "granularity": granularity_context(
                    prompts,
                    session::granularity_of(session),
                    session::phase_of(session),
                    &content,
                ),
                "feedback": kickoff_feedback(conn, prompts, session, extra).await?,
            }))
        }
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
            "prior": prior_report_context(conn, prompts, session, extra).await?,
        })),
        "archivist.kickoff.maintenance" => Ok(json!({
            "book": book_context(conn, prompts, session.book_id).await?,
            "track": track_context(conn, prompts, session.track_id).await?,
            "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
            "systems": systems_context(session),
            "goals": goals_context(conn, session).await?,
        })),
        "archivist.kickoff.seed" => Ok(json!({
            "book": book_context(conn, prompts, session.book_id).await?,
            "settings": settings_context(conn, session.book_id).await?,
            "systems": systems_context(session),
        })),
        "archivist.kickoff.resync" => Ok(json!({
            "book": book_context(conn, prompts, session.book_id).await?,
            "track": track_context(conn, prompts, session.track_id).await?,
            "chapter": chapter_context(conn, prompts, session.chapter_id, session.book_id).await?,
            "systems": systems_context(session),
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
                track.kind.as_str(),
                &track.title,
                i64::from(chapter.ord),
                &chapter.title,
                !chapter.title.is_empty(),
            )
        }
        None => String::new(),
    };
    Ok(TrackContext {
        is_main: track.kind == TrackKind::Main,
        title: if track.kind == TrackKind::Main {
            String::new()
        } else {
            track.title.clone()
        },
        kind: track.kind.as_str().to_owned(),
        synopsis: track.synopsis,
        canon_mode: track.canon_mode.as_str().to_owned(),
        anchor_kind: track.anchor_kind.as_str().to_owned(),
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
    let content: ChapterContent = chapter.content.clone().into_inner();
    Ok(ChapterContext {
        id: chapter.id,
        number: chapter.ord,
        // The templates append `《title》` themselves when a title exists, so the label carries
        // none (spec/09 §1: `with_title` is for callers that need the whole reference in one
        // string, such as the memory window).
        label: prompts.chapter_label(
            track.kind.as_str(),
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
        has_draft: text::content_word_count(&content) > 0,
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
    Ok(book.settings.clone().into_inner())
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
        .filter(setting_docs::COLUMN.book_id.eq(book_id))
        .all(conn)
        .await?;
    let mut outline = String::new();
    let mut world = String::new();
    let mut protagonist = String::new();
    for doc in docs {
        match doc.kind {
            crate::db::json::SettingDocKind::Outline => outline = doc.content,
            crate::db::json::SettingDocKind::World => world = doc.content,
            crate::db::json::SettingDocKind::Protagonist => protagonist = doc.content,
        }
    }
    let characters = character_designs::Entity::find()
        .filter(character_designs::COLUMN.book_id.eq(book_id))
        .order_by_asc(character_designs::COLUMN.rank)
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
    // The scopes the book's writer and reviewer may search (spec/07 §1: "the settings block lists
    // them (name + description)"); the attached set is exactly what `visible_scope_ids` gates the
    // tools with, so the model never learns a scope it cannot search.
    let scope_ids: Vec<i32> = book_scopes::Entity::find()
        .filter(book_scopes::COLUMN.book_id.eq(book_id))
        .order_by_asc(book_scopes::COLUMN.scope_id)
        .all(conn)
        .await?
        .into_iter()
        .map(|row| row.scope_id)
        .collect();
    let scopes = if scope_ids.is_empty() {
        Vec::new()
    } else {
        knowledge_scopes::Entity::find()
            .filter(knowledge_scopes::COLUMN.id.is_in(scope_ids))
            .order_by_asc(knowledge_scopes::COLUMN.name)
            .all(conn)
            .await?
            .into_iter()
            .map(|row| ScopeContext {
                name: row.name,
                description: row.description,
            })
            .collect()
    };
    Ok(SettingsContext {
        outline,
        world,
        protagonist,
        characters,
        scopes,
    })
}

/// `block.memory`: the tiered memory snapshot of the target chapter (spec/06 §4).
///
/// `dedupe_recent_tail` is true for a kickoff that also carries `block.recent_chapters`: when the
/// previous chapter is injected in full there, its `previous_tail` excerpt is a strict subset and
/// is dropped (design §7). Summaries stay — they are condensed accounts, not copies.
async fn memory_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    session: &agent_sessions::Model,
    dedupe_recent_tail: bool,
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
    let systems = session::canon_systems_of(session);
    let mut snapshot =
        crate::services::memory::snapshot_with_systems(conn, prompts, &track, &chapter, &systems)
            .await?;
    if dedupe_recent_tail {
        let recent =
            crate::services::memory::recent_chapters(conn, prompts, &track, &chapter).await?;
        crate::services::memory::dedupe_previous_tail(&mut snapshot, &recent);
    }
    Ok(crate::services::memory::memory_context(&snapshot))
}

/// `block.writing_skills`: the full bodies of the book's selected skills (design §6).
async fn writing_skills_context(
    conn: &impl ConnectionTrait,
    session: &agent_sessions::Model,
) -> Result<Vec<WritingSkillContext>, AppError> {
    let Some(book_id) = session.book_id else {
        return Ok(Vec::new());
    };
    let rows = crate::services::skills::selected_skills(conn, book_id).await?;
    Ok(rows
        .into_iter()
        // A selectable built-in can still have an empty body (design §5, D-148): it injects
        // nothing then, so the block keeps its heading only when a non-empty skill exists.
        .filter(|row| !row.body.trim().is_empty())
        .map(|row| WritingSkillContext {
            name: row.name,
            category: row.category,
            body: row.body,
        })
        .collect())
}

/// `block.recent_chapters`: the last N official chapters in full (design §7).
async fn recent_chapters_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    session: &agent_sessions::Model,
) -> Result<Vec<RecentChapterContext>, AppError> {
    let (Some(_book_id), Some(chapter_id)) = (session.book_id, session.chapter_id) else {
        return Ok(Vec::new());
    };
    let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(conn).await? else {
        return Ok(Vec::new());
    };
    let Some(track) = tracks::Entity::find_by_id(chapter.track_id)
        .one(conn)
        .await?
    else {
        return Ok(Vec::new());
    };
    let rows = crate::services::memory::recent_chapters(conn, prompts, &track, &chapter).await?;
    Ok(rows
        .into_iter()
        .map(|row| RecentChapterContext {
            number: row.number,
            label: row.label,
            title: row.title,
            text: row.text,
            partial: row.partial,
        })
        .collect())
}

/// The chapter view `block.memory` renders: label and title only, and the outline only when the
/// kickoff has no `block.chapter_draft` and the task does not show the old outline itself
/// (spec/06 §4.2, D-050).
fn memory_chapter_view(
    chapter: ChapterContext,
    plan: KickoffPlan,
    chapter_draft_renders: bool,
) -> ChapterContext {
    let renders_outline = plan.renders_outline_elsewhere(chapter_draft_renders);
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

/// `block.cast_index`: up to 300 visible entities with aliases, so the archivist matches existing
/// entities instead of duplicating them (spec/06 §4.5).
async fn cast_index_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    session: &agent_sessions::Model,
) -> Result<Vec<CastEntryContext>, AppError> {
    let Some(book_id) = session.book_id else {
        return Ok(Vec::new());
    };
    let track = match session.track_id {
        Some(track_id) => tracks::Entity::find_by_id(track_id).one(conn).await?,
        None => None,
    };
    let visibility = crate::services::canon::visibility_of_track(book_id, track.as_ref());
    let mut rows = crate::services::canon::visible_entities(conn, visibility)
        .await
        .map_err(|err| err.into_app_error())?;
    rows.sort_by(|a, b| {
        b.importance
            .cmp(&a.importance)
            .then_with(|| b.last_chapter_id.cmp(&a.last_chapter_id))
            .then_with(|| a.id.cmp(&b.id))
    });
    rows.truncate(300);
    Ok(rows
        .into_iter()
        .map(|row| CastEntryContext {
            id: row.id,
            name: row.name,
            kind_label: prompts.label("entity_kind", row.kind.as_str()),
            status_label: prompts.label("entity_status", row.status.as_str()),
            status: row.status.as_str().to_owned(),
            aliases: row.aliases.clone().into_inner(),
            summary: row.summary,
        })
        .collect())
}

/// `block.chapter_diff` (spec/06 §8): the paragraph-level diff between `synced_version_id` and the
/// new official version, so the re-sync task shows only what the author changed.
async fn chapter_diff_context(
    conn: &impl ConnectionTrait,
    session: &agent_sessions::Model,
) -> Result<DiffContext, AppError> {
    let Some(chapter_id) = session.chapter_id else {
        return Ok(DiffContext {
            changes: Vec::new(),
        });
    };
    let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(conn).await? else {
        return Ok(DiffContext {
            changes: Vec::new(),
        });
    };
    let previous = match chapter.synced_version_id {
        Some(version_id) => chapter_versions::Entity::find_by_id(version_id)
            .one(conn)
            .await?
            .map(|row| row.content.clone().into_inner())
            .unwrap_or_default(),
        None => ChapterContent::default(),
    };
    let current = match chapter.official_version_id {
        Some(version_id) => chapter_versions::Entity::find_by_id(version_id)
            .one(conn)
            .await?
            .map(|row| row.content.clone().into_inner())
            .unwrap_or_default(),
        None => chapter.content.clone().into_inner(),
    };
    Ok(DiffContext {
        changes: paragraph_diff(&previous, &current),
    })
}

/// The paragraph diff of two chapter contents (spec/06 §8): modifications keep their pid, insertions
/// are located by their preceding pid, deletions carry their old text.
fn paragraph_diff(previous: &ChapterContent, current: &ChapterContent) -> Vec<DiffChangeContext> {
    use std::collections::BTreeMap;
    let mut old_by_pid: BTreeMap<&str, &str> = BTreeMap::new();
    for paragraph in &previous.paragraphs {
        old_by_pid.insert(paragraph.pid.as_str(), paragraph.text.as_str());
    }
    let mut changes = Vec::new();
    let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
    let mut previous_pid: Option<String> = None;
    for paragraph in &current.paragraphs {
        seen.insert(paragraph.pid.as_str(), ());
        match old_by_pid.get(paragraph.pid.as_str()) {
            Some(old) if *old == paragraph.text => {}
            Some(old) => changes.push(DiffChangeContext {
                op: "modify".to_owned(),
                pid: paragraph.pid.clone(),
                after: None,
                old: (*old).to_owned(),
                new: paragraph.text.clone(),
            }),
            None => changes.push(DiffChangeContext {
                op: "insert".to_owned(),
                pid: paragraph.pid.clone(),
                after: previous_pid.clone(),
                old: String::new(),
                new: paragraph.text.clone(),
            }),
        }
        previous_pid = Some(paragraph.pid.clone());
    }
    for paragraph in &previous.paragraphs {
        if !seen.contains_key(paragraph.pid.as_str()) {
            changes.push(DiffChangeContext {
                op: "delete".to_owned(),
                pid: paragraph.pid.clone(),
                after: None,
                old: paragraph.text.clone(),
                new: String::new(),
            });
        }
    }
    changes
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
    let content: ChapterContent = chapter.content.clone().into_inner();
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
        .filter(
            crate::db::entities::chapter_feedback::COLUMN
                .chapter_id
                .eq(chapter_id),
        )
        .order_by_desc(crate::db::entities::chapter_feedback::COLUMN.id)
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

/// `prior` of `reviewer.kickoff` (spec/03 §6): the previous report a new re-review session
/// replaces, with the author's comment from `pending_comment`.
async fn prior_report_context(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    session: &agent_sessions::Model,
    extra: Option<&Value>,
) -> Result<Value, AppError> {
    let Some(report_id) = extra
        .and_then(|value| value.get("prior_report_id"))
        .and_then(Value::as_i64)
    else {
        return Ok(Value::Null);
    };
    let Some(report) = review_reports::Entity::find_by_id(report_id as i32)
        .one(conn)
        .await?
    else {
        return Ok(Value::Null);
    };
    let verdict = report.verdict;
    Ok(json!(PriorReportContext {
        attempt: report.attempt,
        verdict_label: prompts.label("verdict", verdict.as_str()),
        summary: report.summary,
        feedback: report.feedback,
        comment: session.pending_comment.clone().unwrap_or_default(),
    }))
}

/// Renders the stored `ReviewIssue[]` of a feedback row for `vars.feedback.issues`.
pub fn render_issues(
    prompts: &PromptSet,
    issues: &[json::ReviewIssue],
) -> Vec<FeedbackIssueContext> {
    issues
        .iter()
        .map(|issue| FeedbackIssueContext {
            severity_label: prompts.label("severity", issue.severity.as_str()),
            category_label: prompts.label("category", issue.category.as_str()),
            pids: issue.pids.clone(),
            quote: issue.quote.clone(),
            problem: issue.problem.clone(),
            suggestion: issue.suggestion.clone(),
        })
        .collect()
}

/// The `feedback` of `writer.kickoff.chapter` and `writer.kickoff.rollover` (spec/03 §9, D-146).
///
/// `None` means the task renders its non-redraft branch. At session creation a redraft's feedback
/// row is not linked to the session yet (`chapter_feedback.session_id` is written after
/// `create_inner` returns), so the caller passes its id through `kickoff_extra.feedback_id`. A
/// rollover instead walks the session chain: the row whose `session_id` is in the chain, or null
/// when there is none. Reading "the chapter's latest row" was the defect: a plain drafting
/// rollover printed the redraft sentence with dangling feedback.
async fn kickoff_feedback(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    session: &agent_sessions::Model,
    extra: Option<&Value>,
) -> Result<Option<FeedbackContext>, AppError> {
    if let Some(feedback_id) = extra
        .and_then(|value| value.get("feedback_id"))
        .and_then(Value::as_i64)
    {
        return feedback_by_id(conn, prompts, feedback_id as i32).await;
    }
    // Walk the chain from the root to this session.
    let mut chain = vec![session.id];
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
        chain.push(parent.id);
        cursor = parent.parent_session_id;
    }
    let feedback = crate::db::entities::chapter_feedback::Entity::find()
        .filter(
            crate::db::entities::chapter_feedback::COLUMN
                .session_id
                .is_in(chain),
        )
        .order_by_desc(crate::db::entities::chapter_feedback::COLUMN.id)
        .one(conn)
        .await?;
    Ok(feedback.map(|row| FeedbackContext {
        text: row.text,
        issues: render_issues(prompts, &row.issues),
    }))
}

/// One feedback row by id, when it exists.
async fn feedback_by_id(
    conn: &impl ConnectionTrait,
    prompts: &PromptSet,
    feedback_id: i32,
) -> Result<Option<FeedbackContext>, AppError> {
    let Some(feedback) = crate::db::entities::chapter_feedback::Entity::find_by_id(feedback_id)
        .one(conn)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(FeedbackContext {
        text: feedback.text,
        issues: render_issues(prompts, &feedback.issues),
    }))
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
        "feedback": kickoff_feedback(conn, prompts, session, None).await?,
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
        .filter(agent_tool_calls::COLUMN.session_id.eq(session.id))
        .filter(agent_tool_calls::COLUMN.name.eq("submit_revision_plan"))
        .order_by_desc(agent_tool_calls::COLUMN.id)
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

    // Every note is stamped with the instant it was written, because the two sources are
    // interleaved in the author's history: they can steer, then give feedback on the next hold,
    // then steer again. Collecting each source separately per ancestor would group them and tell
    // the model an order the author never wrote (spec/03 §9 "in order ... newest last").
    //
    // The chain runs root → this session, so an ancestor's index orders the ancestors while the
    // timestamp orders the notes inside one ancestor.
    let session_ids: Vec<i32> = chain.iter().map(|ancestor| ancestor.id).collect();
    let call_rows = agent_tool_calls::Entity::find()
        .filter(
            agent_tool_calls::COLUMN
                .session_id
                .is_in(session_ids.clone()),
        )
        .order_by_asc(agent_tool_calls::COLUMN.id)
        .all(conn)
        .await?;
    let mut calls_by_session: std::collections::HashMap<i32, Vec<agent_tool_calls::Model>> =
        std::collections::HashMap::new();
    for call in call_rows {
        calls_by_session
            .entry(call.session_id)
            .or_default()
            .push(call);
    }
    let inbox_rows = crate::db::entities::agent_inbox::Entity::find()
        .filter(
            crate::db::entities::agent_inbox::COLUMN
                .session_id
                .is_in(session_ids),
        )
        .filter(
            crate::db::entities::agent_inbox::COLUMN
                .kind
                .eq(InboxKind::UserMessage),
        )
        .filter(
            crate::db::entities::agent_inbox::COLUMN
                .consumed_at
                .is_not_null(),
        )
        .order_by_asc(crate::db::entities::agent_inbox::COLUMN.id)
        .all(conn)
        .await?;
    let mut inbox_by_session: std::collections::HashMap<
        i32,
        Vec<crate::db::entities::agent_inbox::Model>,
    > = std::collections::HashMap::new();
    for item in inbox_rows {
        inbox_by_session
            .entry(item.session_id)
            .or_default()
            .push(item);
    }
    let mut notes: Vec<(usize, chrono::DateTime<chrono::Utc>, i32, String)> = Vec::new();
    for (ancestor_index, ancestor) in chain.iter().enumerate() {
        let calls = calls_by_session.remove(&ancestor.id).unwrap_or_default();
        let items = inbox_by_session.remove(&ancestor.id).unwrap_or_default();
        // Feedback given on a held call is stored in that call's appendices; the call's
        // `finished_at` is when the author wrote it (the resume that sealed it).
        for call in calls {
            let appendices: Vec<json::Appendix> = call.appendices.clone().into_inner();
            for appendix in appendices {
                if appendix.kind == json::AppendixKind::UserFeedback
                    && !appendix.text.trim().is_empty()
                {
                    let at = call.finished_at.unwrap_or(call.created_at);
                    notes.push((ancestor_index, at, call.id, appendix.text.trim().to_owned()));
                }
            }
        }
        // Steering messages are consumed inbox rows, stamped when the author sent them.
        for item in items {
            if !item.content.trim().is_empty() {
                notes.push((
                    ancestor_index,
                    item.created_at,
                    item.id,
                    item.content.trim().to_owned(),
                ));
            }
        }
    }
    notes.sort_by_key(|(ancestor_index, at, id, _)| (*ancestor_index, *at, *id));
    let notes: Vec<String> = notes.into_iter().map(|(_, _, _, text)| text).collect();

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
    Ok(chapter.content.clone().into_inner())
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
    let Some(task_id) = research_task_id else {
        return Ok(json!({
            "topic": "",
            "requirements": "",
            "scope_name": "",
            "scope_description": "",
            "language_name": prompts.label("language", "en"),
            "existing_titles": [],
            "max_tool_calls": MAX_RESEARCH_TOOL_CALLS,
            "max_entries": MAX_RESEARCH_ENTRIES,
        }));
    };
    let task = research_tasks::Entity::find_by_id(task_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("research task {task_id}")))?;
    // spec/07 §8 step 2: the kickoff carries the scope's name **and description** plus the titles
    // of the entries already in it (at most 200), so the researcher extends existing material
    // instead of duplicating it. `run` resolves the scope before the session starts, so a real
    // researcher session always has `scope_id`; a task without one is a shape no session reaches
    // (asserted in `crates/server/tests/research_tasks.rs`), and it falls back to the task's
    // requested name with an empty description and no titles.
    let (scope_name, scope_description, existing_titles) = match task.scope_id {
        Some(scope_id) => {
            let scope = knowledge_scopes::Entity::find_by_id(scope_id)
                .one(conn)
                .await?
                .ok_or_else(|| AppError::not_found(format!("knowledge scope {scope_id}")))?;
            let titles = crate::services::research::existing_titles(conn, scope_id).await?;
            (scope.name, scope.description, titles)
        }
        None => (task.scope_name.clone(), String::new(), Vec::new()),
    };
    Ok(json!({
        "topic": task.topic,
        "requirements": task.requirements,
        "scope_name": scope_name,
        "scope_description": scope_description,
        "language_name": prompts.label("language", &task.language),
        "existing_titles": existing_titles,
        "max_tool_calls": MAX_RESEARCH_TOOL_CALLS,
        "max_entries": MAX_RESEARCH_ENTRIES,
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
    let snapshot: json::ModelSnapshot = session.model_snapshot.clone().into_inner();
    let options: nekowriter_llm::ModelOptions =
        serde_json::from_value(snapshot.options.clone()).unwrap_or_default();
    let tools: Vec<nekowriter_llm::ToolDef> = session.tools.clone().into_inner();
    let role = session::role_of(session);

    let mut messages = Vec::new();
    for message in super::runner::messages_of(conn, session.id).await? {
        if message.role == MessageRole::User {
            let blocks: Vec<StoredBlock> = message.content.clone().into_inner();
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
            .clone()
            .map(|value| value.into_inner())
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
                is_error: call.is_error,
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
    // The review attempt about to be written: a re-review carries a previous report, so its
    // attempt is one more than the chapter's newest report (spec/04 §7.2).
    let review_attempt = match session.chapter_id {
        Some(chapter_id) => crate::db::entities::review_reports::Entity::find()
            .filter(
                crate::db::entities::review_reports::COLUMN
                    .chapter_id
                    .eq(chapter_id),
            )
            .order_by_desc(crate::db::entities::review_reports::COLUMN.attempt)
            .one(conn)
            .await?
            .map(|row| row.attempt + 1)
            .unwrap_or(1),
        None => 1,
    };
    // The draft paragraphs let the Demo build a `replace_text` whose `old` occurs exactly once
    // (D-077).
    let draft = match session.chapter_id {
        Some(chapter_id) => {
            let chapter = chapters::Entity::find_by_id(chapter_id).one(conn).await?;
            match chapter {
                Some(chapter) => {
                    let content: ChapterContent = chapter.content.clone().into_inner();
                    content
                        .paragraphs
                        .into_iter()
                        .map(|paragraph| nekowriter_llm::DemoParagraph {
                            pid: paragraph.pid,
                            text: paragraph.text,
                        })
                        .collect()
                }
                None => Vec::new(),
            }
        }
        None => Vec::new(),
    };
    let canon_mode = match session.track_id {
        Some(track_id) => tracks::Entity::find_by_id(track_id)
            .one(conn)
            .await?
            .map(|track| track.canon_mode)
            .unwrap_or_else(|| CanonMode::Canon),
        None => CanonMode::Canon,
    };
    // The entries the research task already has, so the demo researcher continues instead of
    // duplicating (spec/04 §7.2).
    let saved_entries = match session.research_task_id {
        Some(task_id) => crate::services::research::entry_count(conn, task_id)
            .await?
            .max(0) as u32,
        None => 0,
    };
    // The character designs, for the Demo's `canon_seed` plan (spec/04 §7.2, spec/06 §7).
    let characters = match session.book_id {
        Some(book_id) => character_designs::Entity::find()
            .filter(character_designs::COLUMN.book_id.eq(book_id))
            .order_by_asc(character_designs::COLUMN.rank)
            .all(conn)
            .await?
            .into_iter()
            .map(|row| nekowriter_llm::DemoCharacter {
                name: row.name,
                role_label: row.role_label,
                summary: row.summary,
                profile: row.profile,
            })
            .collect(),
        None => Vec::new(),
    };
    // Which setting documents the author approved at their current revision, so the Demo's
    // conception plan revises only the rejected ones (spec/04 §7.2, spec/05 §7).
    let approved_docs = match session.book_id {
        Some(book_id) => setting_docs::Entity::find()
            .filter(setting_docs::COLUMN.book_id.eq(book_id))
            .all(conn)
            .await?
            .into_iter()
            .filter(|doc| doc.approved_revision == Some(doc.revision))
            .map(|doc| doc.kind.as_str().to_owned())
            .collect(),
        None => Vec::new(),
    };
    let goal_task_id = match (session.kind.as_str(), session.track_id, session.chapter_id) {
        ("chapter_maintenance", Some(track_id), Some(chapter_id)) => {
            let track = tracks::Entity::find_by_id(track_id).one(conn).await?;
            let chapter = chapters::Entity::find_by_id(chapter_id).one(conn).await?;
            match (track, chapter) {
                (Some(track), Some(chapter)) => crate::services::goals::relevant_nodes(
                    conn,
                    session
                        .book_id
                        .ok_or_else(|| AppError::internal("maintenance session has no book"))?,
                    &track,
                    &chapter,
                )
                .await?
                .nodes
                .into_iter()
                .find(|node| node.level == crate::db::json::GoalLevel::Task)
                .map(|node| node.id),
                _ => None,
            }
        }
        _ => None,
    };
    Ok(nekowriter_llm::DemoHints {
        kind: session.kind.as_str().to_owned(),
        phase: session.phase.as_str().to_owned(),
        granularity: session.granularity.map(|value| value.as_str().to_owned()),
        step_in_phase: counters.steps.max(0) as u32,
        language,
        tools: frozen_tool_names(session),
        protagonist: protagonist_name(conn, session).await?,
        review_attempt: review_attempt.max(1) as u32,
        canon_mode: canon_mode.as_str().to_owned(),
        draft,
        saved_entries,
        characters,
        approved_docs,
        goal_task_id,
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
                .filter(entities::COLUMN.book_id.eq(book_id))
                .filter(entities::COLUMN.is_protagonist.eq(true))
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
            // The rollover block list is its own: it deliberately excludes
            // `block.recent_chapters` (design §7, session-kind table), so `block.memory` must keep
            // `previous_tail` here even though the parent is a `chapter_write` session.
            let mut emitted_blocks = vec![
                "block.settings",
                "block.canon_systems",
                "block.writing_skills",
                "block.memory",
                "block.goals",
                "block.chapter_draft",
                "writer.kickoff.rollover",
            ];
            // Rollover has its own list; apply the same system-owned block filter as
            // `assemble_kickoff`, reading the child's inherited frozen systems (R1, task 13.4).
            let systems = session::canon_systems_of(&parent);
            emitted_blocks.retain(|key| {
                system_of_block(key).is_none_or(|system| system_is_on(&systems, system))
            });
            let chapter_draft_renders =
                block_renders_non_empty(tx, &prompts, &parent, plan, "block.chapter_draft", None)
                    .await?;
            for key in emitted_blocks.iter().copied() {
                let context = block_context(
                    tx,
                    &prompts,
                    &parent,
                    plan,
                    RenderedBlocks {
                        chapter_draft: chapter_draft_renders,
                        ..RenderedBlocks::default()
                    },
                    key,
                    None,
                )
                .await?;
                let rendered = prompts
                    .render_value(key, crate::prompts::template_value(&context))
                    .map_err(|err| err.into_app_error())?;
                if !rendered.text.trim().is_empty() {
                    // Only the settings block places a cache breakpoint (spec/03 §10); the others
                    // store no flag, so `StoredBlock::cache` stays absent. Writing `Some(false)`
                    // here would claim a decision the spec does not make.
                    blocks.push(StoredBlock::Text {
                        text: rendered.text,
                        cache: if block_is_cached(key) {
                            Some(true)
                        } else {
                            None
                        },
                    });
                }
            }
            let now = chrono::Utc::now();
            let child = agent_sessions::ActiveModel {
                book_id: sea_orm::Set(parent.book_id),
                role: sea_orm::Set(parent.role),
                kind: sea_orm::Set(kind),
                chapter_id: sea_orm::Set(parent.chapter_id),
                track_id: sea_orm::Set(parent.track_id),
                research_task_id: sea_orm::Set(None),
                model_profile_id: sea_orm::Set(parent.model_profile_id),
                model_snapshot: sea_orm::Set(parent.model_snapshot.clone()),
                system_prompt: sea_orm::Set(parent.system_prompt.clone()),
                tools: sea_orm::Set(parent.tools.clone()),
                status: sea_orm::Set(json::SessionStatus::Running),
                phase: sea_orm::Set(parent.phase),
                granularity: sea_orm::Set(parent.granularity),
                auto_pause: sea_orm::Set(parent.auto_pause),
                pause_after_outline: sea_orm::Set(parent.pause_after_outline),
                pause_requested: sea_orm::Set(false),
                held_tool_call_id: sea_orm::Set(None),
                base_content_version: sea_orm::Set(parent.base_content_version),
                pending_comment: sea_orm::Set(parent.pending_comment.clone()),
                view_snapshot: sea_orm::Set(None),
                counters: sea_orm::Set(JsonText(SessionCounters::default())),
                usage: sea_orm::Set(JsonText(Usage::default())),
                error: sea_orm::Set(None),
                parent_session_id: sea_orm::Set(Some(parent.id)),
                canon_systems: sea_orm::Set(parent.canon_systems.clone()),
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
        // `finish_revision` (M6.2, spec/05 §4): a revision version, then `in_review`.
        CompletionKind::Revision => {
            if let Some(chapter_id) = session.chapter_id {
                events.extend(
                    super::completion::finish_revision(tx, session, call, chapter_id).await?,
                );
            }
        }
        // M6.1 owns the review report.
        CompletionKind::Review => {}
        // `finish_maintenance` (M6.3, spec/06 §6): `synced`, or `stale` when the author edited
        // the text during maintenance (D-026). A `canon_seed` session has no chapter: its
        // completion links the remaining character designs (spec/06 §7).
        CompletionKind::Maintenance => {
            if let Some(chapter_id) = session.chapter_id {
                events.extend(
                    super::completion::finish_maintenance(tx, session, call, chapter_id).await?,
                );
            } else if super::session::kind_of(session) == SessionKind::CanonSeed {
                events.extend(super::completion::finish_seed(tx, session).await?);
            }
        }
        // `submit_outline` in an outline session: the staged outline was applied by the
        // finalizing transaction; the completion itself has no further domain effect (D-146).
        CompletionKind::Outline => {}
        // `finish_research` (M8.3, spec/07 §8 step 5): the task is `done`, with its report and the
        // count of the entries every run of it created.
        CompletionKind::Research => {
            let report = Some(call.input.as_str())
                .filter(|raw| !raw.is_empty())
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .and_then(|value| {
                    value
                        .get("report")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            events.extend(crate::services::research::finish(tx, session, &report).await?);
        }
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
    message.request_meta.clone().map(|value| value.into_inner())
}

/// Reads the frozen tool names of a session (spec/03 §1.3).
pub fn frozen_tool_names(session: &agent_sessions::Model) -> Vec<String> {
    let tools: Vec<nekowriter_llm::ToolDef> = session.tools.clone().into_inner();
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
        .filter(chapter_versions::COLUMN.chapter_id.eq(chapter_id))
        .order_by_desc(chapter_versions::COLUMN.id)
        .one(conn)
        .await?;
    Ok(row.map(|row| row.kind))
}

/// The calls of a message that are still unresolved (used by tests and the debug endpoint).
pub async fn unresolved_calls(
    conn: &impl ConnectionTrait,
    message_id: i32,
) -> Result<Vec<agent_tool_calls::Model>, AppError> {
    Ok(agent_tool_calls::Entity::find()
        .filter(agent_tool_calls::COLUMN.message_id.eq(message_id))
        .filter(agent_tool_calls::COLUMN.status.is_in([
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
        .filter(
            crate::db::entities::agent_inbox::COLUMN
                .session_id
                .eq(session_id),
        )
        .filter(
            crate::db::entities::agent_inbox::COLUMN
                .consumed_at
                .is_null(),
        )
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
        .filter(setting_docs::COLUMN.book_id.eq(book_id))
        .all(conn)
        .await?;
    Ok(docs
        .into_iter()
        .map(|doc| (doc.kind.as_str().to_owned(), doc.content))
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
