//! The tool registry: the fixed per-role tool lists, phase permissions and the handlers of this
//! milestone (spec/05 §2, spec/03 §1.2).
//!
//! Each role has **one fixed, ordered tool list** frozen into the session at creation, so the
//! provider's cached prefix stays valid (D-007). Phases only restrict which tools may run: a call
//! to a tool that is not permitted in the current phase is answered with `error.tool_not_allowed`
//! and has no effect.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::db::json::{AgentRole, CanonSystem, CanonSystems, Phase, SessionKind};

use super::ToolHandler;

/// Tools that change chapter text and hold under `auto_pause` (spec/03 §5.2).
pub const WRITE_TOOLS: [&str; 5] = [
    "insert_text",
    "insert_paragraphs",
    "replace_text",
    "rewrite_paragraph",
    "delete_paragraphs",
];

/// True for a tool that edits chapter text, which `auto_pause` holds (spec/03 §5.2).
///
/// The test-only `test_write` counts as one when the `testing` feature is on, so the runtime
/// tests can drive the hold path without the writer tools of M4 (spec/03 §15 test 3's mechanism).
pub fn is_write_tool(name: &str) -> bool {
    if WRITE_TOOLS.contains(&name) {
        return true;
    }
    #[cfg(any(test, feature = "testing"))]
    if name == "test_write" {
        return true;
    }
    false
}

/// The writer role's retrieval tools: the first ten of its list (spec/05 §2).
pub const RETRIEVAL_TOOLS: [&str; 10] = [
    "get_entity",
    "list_entities",
    "get_relations",
    "query_timeline",
    "list_foreshadowing",
    "search_canon",
    "search_knowledge",
    "read_knowledge_entry",
    "search_chapters",
    "read_chapter",
];

/// The planner role's fixed tool list, in order (spec/05 §2, D-146): the ten retrieval tools,
/// `read_draft` and the terminal `submit_outline`.
pub const PLANNER_TOOLS: [&str; 12] = [
    "get_entity",
    "list_entities",
    "get_relations",
    "query_timeline",
    "list_foreshadowing",
    "search_canon",
    "search_knowledge",
    "read_knowledge_entry",
    "search_chapters",
    "read_chapter",
    "read_draft",
    "submit_outline",
];

/// The writer role's fixed tool list, in order (spec/05 §2).
///
/// Drafting-only since D-146: `submit_outline` belongs to the planner's list.
pub const WRITER_TOOLS: [&str; 19] = [
    "get_entity",
    "list_entities",
    "get_relations",
    "query_timeline",
    "list_foreshadowing",
    "search_canon",
    "search_knowledge",
    "read_knowledge_entry",
    "search_chapters",
    "read_chapter",
    "submit_revision_plan",
    "read_draft",
    "insert_text",
    "insert_paragraphs",
    "replace_text",
    "rewrite_paragraph",
    "delete_paragraphs",
    "finish_draft",
    "finish_revision",
];

/// The reviewer role's fixed tool list (spec/05 §2).
pub const REVIEWER_TOOLS: [&str; 11] = [
    "get_entity",
    "list_entities",
    "get_relations",
    "query_timeline",
    "list_foreshadowing",
    "search_canon",
    "search_knowledge",
    "read_knowledge_entry",
    "search_chapters",
    "read_chapter",
    "submit_review",
];

/// The archivist role's fixed tool list (spec/05 §2).
pub const ARCHIVIST_TOOLS: [&str; 22] = [
    "get_entity",
    "list_entities",
    "get_relations",
    "query_timeline",
    "list_foreshadowing",
    "search_canon",
    "read_chapter",
    "create_entity",
    "update_entity",
    "set_relation",
    "end_relation",
    "record_appearances",
    "add_timeline_event",
    "update_timeline_event",
    "add_memory_note",
    "retire_memory_note",
    "add_foreshadowing",
    "resolve_foreshadowing",
    "set_chapter_summary",
    "propose_goal_change",
    "propose_setting_change",
    "finish_maintenance",
];

