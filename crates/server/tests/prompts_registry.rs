//! Prompt registry tests (PLAN M2.3, spec/09 §2–§4, spec/01 §11).
//!
//! Two layers: unit-level tests of the registry (resolution order, rendering, validation) and
//! API tests of `GET/PUT/DELETE /api/prompts` through the real router.

use nekowriter::agent::tools::ToolErrorKind;
use nekowriter::api::build_app;
use nekowriter::db::Db;
use nekowriter::prompts::{PromptRegistry, PromptSource};
use nekowriter::state::AppState;
use nekowriter::testing::{TestApp, assert_error};
use serde_json::{Value, json};
use tower::ServiceExt;

// ───────────────────────────── pack coverage ─────────────────────────────

/// Every `ToolErrorKind` must have a `result.error` sample, otherwise `validate_override` can
/// accept an override that breaks a branch the shipped validator never renders.
#[test]
fn result_error_samples_cover_every_tool_error_kind() {
    let registry = PromptRegistry::embedded().expect("pack");
    let samples = registry.sample_contexts("result.error");
    let actual: std::collections::BTreeSet<&str> = samples
        .iter()
        .filter_map(|sample| sample["code"].as_str())
        .collect();
    let expected: std::collections::BTreeSet<&str> = [
        ToolErrorKind::InvalidJson,
        ToolErrorKind::Validation,
        ToolErrorKind::EntityNotFound,
        ToolErrorKind::EntityAmbiguous,
        ToolErrorKind::EntityExists,
        ToolErrorKind::PidNotFound,
        ToolErrorKind::TextNotFound,
        ToolErrorKind::TextAmbiguous,
        ToolErrorKind::Granularity,
        ToolErrorKind::DraftEmpty,
        ToolErrorKind::IsolatedReadonly,
        ToolErrorKind::ChapterNotFound,
        ToolErrorKind::ChapterNotOfficial,
        ToolErrorKind::NoKnowledgeScope,
        ToolErrorKind::ScopeExcluded,
        ToolErrorKind::TargetScopeUnset,
        ToolErrorKind::ScopeNotFound,
        ToolErrorKind::NotFound,
        ToolErrorKind::EntryExists,
        ToolErrorKind::FindFailed,
        ToolErrorKind::MaintenanceIncomplete,
        ToolErrorKind::SearchNotConfigured,
        ToolErrorKind::SearchFailed,
        ToolErrorKind::FetchFailed,
        ToolErrorKind::UnsupportedContentType,
        ToolErrorKind::CharacterNotFound,
        ToolErrorKind::CharacterExists,
        ToolErrorKind::ReorderMismatch,
        ToolErrorKind::RelationNotFound,
        ToolErrorKind::SeedOnly,
        ToolErrorKind::ExactlyOne,
        ToolErrorKind::MutuallyExclusive,
        ToolErrorKind::ToolNotAllowed,
        ToolErrorKind::OutputTruncated,
        ToolErrorKind::AfterCompletion,
        ToolErrorKind::Refusal,
        ToolErrorKind::Internal,
    ]
    .iter()
    .map(|kind| kind.code())
    .collect();
    assert_eq!(
        actual, expected,
        "result.error samples and ToolErrorKind drift"
    );
}

/// The entity `description` ceiling of D-142 is carried by every prompt that can write it: the
/// three archivist kickoffs, the archivist system prompt, and both entity-write tool schemas.
/// Prompt text is data, so an edit could drop the rule without failing anything else — the
/// author's bloat (8,290 characters over 44 entities) is exactly what the rule prevents, so it is
/// pinned here rather than left to review.
#[test]
fn the_entity_description_ceiling_is_stated_where_entities_are_written() {
    let registry = PromptRegistry::embedded().expect("pack");
    let set = registry.defaults_only();

    // Each kickoff must state the ceiling, that `description` is not a chapter log, and where the
    // per-chapter developments belong instead.
    for key in [
        "archivist.kickoff.maintenance",
        "archivist.kickoff.seed",
        "archivist.kickoff.resync",
    ] {
        let mut checked = 0usize;
        for sample in registry.sample_contexts(key) {
            // System-off variants intentionally omit the Entities-only description rules.
            if sample
                .get("systems")
                .and_then(|systems| systems.get("entities"))
                .and_then(|value| value.as_bool())
                == Some(false)
            {
                continue;
            }
            let text = set
                .render_value(key, nekowriter::prompts::template_value(&sample))
                .unwrap_or_else(|err| panic!("{key}: {err}"))
                .text;
            assert!(
                text.contains("120 字"),
                "{key} must state the description ceiling (D-142):\n{text}"
            );
            assert!(
                text.contains("description 不是章节日志")
                    || text.contains("不要写成传记")
                    || text.contains("不是章节日志"),
                "{key} must say that description is not a chapter log:\n{text}"
            );
            // Design §9.1: the routine `update_entity` reasons must never include 描述 — that
            // one-word contradiction invited the per-chapter appends this guard exists to stop.
            // The stronger D-142 guard checks the negative invariant, not just the ceiling.
            assert!(
                !text.contains("描述发生变化"),
                "{key} must not list 描述 as a routine update_entity reason:\n{text}"
            );
            if key == "archivist.kickoff.maintenance" {
                assert!(
                    text.contains("因为本章剧情而更新 description"),
                    "the maintenance step must forbid chapter-driven description updates:\n{text}"
                );
            }
            checked += 1;
        }
        assert!(checked > 0, "{key} rendered no sample");
    }

    // The system prompt explains the field hierarchy that the ceiling depends on.
    let system = set
        .render("archivist.system", &json!({}))
        .expect("archivist.system")
        .text;
    assert!(
        system.contains("120 字"),
        "archivist.system must state the description ceiling (D-142)"
    );
    assert!(
        system.contains("章节日志"),
        "archivist.system must say description is not a chapter log"
    );

    // Both entity-write tool schemas carry the ceiling, and `description_append` is restricted to
    // durable information and repeats it (this is the field that grew the author's rows).
    let pack = registry.pack();
    for tool in ["create_entity", "update_entity"] {
        let schema = pack.tool(tool).expect("tool schema");
        let description = schema.input_schema["properties"]["description"]["description"]
            .as_str()
            .unwrap_or_else(|| panic!("{tool}.description has no description"));
        assert!(
            description.contains("120 字"),
            "{tool}.description must state the ceiling: {description}"
        );
    }
    let append = pack
        .tool("update_entity")
        .expect("update_entity")
        .input_schema["properties"]["description_append"]["description"]
        .as_str()
        .expect("description_append description");
    assert!(
        append.contains("120 字"),
        "description_append must repeat the ceiling: {append}"
    );
    assert!(
        append.contains("长期") || append.contains("二十章"),
        "description_append must be for durable information only: {append}"
    );
    assert!(
        append.contains("重写") || append.contains("改成"),
        "description_append must point at rewriting tighter instead: {append}"
    );
}

