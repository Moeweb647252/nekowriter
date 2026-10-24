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

#[test]
fn other_kinds_reply_with_text_for_now() {
    let provider = DemoProvider::new();
    for (kind, phase) in [
        ("chapter_revise", "revision_planning"),
        ("chapter_review", "none"),
        ("chapter_maintenance", "none"),
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