/// The conception role's fixed tool list (spec/05 §2).
pub const CONCEPTION_TOOLS: [&str; 12] = [
    "ask_user",
    "search_knowledge",
    "read_knowledge_entry",
    "web_search",
    "web_fetch",
    "write_setting_doc",
    "edit_setting_doc",
    "upsert_character",
    "remove_character",
    "reorder_characters",
    "set_research_topics",
    "request_review",
];

/// The researcher role's fixed tool list (spec/05 §2).
///
/// A topic session freezes [`RESEARCH_TOPIC_TOOLS`]; a knowledge check freezes
/// [`RESEARCH_CHECK_TOOLS`]. `set_target_scope` is deliberately absent from the role list but stays
/// registered, because a session frozen before M20 may still call it (D-168, R8).
pub const RESEARCHER_TOOLS: [&str; 8] = [
    "web_search",
    "web_fetch",
    "search_knowledge",
    "read_knowledge_entry",
    "set_research_topics",
    "save_knowledge_entry",
    "update_knowledge_entry",
    "finish_research",
];

/// The seven tools a topic research session freezes (the pre-M20 baseline).
pub const RESEARCH_TOPIC_TOOLS: [&str; 7] = [
    "web_search",
    "web_fetch",
    "search_knowledge",
    "read_knowledge_entry",
    "save_knowledge_entry",
    "update_knowledge_entry",
    "finish_research",
];

/// The four tools a knowledge check freezes: it plans, it does not collect (D-168, R1).
pub const RESEARCH_CHECK_TOOLS: [&str; 4] = [
    "search_knowledge",
    "read_knowledge_entry",
    "set_research_topics",
    "finish_research",
];

/// The test-only tools of the debug session (spec/10 §2.7, PLAN M3.5).
pub const DEBUG_TOOLS: [&str; 5] = ["echo", "test_write", "test_finish", "test_slow", "test_ask"];

/// The frozen tool list of a role, in its fixed order (spec/05 §2).
pub fn tools_for_role(role: &str) -> &'static [&'static str] {
    match role {
        "planner" => &PLANNER_TOOLS,
        "writer" => &WRITER_TOOLS,
        "reviewer" => &REVIEWER_TOOLS,
        "archivist" => &ARCHIVIST_TOOLS,
        "conception" => &CONCEPTION_TOOLS,
        "researcher" => &RESEARCHER_TOOLS,
        _ => &[],
    }
}

/// The canon system that owns a tool (spec/06 §10.1), if any.
pub fn system_of_tool(tool: &str) -> Option<CanonSystem> {
    match tool {
        "get_entity" | "list_entities" | "get_relations" | "create_entity" | "update_entity"
        | "set_relation" | "end_relation" | "record_appearances" => Some(CanonSystem::Entities),
        "query_timeline" | "add_timeline_event" | "update_timeline_event" => {
            Some(CanonSystem::Timeline)
        }
        "add_memory_note" | "retire_memory_note" | "set_chapter_summary" => {
            Some(CanonSystem::Memory)
        }
        "list_foreshadowing" | "add_foreshadowing" | "resolve_foreshadowing" => {
            Some(CanonSystem::Foreshadowing)
        }
        "propose_goal_change" => Some(CanonSystem::Goals),
        _ => None,
    }
}

/// True when a tool's owning system is on; tools that belong to no system always pass.
pub fn tool_enabled(tool: &str, systems: &CanonSystems) -> bool {
    match system_of_tool(tool) {
        Some(CanonSystem::Entities) => systems.entities,
        Some(CanonSystem::Timeline) => systems.timeline,
        Some(CanonSystem::Memory) => systems.memory,
        Some(CanonSystem::Foreshadowing) => systems.foreshadowing,
        Some(CanonSystem::Goals) => systems.goals,
        None => true,
    }
}