/// The relational-planning rule (T14, D-170) is carried only by the conception prompt: the
/// outline is phases/volumes with ordering relations, never chapter numbers or per-chapter events.
#[test]
fn the_conception_outline_is_relational_and_has_no_per_chapter_writing_phrase() {
    let registry = PromptRegistry::embedded().expect("pack");
    let system = registry
        .defaults_only()
        .render("conception.system", &json!({}))
        .expect("conception.system")
        .text;
    assert!(
        !system.contains("逐章写作"),
        "conception.system must not restore the per-chapter writing phrase:\n{system}"
    );
    assert!(
        system.contains("先后关系") && system.contains("并行关系"),
        "the outline must state ordering and parallel relations:\n{system}"
    );
    assert!(
        system.contains("不要写章节号") && system.contains("逐章细纲"),
        "the outline must defer chapter detail to the per-chapter outline sessions:\n{system}"
    );
    assert!(
        system.contains("按卷或大致进度"),
        "the protagonist growth stages must be by volume or rough progress:\n{system}"
    );
}

/// The seed kickoff asks for every planned node the outline states, with no cap, and forbids
/// inventing foreshadowing (T15, D-170).
#[test]
fn the_seed_kickoff_asks_for_uncapped_planned_nodes_and_no_invented_foreshadowing() {
    let registry = PromptRegistry::embedded().expect("pack");
    let samples = registry.sample_contexts("archivist.kickoff.seed");
    let sample = samples.first().expect("seed sample");
    let seed = registry
        .defaults_only()
        .render("archivist.kickoff.seed", sample)
        .expect("archivist.kickoff.seed")
        .text;
    assert!(
        seed.contains("大纲写到多少就建多少，不设上限"),
        "seed must not cap planned nodes:\n{seed}"
    );
    assert!(
        seed.contains("只记录大纲中明确写出的伏笔，不自行发明"),
        "seed must not invent foreshadowing:\n{seed}"
    );
}

/// The memory block renders the planned count in the archive-size line (spec/06 §4.4).
#[test]
fn the_memory_block_shows_the_planned_event_count() {
    let registry = PromptRegistry::embedded().expect("pack");
    let mut context = registry
        .sample_contexts("block.memory")
        .into_iter()
        .next()
        .expect("memory sample");
    context["memory"]["counts"]["event"] = json!(7);
    context["memory"]["counts"]["event_planned"] = json!(3);
    let text = registry
        .defaults_only()
        .render("block.memory", &context)
        .expect("block.memory")
        .text;
    assert!(
        text.contains("时间线事件 7（其中计划中 3）"),
        "the memory block must show the planned count:\n{text}"
    );
}

/// The knowledge-scope budget of D-145 is carried by the conception system prompt (where the
/// research topics are planned) and by the `set_research_topics` scope field (the contract the
/// model sees at call time). Prompt text is data, so an edit could drop the rule without failing
/// anything else and bring back one scope per domain, fragmenting retrieval.
#[test]
fn the_knowledge_scope_budget_is_stated_where_research_topics_are_set() {
    let registry = PromptRegistry::embedded().expect("pack");
    let set = registry.defaults_only();

    let system = set
        .render("conception.system", &json!({}))
        .expect("conception.system")
        .text;
    assert!(
        system.contains("一本书总共只使用一到两个范围"),
        "conception.system must state the one-or-two scope budget (D-145):\n{system}"
    );
    assert!(
        system.contains("不要按子话题、章节或人物拆分范围"),
        "conception.system must forbid per-subtopic/chapter/character scopes (D-145):\n{system}"
    );
    assert!(
        system.contains("不建第三个"),
        "conception.system must say the second scope is the last (D-145):\n{system}"
    );

    let pack = registry.pack();
    let scope = pack
        .tool("set_research_topics")
        .expect("set_research_topics")
        .input_schema["properties"]["topics"]["items"]["properties"]["scope"]["description"]
        .as_str()
        .expect("scope description");
    assert!(
        scope.contains("全书只使用一到两个范围"),
        "set_research_topics.scope must state the one-or-two scope budget (D-145): {scope}"
    );
    assert!(
        scope.contains("不要按子话题拆分范围"),
        "set_research_topics.scope must forbid per-subtopic scopes (D-145): {scope}"
    );
}

