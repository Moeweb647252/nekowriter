//! Typed context structs of the prompt templates (spec/09 §2).
//!
//! One struct per template-variable family. Code builds these, serializes them with serde and
//! hands the result to [`PromptSet::render`](super::PromptSet::render); no template ever receives
//! a hand-built JSON object, so a renamed field is a compile error rather than a runtime
//! undefined. Labels (every `*_label`) are already rendered by the registry through
//! `shared.label`/`shared.chapter_label` (spec/09 §1); this module carries no model-facing text.

use serde::Serialize;

/// `book` (`vars.book` in `variables.toml`).
#[derive(Debug, Clone, Serialize)]
pub struct BookContext {
    pub title: String,
    pub genre: String,
    pub idea: String,
    /// Writing language code (BCP-47).
    pub writing_language: String,
    /// Writing language display name, rendered through `shared.label` (`language`).
    pub writing_language_name: String,
    /// True for languages written without spaces (zh, ja, …).
    pub unspaced: bool,
}

/// `track` (`vars.track`).
#[derive(Debug, Clone, Serialize)]
pub struct TrackContext {
    pub is_main: bool,
    /// `main` | `side`.
    pub kind: String,
    /// Empty for the main track.
    pub title: String,
    pub synopsis: String,
    /// `canon` | `isolated`.
    pub canon_mode: String,
    /// `none` | `before_start` | `after_chapter` | `after_end`.
    pub anchor_kind: String,
    /// Label of the anchor chapter (`after_chapter` only), rendered by `shared.chapter_label`.
    pub anchor_label: String,
}

/// `chapter` (`vars.chapter`).
#[derive(Debug, Clone, Serialize)]
pub struct ChapterContext {
    pub id: i32,
    /// Number within its track.
    pub number: i32,
    /// Label rendered by `shared.chapter_label`.
    pub label: String,
    pub title: String,
    pub guidance: String,
    pub outline: String,
    pub reference_notes: String,
    pub target_min: i32,
    pub target_max: i32,
    pub word_count: i32,
    pub has_draft: bool,
}

/// `settings` (`vars.settings`) for `block.settings`.
#[derive(Debug, Clone, Serialize)]
pub struct SettingsContext {
    pub outline: String,
    pub world: String,
    pub protagonist: String,
    pub characters: Vec<CharacterContext>,
    pub scopes: Vec<ScopeContext>,
}

/// One character design inside `settings.characters`.
#[derive(Debug, Clone, Serialize)]
pub struct CharacterContext {
    pub rank: i32,
    pub name: String,
    /// The stored role label (the design's Chinese protagonist/antagonist wording, D-041):
    /// the one label that is stored text rather than rendered (spec/09 §1).
    pub role_label: String,
    pub summary: String,
    pub profile: String,
}

/// One attached knowledge scope inside `settings.scopes`.
#[derive(Debug, Clone, Serialize)]
pub struct ScopeContext {
    pub name: String,
    pub description: String,
}

/// One cast-index entry of `block.cast_index`.
#[derive(Debug, Clone, Serialize)]
pub struct CastEntryContext {
    pub id: i32,
    pub name: String,
    /// Rendered by `shared.label` (`entity_kind`).
    pub kind_label: String,
    pub status: String,
    /// Rendered by `shared.label` (`entity_status`).
    pub status_label: String,
    pub aliases: Vec<String>,
    pub summary: String,
}

/// `skills` (`vars.skills`) for `block.writing_skills` (design §6).
#[derive(Debug, Clone, Serialize)]
pub struct WritingSkillsContext {
    pub skills: Vec<WritingSkillContext>,
}

/// One selected writing skill: its full body, not a summary (design §6: injection is full).
#[derive(Debug, Clone, Serialize)]
pub struct WritingSkillContext {
    pub name: String,
    pub category: String,
    pub body: String,
}

/// `chapters` (`vars.chapters`) for `block.recent_chapters` (design §7).
#[derive(Debug, Clone, Serialize)]
pub struct RecentChaptersContext {
    pub chapters: Vec<RecentChapterContext>,
}