/// The frozen tool list for a session: the role's fixed order minus disabled systems
/// (spec/05 §2, spec/03 §1.2). `search_canon` survives unless all four systems it reads are off;
/// `propose_goal_change` also needs a maintenance session whose chapter has relevant goals.
pub fn tools_for_session(
    role: &str,
    kind: SessionKind,
    systems: &CanonSystems,
    has_relevant_goals: bool,
    preparation: bool,
    registry: &ToolRegistry,
) -> Vec<&'static str> {
    let role_tools: &[&str] = if role == "researcher" {
        if preparation {
            &RESEARCH_CHECK_TOOLS
        } else {
            &RESEARCH_TOPIC_TOOLS
        }
    } else {
        tools_for_role(role)
    };
    role_tools
        .iter()
        .copied()
        .filter(|tool| registry.has(tool))
        // `propose_goal_change` is frozen only for a maintenance session with relevant goals.
        .filter(|tool| match *tool {
            "search_canon" => systems.any_search_system(),
            "propose_goal_change" => {
                kind == SessionKind::ChapterMaintenance && systems.goals && has_relevant_goals
            }
            other => tool_enabled(other, systems),
        })
        .collect()
}

/// True when `tool` may run in this session (spec/03 §1.2, spec/05 §2).
///
/// The session's **frozen** tool list is the authority on which tools exist for the session
/// (spec/03 §1.2); the phase and kind rules only restrict which of them may run now. A session
/// without phases permits every tool it froze; `canon_seed` forbids the three chapter-scoped
/// archivist tools (spec/05 §2).
pub fn is_permitted(kind: SessionKind, phase: Phase, tool: &str, frozen: &[String]) -> bool {
    if !frozen.iter().any(|name| name == tool) {
        // The session never froze this tool.
        return false;
    }
    // The test-only tools of the debug session have no phase rules: they exist so the runtime
    // paths can be driven without a real tool (spec/10 §2.7, PLAN M3.1). The debug session is the
    // only one that freezes them, so this cannot widen a real session (D-072).
    if DEBUG_TOOLS.contains(&tool) {
        return true;
    }
    match kind {
        SessionKind::ChapterWrite | SessionKind::ChapterRevise => match phase {
            Phase::Drafting => {
                matches!(tool, "read_draft" | "finish_draft") || WRITE_TOOLS.contains(&tool)
            }
            Phase::RevisionPlanning => {
                RETRIEVAL_TOOLS.contains(&tool)
                    || matches!(tool, "read_draft" | "submit_revision_plan")
            }
            Phase::Revising => {
                matches!(tool, "read_draft" | "finish_revision") || WRITE_TOOLS.contains(&tool)
            }
            // `Planning` is historical: an old frozen row may still carry it, but no tool runs.
            Phase::None | Phase::Planning => false,
        },
        SessionKind::CanonSeed => !matches!(
            tool,
            "record_appearances" | "set_chapter_summary" | "read_chapter"
        ),
        // Phase-free kinds permit every tool they froze (the role list already encoded them at
        // creation).
        SessionKind::ChapterOutline
        | SessionKind::ChapterMaintenance
        | SessionKind::ChapterResync
        | SessionKind::ChapterReview
        | SessionKind::Conception
        | SessionKind::Research => true,
    }
}

/// The session's frozen tools that its kind and phase permit, in frozen order
/// (used by `error.tool_not_allowed`).
pub fn allowed_tools(kind: SessionKind, phase: Phase, frozen: &[String]) -> Vec<String> {
    frozen
        .iter()
        .filter(|tool| is_permitted(kind, phase, tool, frozen))
        .cloned()
        .collect()
}

/// The typed role that runs a session kind (spec/03 §1.1).
pub fn role_enum_of_kind(kind: SessionKind) -> AgentRole {
    match kind {
        SessionKind::Conception => AgentRole::Conception,
        SessionKind::Research => AgentRole::Researcher,
        SessionKind::ChapterOutline => AgentRole::Planner,
        SessionKind::ChapterWrite | SessionKind::ChapterRevise => AgentRole::Writer,
        SessionKind::ChapterReview => AgentRole::Reviewer,
        SessionKind::ChapterMaintenance | SessionKind::CanonSeed | SessionKind::ChapterResync => {
            AgentRole::Archivist
        }
    }
}

/// The role that runs a session kind (spec/03 §1.1).
pub fn role_of_kind(kind: SessionKind) -> &'static str {
    role_enum_of_kind(kind).as_str()
}

/// The initial phase of a session kind (spec/03 §1.1, §4 step 3).
pub fn initial_phase(kind: SessionKind) -> Phase {
    match kind {
        SessionKind::ChapterWrite => Phase::Drafting,
        SessionKind::ChapterRevise => Phase::RevisionPlanning,
        _ => Phase::None,
    }
}

