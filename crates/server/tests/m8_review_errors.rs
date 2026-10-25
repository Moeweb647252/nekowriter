//! M8 review: the model-facing error-text class (spec/09 §2, spec/05 §1.5).
//!
//! The renderer is strict: `result.error` branches read named fields, and a `ToolError` that
//! carries a field the branch does not read (or omits one it does) renders to an EMPTY text, so
//! the model gets no explanation at all. This file pins the mechanism the class-level test in
//! `agent/tools/mod.rs` relies on, and checks every M8 `ToolError` construction through the real
//! prompt pack.

use nekowriter::agent::tools::{ToolError, ToolErrorKind, ToolOutcome};
use nekowriter::prompts::PromptRegistry;
use serde_json::{Value, json};

fn prompts() -> nekowriter::prompts::PromptSet {
    PromptRegistry::embedded()
        .expect("the embedded prompt pack")
        .defaults_only()
}

/// The class-level test asserts every kind renders non-empty text and never falls through to the
/// internal-error wording. This proves the assertion has teeth: a branch whose fields are wrong
/// renders nothing, so a call site that drifts from its branch is caught by the same assertion.
#[test]
fn a_wrong_field_renders_empty_so_the_class_test_would_fail() {
    // `fetch_failed` reads `reason` and `url`; the URL is deliberately missing here.
    let missing_url = ToolError::new(ToolErrorKind::FetchFailed).with("reason", "blocked");
    let outcome = ToolOutcome::error(&prompts(), &missing_url);
    assert!(outcome.is_error);
    assert!(
        outcome.text.trim().is_empty(),
        "the strict renderer must yield an empty text, not a fallback: {:?}",
        outcome.text
    );

    // A field with the wrong name is the same shape: `entry_exists` reads `title`, not `name`.
    let wrong_name = ToolError::new(ToolErrorKind::EntryExists)
        .with("name", "Gate rules")
        .with("id", 7);
    let outcome = ToolOutcome::error(&prompts(), &wrong_name);
    assert!(
        outcome.text.trim().is_empty(),
        "a misnamed field must not silently render: {:?}",
        outcome.text
    );
}

/// Every kind the crate knows, with the fields its branch reads. The list mirrors the class-level
/// test in `agent/tools/mod.rs`; the assertion here is that none of them renders empty or falls
/// through to the internal text (parity is checked by the crate test itself).
#[test]
fn every_m8_error_kind_renders_its_own_text() {
    let cases = vec![
        ToolError::new(ToolErrorKind::SearchNotConfigured),
        ToolError::new(ToolErrorKind::SearchFailed).with("reason", "timeout"),
        ToolError::new(ToolErrorKind::SearchFailed)
            .with("reason", "status")
            .with("status", 503),
        ToolError::new(ToolErrorKind::FetchFailed)
            .with("reason", "blocked")
            .with("url", "http://127.0.0.1/"),
        ToolError::new(ToolErrorKind::FetchFailed)
            .with("reason", "status")
            .with("status", 404)
            .with("url", "https://example.com/missing"),
        ToolError::new(ToolErrorKind::UnsupportedContentType)
            .with("content_type", "application/pdf")
            .with("url", "https://example.com/a.pdf"),
        ToolError::new(ToolErrorKind::NotFound)
            .with("what_label", "知识条目")
            .with("id", 7),
        ToolError::new(ToolErrorKind::EntryExists)
            .with("title", "Gate rules")
            .with("id", 7),
        ToolError::new(ToolErrorKind::ScopeNotFound)
            .with("name", "Elsewhere")
            .with("available", vec!["Target".to_owned()]),
        ToolError::new(ToolErrorKind::FindFailed)
            .with("doc_label", "知识条目")
            .with("find", "old")
            .with("count", 2),
        ToolError::new(ToolErrorKind::Validation).with("errors", vec!["title must not be empty"]),
        ToolError::new(ToolErrorKind::MutuallyExclusive).with("fields", vec!["content", "edits"]),
        ToolError::new(ToolErrorKind::NoKnowledgeScope),
    ];
    let prompts = prompts();
    for case in cases {
        let outcome = ToolOutcome::error(&prompts, &case);
        assert!(outcome.is_error, "{}", case.kind.code());
        assert!(
            !outcome.text.trim().is_empty(),
            "{} rendered an empty text: its fields do not match result.error",
            case.kind.code()
        );
        assert!(
            !outcome.text.contains("{{"),
            "{} left a template placeholder: {}",
            case.kind.code(),
            outcome.text
        );
        assert!(
            !outcome.text.contains("内部错误"),
            "{} fell through to the internal text: {}",
            case.kind.code(),
            outcome.text
        );
    }
}

/// The M8 tools build their errors through helpers; this exercises the wire-level shapes the
/// helpers produce for the two research errors that were previously at risk (`not_found`,
/// `scope_not_found`, `find_failed`) and asserts each names the arguments the model needs to act.
#[test]
fn the_research_error_shapes_name_what_the_model_must_change() {
    let prompts = prompts();
    let cases: Vec<(ToolError, &str)> = vec![
        (
            ToolError::new(ToolErrorKind::NotFound)
                .with("what_label", "知识条目")
                .with("id", 42),
            "#42",
        ),
        (
            ToolError::new(ToolErrorKind::ScopeNotFound)
                .with("name", "Elsewhere")
                .with("available", vec!["Target".to_owned()]),
            "Elsewhere",
        ),
        (
            ToolError::new(ToolErrorKind::FindFailed)
                .with("doc_label", "知识条目")
                .with("find", "old sentence")
                .with("count", 0),
            "old sentence",
        ),
    ];
    for (error, expected) in cases {
        let outcome = ToolOutcome::error(&prompts, &error);
        assert!(
            outcome.text.contains(expected),
            "{} should name {expected:?}: {}",
            error.kind.code(),
            outcome.text
        );
    }
}

/// `web_search` and `web_fetch` result payloads must be renderable: their templates read
/// `query`/`hits` and `url`/`title`/…/`body`, and a missing key is a render error (internal).
#[test]
fn the_web_result_templates_render_the_arguments_the_tools_pass() {
    let prompts = prompts();
    let values: Vec<(&str, Value)> = vec![
        (
            "result.web_search",
            json!({
                "query": "tea",
                "hits": [{
                    "title": "T",
                    "url": "https://example.com/a",
                    "snippet": "s",
                    "published": "",
                }],
            }),
        ),
        (
            "result.web_fetch",
            json!({
                "url": "https://example.com/a",
                "title": "T",
                "offset": 0,
                "end": 4,
                "total": 4,
                "next_offset": serde_json::Value::Null,
                "body": "body",
            }),
        ),
        ("result.finish_research", json!({ "entries": 0 })),
        (
            "result.knowledge_write",
            json!({ "action": "created", "id": 1, "title": "T", "scope": "S" }),
        ),
    ];
    for (key, value) in values {
        let rendered = prompts
            .render_value(key, nekowriter::prompts::template_value(&value))
            .unwrap_or_else(|error| panic!("{key} did not render: {error}"));
        assert!(
            !rendered.text.trim().is_empty(),
            "{key} rendered an empty text"
        );
    }
}
