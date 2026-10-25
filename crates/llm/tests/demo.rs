//! Demo provider behavior tests (spec/04 §7.2, DECISIONS D-019).

use nekowriter_llm::demo::{DemoProvider, choose_action};
use nekowriter_llm::types::{ChatRequest, DemoHints, ModelOptions, StopReason, StreamEvent};

fn hints(kind: &str, phase: &str, language: &str) -> DemoHints {
    DemoHints {
        kind: kind.to_owned(),
        phase: phase.to_owned(),
        granularity: Some("paragraph".to_owned()),
        step_in_phase: 0,
        language: language.to_owned(),
        tools: Vec::new(),
        protagonist: None,
        review_attempt: 1,
        canon_mode: "canon".to_owned(),
        draft: Vec::new(),
        saved_entries: 0,
    }
}

fn request(hints: Option<DemoHints>) -> ChatRequest {
    let mut request = ChatRequest::ping("demo-model", "");
    request.demo_hints = hints;
    request.options = ModelOptions::default();
    request
}

#[test]
fn list_models_returns_the_demo_model() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let provider = DemoProvider::new();
    let models = runtime.block_on(provider.list_models()).expect("models");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "demo-model");
    assert_eq!(models[0].context_window, Some(200_000));
    assert_eq!(models[0].max_output_tokens, Some(16_384));
}

#[test]
fn planning_submits_an_outline_in_the_book_language() {
    let provider = DemoProvider::new();
    for language in ["zh-CN", "en"] {
        let plan = choose_action(
            &hints("chapter_write", "planning", language),
            provider.lang(Some(&hints("chapter_write", "planning", language))),
        );
        assert!(
            plan.text.is_none(),
            "{language}: no prose reply while planning"
        );
        assert_eq!(plan.tools.len(), 1);
        assert_eq!(plan.tools[0].1, "submit_outline");
        let outline = plan.tools[0].2["outline"].as_str().expect("outline");
        assert!(!outline.is_empty(), "{language}: the corpus has an outline");
    }
}

/// Every kind that has no behavior of its own yet replies with the closing message, so the agent
/// panel shows something and the runtime's nudge path is exercised.
#[test]
fn other_kinds_reply_with_text_for_now() {
    // `research` is no longer in this list: M8 gives it a real plan (see
    // `the_researcher_saves_entries_then_finishes` below).
    let provider = DemoProvider::new();
    for (kind, phase) in [
        ("canon_seed", "none"),
        ("conception", "none"),
        ("unknown_kind", "none"),
    ] {
        let hints = hints(kind, phase, "en");
        let plan = choose_action(&hints, provider.lang(Some(&hints)));
        assert!(plan.tools.is_empty(), "{kind}/{phase}: no tools yet");
        assert_eq!(
            plan.text.as_deref(),
            Some(provider.lang(Some(&hints)).closing_message.as_str()),
            "{kind}/{phase}"
        );
    }
}

#[tokio::test]
async fn streams_events_and_reports_estimated_usage() {
    let provider = DemoProvider::new();
    let planning_request = request(Some(hints("chapter_write", "planning", "zh-CN")));
    let mut events = Vec::new();
    let turn = provider
        .run(&planning_request, &mut |event| events.push(event))
        .await
        .expect("turn");
    assert_eq!(turn.stop_reason, StopReason::ToolUse);
    assert_eq!(turn.tool_calls.len(), 1);
    assert_eq!(turn.tool_calls[0].name, "submit_outline");
    // Falls back to a text reply without hints (used by the connection test).
    assert!(turn.usage.estimated);
    assert!(turn.usage.input_tokens > 0);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::Usage(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::ToolCallStart { .. }))
    );

    let no_hints = request(None);
    let turn = provider.run(&no_hints, &mut |_e| {}).await.expect("turn");
    assert_eq!(turn.stop_reason, StopReason::EndTurn);
    assert!(!turn.text.is_empty());
}