/// The role's `shared.label` enum name for the phase, used for `phase_label`
/// (`notice.tool_not_allowed`).
pub fn phase_label(prompts: &crate::prompts::PromptSet, phase: Phase) -> String {
    prompts.label("phase", phase.as_str())
}

/// The tool handlers compiled into the runtime, by name.
pub struct ToolRegistry {
    handlers: BTreeMap<String, Arc<dyn ToolHandler>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    /// The production registry: the test-only tools are excluded unless the debug session asks
    /// for them, because they are not part of any role list.
    pub fn new() -> Self {
        let mut handlers: BTreeMap<String, Arc<dyn ToolHandler>> = BTreeMap::new();
        // The writer tools of M4 (spec/05 §4). They are registered unconditionally: a session that
        // did not freeze one can still never run it (spec/03 §1.2, D-072).
        for handler in super::writer::all() {
            handlers.insert(handler.name().to_owned(), handler);
        }
        // The retrieval tools of M5 (spec/05 §3).
        for handler in super::retrieval::all() {
            handlers.insert(handler.name().to_owned(), handler);
        }
        // The four search tools of M7 (spec/05 §3).
        for handler in super::search::all() {
            handlers.insert(handler.name().to_owned(), handler);
        }
        // The reviewer tool of M6.1 (spec/05 §5).
        for handler in super::reviewer::all() {
            handlers.insert(handler.name().to_owned(), handler);
        }
        // The archivist write tools and the setting-proposal tool of M6.3–6.4 (spec/05 §6).
        for handler in super::archivist::all() {
            handlers.insert(handler.name().to_owned(), handler);
        }
        for handler in super::proposals::all() {
            handlers.insert(handler.name().to_owned(), handler);
        }
        for handler in super::goal_proposals::all() {
            handlers.insert(handler.name().to_owned(), handler);
        }
        // The two web tools of M8 (spec/05 §7–§8). They are the only path to the network.
        for handler in super::web::all() {
            handlers.insert(handler.name().to_owned(), handler);
        }
        // The researcher's write tools (spec/05 §8): target-scope only.
        for handler in super::research::all() {
            handlers.insert(handler.name().to_owned(), handler);
        }
        // The conception tools of M9 (spec/05 §7).
        for handler in super::conception::all() {
            handlers.insert(handler.name().to_owned(), handler);
        }
        for handler in super::test_tools::all() {
            handlers.insert(handler.name().to_owned(), handler);
        }
        Self { handlers }
    }