/// One recent official chapter injected in full (or front-cut, with `partial` set).
#[derive(Debug, Clone, Serialize)]
pub struct RecentChapterContext {
    /// Number within its own track (the label disambiguates a main-track top-up for a side story).
    pub number: i32,
    /// Rendered by `shared.chapter_label`; carries the track title for a side story's top-up.
    pub label: String,
    pub title: String,
    pub text: String,
    /// True when the chapter exceeded its budget and its beginning was omitted.
    pub partial: bool,
}

/// `systems` (`vars.systems`) for `block.canon_systems`, `block.memory`, `block.goals` and the
/// three archivist tasks (D-150).
#[derive(Debug, Clone, Serialize)]
pub struct SystemsContext {
    pub entities: bool,
    pub timeline: bool,
    pub memory: bool,
    pub foreshadowing: bool,
    pub goals: bool,
    /// The enabled canon systems in ownership order (`entities`, `timeline`, `memory`,
    /// `foreshadowing`); templates use it to join only the enabled kinds.
    pub enabled: Vec<String>,
}

impl From<&crate::db::json::CanonSystems> for SystemsContext {
    fn from(systems: &crate::db::json::CanonSystems) -> Self {
        let mut enabled = Vec::new();
        if systems.entities {
            enabled.push("entities".to_owned());
        }
        if systems.timeline {
            enabled.push("timeline".to_owned());
        }
        if systems.memory {
            enabled.push("memory".to_owned());
        }
        if systems.foreshadowing {
            enabled.push("foreshadowing".to_owned());
        }
        Self {
            entities: systems.entities,
            timeline: systems.timeline,
            memory: systems.memory,
            foreshadowing: systems.foreshadowing,
            goals: systems.goals,
            enabled,
        }
    }
}

/// `goals` (`vars.goals`) for `block.goals`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GoalsContext {
    /// Relevant nodes in render order: goals, then plans, then tasks under their parents.
    pub nodes: Vec<GoalNodeContext>,
    /// Relevant nodes left out by the 30-node cap.
    pub omitted: i32,
}

/// One node of `block.goals`.
#[derive(Debug, Clone, Serialize)]
pub struct GoalNodeContext {
    pub id: i32,
    /// `goal` | `plan` | `task` (the template uses it as the tag name).
    pub level: String,
    pub title: String,
    pub purpose: String,
    /// `chapters` attribute, e.g. `12–15` (empty when unbounded).
    pub chapters: String,
    pub overdue: bool,
    /// Format `"名字(#12)"`, only when Entities is on (plan §4.5).
    pub entities: Vec<String>,
    /// Format `"#3 标题"`, only when Timeline is on.
    pub events: Vec<String>,
    /// Format `"#9 标题"`, only when Foreshadowing is on.
    pub foreshadowing: Vec<String>,
}

/// `memory` (`vars.memory`) for `block.memory`.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryContext {
    pub is_first: bool,
    /// Number of chapters in the mid-term window.
    pub window_mid: i32,
    pub recent_summaries: Vec<ChapterSummaryContext>,
    pub previous_tail: Option<PreviousTailContext>,
    pub protagonist: Option<ProtagonistStateContext>,
    pub recent_changes: Vec<RecentChangeContext>,
    pub appearances: Vec<AppearanceCountContext>,
    pub absent_important: Vec<AbsentEntityContext>,
    pub key_notes: Vec<KeyNoteContext>,
    pub open_foreshadowing: Vec<OpenForeshadowingContext>,
    pub cast: Vec<CastSummaryContext>,
    pub counts: MemoryCountsContext,
}

/// One recent chapter summary.
#[derive(Debug, Clone, Serialize)]
pub struct ChapterSummaryContext {
    /// Rendered by `shared.chapter_label`.
    pub label: String,
    pub title: String,
    pub summary: String,
}

/// The tail of the previous chapter.
#[derive(Debug, Clone, Serialize)]
pub struct PreviousTailContext {
    pub label: String,
    pub text: String,
}

/// The protagonist's current state.
#[derive(Debug, Clone, Serialize)]
pub struct ProtagonistStateContext {
    pub id: i32,
    pub name: String,
    pub status: String,
    pub status_label: String,
    pub state: String,
    /// In a side story: the newest change is from after the anchor (spec/06 §4.1).
    pub after_anchor: bool,
    pub attributes: Vec<AttributeContext>,
}