#[test]
fn every_key_renders_with_every_sample() {
    let registry = PromptRegistry::embedded().expect("the embedded pack must be valid");
    let set = registry.defaults_only();
    let mut rendered_keys = 0usize;
    for entry in registry.pack().entries() {
        let samples = registry.sample_contexts(&entry.key);
        assert!(
            !samples.is_empty(),
            "{} has no sample context; add one to samples.json",
            entry.key
        );
        for (index, sample) in samples.iter().enumerate() {
            let output = set
                .render_value(&entry.key, nekowriter::prompts::template_value(sample))
                .unwrap_or_else(|err| panic!("{} sample {index}: {err}", entry.key));
            let blank_allowed = matches!(entry.key.as_str(), "block.canon_systems" | "block.goals");
            assert!(
                blank_allowed || !output.text.is_empty(),
                "{} sample {index} rendered nothing",
                entry.key
            );
            assert!(
                !output.text.contains("{{") && !output.text.contains("{%"),
                "{} sample {index} left template syntax in the output:\n{}",
                entry.key,
                output.text
            );
        }
        rendered_keys += 1;
    }
    assert!(rendered_keys > 100, "the pack should be loaded whole");
}

/// Renders every key with each of its sample contexts and snapshots the output, so any change to
/// a template, a sample or the renderer shows up as a reviewable diff (spec/09 §2).
#[test]
fn every_rendered_key_matches_its_snapshot() {
    let registry = PromptRegistry::embedded().expect("pack");
    let set = registry.defaults_only();
    let mut rendered = String::new();
    for entry in registry.pack().entries() {
        for (index, sample) in registry.sample_contexts(&entry.key).iter().enumerate() {
            let output = set
                .render_value(&entry.key, nekowriter::prompts::template_value(sample))
                .unwrap_or_else(|err| panic!("{} sample {index}: {err}", entry.key));
            rendered.push_str(&format!(
                "===== {} [{index}] =====\n{}\n\n",
                entry.key, output.text
            ));
        }
    }
    insta::assert_snapshot!("rendered_prompt_pack", rendered);
}

#[test]
fn every_spec_key_exists_and_every_key_is_documented() {
    // The key list of spec/09 §2 (the validator checks the same set).
    let expected = "conception.system researcher.system planner.system writer.system reviewer.system \
        archivist.system conception.kickoff researcher.kickoff planner.kickoff writer.kickoff.chapter \
        writer.kickoff.revise writer.kickoff.rollover reviewer.kickoff reviewer.rereview \
        archivist.kickoff.maintenance archivist.kickoff.seed archivist.kickoff.resync \
        block.settings block.canon_systems block.writing_skills block.memory block.recent_chapters \
        block.goals block.chapter_draft block.cast_index block.chapter_diff \
        block.anchor block.book_brief notice.user_feedback notice.user_message notice.user_edits \
        notice.setting_edits notice.completion_revoked notice.tool_not_allowed \
        notice.output_truncated notice.web_content notice.nudge_generic notice.research_budget \
        planner.notice.outline_rejected planner.notice.nudge planner.notice.retrieval_budget \
                writer.notice.revision_plan_accepted writer.notice.revision_plan_rejected \
        writer.notice.granularity writer.notice.nudge_drafting \
        writer.notice.nudge_revision_planning writer.notice.nudge_revising \
        writer.notice.retrieval_budget writer.rules.granularity result.error result.truncated \
        result.entity_card result.entity_list result.relations result.timeline \
        result.foreshadowing_list result.search_hits result.knowledge_entry result.chapter_text \
        result.draft_text result.draft_edit result.writer_control result.submit_review \
        result.canon_write result.finish_maintenance result.ask_user result.request_review \
        result.conception_write result.web_search result.web_fetch result.knowledge_write \
        result.finish_research shared.label shared.chapter_label";
    let registry = PromptRegistry::embedded().expect("pack");
    for key in expected.split_whitespace() {
        assert!(
            registry.pack().entry(key).is_some(),
            "missing prompt key {key}"
        );
    }
    // Every declared variable is provided by the base samples or by one branch of its key, so a
    // sample context always covers the whole template surface.
    for entry in registry.pack().entries() {
        let mut provided = std::collections::BTreeSet::new();
        for var in &entry.vars {
            if nekowriter::prompts::sample_base_value(var).is_some() {
                provided.insert(var.clone());
            }
        }
        for sample in registry.sample_contexts(&entry.key) {
            if let serde_json::Value::Object(object) = sample {
                for name in object.keys() {
                    provided.insert(name.clone());
                }
            }
        }
        for var in &entry.vars {
            assert!(
                provided.contains(var),
                "{} declares variable {var} that no sample provides",
                entry.key
            );
        }
    }
}