    /// A registry with the debug tools only (the test-only session of spec/10 §2.7).
    pub fn debug() -> Self {
        Self::new()
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn ToolHandler>> {
        self.handlers.get(name)
    }

    /// True when the runtime knows a handler for the tool (the definition is separate).
    pub fn has(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.handlers.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every tool name any role may hold (so the frozen-list filter never hides a name).
    fn all_frozen() -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for role in [
            "planner",
            "writer",
            "reviewer",
            "archivist",
            "conception",
            "researcher",
        ] {
            for name in tools_for_role(role) {
                names.push((*name).to_owned());
            }
        }
        for name in DEBUG_TOOLS {
            names.push(name.to_owned());
        }
        names
    }

    fn writer_frozen() -> Vec<String> {
        WRITER_TOOLS.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn switch_filtering_keeps_the_fixed_order_and_the_archivist_list_grows_to_22() {
        let all = CanonSystems::default();
        let full = tools_for_session(
            "archivist",
            SessionKind::ChapterMaintenance,
            &all,
            true,
            false,
            &ToolRegistry::new(),
        );
        assert!(ToolRegistry::new().has("propose_goal_change"));
        assert_eq!(full.len(), 22);
        assert_eq!(full[19], "propose_goal_change");
        assert_eq!(full[20], "propose_setting_change");
        assert_eq!(full[21], "finish_maintenance");

        let without_relevant_goals = tools_for_session(
            "archivist",
            SessionKind::ChapterMaintenance,
            &all,
            false,
            false,
            &ToolRegistry::new(),
        );
        assert_eq!(without_relevant_goals.len(), 21);
        assert!(!without_relevant_goals.contains(&"propose_goal_change"));

        let no_entities = CanonSystems {
            entities: false,
            ..all.clone()
        };
        let filtered = tools_for_session(
            "archivist",
            SessionKind::ChapterMaintenance,
            &no_entities,
            true,
            false,
            &ToolRegistry::new(),
        );
        let expected: Vec<&str> = full
            .iter()
            .copied()
            .filter(|tool| {
                tool != &"get_entity"
                    && tool != &"list_entities"
                    && tool != &"get_relations"
                    && tool != &"create_entity"
                    && tool != &"update_entity"
                    && tool != &"set_relation"
                    && tool != &"end_relation"
                    && tool != &"record_appearances"
            })
            .collect();
        assert_eq!(filtered, expected);

        let all_off = CanonSystems {
            entities: false,
            timeline: false,
            memory: false,
            foreshadowing: false,
            goals: false,
        };
        let bare = tools_for_session(
            "archivist",
            SessionKind::ChapterMaintenance,
            &all_off,
            true,
            false,
            &ToolRegistry::new(),
        );
        assert!(!bare.contains(&"search_canon"));
        assert!(!bare.contains(&"propose_goal_change"));
        assert!(bare.contains(&"propose_setting_change"));
        assert!(bare.contains(&"finish_maintenance"));
    }

    #[test]
    fn role_lists_match_spec_05_section_2() {
        assert_eq!(tools_for_role("planner"), &PLANNER_TOOLS);
        assert_eq!(tools_for_role("writer"), &WRITER_TOOLS);
        assert_eq!(tools_for_role("reviewer"), &REVIEWER_TOOLS);
        assert_eq!(tools_for_role("archivist"), &ARCHIVIST_TOOLS);
        assert_eq!(tools_for_role("conception"), &CONCEPTION_TOOLS);
        assert_eq!(tools_for_role("researcher"), &RESEARCHER_TOOLS);
        assert_eq!(tools_for_role("unknown").len(), 0);
    }

    #[test]
    fn retrieval_tools_are_the_first_ten_writer_tools() {
        assert_eq!(&WRITER_TOOLS[..10], &RETRIEVAL_TOOLS);
        assert_eq!(&PLANNER_TOOLS[..10], &RETRIEVAL_TOOLS);
    }

    /// The planner's frozen list is its permission gate: phase-free, it may run all twelve tools
    /// and none of the drafting tools (spec/05 §2, D-146).
    #[test]
    fn the_planner_permits_its_whole_list_and_no_drafting_tool() {
        let frozen: Vec<String> = PLANNER_TOOLS
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        for tool in PLANNER_TOOLS {
            assert!(is_permitted(
                SessionKind::ChapterOutline,
                Phase::None,
                tool,
                &frozen
            ));
        }
        for tool in WRITE_TOOLS {
            assert!(!is_permitted(
                SessionKind::ChapterOutline,
                Phase::None,
                tool,
                &frozen
            ));
        }
        assert!(!is_permitted(
            SessionKind::ChapterOutline,
            Phase::None,
            "submit_outline",
            &[]
        ));
    }

    /// The phase matrix of spec/05 §2, tool by tool.
    #[test]
    fn writer_phase_permission_matrix() {
        // drafting: read_draft, DRAFT_EDIT, finish_draft
        for tool in WRITE_TOOLS {
            assert!(is_permitted(
                SessionKind::ChapterWrite,
                Phase::Drafting,
                tool,
                &all_frozen()
            ));
            assert!(!is_permitted(
                SessionKind::ChapterWrite,
                Phase::Revising,
                "submit_outline",
                &all_frozen()
            ));
        }
        assert!(is_permitted(
            SessionKind::ChapterWrite,
            Phase::Drafting,
            "finish_draft",
            &all_frozen()
        ));
        assert!(!is_permitted(
            SessionKind::ChapterWrite,
            Phase::Drafting,
            "finish_revision",
            &all_frozen()
        ));

        // revision_planning: RETRIEVAL, read_draft, submit_revision_plan
        assert!(is_permitted(
            SessionKind::ChapterRevise,
            Phase::RevisionPlanning,
            "submit_revision_plan",
            &all_frozen()
        ));
        assert!(!is_permitted(
            SessionKind::ChapterRevise,
            Phase::RevisionPlanning,
            "submit_outline",
            &all_frozen()
        ));

        // revising: read_draft, DRAFT_EDIT, finish_revision
        for tool in WRITE_TOOLS {
            assert!(is_permitted(
                SessionKind::ChapterRevise,
                Phase::Revising,
                tool,
                &all_frozen()
            ));
        }
        assert!(is_permitted(
            SessionKind::ChapterRevise,
            Phase::Revising,
            "finish_revision",
            &all_frozen()
        ));
        assert!(!is_permitted(
            SessionKind::ChapterRevise,
            Phase::Revising,
            "finish_draft",
            &all_frozen()
        ));
    }

    #[test]
    fn phase_free_sessions_allow_their_whole_role_list() {
        for tool in REVIEWER_TOOLS {
            assert!(is_permitted(
                SessionKind::ChapterReview,
                Phase::None,
                tool,
                &all_frozen()
            ));
        }
        for tool in CONCEPTION_TOOLS {
            assert!(is_permitted(
                SessionKind::Conception,
                Phase::None,
                tool,
                &all_frozen()
            ));
        }
        for tool in RESEARCHER_TOOLS {
            assert!(is_permitted(
                SessionKind::Research,
                Phase::None,
                tool,
                &all_frozen()
            ));
        }
        for tool in ARCHIVIST_TOOLS {
            assert!(is_permitted(
                SessionKind::ChapterMaintenance,
                Phase::None,
                tool,
                &all_frozen()
            ));
        }
    }

    #[test]
    fn canon_seed_forbids_the_three_chapter_tools() {
        for tool in ["record_appearances", "set_chapter_summary", "read_chapter"] {
            assert!(
                !is_permitted(SessionKind::CanonSeed, Phase::None, tool, &all_frozen()),
                "{tool} is forbidden in canon_seed"
            );
        }
        assert!(is_permitted(
            SessionKind::CanonSeed,
            Phase::None,
            "create_entity",
            &all_frozen()
        ));
        assert!(is_permitted(
            SessionKind::CanonSeed,
            Phase::None,
            "finish_maintenance",
            &all_frozen()
        ));
    }

    /// A tool the session did not freeze is never permitted, whatever the phase says
    /// (spec/03 §1.2: the frozen list is the session's tool set).
    #[test]
    fn a_tool_outside_the_frozen_list_is_never_permitted() {
        assert!(!is_permitted(
            SessionKind::ChapterReview,
            Phase::None,
            "insert_text",
            &reviewer_frozen()
        ));
        assert!(!is_permitted(
            SessionKind::ChapterWrite,
            Phase::Drafting,
            "submit_review",
            &writer_frozen()
        ));
        assert!(!is_permitted(
            SessionKind::Conception,
            Phase::None,
            "create_entity",
            &conception_frozen()
        ));
        // Even a permitted-in-principle tool is rejected when the session never froze it.
        assert!(!is_permitted(
            SessionKind::ChapterWrite,
            Phase::Drafting,
            "insert_text",
            &[]
        ));
    }

    fn reviewer_frozen() -> Vec<String> {
        REVIEWER_TOOLS
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    fn conception_frozen() -> Vec<String> {
        CONCEPTION_TOOLS
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    #[test]
    fn allowed_tools_lists_only_the_permitted_phase_tools() {
        let drafting = allowed_tools(SessionKind::ChapterWrite, Phase::Drafting, &writer_frozen());
        assert_eq!(drafting.len(), 7, "{drafting:?}");
        assert!(!drafting.contains(&"get_entity".to_owned()));
    }

    #[test]
    fn initial_phases_follow_spec_03_section_1_1() {
        assert_eq!(initial_phase(SessionKind::ChapterWrite), Phase::Drafting);
        assert_eq!(initial_phase(SessionKind::ChapterOutline), Phase::None);
        assert_eq!(
            initial_phase(SessionKind::ChapterRevise),
            Phase::RevisionPlanning
        );
        assert_eq!(initial_phase(SessionKind::ChapterReview), Phase::None);
    }
}