/// One entity attribute.
#[derive(Debug, Clone, Serialize)]
pub struct AttributeContext {
    pub key: String,
    pub value: String,
}

/// One recent canon change.
#[derive(Debug, Clone, Serialize)]
pub struct RecentChangeContext {
    pub chapter_label: String,
    pub target: String,
    pub note: String,
}

/// An entity's appearance count in the mid-term window.
#[derive(Debug, Clone, Serialize)]
pub struct AppearanceCountContext {
    pub id: i32,
    pub name: String,
    pub count: i32,
    pub chapters: Vec<AppearanceChapterContext>,
}

/// One chapter an entity appeared in.
#[derive(Debug, Clone, Serialize)]
pub struct AppearanceChapterContext {
    pub label: String,
    /// Rendered by `shared.label` (`appearance_role`).
    pub role_label: String,
}

/// An important entity that has not appeared lately.
#[derive(Debug, Clone, Serialize)]
pub struct AbsentEntityContext {
    pub id: i32,
    pub name: String,
    pub last_label: String,
}

/// One key memory note.
#[derive(Debug, Clone, Serialize)]
pub struct KeyNoteContext {
    pub id: i32,
    /// Rendered by `shared.label` (`note_kind`).
    pub kind_label: String,
    pub content: String,
}

/// One open foreshadowing.
#[derive(Debug, Clone, Serialize)]
pub struct OpenForeshadowingContext {
    pub id: i32,
    pub title: String,
    pub planted_label: String,
}

/// One cast summary (long-term memory).
#[derive(Debug, Clone, Serialize)]
pub struct CastSummaryContext {
    pub id: i32,
    pub name: String,
    pub kind_label: String,
    pub summary: String,
}

/// The counts line of `block.memory`.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryCountsContext {
    pub character: i32,
    pub organization: i32,
    pub location: i32,
    pub item: i32,
    pub other: i32,
    pub event: i32,
    pub fact: i32,
    pub world_rule: i32,
    pub foreshadowing_open: i32,
}

/// `draft` (`vars.draft`) for `block.chapter_draft` and the writer tasks.
#[derive(Debug, Clone, Serialize)]
pub struct DraftContext {
    pub paragraphs: Vec<ParagraphContext>,
    pub tail_open: bool,
    pub word_count: i32,
    pub paragraph_count: i32,
}

/// One paragraph of a draft or a chapter text.
#[derive(Debug, Clone, Serialize)]
pub struct ParagraphContext {
    pub pid: String,
    pub text: String,
}

/// `feedback` (`vars.feedback`): the author's feedback with its ticked issues.
#[derive(Debug, Clone, Serialize)]
pub struct FeedbackContext {
    pub text: String,
    pub issues: Vec<FeedbackIssueContext>,
}

/// One ticked review issue inside `feedback`.
#[derive(Debug, Clone, Serialize)]
pub struct FeedbackIssueContext {
    /// Rendered by `shared.label` (`severity`).
    pub severity_label: String,
    /// Rendered by `shared.label` (`category`).
    pub category_label: String,
    pub pids: Vec<String>,
    pub quote: String,
    pub problem: String,
    pub suggestion: String,
}

/// `granularity` (`vars.granularity`) for the writer rules and notices.
#[derive(Debug, Clone, Serialize)]
pub struct GranularityContext {
    /// `sentence` | `paragraph` | `free` | `chapter`.
    pub key: String,
    /// Rendered by `shared.label` (`granularity`).
    pub label: String,
    /// True in the revision phases.
    pub revising: bool,
    pub tail_open: bool,
    pub last_pid: String,
}

/// `prior` (`vars.prior`): the previous review report a re-review replaces.
#[derive(Debug, Clone, Serialize)]
pub struct PriorReportContext {
    pub attempt: i32,
    pub verdict_label: String,
    pub summary: String,
    pub feedback: String,
    /// The author's comment on that report.
    pub comment: String,
}

/// `review` (`vars.review`) for `reviewer.rereview`.
#[derive(Debug, Clone, Serialize)]
pub struct RereviewContext {
    pub comment: String,
    pub attempt: i32,
}

/// `research` (`vars.research`) for `researcher.kickoff`.
#[derive(Debug, Clone, Serialize)]
pub struct ResearchContext {
    pub topic: String,
    pub requirements: String,
    pub scope_name: String,
    pub scope_description: String,
    pub language_name: String,
    pub existing_titles: Vec<String>,
    pub max_tool_calls: i32,
    pub max_entries: i32,
}