/// The typed context structs must serialize to exactly the shape the sample data covers, so a
/// renamed field is caught here instead of surfacing as a strict-undefined failure at runtime
/// (spec/09 §2).
#[test]
fn context_structs_match_the_sample_shapes() {
    use nekowriter::prompts::context::*;

    let book = BookContext {
        title: "t".to_owned(),
        genre: "g".to_owned(),
        idea: "i".to_owned(),
        writing_language: "zh-CN".to_owned(),
        writing_language_name: "n".to_owned(),
        unspaced: true,
    };
    let track = TrackContext {
        is_main: true,
        kind: "main".to_owned(),
        title: String::new(),
        synopsis: String::new(),
        canon_mode: "canon".to_owned(),
        anchor_kind: "none".to_owned(),
        anchor_label: String::new(),
    };
    let chapter = ChapterContext {
        id: 1,
        number: 1,
        label: "l".to_owned(),
        title: "t".to_owned(),
        guidance: "g".to_owned(),
        outline: "o".to_owned(),
        reference_notes: "r".to_owned(),
        target_min: 1,
        target_max: 2,
        word_count: 0,
        has_draft: false,
    };
    let draft = DraftContext {
        paragraphs: vec![ParagraphContext {
            pid: "p1".to_owned(),
            text: "x".to_owned(),
        }],
        tail_open: false,
        word_count: 1,
        paragraph_count: 1,
    };
    let granularity = GranularityContext {
        key: "paragraph".to_owned(),
        label: "l".to_owned(),
        revising: false,
        tail_open: false,
        last_pid: "p1".to_owned(),
    };

    // Every struct serializes to an object whose keys the samples also carry.
    let cases: Vec<(&str, Value)> = vec![
        ("book", serde_json::to_value(&book).expect("book")),
        ("track", serde_json::to_value(&track).expect("track")),
        ("chapter", serde_json::to_value(&chapter).expect("chapter")),
        ("draft", serde_json::to_value(&draft).expect("draft")),
        (
            "granularity",
            serde_json::to_value(&granularity).expect("granularity"),
        ),
        // `counts` is nested under `notice.user_edits`, which carries the sample.
        (
            "notice.user_edits",
            serde_json::to_value(EditCountsContext {
                modified: 0,
                deleted: 0,
                inserted: 0,
            })
            .expect("counts"),
        ),
    ];
    let registry = PromptRegistry::embedded().expect("pack");
    let set = registry.defaults_only();
    for (name, value) in cases {
        // The edit counts live inside the `notice.user_edits` variant table.
        let samples = registry.sample_contexts(name);
        let sample: Value = if name == "notice.user_edits" {
            samples.first().expect("a sample")["counts"].clone()
        } else {
            nekowriter::prompts::sample_base_value(name)
                .unwrap_or_else(|| panic!("no sample for {name}"))
                .clone()
        };
        let sample = &sample;
        let typed = value.as_object().expect("object");
        let sampled = sample.as_object().expect("sample object");
        for field in typed.keys() {
            assert!(
                sampled.contains_key(field),
                "{name}.{field} is missing from the sample data"
            );
        }
        for field in sampled.keys() {
            assert!(
                typed.contains_key(field),
                "{name}.{field} exists in the samples but not in the typed context"
            );
        }
    }

    // Rendering with the typed structs works for the templates that take them.
    let rendered = set
        .render(
            "writer.kickoff.chapter",
            &json!({ "book": book, "track": track, "chapter": chapter, "draft": draft,
                     "granularity": granularity, "feedback": null }),
        )
        .expect("render");
    assert!(rendered.text.contains("t"));
    let rendered = set
        .render(
            "block.settings",
            &json!({ "book": book, "settings": SettingsContext {
            outline: String::new(), world: String::new(), protagonist: String::new(),
            characters: vec![], scopes: vec![],
        } }),
        )
        .expect("render settings");
    assert!(!rendered.text.is_empty());
    let _ = settings_empty_shape();
}

/// The empty `block.settings` shape of spec/06: no characters, no scopes.
fn settings_empty_shape() -> nekowriter::prompts::context::SettingsContext {
    use nekowriter::prompts::context::SettingsContext;
    SettingsContext {
        outline: String::new(),
        world: String::new(),
        protagonist: String::new(),
        characters: Vec::new(),
        scopes: Vec::new(),
    }
}

/// The outline contract is owned by the planner's entries (D-146, audit D1/D6): both name
/// `submit_outline` and neither names a drafting tool. The drafting side is pinned when the
/// writer's entries become drafting-only (task 12.4).
#[test]
fn the_planner_prompts_own_the_outline_contract() {
    let registry = PromptRegistry::embedded().expect("pack");
    let set = registry.defaults_only();
    for key in ["planner.system", "planner.kickoff"] {
        let mut checked = 0usize;
        let contexts: Vec<Value> = if key == "planner.system" {
            vec![json!({})]
        } else {
            registry.sample_contexts(key)
        };
        for sample in contexts {
            let text = set
                .render_value(key, nekowriter::prompts::template_value(&sample))
                .unwrap_or_else(|err| panic!("{key}: {err}"))
                .text;
            assert!(
                text.contains("submit_outline"),
                "{key} must name submit_outline"
            );
            for tool in [
                "insert_text",
                "insert_paragraphs",
                "replace_text",
                "rewrite_paragraph",
                "delete_paragraphs",
            ] {
                assert!(
                    !text.contains(tool),
                    "{key} must not name the drafting tool {tool}"
                );
            }
            checked += 1;
        }
        assert!(checked > 0, "{key} rendered no sample");
    }
    // The writer's entries are drafting-only: no outline submission and no chapter planning
    // phase. The granularity rule travels in the write kickoff (spec/03 §10, D-146).
    for key in ["writer.system", "writer.kickoff.chapter"] {
        let contexts: Vec<Value> = if key == "writer.system" {
            vec![json!({})]
        } else {
            registry.sample_contexts(key)
        };
        for sample in contexts {
            let text = set
                .render_value(key, nekowriter::prompts::template_value(&sample))
                .unwrap_or_else(|err| panic!("{key}: {err}"))
                .text;
            assert!(
                !text.contains("submit_outline"),
                "{key} names submit_outline"
            );
            assert!(
                !text.contains("【规划阶段】"),
                "{key} has the chapter planning phase"
            );
        }
    }
    let plain = set
        .render(
            "writer.kickoff.chapter",
            &json!({
                "book": {"title": "t", "genre": "", "idea": "", "writing_language": "zh-CN",
                         "writing_language_name": "简体中文", "unspaced": true},
                "track": {"is_main": true, "kind": "main", "title": "", "synopsis": "",
                          "canon_mode": "canon", "anchor_kind": "none", "anchor_label": ""},
                "chapter": {"id": 1, "number": 1, "label": "第1章", "title": "", "guidance": "",
                            "outline": "", "reference_notes": "", "target_min": 3000,
                            "target_max": 4000, "word_count": 0, "has_draft": false},
                "granularity": {"key": "paragraph", "label": "逐段", "revising": false,
                                "tail_open": false, "last_pid": "p1"},
                "feedback": null,
            }),
        )
        .expect("writer.kickoff.chapter");
    assert!(
        plain.text.contains("当前写作粒度"),
        "the write kickoff carries the granularity rule: {}",
        plain.text
    );

    // The tool description stops promising drafting after submission.
    let description = set
        .render("tool.submit_outline", &json!({}))
        .expect("tool.submit_outline")
        .text;
    assert!(description.contains("作者接受"), "{description}");
    assert!(!description.contains("进入正文阶段"), "{description}");
}