#[test]
fn corpus_parses_for_both_languages() {
    let provider = DemoProvider::new();
    for language in ["zh-CN", "en"] {
        let hints = hints("chapter_write", "planning", language);
        let corpus = provider.lang(Some(&hints));
        assert!(!corpus.sentences.is_empty(), "{language}: sentences");
        assert!(!corpus.paragraphs.is_empty(), "{language}: paragraphs");
        assert!(!corpus.outline.is_empty(), "{language}: outline");
        assert!(
            !corpus.setting_docs.outline.is_empty(),
            "{language}: outline doc"
        );
        assert!(
            !corpus.setting_docs.world.is_empty(),
            "{language}: world doc"
        );
        assert!(
            !corpus.setting_docs.protagonist.is_empty(),
            "{language}: protagonist doc"
        );
        assert!(!corpus.characters.is_empty(), "{language}: characters");
        assert!(!corpus.questions.is_empty(), "{language}: questions");
        assert!(
            !corpus.knowledge_entries.is_empty(),
            "{language}: knowledge entries"
        );
        assert!(
            !corpus.research_topics.is_empty(),
            "{language}: research topics"
        );
        assert!(
            !corpus.closing_message.is_empty(),
            "{language}: closing message"
        );
        assert!(
            !corpus.review_summary.is_empty(),
            "{language}: review summary"
        );
        assert!(!corpus.review_issue.is_empty(), "{language}: review issue");
        assert!(
            !corpus.revision_plan.is_empty(),
            "{language}: revision plan"
        );
        assert!(
            !corpus.revision_edit.is_empty(),
            "{language}: revision edit"
        );
        assert!(
            !corpus.summary_template.is_empty(),
            "{language}: summary template"
        );
        assert!(!corpus.entity_state.is_empty(), "{language}: entity state");
        assert!(
            !corpus.timeline_title.is_empty(),
            "{language}: timeline title"
        );
        assert!(
            !corpus.proposal_text.is_empty(),
            "{language}: proposal text"
        );
        assert!(
            !corpus.review_feedback.is_empty(),
            "{language}: review feedback"
        );
        assert!(
            !corpus.review_suggestion.is_empty(),
            "{language}: review suggestion"
        );
        assert!(
            !corpus.maintenance_report.is_empty(),
            "{language}: maintenance report"
        );
        assert!(
            !corpus.proposal_rationale.is_empty(),
            "{language}: proposal rationale"
        );
    }
}

/// spec/04 §7.2: the reviewer asks for a revision with one issue on `p2` on the first attempt and
/// passes afterwards.
#[test]
fn reviewer_revises_on_the_first_attempt_and_passes_later() {
    let provider = DemoProvider::new();
    for language in ["zh-CN", "en"] {
        let mut first = hints("chapter_review", "none", language);
        first.review_attempt = 1;
        let plan = choose_action(&first, provider.lang(Some(&first)));
        assert_eq!(plan.tools.len(), 1, "{language}");
        assert_eq!(plan.tools[0].1, "submit_review");
        assert_eq!(plan.tools[0].2["verdict"], "revise");
        assert_eq!(plan.tools[0].2["issues"][0]["pids"][0], "p2");

        let mut second = first.clone();
        second.review_attempt = 2;
        let plan = choose_action(&second, provider.lang(Some(&second)));
        assert_eq!(plan.tools[0].2["verdict"], "pass", "{language}");
        assert_eq!(plan.tools[0].2["issues"], serde_json::json!([]));
    }
}

/// D-077: the revising step rewrites the first draft paragraph by its exact text, so `old`
/// occurs exactly once; without a draft hint it finishes directly.
#[test]
fn revising_rewrites_a_known_paragraph_then_finishes() {
    use nekowriter_llm::types::DemoParagraph;
    let provider = DemoProvider::new();
    let mut revising = hints("chapter_revise", "revising", "zh-CN");
    revising.step_in_phase = 1;
    revising.draft = vec![DemoParagraph {
        pid: "p1".to_owned(),
        text: "第一段的原文，只出现一次。".to_owned(),
    }];
    let lang = provider.lang(Some(&revising));
    let plan = choose_action(&revising, lang);
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].1, "replace_text");
    assert_eq!(plan.tools[0].2["pid"], "p1");
    assert_eq!(plan.tools[0].2["old"], "第一段的原文，只出现一次。");
    assert_eq!(plan.tools[0].2["new"], lang.revision_edit);

    let mut finish = revising.clone();
    finish.step_in_phase = 2;
    let plan = choose_action(&finish, provider.lang(Some(&finish)));
    assert_eq!(plan.tools[0].1, "finish_revision");

    revising.draft.clear();
    revising.step_in_phase = 1;
    let plan = choose_action(&revising, provider.lang(Some(&revising)));
    assert_eq!(
        plan.tools[0].1, "finish_revision",
        "without a known paragraph the demo never invents a `old` text"
    );
}