/// `conception` (`vars.conception`) for `conception.kickoff`.
#[derive(Debug, Clone, Serialize)]
pub struct ConceptionContext {
    pub title: String,
    pub idea: String,
    pub genre: String,
    pub writing_language_name: String,
}

/// `diff` (`vars.diff`) for `block.chapter_diff`.
#[derive(Debug, Clone, Serialize)]
pub struct DiffContext {
    pub changes: Vec<DiffChangeContext>,
}

/// One changed paragraph of a chapter diff.
#[derive(Debug, Clone, Serialize)]
pub struct DiffChangeContext {
    /// `insert` | `delete` | `modify`.
    pub op: String,
    pub pid: String,
    pub after: Option<String>,
    pub old: String,
    pub new: String,
}

/// The `counts` of `notice.user_edits`.
#[derive(Debug, Clone, Serialize)]
pub struct EditCountsContext {
    pub modified: i32,
    pub deleted: i32,
    pub inserted: i32,
}

/// One modified paragraph of `notice.user_edits`.
#[derive(Debug, Clone, Serialize)]
pub struct ModifiedParagraphContext {
    pub pid: String,
    pub old: String,
    pub new: String,
}

/// One deleted paragraph of `notice.user_edits`.
#[derive(Debug, Clone, Serialize)]
pub struct DeletedParagraphContext {
    pub pid: String,
    pub old: String,
}

/// One inserted paragraph of `notice.user_edits`.
#[derive(Debug, Clone, Serialize)]
pub struct InsertedParagraphContext {
    pub pid: String,
    pub after: Option<String>,
    pub text: String,
}

/// `notice.setting_edits`: one changed document, character list or topic list.
#[derive(Debug, Clone, Serialize)]
pub struct SettingEditContext {
    pub doc_label: String,
    /// `doc` | `characters` | `topics`.
    pub kind: String,
    pub full_text: String,
    pub lines: Vec<SettingEditLineContext>,
    pub characters: Vec<SettingEditCharacterContext>,
    pub topics: Vec<SettingEditTopicContext>,
}

/// One changed line of a setting document.
#[derive(Debug, Clone, Serialize)]
pub struct SettingEditLineContext {
    /// `+` | `-`.
    pub op: String,
    pub text: String,
}

/// One changed character design inside `notice.setting_edits`.
#[derive(Debug, Clone, Serialize)]
pub struct SettingEditCharacterContext {
    pub rank: i32,
    pub name: String,
    pub role_label: String,
    pub summary: String,
    pub profile: String,
    pub changed_profile: String,
}

/// One changed research topic inside `notice.setting_edits`.
#[derive(Debug, Clone, Serialize)]
pub struct SettingEditTopicContext {
    pub topic: String,
    pub scope: String,
    pub requirements: String,
}

/// One reviewed document of `result.request_review`.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewItemContext {
    pub doc_label: String,
    pub approved: bool,
    pub feedback: String,
}

/// One answer of `result.ask_user`.
#[derive(Debug, Clone, Serialize)]
pub struct AskUserAnswerContext {
    pub question: String,
    pub selected: Vec<String>,
    pub other: String,
}

/// One relation line of `result.entity_card` / `result.relations`.
#[derive(Debug, Clone, Serialize)]
pub struct RelationLineContext {
    pub subject: String,
    pub object: String,
    pub kind: String,
    pub ended: bool,
    pub until_label: String,
    pub description: String,
    pub since_label: String,
}

/// One recent change line of `result.entity_card`.
#[derive(Debug, Clone, Serialize)]
pub struct ChangeLineContext {
    pub chapter_label: String,
    pub note: String,
    pub after_anchor: bool,
}

/// The appearance summary of `result.entity_card`.
#[derive(Debug, Clone, Serialize)]
pub struct AppearanceSummaryContext {
    pub first_label: String,
    pub last_label: String,
    pub window_size: i32,
    pub window_count: i32,
}

/// The design link of `result.entity_card`.
#[derive(Debug, Clone, Serialize)]
pub struct DesignLinkContext {
    pub role_label: String,
    pub summary: String,
}