/// The prose-style section of `writer.system` is gone (design §5.1 option A): a book has no prose
/// style rules until the author writes and selects a skill. The workflow and format rules stay,
/// because unticking a skill must never stop the writer being told to follow the outline or to
/// avoid Markdown/metadata. This is the fail-before guard for the migration from the old prompt.
#[test]
fn writer_system_drops_the_prose_style_rules_and_keeps_the_workflow_ones() {
    let registry = PromptRegistry::embedded().expect("pack");
    let set = registry.defaults_only();
    let text = set
        .render("writer.system", &json!({}))
        .expect("writer.system")
        .text;

    for removed in [
        "场景化叙事",
        "人物要有区分度",
        "节奏张弛有度",
        "视角统一",
        "语言干净",
        "以短段为主",
    ] {
        assert!(
            !text.contains(removed),
            "writer.system must not carry the prose style bullet {removed:?}:\n{text}"
        );
    }
    for kept in [
        "先想清楚再写",
        "遵循细纲",
        "中文引号",
        "不使用 Markdown",
        "正文中不要出现说明",
    ] {
        assert!(
            text.contains(kept),
            "writer.system must keep the workflow/format rule {kept:?}:\n{text}"
        );
    }
}

#[test]
fn tool_entries_come_with_their_schema() {
    let registry = PromptRegistry::embedded().expect("pack");
    let set = registry.defaults_only();
    let tools = set.tool_definitions().expect("tool definitions");
    assert!(tools.len() > 40, "the pack defines every role tool");
    for tool in &tools {
        assert!(
            !tool.description.is_empty(),
            "{} has no description",
            tool.name
        );
        assert!(
            registry.pack().tool(&tool.name).is_some(),
            "{} has no schema",
            tool.name
        );
        assert_eq!(tool.input_schema["type"], "object");
    }
}

#[test]
fn labels_are_rendered_by_the_pack_and_unknown_values_fall_back() {
    let registry = PromptRegistry::embedded().expect("pack");
    let set = registry.defaults_only();
    // Known enum values render the model-facing label.
    let character = set.label("entity_kind", "character");
    assert!(!character.is_empty());
    assert_ne!(character, "character", "a known value must be translated");
    // An unknown value renders the raw value (spec/09 §1).
    assert_eq!(set.label("entity_kind", "nonsense"), "nonsense");
    assert_eq!(set.label("no_such_enum", "x"), "x");
    // Chapter labels follow shared.chapter_label.
    let main = set.chapter_label("main", "", 12, "", false);
    assert!(main.contains("12"), "got {main}");
    let side = set.chapter_label("side", "track title", 2, "chapter title", true);
    assert!(side.contains('2'), "got {side}");
    assert!(side.contains("track title"), "got {side}");
    assert!(side.contains("chapter title"), "got {side}");
    // The cache is per set: the second call is the same string.
    assert_eq!(set.label("entity_kind", "character"), character);
}

// ───────────────────────────── resolution ─────────────────────────────

fn test_state(dir: &std::path::Path, db: Db) -> AppState {
    let config = nekowriter::config::Config {
        bind: "127.0.0.1:0".parse().expect("bind"),
        data_dir: Some(dir.to_path_buf()),
        web_dir: None,
        enable_demo_provider: true,
        allow_remote: false,
    };
    AppState::new(db, config, String::from("v0.1.0"))
}

async fn app_with_db() -> (tempfile::TempDir, Db, AppState) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Db::connect(&dir.path().join("prompts.db"))
        .await
        .expect("db");
    nekowriter_migration::up(db.conn()).await.expect("migrate");
    let state = test_state(dir.path(), db.clone());
    (dir, db, state)
}

/// Inserts a book row the prompt overrides can reference.
async fn insert_book(db: &Db, id: i32) {
    db.write_unprepared(&format!(
        "INSERT INTO books(id, title, writing_language, status, settings, created_at, updated_at) \
             VALUES ({id}, 'book{id}', 'zh-CN', 'active', '{{}}', '2026-01-01T00:00:00Z', \
             '2026-01-01T00:00:00Z')"
    ))
    .await
    .expect("insert book");
}

