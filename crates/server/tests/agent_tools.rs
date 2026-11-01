//! Tool framework tests (PLAN M3.2, spec/05 §1–§2, §9).
//!
//! These cover the parts of the framework that exist before the concrete tools: the shipped
//! definitions in `prompts/tools.toml`, JSON Schema validation, the role lists, the phase
//! permission matrix and the entity resolver. The per-tool happy paths and error tables arrive
//! with the milestones that own those tools.

use std::collections::BTreeSet;

use nekowriter::agent::tools::registry::{
    ARCHIVIST_TOOLS, CONCEPTION_TOOLS, PLANNER_TOOLS, RESEARCHER_TOOLS, RETRIEVAL_TOOLS,
    REVIEWER_TOOLS, WRITE_TOOLS, WRITER_TOOLS, allowed_tools, initial_phase, is_permitted,
    role_of_kind, tools_for_role,
};
use nekowriter::agent::tools::spec::{ToolSpecs, validate_input};
use nekowriter::agent::tools::{ToolErrorKind, ToolRegistry, resolve::EntityResolver};
use nekowriter::db::json::{Phase, SessionKind};
use nekowriter::prompts::PromptRegistry;

// ───────────────────────────── shipped definitions ─────────────────────────────

/// spec/05 §1.3: every tool's `examples` validates against both the JSON Schema and the Rust
/// input struct, the compile-time list equals the TOML list, and no definition is orphaned.
#[test]
fn every_tool_example_matches_its_schema_and_the_registry() {
    let pack = PromptRegistry::embedded().expect("pack").pack().clone();
    let specs = ToolSpecs::from_pack(&pack);

    // (1) No orphan definitions: every tool in the TOML is reachable from some role list, and
    //     every name a role lists exists in the TOML.
    let mut role_tools: BTreeSet<&str> = BTreeSet::new();
    for role in [
        "planner",
        "writer",
        "reviewer",
        "archivist",
        "conception",
        "researcher",
    ] {
        for name in tools_for_role(role) {
            role_tools.insert(name);
        }
    }
    let pack_tools: BTreeSet<&str> = pack.tools().map(|tool| tool.name.as_str()).collect();
    for name in &pack_tools {
        assert!(
            role_tools.contains(name),
            "tool {name} is defined in tools.toml but no role can call it"
        );
    }
    for name in &role_tools {
        assert!(
            pack_tools.contains(name),
            "role lists the tool {name}, which tools.toml does not define"
        );
    }
    assert_eq!(specs.len(), pack_tools.len());

    // (2) The registry knows a handler for every tool a role can call (the real handlers arrive
    //     with their milestones; M3 ships the framework and the test-only tools).
    let registry = ToolRegistry::new();
    assert!(!registry.is_empty(), "the test-only tools are registered");

    // (3) Every example validates against the compiled JSON Schema.
    for tool in pack.tools() {
        assert!(
            !tool.examples.is_empty(),
            "tool {} has no example (spec/05 §1.3)",
            tool.name
        );
        for (index, example) in tool.examples.iter().enumerate() {
            let raw = serde_json::to_string(example).expect("example");
            validate_input(&specs, &tool.name, &raw).unwrap_or_else(|error| {
                panic!(
                    "tool {} example {index} failed schema validation: {error:?}\n{raw}",
                    tool.name
                )
            });
        }
    }
}

/// Every tool definition sets `additionalProperties = false` (the tools.toml header contract).
#[test]
fn every_input_schema_is_closed() {
    let pack = PromptRegistry::embedded().expect("pack").pack().clone();
    for tool in pack.tools() {
        assert_eq!(
            tool.input_schema.get("type"),
            Some(&serde_json::json!("object")),
            "tool {} must take an object",
            tool.name
        );
        assert_eq!(
            tool.input_schema.get("additionalProperties"),
            Some(&serde_json::json!(false)),
            "tool {} must set additionalProperties = false",
            tool.name
        );
    }
}