/// spec/06 §6: the archivist walks appearances → state → timeline → summary → proposal →
/// finish, and creates the character entity first when the book has no canon yet.
#[test]
fn maintenance_walks_the_procedure_and_skips_the_proposal_when_isolated() {
    let provider = DemoProvider::new();
    let mut hints = hints("chapter_maintenance", "none", "zh-CN");
    hints.protagonist = Some("沈砚".to_owned());
    let lang = provider.lang(Some(&hints));
    let names: Vec<String> = (0..7)
        .map(|step| {
            let mut step_hints = hints.clone();
            step_hints.step_in_phase = step;
            choose_action(&step_hints, lang).tools[0].1.clone()
        })
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    assert_eq!(
        names,
        vec![
            "record_appearances",
            "update_entity",
            "add_timeline_event",
            "set_chapter_summary",
            "propose_setting_change",
            "finish_maintenance",
            "finish_maintenance",
        ]
    );
    let update = {
        let mut step_hints = hints.clone();
        step_hints.step_in_phase = 1;
        choose_action(&step_hints, lang).tools[0].2.clone()
    };
    assert_eq!(update["entity"], "沈砚");
    assert!(
        !update["state"].as_str().unwrap_or_default().is_empty(),
        "the state update carries the corpus state"
    );

    // An isolated track never proposes a setting change (spec/06 §6 step 9).
    let mut isolated = hints.clone();
    isolated.canon_mode = "isolated".to_owned();
    isolated.step_in_phase = 4;
    let plan = choose_action(&isolated, lang);
    assert_eq!(plan.tools[0].1, "finish_maintenance");

    // Without a protagonist the demo creates the entity it will record first.
    let mut no_protagonist = hints.clone();
    no_protagonist.protagonist = None;
    no_protagonist.step_in_phase = 0;
    let plan = choose_action(&no_protagonist, lang);
    assert_eq!(plan.tools[0].1, "create_entity");
}

/// spec/04 §7.2: planning looks the protagonist up first (when there is one), then submits a
/// three-scene outline with reference notes and a title.
#[test]
fn planning_starts_with_get_entity_when_the_book_has_a_protagonist() {
    let provider = DemoProvider::new();
    let mut with_protagonist = hints("chapter_write", "planning", "zh-CN");
    with_protagonist.protagonist = Some("沈砚".to_owned());
    let lang = provider.lang(Some(&with_protagonist));
    let first = choose_action(&with_protagonist, lang);
    assert_eq!(first.tools.len(), 1);
    assert_eq!(first.tools[0].1, "get_entity");
    assert_eq!(first.tools[0].2["entity"], "沈砚");

    // The step after the lookup submits the outline.
    let mut second = with_protagonist.clone();
    second.step_in_phase = 1;
    let plan = choose_action(&second, lang);
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].1, "submit_outline");
}

/// Without a protagonist, planning goes straight to the outline (the demo never invents one).
#[test]
fn planning_without_a_protagonist_goes_straight_to_the_outline() {
    let provider = DemoProvider::new();
    let h = hints("chapter_write", "planning", "zh-CN");
    let plan = choose_action(&h, provider.lang(Some(&h)));
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].1, "submit_outline");
}

/// spec/04 §7.2: planning submits a three-scene outline with reference notes and a title.
#[test]
fn planning_carries_notes_and_a_title() {
    let provider = DemoProvider::new();
    for language in ["zh-CN", "en"] {
        let h = hints("chapter_write", "planning", language);
        let lang = provider.lang(Some(&h));
        let plan = choose_action(&h, lang);
        let arguments = &plan.tools[0].2;
        assert!(
            arguments["reference_notes"]
                .as_str()
                .is_some_and(|notes| !notes.is_empty()),
            "{language}: the planning step submits reference notes"
        );
        assert_eq!(
            arguments["title"].as_str(),
            Some(lang.chapter_title.as_str()),
            "{language}: the planning step proposes a title"
        );
    }
}

/// spec/04 §7.2: the drafting step matches the granularity it was told to write at.
#[test]
fn drafting_matches_the_granularity() {
    let provider = DemoProvider::new();
    for language in ["zh-CN", "en"] {
        for (granularity, expect_break) in [
            ("sentence", false),
            ("paragraph", false),
            ("free", true),
            ("chapter", true),
        ] {
            let mut h = hints("chapter_write", "drafting", language);
            h.granularity = Some(granularity.to_owned());
            let lang = provider.lang(Some(&h));
            let plan = choose_action(&h, lang);
            assert_eq!(plan.tools.len(), 1, "{language}/{granularity}");
            assert_eq!(plan.tools[0].1, "insert_text", "{language}/{granularity}");
            let text = plan.tools[0].2["text"].as_str().expect("text");
            assert!(!text.is_empty(), "{language}/{granularity}: some text");
            assert_eq!(
                text.contains('\n'),
                expect_break,
                "{language}/{granularity}: line breaks separate paragraphs only when allowed"
            );
        }
    }
}

/// A `sentence` drafting step never exceeds the 400-character limit, and the tail alternates so
/// the chapter gains several paragraphs.
#[test]
fn sentence_steps_obey_the_matrix() {
    let provider = DemoProvider::new();
    for step in 1..12u32 {
        let mut h = hints("chapter_write", "drafting", "zh-CN");
        h.granularity = Some("sentence".to_owned());
        h.step_in_phase = step;
        let lang = provider.lang(Some(&h));
        let plan = choose_action(&h, lang);
        // Past the drafting window the demo completes the chapter with `finish_draft`.
        if plan.tools[0].1 == "finish_draft" {
            continue;
        }
        let text = plan.tools[0].2["text"].as_str().expect("text");
        assert!(text.chars().count() <= 400, "step {step}: {text}");
        assert!(!text.contains('\n'), "step {step}: no line break");
        assert!(
            plan.tools[0].2["paragraph_end"].is_boolean(),
            "step {step}: paragraph_end is stated"
        );
    }
}

