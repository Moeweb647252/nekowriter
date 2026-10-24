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
    let provider = DemoProvider::new();
    for (kind, phase) in [
        ("chapter_review", "none"),
        ("chapter_maintenance", "none"),
        ("canon_seed", "none"),
        ("conception", "none"),
        ("research", "none"),
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
    }
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