/// An unknown key is rejected by the schema, so a typo never reaches a handler
/// (spec/05 §1.4 step 2).
#[test]
fn schema_validation_rejects_unknown_keys_and_wrong_types() {
    let pack = PromptRegistry::embedded().expect("pack").pack().clone();
    let specs = ToolSpecs::from_pack(&pack);
    let error = validate_input(&specs, "get_entity", r#"{"entity":"x","nope":1}"#)
        .expect_err("unknown key");
    assert_eq!(error.kind, ToolErrorKind::Validation);

    let error = validate_input(&specs, "get_entity", r#"{"entity":42}"#).expect_err("type");
    assert_eq!(error.kind, ToolErrorKind::Validation);

    // The required key is enforced too.
    let error = validate_input(&specs, "get_entity", "{}").expect_err("required");
    assert_eq!(error.kind, ToolErrorKind::Validation);
    let errors = error
        .fields
        .get("errors")
        .and_then(|value| value.as_array())
        .expect("errors");
    assert!(!errors.is_empty(), "a path and message are reported");

    // At most three errors are reported (spec/05 §1.4).
    let error = validate_input(&specs, "get_entity", r#"{"a":1,"b":2,"c":3,"d":4}"#)
        .expect_err("validation");
    let errors = error
        .fields
        .get("errors")
        .and_then(|value| value.as_array())
        .expect("errors");
    assert!(errors.len() <= 3, "{errors:?}");
}

/// `invalid_json` carries the first 500 characters of the raw arguments (spec/05 §1.4 step 1).
#[test]
fn invalid_json_reports_the_raw_prefix() {
    let specs = ToolSpecs::from_pack(PromptRegistry::embedded().expect("pack").pack());
    let long = format!("{{\"text\": \"{}\"", "x".repeat(800));
    let error = validate_input(&specs, "insert_text", &long).expect_err("invalid");
    assert_eq!(error.kind, ToolErrorKind::InvalidJson);
    let raw = error
        .fields
        .get("raw")
        .and_then(|value| value.as_str())
        .expect("raw");
    assert_eq!(raw.chars().count(), 500);
}

// ───────────────────────────── role lists ─────────────────────────────

/// The role lists are exactly the spec/05 §2 tables, in order.
#[test]
fn the_role_lists_are_the_spec_tables_in_order() {
    // Writer: 10 retrieval tools, then the writer-specific ones; `submit_outline` belongs to
    // the planner (D-146).
    assert_eq!(WRITER_TOOLS.len(), 19);
    assert_eq!(&WRITER_TOOLS[..10], &RETRIEVAL_TOOLS);
    assert_eq!(
        &WRITER_TOOLS[10..],
        &[
            "submit_revision_plan",
            "read_draft",
            "insert_text",
            "insert_paragraphs",
            "replace_text",
            "rewrite_paragraph",
            "delete_paragraphs",
            "finish_draft",
            "finish_revision",
        ]
    );
    assert_eq!(PLANNER_TOOLS.len(), 12);
    assert_eq!(&PLANNER_TOOLS[10..], &["read_draft", "submit_outline"]);
    assert_eq!(REVIEWER_TOOLS.len(), 11);
    assert_eq!(REVIEWER_TOOLS[10], "submit_review");
    assert_eq!(ARCHIVIST_TOOLS.len(), 22);
    assert_eq!(ARCHIVIST_TOOLS[7], "create_entity");
    assert_eq!(ARCHIVIST_TOOLS[19], "propose_goal_change");
    assert_eq!(ARCHIVIST_TOOLS[21], "finish_maintenance");
    assert_eq!(CONCEPTION_TOOLS.len(), 10);
    assert_eq!(CONCEPTION_TOOLS[0], "ask_user");
    assert_eq!(RESEARCHER_TOOLS.len(), 8);
    assert_eq!(RESEARCHER_TOOLS[4], "set_target_scope");
    assert_eq!(RESEARCHER_TOOLS[7], "finish_research");
    // DRAFT_EDIT is exactly the five text-editing tools.
    assert_eq!(
        WRITE_TOOLS,
        [
            "insert_text",
            "insert_paragraphs",
            "replace_text",
            "rewrite_paragraph",
            "delete_paragraphs"
        ]
    );
}

/// Every session kind maps to the role of spec/03 §1.1.
#[test]
fn session_kinds_map_to_their_roles() {
    for (kind, role) in [
        (SessionKind::Conception, "conception"),
        (SessionKind::Research, "researcher"),
        (SessionKind::ChapterOutline, "planner"),
        (SessionKind::ChapterWrite, "writer"),
        (SessionKind::ChapterRevise, "writer"),
        (SessionKind::ChapterReview, "reviewer"),
        (SessionKind::ChapterMaintenance, "archivist"),
        (SessionKind::CanonSeed, "archivist"),
        (SessionKind::ChapterResync, "archivist"),
    ] {
        assert_eq!(role_of_kind(kind), role, "{kind:?}");
    }
    assert_eq!(initial_phase(SessionKind::ChapterWrite), Phase::Drafting);
    assert_eq!(
        initial_phase(SessionKind::ChapterRevise),
        Phase::RevisionPlanning
    );
    assert_eq!(initial_phase(SessionKind::ChapterReview), Phase::None);
}

// ───────────────────────────── permission matrix ─────────────────────────────

fn frozen(role: &str) -> Vec<String> {
    tools_for_role(role)
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
}

/// The complete phase × tool matrix for the writer role (spec/05 §2).
#[test]
fn the_writer_permission_matrix_is_complete() {
    let writer = frozen("writer");
    // The planning row is legacy-only: a session started before the split froze the old 20-tool
    // list, `submit_outline` included (D-146).
    let legacy: Vec<String> = nekowriter::agent::tools::registry::LEGACY_WRITER_TOOLS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let planning = [
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
        "read_draft",
    ];
    let drafting = [
        "read_draft",
        "insert_text",
        "insert_paragraphs",
        "replace_text",
        "rewrite_paragraph",
        "delete_paragraphs",
        "finish_draft",
    ];
    let revision_planning = [
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
    ];
    let revising = [
        "read_draft",
        "insert_text",
        "insert_paragraphs",
        "replace_text",
        "rewrite_paragraph",
        "delete_paragraphs",
        "finish_revision",
    ];
    for (phase, allowed, frozen_tools) in [
        (Phase::Planning, planning.as_slice(), legacy.as_slice()),
        (Phase::Drafting, drafting.as_slice(), writer.as_slice()),
        (
            Phase::RevisionPlanning,
            revision_planning.as_slice(),
            writer.as_slice(),
        ),
        (Phase::Revising, revising.as_slice(), writer.as_slice()),
    ] {
        for tool in frozen_tools {
            let expected = allowed.contains(&tool.as_str());
            assert_eq!(
                is_permitted(SessionKind::ChapterWrite, phase, tool, frozen_tools),
                expected,
                "{tool} in {phase:?}"
            );
        }
        assert_eq!(
            allowed_tools(SessionKind::ChapterWrite, phase, frozen_tools),
            allowed
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>(),
            "allowed_tools of {phase:?} lists the permitted tools in frozen order"
        );
    }
}

/// Every phase-free role permits its whole frozen list, and `canon_seed` forbids exactly the
/// three chapter-scoped archivist tools (spec/05 §2).
#[test]
fn the_phase_free_roles_permit_their_whole_list() {
    for (kind, role) in [
        (SessionKind::ChapterReview, "reviewer"),
        (SessionKind::Conception, "conception"),
        (SessionKind::Research, "researcher"),
        (SessionKind::ChapterMaintenance, "archivist"),
        (SessionKind::ChapterResync, "archivist"),
    ] {
        let tools = frozen(role);
        for tool in &tools {
            assert!(
                is_permitted(kind, Phase::None, tool, &tools),
                "{tool} in {kind:?}"
            );
        }
    }

    let archivist = frozen("archivist");
    for tool in ["record_appearances", "set_chapter_summary", "read_chapter"] {
        assert!(
            !is_permitted(SessionKind::CanonSeed, Phase::None, tool, &archivist),
            "{tool} is forbidden in canon_seed"
        );
    }
    for tool in &archivist {
        if ["record_appearances", "set_chapter_summary", "read_chapter"].contains(&tool.as_str()) {
            continue;
        }
        assert!(is_permitted(
            SessionKind::CanonSeed,
            Phase::None,
            tool,
            &archivist
        ));
    }
}

/// A writer phase never permits a tool of another role, nor a tool the session did not freeze.
#[test]
fn foreign_and_unfrozen_tools_are_never_permitted() {
    let writer = frozen("writer");
    let reviewer = frozen("reviewer");
    // `submit_review` is reviewer-only and `insert_text` is writer-only, so each is rejected by
    // the other role's frozen list (the shared retrieval tools legitimately appear in both).
    assert!(!is_permitted(
        SessionKind::ChapterReview,
        Phase::None,
        "insert_text",
        &reviewer
    ));
    assert!(!is_permitted(
        SessionKind::ChapterWrite,
        Phase::Drafting,
        "submit_review",
        &writer
    ));
    // The shared retrieval tools appear in both lists and stay permitted.
    for tool in ["get_entity", "read_chapter"] {
        assert!(is_permitted(
            SessionKind::ChapterReview,
            Phase::None,
            tool,
            &reviewer
        ));
    }
    // A tool outside the frozen list is rejected even when its name exists elsewhere.
    assert!(!is_permitted(
        SessionKind::ChapterReview,
        Phase::None,
        "submit_review",
        &writer
    ));
}

// ───────────────────────────── entity resolution ─────────────────────────────

/// spec/05 §1.7: the documented resolution order, ambiguity and suggestions.
#[test]
fn the_entity_resolver_follows_the_documented_order() {
    use nekowriter::agent::tools::resolve::VisibleEntity;

    let entity = |id: i32, name: &str, aliases: &[&str]| VisibleEntity {
        id,
        name: name.to_owned(),
        aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
        kind_label: "character".to_owned(),
    };
    let resolver = EntityResolver::new(vec![
        entity(12, "Lin Fan", &["A-Fan"]),
        entity(15, "Zhang San", &[]),
        entity(21, "Su Wan", &["Wan'er"]),
        entity(30, "Lin", &[]),
    ]);

    // 1. id, with and without the hash.
    assert_eq!(resolver.resolve("#12").expect("id").id, 12);
    assert_eq!(resolver.resolve("12").expect("id").id, 12);
    // 2. exact name beats the shorter exact name that is also a substring.
    assert_eq!(resolver.resolve("Lin").expect("name").id, 30);
    // 3. exact alias.
    assert_eq!(resolver.resolve("Wan'er").expect("alias").id, 21);
    // 4. case-insensitive name / alias.
    assert_eq!(resolver.resolve("lin fan").expect("case").id, 12);
    assert_eq!(resolver.resolve("a-fan").expect("case alias").id, 12);
    // 5. unique substring.
    assert_eq!(resolver.resolve("Zhang").expect("substring").id, 15);

    // Several substring matches are ambiguous, with the candidates listed.
    let error = resolver.resolve("an").expect_err("ambiguous");
    assert_eq!(error.kind, ToolErrorKind::EntityAmbiguous);
    let candidates = error
        .fields
        .get("candidates")
        .and_then(|value| value.as_array())
        .expect("candidates");
    assert!(candidates.len() >= 2);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate["id"].is_number())
    );

    // No match suggests near misses, capped at five.
    let error = resolver.resolve("Lin Fann").expect_err("not found");
    assert_eq!(error.kind, ToolErrorKind::EntityNotFound);
    let suggestions = error
        .fields
        .get("suggestions")
        .and_then(|value| value.as_array())
        .expect("suggestions");
    assert!(suggestions.len() <= 5);
    assert!(!suggestions.is_empty());
}

// ───────────────────────────── rendering ─────────────────────────────

/// An unknown granularity or enum renders its raw value rather than failing (spec/09 §1).
#[test]
fn the_label_helper_falls_back_to_the_raw_value() {
    let registry = PromptRegistry::embedded().expect("pack");
    let set = registry.defaults_only();
    assert!(!set.label("granularity", "paragraph").is_empty());
    // An unknown value renders as itself, so a new enum variant never breaks rendering.
    assert_eq!(set.label("granularity", "no_such_mode"), "no_such_mode");
    assert_eq!(set.label("no_such_enum", "x"), "x");
}

/// The chapter label helper renders through `shared.chapter_label` (spec/09 §1).
#[test]
fn the_chapter_label_helper_renders_main_and_side_tracks() {
    let registry = PromptRegistry::embedded().expect("pack");
    let set = registry.defaults_only();
    let main = set.chapter_label("main", "", 12, "Night Rain", true);
    assert!(main.contains("12"), "the number appears: {main}");
    assert!(main.contains("Night Rain"), "the title appears: {main}");
    let side = set.chapter_label("side", "The Sword Tomb", 2, "", false);
    assert!(
        side.contains("The Sword Tomb"),
        "a side track names its track: {side}"
    );
}
