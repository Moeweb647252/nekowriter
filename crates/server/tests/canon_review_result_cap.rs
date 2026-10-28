//! M5 review regressions for the default result cap (spec/05 §1.5).
//!
//! "Default cap of 12,000 characters per result; tools with a `max_chars` parameter (`read_chapter`,
//! `read_draft`, `web_fetch`) use that value instead. Longer results are cut at a paragraph or line
//! boundary and end with `result.truncated` (offset hint) or the tool's own continuation hint."
//!
//! The cap is applied where the runner finalizes a call, so it covers every tool. These tests
//! drive real sessions: a long `get_entity` card (no `max_chars`) is cut at a paragraph boundary
//! with the rendered template, while `read_chapter`'s own `max_chars` is left alone — even when
//! the caller asks for more than the global limit.

mod agent_write_harness;

use agent_write_harness::base::runtime;
use nekowriter::prompts::PromptRegistry;
use nekowriter_llm::mock::{ScriptStep, ScriptedProvider};
use serde_json::json;

/// The rendered `result.truncated` text, for the suffix assertion.
fn truncated_notice() -> String {
    PromptRegistry::embedded()
        .expect("the embedded prompt pack")
        .defaults_only()
        .render(
            "result.truncated",
            &json!({ "next_offset": serde_json::Value::Null }),
        )
        .expect("result.truncated")
        .text
}

/// The persisted tool calls of a finished session, as `(name, result_text, is_error)`.
async fn tool_calls(
    runtime: &agent_write_harness::Runtime,
    session_id: i32,
) -> Vec<(String, String, bool)> {
    let session = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await
        .1;
    session["tool_calls"]
        .as_array()
        .expect("tool calls")
        .iter()
        .map(|call| {
            (
                call["name"].as_str().expect("name").to_owned(),
                call["result_text"].as_str().unwrap_or_default().to_owned(),
                call["is_error"].as_bool().expect("is_error"),
            )
        })
        .collect()
}

/// A long `get_entity` card is capped at 12,000 characters, cut at a paragraph boundary, and ends
/// with the rendered `result.truncated` template (spec/05 §1.5).
#[tokio::test]
async fn a_long_result_is_capped_at_a_paragraph_boundary_with_the_template() {
    let mut runtime = runtime(Vec::new()).await;
    // Every description paragraph is exactly 300 characters, separated by a blank line.
    let paragraph = |index: usize| format!("P{index:03}:{}", "a".repeat(295));
    let description = (0..80).map(paragraph).collect::<Vec<_>>().join("\n\n");
    let (status, entity) = runtime
        .app
        .post(
            &format!("/api/books/{}/entities", runtime.book_id),
            json!({
                "kind": "character",
                "name": "Long Winded",
                "description": description,
            }),
        )
        .await;
    assert_eq!(status, 201, "{entity}");

    runtime.app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new().tool("get_entity", json!({ "entity": "Long Winded" })),
    ]));
    let session_id = agent_write_harness::start_planner(&runtime).await;
    runtime.wait_any_status(session_id).await;
    let calls = tool_calls(&runtime, session_id).await;
    let (_, text, is_error) = calls
        .iter()
        .find(|(name, _, _)| name == "get_entity")
        .expect("the get_entity call");
    assert!(!is_error, "{text}");

    let notice = truncated_notice();
    assert!(
        text.ends_with(&notice),
        "the result ends with the rendered result.truncated template"
    );
    let head = text
        .strip_suffix(&notice)
        .expect("the notice suffix")
        .trim_end_matches('\n');
    assert!(
        head.chars().count() <= 12_000,
        "the head keeps the character budget: {}",
        head.chars().count()
    );
    let description_text = head
        .split_once("详细描述：\n")
        .expect("the description section")
        .1;
    let paragraphs: Vec<&str> = description_text.split("\n\n").collect();
    assert!(
        paragraphs
            .iter()
            .all(|candidate| candidate.chars().count() == 300),
        "every kept paragraph is complete: {:?}",
        paragraphs
            .iter()
            .map(|candidate| candidate.chars().count())
            .collect::<Vec<_>>()
    );
    assert!(
        paragraphs.len() < 80,
        "the result was actually shortened: {} paragraphs",
        paragraphs.len()
    );
}

/// `read_chapter` owns its `max_chars` cap: asking for 20,000 characters returns more than the
/// global 12,000 and never gains the global truncation notice (spec/05 §1.5).
#[tokio::test]
async fn a_tool_owned_max_chars_is_not_cut_by_the_global_cap() {
    let mut runtime = runtime(Vec::new()).await;
    // A second main-track chapter with about 19,000 characters of prose and a tail marker.
    let chapter = runtime.app.create_chapter(runtime.track_id, "Two").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    let mut text = String::new();
    for index in 0..20 {
        if index > 0 {
            text.push_str("\n\n");
        }
        text.push_str(&format!("P{index:02}:{}", "x".repeat(940)));
    }
    text.push_str("\n\nEND-MARKER");
    runtime.app.put_text(chapter_id, 0, &text).await;
    runtime.app.set_chapter_status(chapter_id, "official").await;

    runtime
        .app
        .set_provider(ScriptedProvider::new(vec![ScriptStep::new().tool(
            "read_chapter",
            json!({ "chapter": "main 2", "max_chars": 20_000 }),
        )]));
    let session_id = agent_write_harness::start_planner(&runtime).await;
    runtime.wait_any_status(session_id).await;
    let calls = tool_calls(&runtime, session_id).await;
    let (_, text, is_error) = calls
        .iter()
        .find(|(name, _, _)| name == "read_chapter")
        .expect("the read_chapter call");
    assert!(!is_error, "{text}");
    assert!(
        text.chars().count() > 12_000,
        "read_chapter's own cap allows more than the default: {}",
        text.chars().count()
    );
    assert!(
        !text.contains("内容较长，已截断"),
        "the global cap did not cut a tool that owns its cap"
    );
    assert!(text.contains("END-MARKER"), "the whole chapter is there");
}

/// A smaller tool-owned `max_chars` keeps the tool's own continuation hint, not the global
/// truncation template (spec/05 §1.5).
#[tokio::test]
async fn a_smaller_tool_cap_uses_the_tools_own_hint() {
    let mut runtime = runtime(Vec::new()).await;
    let chapter = runtime.app.create_chapter(runtime.track_id, "Two").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    let text = (0..30)
        .map(|index| format!("Paragraph {index} has enough words to fill the budget."))
        .collect::<Vec<_>>()
        .join("\n\n");
    runtime.app.put_text(chapter_id, 0, &text).await;
    runtime.app.set_chapter_status(chapter_id, "official").await;

    runtime
        .app
        .set_provider(ScriptedProvider::new(vec![ScriptStep::new().tool(
            "read_chapter",
            json!({ "chapter": "main 2", "max_chars": 500 }),
        )]));
    let session_id = agent_write_harness::start_planner(&runtime).await;
    runtime.wait_any_status(session_id).await;
    let calls = tool_calls(&runtime, session_id).await;
    let (_, text, is_error) = calls
        .iter()
        .find(|(name, _, _)| name == "read_chapter")
        .expect("the read_chapter call");
    assert!(!is_error, "{text}");
    assert!(
        text.contains("继续阅读请使用 from_pid="),
        "the tool's own continuation hint: {text}"
    );
    assert!(
        !text.contains("内容较长，已截断"),
        "the global template is not used for a tool-owned cap"
    );
}