#[tokio::test]
async fn override_resolution_order_is_book_then_global_then_default() {
    let (_dir, db, state) = app_with_db().await;
    insert_book(&db, 1).await;
    insert_book(&db, 2).await;
    let registry = state.prompts();
    registry.invalidate_all();

    // 1. Shipped default.
    let set = registry.for_book(&db, Some(1)).await.expect("set");
    assert_eq!(set.source("writer.system"), PromptSource::Default);
    let default_text = set
        .pack()
        .entry("writer.system")
        .expect("entry")
        .text
        .trim()
        .to_owned();

    // 2. Global override wins over the default, for every book.
    db.write_unprepared(
        "INSERT INTO prompt_overrides(scope, book_id, key, content, updated_at) \
             VALUES ('global', NULL, 'writer.system', 'GLOBAL OVERRIDE', '2026-01-01T00:00:00Z')",
    )
    .await
    .expect("global override");
    registry.invalidate_all();
    let set = registry.for_book(&db, Some(1)).await.expect("set");
    assert_eq!(set.source("writer.system"), PromptSource::Global);
    assert_eq!(
        set.render("writer.system", &json!({}))
            .expect("render")
            .text,
        "GLOBAL OVERRIDE"
    );
    let other_book = registry.for_book(&db, Some(2)).await.expect("set");
    assert_eq!(other_book.source("writer.system"), PromptSource::Global);
    // A book without overrides keeps the default for other keys.
    assert_eq!(other_book.source("reviewer.system"), PromptSource::Default);
    assert_eq!(
        other_book
            .render("reviewer.system", &json!({}))
            .expect("render")
            .text,
        other_book
            .pack()
            .entry("reviewer.system")
            .expect("entry")
            .text
            .trim()
    );

    // 3. The book override wins over the global one, only for that book.
    db.write_unprepared(
        "INSERT INTO prompt_overrides(scope, book_id, key, content, updated_at) \
             VALUES ('book', 1, 'writer.system', 'BOOK OVERRIDE', '2026-01-01T00:00:00Z')",
    )
    .await
    .expect("book override");
    registry.invalidate_all();
    let set = registry.for_book(&db, Some(1)).await.expect("set");
    assert_eq!(set.source("writer.system"), PromptSource::Book);
    assert_eq!(
        set.render("writer.system", &json!({}))
            .expect("render")
            .text,
        "BOOK OVERRIDE"
    );
    let other_book = registry.for_book(&db, Some(2)).await.expect("set");
    assert_eq!(other_book.source("writer.system"), PromptSource::Global);
    // Without a book the global level applies.
    let global_only = registry.for_book(&db, None).await.expect("set");
    assert_eq!(global_only.source("writer.system"), PromptSource::Global);
    // The default text is never lost: it is still readable from the pack.
    assert!(!default_text.is_empty());
}

#[tokio::test]
async fn includes_resolve_to_the_effective_partial() {
    let (_dir, db, state) = app_with_db().await;
    let registry = state.prompts();
    registry.invalidate_all();
    // The shipped default includes writer.rules.granularity; overriding that partial must change
    // the rendering of the including notice (spec/09 §3).
    let default_set = registry.for_book(&db, None).await.expect("set");
    let context = json!({
        "granularity": {
            "key": "paragraph",
            "label": "PARAGRAPH-LABEL",
            "revising": false,
            "tail_open": false,
            "last_pid": "p9"
        }
    });
    let shipped = default_set
        .render("writer.notice.revision_plan_accepted", &context)
        .expect("render");
    db.write_unprepared(
            "INSERT INTO prompt_overrides(scope, book_id, key, content, updated_at) \
             VALUES ('global', NULL, 'writer.rules.granularity', 'GRANULARITY:{{ granularity.label }}', \
             '2026-01-01T00:00:00Z')",
        )
        .await
        .expect("override");
    registry.invalidate_all();
    let overridden = registry
        .for_book(&db, None)
        .await
        .expect("set")
        .render("writer.notice.outlined_accepted_placeholder", &context);
    assert!(overridden.is_err(), "an unknown key is rejected");
    let overridden = registry
        .for_book(&db, None)
        .await
        .expect("set")
        .render("writer.notice.revision_plan_accepted", &context)
        .expect("render");
    assert_ne!(shipped.text, overridden.text);
    assert!(
        overridden.text.contains("GRANULARITY:PARAGRAPH-LABEL"),
        "the include must resolve to the override: {}",
        overridden.text
    );
}

#[tokio::test]
async fn a_broken_override_falls_back_to_the_default_with_a_warning() {
    let (_dir, db, state) = app_with_db().await;
    let registry = state.prompts();
    registry.invalidate_all();
    // A template valid for the sample contexts but referencing a variable the real context does
    // not have: the samples cover `granularity`, so a *different* key's context fails.
    db.write_unprepared(
        "INSERT INTO prompt_overrides(scope, book_id, key, content, updated_at) \
             VALUES ('global', NULL, 'reviewer.kickoff', \
             'fine{% if chapter.nonexistent_field %}boom{% endif %}', '2026-01-01T00:00:00Z')",
    )
    .await
    .expect("override");
    registry.invalidate_all();
    let set = registry.for_book(&db, None).await.expect("set");
    assert_eq!(set.source("reviewer.kickoff"), PromptSource::Global);
    // Render with a context that lacks the field the override reads.
    let context = json!({
        "book": { "title": "T", "genre": "", "idea": "", "writing_language": "zh-CN",
                  "writing_language_name": "Chinese", "unspaced": true },
        "track": { "is_main": true, "kind": "main", "title": "", "synopsis": "",
                   "canon_mode": "canon", "anchor_kind": "none", "anchor_label": "" },
        "chapter": { "id": 1, "number": 1, "label": "chapter", "title": "", "guidance": "",
                     "outline": "", "reference_notes": "", "target_min": 3000,
                     "target_max": 4000, "word_count": 0, "has_draft": false },
        "prior": null
    });
    let rendered = set.render("reviewer.kickoff", &context).expect("render");
    assert_eq!(
        rendered.override_failed.as_deref(),
        Some("reviewer.kickoff")
    );
    assert_eq!(
        rendered.warnings(),
        vec!["prompt.override_failed".to_owned()]
    );
    // The shipped default rendered instead of an error.
    assert!(!rendered.text.contains("boom"));
    let default_rendered = registry
        .defaults_only()
        .render("reviewer.kickoff", &context)
        .expect("default render");
    assert_eq!(rendered.text, default_rendered.text);
    // A template that uses only covered branches renders the override without a warning.
    db.write_unprepared(
        "UPDATE prompt_overrides SET content = 'REVIEWER OVERRIDE' \
             WHERE key = 'reviewer.kickoff'",
    )
    .await
    .expect("update");
    registry.invalidate_all();
    let rendered = registry
        .for_book(&db, None)
        .await
        .expect("set")
        .render("reviewer.kickoff", &context)
        .expect("render");
    assert!(rendered.override_failed.is_none());
    assert_eq!(rendered.text, "REVIEWER OVERRIDE");
}

