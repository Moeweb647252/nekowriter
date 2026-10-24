//! The tool registry: the fixed per-role tool lists, phase permissions and the handlers of this
//! milestone (spec/05 §2, spec/03 §1.2).
//!
//! Each role has **one fixed, ordered tool list** frozen into the session at creation, so the
//! provider's cached prefix stays valid (D-007). Phases only restrict which tools may run: a call
//! to a tool that is not permitted in the current phase is answered with `error.tool_not_allowed`
//! and has no effect.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::db::json::{Granularity, Phase, SessionKind};

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

/// The writer role's fixed tool list, in order (spec/05 §2).
pub const WRITER_TOOLS: [&str; 20] = [
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
    "submit_outline",
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
pub const ARCHIVIST_TOOLS: [&str; 21] = [
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
    "propose_setting_change",
    "finish_maintenance",
];

/// The conception role's fixed tool list (spec/05 §2).
pub const CONCEPTION_TOOLS: [&str; 10] = [
    "ask_user",
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
pub const RESEARCHER_TOOLS: [&str; 7] = [
    "web_search",
    "web_fetch",
    "search_knowledge",
    "read_knowledge_entry",
    "save_knowledge_entry",
    "update_knowledge_entry",
    "finish_research",
];

/// The test-only tools of the debug session (spec/10 §2.7, PLAN M3.5).
pub const DEBUG_TOOLS: [&str; 4] = ["echo", "test_write", "test_finish", "test_slow"];

/// The frozen tool list of a role, in its fixed order (spec/05 §2).
pub fn tools_for_role(role: &str) -> &'static [&'static str] {
    match role {
        "writer" => &WRITER_TOOLS,
        "reviewer" => &REVIEWER_TOOLS,
        "archivist" => &ARCHIVIST_TOOLS,
        "conception" => &CONCEPTION_TOOLS,
        "researcher" => &RESEARCHER_TOOLS,
        _ => &[],
    }
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
            Phase::Planning => {
                RETRIEVAL_TOOLS.contains(&tool) || matches!(tool, "read_draft" | "submit_outline")
            }
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
            Phase::None => false,
        },
        SessionKind::CanonSeed => !matches!(
            tool,
            "record_appearances" | "set_chapter_summary" | "read_chapter"
        ),
        // Phase-free kinds permit every tool they froze (the role list already encoded them at
        // creation).
        SessionKind::ChapterMaintenance
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

/// The role that runs a session kind (spec/03 §1.1).
pub fn role_of_kind(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Conception => "conception",
        SessionKind::Research => "researcher",
        SessionKind::ChapterWrite | SessionKind::ChapterRevise => "writer",
        SessionKind::ChapterReview => "reviewer",
        SessionKind::ChapterMaintenance | SessionKind::CanonSeed | SessionKind::ChapterResync => {
            "archivist"
        }
    }
}

/// The initial phase of a session kind (spec/03 §1.1, §4 step 3).
pub fn initial_phase(kind: SessionKind) -> Phase {
    match kind {
        SessionKind::ChapterWrite => Phase::Planning,
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

/// True when a granularity value is valid for a writer session (spec/03 §8).
pub fn is_writer_granularity(granularity: Granularity) -> bool {
    matches!(
        granularity,
        Granularity::Sentence | Granularity::Paragraph | Granularity::Free | Granularity::Chapter
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every tool name any role may hold (so the frozen-list filter never hides a name).
    fn all_frozen() -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for role in [
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
    fn role_lists_match_spec_05_section_2() {
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
    }

    /// The phase matrix of spec/05 §2, tool by tool.
    #[test]
    fn writer_phase_permission_matrix() {
        // planning: RETRIEVAL, read_draft, submit_outline
        for tool in RETRIEVAL_TOOLS {
            assert!(is_permitted(
                SessionKind::ChapterWrite,
                Phase::Planning,
                tool,
                &writer_frozen()
            ));
            assert!(!is_permitted(
                SessionKind::ChapterWrite,
                Phase::Drafting,
                tool,
                &all_frozen()
            ));
        }
        assert!(is_permitted(
            SessionKind::ChapterWrite,
            Phase::Planning,
            "read_draft",
            &all_frozen()
        ));
        assert!(is_permitted(
            SessionKind::ChapterWrite,
            Phase::Planning,
            "submit_outline",
            &all_frozen()
        ));
        assert!(!is_permitted(
            SessionKind::ChapterWrite,
            Phase::Planning,
            "insert_text",
            &all_frozen()
        ));
        assert!(!is_permitted(
            SessionKind::ChapterWrite,
            Phase::Planning,
            "finish_draft",
            &all_frozen()
        ));

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
        let planning = allowed_tools(SessionKind::ChapterWrite, Phase::Planning, &writer_frozen());
        assert_eq!(planning.len(), 12, "{planning:?}");
        assert!(planning.contains(&"submit_outline".to_owned()));
        let drafting = allowed_tools(SessionKind::ChapterWrite, Phase::Drafting, &writer_frozen());
        assert_eq!(drafting.len(), 7, "{drafting:?}");
        assert!(!drafting.contains(&"get_entity".to_owned()));
    }

    #[test]
    fn initial_phases_follow_spec_03_section_1_1() {
        assert_eq!(initial_phase(SessionKind::ChapterWrite), Phase::Planning);
        assert_eq!(
            initial_phase(SessionKind::ChapterRevise),
            Phase::RevisionPlanning
        );
        assert_eq!(initial_phase(SessionKind::ChapterReview), Phase::None);
    }
}