/// The `chapter` granularity writes several paragraphs in one call (spec/04 §7.2 "~8").
#[test]
fn chapter_granularity_writes_several_paragraphs_at_once() {
    let provider = DemoProvider::new();
    let mut h = hints("chapter_write", "drafting", "en");
    h.granularity = Some("chapter".to_owned());
    h.step_in_phase = 1;
    let lang = provider.lang(Some(&h));
    let plan = choose_action(&h, lang);
    let text = plan.tools[0].2["text"].as_str().expect("text");
    assert!(
        text.split("\n\n").count() >= 8,
        "the chapter granularity writes the rest of the chapter: {text}"
    );
}

/// spec/04 §7.2: the demo completes the chapter after ~8 paragraphs instead of streaming forever
/// (a `chapter_end` for sentence/paragraph/free, or the `chapter` granularity's own single call).
#[test]
fn drafting_completes_the_chapter_after_the_drafting_window() {
    let provider = DemoProvider::new();
    for language in ["zh-CN", "en"] {
        for granularity in ["sentence", "paragraph", "free"] {
            let mut h = hints("chapter_write", "drafting", language);
            h.granularity = Some(granularity.to_owned());
            // Session step 8 is the completing unit; step 9 confirms a withheld completion.
            h.step_in_phase = 9;
            let lang = provider.lang(Some(&h));
            let plan = choose_action(&h, lang);
            assert_eq!(
                plan.tools[0].1, "finish_draft",
                "{language}/{granularity}: the demo completes the chapter"
            );
        }
        // `chapter` granularity completes through its own call: it never needs finish_draft.
        let mut h = hints("chapter_write", "drafting", language);
        h.granularity = Some("chapter".to_owned());
        h.step_in_phase = 9;
        let lang = provider.lang(Some(&h));
        let plan = choose_action(&h, lang);
        assert_eq!(
            plan.tools[0].1, "insert_text",
            "{language}/chapter: the chapter call itself completes"
        );
    }
}

/// A short chapter's first `chapter_end` is withheld (spec/03 §8), so the demo's `finish_draft`
/// step is what confirms it; the demo keeps writing if the chapter is not done yet.
#[test]
fn a_witheld_completion_is_followed_by_finish_draft() {
    let provider = DemoProvider::new();
    let mut h = hints("chapter_write", "drafting", "en");
    h.granularity = Some("paragraph".to_owned());
    // Session step 8 is drafting unit 8, the completing one.
    h.step_in_phase = 8;
    let lang = provider.lang(Some(&h));
    let plan = choose_action(&h, lang);
    assert_eq!(plan.tools[0].1, "insert_text");
    assert_eq!(
        plan.tools[0].2["chapter_end"], true,
        "the completing unit asks to end the chapter"
    );
    // A withheld short completion is confirmed by the next step's `finish_draft`.
    h.step_in_phase = 9;
    let plan = choose_action(&h, lang);
    assert_eq!(plan.tools[0].1, "finish_draft");
}

/// The Demo researcher's plan (spec/04 §7.2): save the corpus entries one step at a time, then
/// report; a task that already has its entries finishes immediately.
#[test]
fn the_researcher_saves_entries_then_finishes() {
    let provider = DemoProvider::new();
    let lang = provider.lang(Some(&hints("research", "none", "zh-CN")));
    assert!(
        !lang.knowledge_entries.is_empty(),
        "the corpus carries knowledge entries for the researcher"
    );

    // Nothing saved yet: the first step saves the first corpus entry, with its citation.
    let mut hints = hints("research", "none", "zh-CN");
    hints.saved_entries = 0;
    let plan = choose_action(&hints, provider.lang(Some(&hints)));
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].1, "save_knowledge_entry");
    assert_eq!(plan.tools[0].2["title"], lang.knowledge_entries[0].title);
    assert!(
        plan.tools[0].2["source_urls"]
            .as_array()
            .is_some_and(|urls| !urls.is_empty()),
        "the saved entry carries a citation"
    );

    // One saved: the second entry.
    hints.saved_entries = 1;
    let plan = choose_action(&hints, provider.lang(Some(&hints)));
    assert_eq!(plan.tools[0].2["title"], lang.knowledge_entries[1].title);

    // Two saved: finish with the corpus report.
    hints.saved_entries = 2;
    let plan = choose_action(&hints, provider.lang(Some(&hints)));
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].1, "finish_research");
    assert_eq!(plan.tools[0].2["report"], lang.research_report);
    assert!(!lang.research_report.is_empty());
}