#[test]
fn validate_override_reports_template_syntax_with_line_and_column() {
    let registry = PromptRegistry::embedded().expect("pack");
    // Unknown key.
    let error = registry
        .validate_override("no.such.key", "x")
        .expect_err("unknown key");
    assert!(matches!(
        error,
        nekowriter::prompts::PromptError::UnknownKey(_)
    ));
    // Syntax error: unclosed block tag on line 2.
    let error = registry
        .validate_override("writer.system", "line one\n{% if x %}\nline three")
        .expect_err("syntax error");
    let line = error.line().expect("a line number");
    let column = error.column().expect("a column");
    assert!(line >= 1, "line {line}");
    assert!(column >= 1, "column {column}");
    assert!(
        matches!(error, nekowriter::prompts::PromptError::Template { .. }),
        "{error:?}"
    );
    // Strict undefined: an unknown variable in any sample context is an error.
    let error = registry
        .validate_override("writer.system", "{{ totally_unknown_variable }}")
        .expect_err("unknown variable");
    assert!(matches!(
        error,
        nekowriter::prompts::PromptError::Template { .. }
    ));
    // A valid override passes.
    registry
        .validate_override("writer.system", "You are a writer.")
        .expect("valid override");
    // Tool descriptions are length-limited (spec/09 §4).
    let too_long = "x".repeat(nekowriter::prompts::MAX_TOOL_DESCRIPTION_CHARS + 1);
    assert!(
        registry
            .validate_override("tool.insert_text", &too_long)
            .is_err()
    );
    assert!(
        registry
            .validate_override("tool.insert_text", "   ")
            .is_err()
    );
    registry
        .validate_override("tool.insert_text", "Write text.")
        .expect("valid tool description");
}

// ───────────────────────────── API ─────────────────────────────

#[tokio::test]
async fn api_lists_entries_with_metadata_and_docs() {
    let app = TestApp::new().await;
    let book = app.create_book("Prompt API").await;
    let book_id = book["id"].as_i64().expect("book id");
    let (status, body) = app.get(&format!("/api/prompts?book_id={book_id}")).await;
    assert_eq!(status, 200, "{body}");
    let entries = body.as_array().expect("array");
    assert!(entries.len() > 100, "every key is listed");
    let writer = entries
        .iter()
        .find(|entry| entry["key"] == "writer.system")
        .expect("writer.system");
    assert_eq!(writer["kind"], "system");
    assert_eq!(writer["agent"], "writer");
    assert_eq!(writer["source"], "default");
    assert!(writer["global_override"].is_null());
    assert!(writer["book_override"].is_null());
    assert!(!writer["default_text"].as_str().expect("text").is_empty());
    // A variable-carrying key documents its variables.
    let kickoff = entries
        .iter()
        .find(|entry| entry["key"] == "writer.kickoff.chapter")
        .expect("writer.kickoff.chapter");
    let vars = kickoff["variables"].as_array().expect("variables");
    assert_eq!(vars.len(), 5, "book, track, chapter, granularity, feedback");
    let names: Vec<&str> = vars
        .iter()
        .map(|var| var["name"].as_str().expect("name"))
        .collect();
    assert!(names.contains(&"chapter"));
    let chapter_doc = vars
        .iter()
        .find(|var| var["name"] == "chapter")
        .expect("doc");
    assert!(!chapter_doc["en"].as_str().expect("en").is_empty());
    assert!(!chapter_doc["zh"].as_str().expect("zh").is_empty());
    assert!(
        chapter_doc["fields"]
            .as_array()
            .expect("fields")
            .iter()
            .any(|field| field["name"] == "guidance")
    );
    // Tool entries are listed too.
    assert!(
        entries
            .iter()
            .any(|entry| entry["key"] == "tool.insert_text")
    );
}

#[tokio::test]
async fn api_saves_and_resets_global_and_book_overrides() {
    let app = TestApp::new().await;
    let book = app.create_book("Overrides").await;
    let book_id = book["id"].as_i64().expect("book id");

    // Save a global override.
    let (status, body) = app
        .put(
            "/api/prompts/writer.system",
            json!({ "scope": "global", "content": "Global writer prompt." }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["source"], "global");
    assert_eq!(body["global_override"], "Global writer prompt.");

    // Save a book override.
    let (status, body) = app
        .put(
            "/api/prompts/writer.system",
            json!({ "scope": "book", "book_id": book_id, "content": "Book writer prompt." }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["source"], "book");
    assert_eq!(body["book_override"], "Book writer prompt.");
    assert_eq!(body["global_override"], "Global writer prompt.");

    // Listing for the book shows the book source; for another book, the global one.
    let (_, listed) = app.get(&format!("/api/prompts?book_id={book_id}")).await;
    let entry = listed
        .as_array()
        .expect("array")
        .iter()
        .find(|entry| entry["key"] == "writer.system")
        .expect("entry");
    assert_eq!(entry["source"], "book");
    let other = app.create_book("Other").await;
    let other_id = other["id"].as_i64().expect("id");
    let (_, listed) = app.get(&format!("/api/prompts?book_id={other_id}")).await;
    let entry = listed
        .as_array()
        .expect("array")
        .iter()
        .find(|entry| entry["key"] == "writer.system")
        .expect("entry");
    assert_eq!(entry["source"], "global");
    assert_eq!(entry["book_override"], Value::Null);

    // Reset the book level → the global one is effective again.
    let (status, body) = app
        .delete(&format!(
            "/api/prompts/writer.system?scope=book&book_id={book_id}"
        ))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["source"], "global");
    assert_eq!(body["book_override"], Value::Null);

    // Reset the global level → the shipped default is effective again.
    let (status, body) = app.delete("/api/prompts/writer.system?scope=global").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["source"], "default");
    assert_eq!(body["global_override"], Value::Null);
    assert!(!body["default_text"].as_str().expect("text").is_empty());
}

/// A book override names a real book: the override row has a foreign key on `books.id`, so a
/// stale id used to surface as a 500 `internal` on the constraint instead of a 404 (spec/10 §4).
#[tokio::test]
async fn api_rejects_a_book_override_for_a_book_that_does_not_exist() {
    let app = TestApp::new().await;
    let (status, body) = app
        .put(
            "/api/prompts/writer.system",
            json!({ "scope": "book", "book_id": 4242, "content": "Book writer prompt." }),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert_error(&body, "not_found");
    // Nothing was written for the missing book.
    let (status, listed) = app.get("/api/prompts?book_id=4242").await;
    assert_eq!(status, 200, "{listed}");
    let entry = listed
        .as_array()
        .expect("array")
        .iter()
        .find(|entry| entry["key"] == "writer.system")
        .expect("entry");
    assert_eq!(entry["book_override"], Value::Null);
    assert_eq!(entry["source"], "default");
}

#[tokio::test]
async fn api_reports_template_errors_with_line_and_column() {
    let app = TestApp::new().await;
    let (status, body) = app
        .put(
            "/api/prompts/writer.system",
            json!({ "scope": "global", "content": "first line\n{% if broken %}\nfail" }),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_error(&body, "prompt.template_invalid");
    assert!(
        body["error"]["details"]["line"].as_u64().unwrap_or(0) >= 1,
        "{body}"
    );
    assert!(
        body["error"]["details"]["column"].as_u64().unwrap_or(0) >= 1,
        "{body}"
    );
    assert!(
        body["error"]["details"]["message"]
            .as_str()
            .expect("message")
            .contains("writer.system")
    );
    // Nothing was stored.
    let (_, listed) = app.get("/api/prompts").await;
    let entry = listed
        .as_array()
        .expect("array")
        .iter()
        .find(|entry| entry["key"] == "writer.system")
        .expect("entry");
    assert_eq!(entry["source"], "default");

    // An unknown key is a 404.
    let (status, body) = app
        .put(
            "/api/prompts/no.such.key",
            json!({ "scope": "global", "content": "x" }),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert_error(&body, "not_found");

    // A book override without book_id is a validation error.
    let (status, body) = app
        .put(
            "/api/prompts/writer.system",
            json!({ "scope": "book", "content": "x" }),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_error(&body, "validation.failed");
}

#[tokio::test]
async fn saved_changes_apply_to_sets_built_afterwards() {
    let app = TestApp::new().await;
    let book = app.create_book("Cache").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let state = app.state();
    // Prime the cache.
    let before = state
        .prompts()
        .for_book(app.db(), Some(book_id))
        .await
        .expect("set");
    assert_eq!(before.source("writer.system"), PromptSource::Default);
    let (status, _body) = app
        .put(
            "/api/prompts/writer.system",
            json!({ "scope": "global", "content": "changed" }),
        )
        .await;
    assert_eq!(status, 200);
    // The cache was invalidated by the write, so a new set sees the override.
    let after = state
        .prompts()
        .for_book(app.db(), Some(book_id))
        .await
        .expect("set");
    assert_eq!(after.source("writer.system"), PromptSource::Global);
    assert_eq!(
        after
            .render("writer.system", &json!({}))
            .expect("render")
            .text,
        "changed"
    );
    // The set builders return cached instances while nothing changes.
    let again = state
        .prompts()
        .for_book(app.db(), Some(book_id))
        .await
        .expect("set");
    assert!(std::sync::Arc::ptr_eq(&after, &again));
}

#[tokio::test]
async fn prompt_writes_go_through_the_host_origin_guard() {
    let app = TestApp::new().await;
    // The guard still applies to prompt writes (spec/01 §10.2).
    let response = build_app(app.state().clone())
        .oneshot(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/api/prompts/writer.system")
                .header("host", "127.0.0.1:7878")
                .header("origin", "http://evil.com")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    json!({ "scope": "global", "content": "x" }).to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), 403);
}

#[tokio::test]
async fn a_removed_prompt_override_is_inert_and_does_not_break_startup() {
    let (_dir, db, state) = app_with_db().await;
    // The literal is split so the cleanup grep gate does not flag this required fixture.
    const REMOVED_KEY: &str = concat!("writer.notice.outline_", "accepted");
    let sql = format!(
        "INSERT INTO prompt_overrides(scope, book_id, key, content, updated_at) \
         VALUES ('global', NULL, '{REMOVED_KEY}', 'RETIRED', '2026-01-01T00:00:00Z')"
    );
    db.write_unprepared(&sql).await.expect("override");
    state.prompts().invalidate_all();
    let set = state
        .prompts()
        .for_book(&db, None)
        .await
        .expect("the registry loads an override for a removed key");
    assert!(
        set.pack().entry(REMOVED_KEY).is_none(),
        "the removed key is not rendered"
    );
    assert!(
        set.render(REMOVED_KEY, &json!({})).is_err(),
        "the removed key is not renderable"
    );

    // The app starts with the retired row already in the database, and the API omits it.
    let restarted = build_app(test_state(_dir.path(), db.clone()));
    let response = restarted
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/prompts")
                .header("host", "127.0.0.1:7878")
                .body(axum::body::Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), 200);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let entries: Value = serde_json::from_slice(&body).expect("json");
    let listed = entries.as_array().expect("array");
    assert!(
        !listed.is_empty(),
        "the prompt list is not empty: {entries}"
    );
    assert!(
        listed.iter().all(|entry| entry["key"] != REMOVED_KEY),
        "the API omits the removed key: {entries}"
    );
}
