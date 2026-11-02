//! Writer tools end to end (PLAN M4.2, spec/05 §4, spec/03 §8).
//!
//! Every test drives a real `chapter_write` session whose provider is the `ScriptedProvider`, so
//! the assertions read the persisted rows: the chapter content, `content_version`, the tool call
//! results and the session counters.

mod agent_write_harness;

use std::time::Duration;

use agent_write_harness::{
    call_ui, chapter_content, chapter_row, phase_of, runtime, start_planner, start_writer,
};
use nekowriter::db::json::{Granularity, Phase, SessionStatus, ToolCallStatus};
use nekowriter::events::ServerEvent;
use nekowriter_llm::mock::{ScriptStep, chunk_text};
use serde_json::json;
use tokio::sync::broadcast::error::TryRecvError;

/// Every event published since the last drain (the pattern of `api_events.rs`).
async fn drain(
    receiver: &mut tokio::sync::broadcast::Receiver<nekowriter::events::EventEnvelope>,
) -> Vec<ServerEvent> {
    let mut events = Vec::new();
    tokio::task::yield_now().await;
    loop {
        match receiver.try_recv() {
            Ok(envelope) => events.push(envelope.event),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Lagged(_)) => continue,
            Err(TryRecvError::Closed) => break,
        }
    }
    events
}

/// Two `finish_draft` steps: a short chapter's first completion is withheld (spec/03 §8, D-029).
fn finish_draft_twice() -> [ScriptStep; 2] {
    [
        ScriptStep::new().tool("finish_draft", json!({})),
        ScriptStep::new().tool("finish_draft", json!({})),
    ]
}

/// Appends the two `finish_draft` steps a short chapter needs: the first is withheld once
/// (spec/03 §8, D-029), the second completes the session.
fn with_finish(mut steps: Vec<ScriptStep>) -> Vec<ScriptStep> {
    steps.extend(finish_draft_twice());
    steps
}

fn insert_step(text: &str, extra: serde_json::Value) -> ScriptStep {
    let mut arguments = json!({ "text": text });
    if let Some(object) = extra.as_object() {
        for (key, value) in object {
            arguments[key] = value.clone();
        }
    }
    ScriptStep::new().tool("insert_text", arguments)
}

// ───────────────────────────── granularity matrix ─────────────────────────────

/// spec/03 §8 `sentence`: exactly one sentence, no line break, up to 400 characters.
#[tokio::test]
async fn sentence_granularity_accepts_one_sentence_and_rejects_a_break_and_a_long_one() {
    let runtime = runtime(with_finish(vec![
        insert_step("他抬起头，望向剑冢深处那一点幽蓝的光。", json!({})),
        // A line break is rejected under `sentence`.
        insert_step("第一句。\n第二句。", json!({})),
        // 401 characters is over the limit.
        insert_step(&"字".repeat(401), json!({})),
    ]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Sentence, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let inserts: Vec<_> = calls
        .iter()
        .filter(|call| call.name == "insert_text")
        .collect();
    assert_eq!(inserts.len(), 3);
    assert!(!inserts[0].is_error, "one sentence is accepted");
    assert!(inserts[1].is_error, "a line break is rejected");
    assert!(inserts[2].is_error, "a 401-character segment is rejected");
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(
        content.paragraphs.len(),
        1,
        "only the accepted sentence was written: {:?}",
        content.paragraphs
    );
    assert_eq!(
        content.paragraphs[0].text,
        "他抬起头，望向剑冢深处那一点幽蓝的光。"
    );
}

/// spec/03 §8 `paragraph`: exactly one paragraph, no line break, up to 3,000 characters.
#[tokio::test]
async fn paragraph_granularity_accepts_one_paragraph_and_rejects_a_break() {
    let runtime = runtime(with_finish(vec![
        insert_step("第一段。", json!({})),
        insert_step("一段。\n另一段。", json!({})),
    ]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let inserts: Vec<_> = calls
        .iter()
        .filter(|call| call.name == "insert_text")
        .collect();
    assert!(!inserts[0].is_error);
    assert!(
        inserts[1].is_error,
        "a line break is rejected under paragraph"
    );
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 1);
    assert!(
        !content.tail_open,
        "paragraph mode always closes the paragraph"
    );
}

/// spec/03 §8 `free`: any text, line breaks separate paragraphs, `paragraph_end` decides the tail.
#[tokio::test]
async fn free_granularity_writes_several_paragraphs_and_keeps_an_open_tail() {
    let runtime = runtime(with_finish(vec![
        insert_step("第一段。\n\n第二段开始", json!({ "paragraph_end": false })),
        // The next call continues the open tail (spec/03 §8 "open tail").
        insert_step("，接在末尾。", json!({ "paragraph_end": true })),
    ]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Free, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 2, "{:?}", content.paragraphs);
    assert_eq!(content.paragraphs[0].text, "第一段。");
    // Chinese is unspaced, so no space is inserted at the seam (spec/03 §8 joining).
    assert_eq!(content.paragraphs[1].text, "第二段开始，接在末尾。");
}

/// spec/03 §8 `chapter`: the rest of the chapter in one call; `chapter_end` is implied.
#[tokio::test]
async fn chapter_granularity_completes_in_one_call() {
    let runtime = runtime(with_finish(vec![insert_step(
        "第一段。\n\n第二段。\n\n第三段。",
        json!({}),
    )]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Chapter, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 3);
    assert!(!content.tail_open);
    // The completion handler ran: a draft version and `in_review` (spec/08 §1.1).
    let (status, _, _) = chapter_row(&runtime, runtime.chapter_id).await;
    assert_eq!(status, "in_review");
    assert_eq!(runtime.version_kinds().await, vec!["draft".to_owned()]);
}

/// spec/03 §8: under `sentence` and `paragraph`, `insert_paragraphs` and `rewrite_paragraph` also
/// take a single paragraph, so granularity cannot be bypassed.
#[tokio::test]
async fn paragraph_edits_are_limited_to_one_paragraph_under_sentence_and_paragraph() {
    let runtime = runtime(with_finish(vec![
        insert_step("第一段。", json!({})),
        // insert_paragraphs with two paragraphs is rejected under `paragraph`.
        ScriptStep::new().tool(
            "insert_paragraphs",
            json!({ "after": "p1", "text": "第二段。\n第三段。" }),
        ),
        // One paragraph is fine.
        ScriptStep::new().tool(
            "insert_paragraphs",
            json!({ "after": "p1", "text": "第二段。" }),
        ),
        // rewrite_paragraph with two paragraphs is rejected too.
        ScriptStep::new().tool(
            "rewrite_paragraph",
            json!({ "pid": "p2", "text": "甲。\n乙。" }),
        ),
        ScriptStep::new().tool(
            "rewrite_paragraph",
            json!({ "pid": "p2", "text": "重写后的第二段。" }),
        ),
    ]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let paragraph_calls: Vec<_> = calls
        .iter()
        .filter(|call| call.name == "insert_paragraphs" || call.name == "rewrite_paragraph")
        .collect();
    assert_eq!(paragraph_calls.len(), 4);
    assert!(paragraph_calls[0].is_error, "two paragraphs are rejected");
    assert!(!paragraph_calls[1].is_error);
    assert!(paragraph_calls[2].is_error, "rewrite is limited too");
    assert!(!paragraph_calls[3].is_error);
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 2);
    assert_eq!(content.paragraphs[1].text, "重写后的第二段。");
}

/// spec/03 §8: `free` and `chapter` let those tools write several paragraphs.
#[tokio::test]
async fn free_granularity_lets_paragraph_edits_write_several_paragraphs() {
    let runtime = runtime(with_finish(vec![
        insert_step("第一段。", json!({})),
        ScriptStep::new().tool(
            "insert_paragraphs",
            json!({ "after": "p1", "text": "甲。\n乙。" }),
        ),
    ]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Free, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let inserted = calls
        .iter()
        .find(|call| call.name == "insert_paragraphs")
        .expect("the call");
    assert!(
        !inserted.is_error,
        "{}",
        inserted.raw_result_text.clone().unwrap_or_default()
    );
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 3);
}

/// spec/03 §8 "which rule applies": the rule is the granularity of the assistant message's
/// `request_meta`, never the session's current value.
#[tokio::test]
async fn a_call_generated_under_paragraph_is_validated_against_paragraph_after_a_switch() {
    // The distinguishing cell: 500 characters is a valid `paragraph` (≤ 3,000) and an invalid
    // `sentence` (≤ 400). Anything shorter would pass under either mode and prove nothing.
    let in_flight = "字".repeat(500);
    // Step 2's arguments stream slowly, so the PATCH lands while the call is being generated but
    // before it executes. Its `request_meta` is already `paragraph`.
    let runtime = runtime(with_finish(vec![
        insert_step(&in_flight, json!({})).delay_per_event(Duration::from_millis(50)),
        ScriptStep::new().tool("insert_text", json!({ "text": "后续一句。" })),
    ]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    // Wait until the session is drafting (the outline was accepted).
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while phase_of(&runtime, session_id).await != Phase::Drafting {
        assert!(
            std::time::Instant::now() < deadline,
            "never reached drafting"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // Wait for the first model request to be issued, so the PATCH really lands while the call is
    // generated under `paragraph` (the session is already drafting at creation now).
    while runtime.scripted.requests().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the first request never started"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // Switch to `sentence` while the call is in flight.
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/sessions/{session_id}"),
            json!({ "granularity": "sentence" }),
        )
        .await;
    assert_eq!(status, 200, "patch granularity: {body}");

    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let inserts: Vec<_> = calls
        .iter()
        .filter(|call| call.name == "insert_text")
        .collect();
    assert_eq!(inserts.len(), 2);
    assert!(
        !inserts[0].is_error,
        "500 characters is valid under the `paragraph` the call was generated under, even though \
         the session is now `sentence` (≤ 400): {}",
        inserts[0].raw_result_text.clone().unwrap_or_default()
    );
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(
        content.paragraphs[0].text, in_flight,
        "the in-flight call's text was applied in full"
    );
    // The call generated *after* the switch does use the new mode: 500 characters is too long.
    let after = inserts[1].raw_result_text.clone().unwrap_or_default();
    assert!(
        !inserts[1].is_error,
        "the following call's short text is valid under `sentence` too: {after}"
    );
    // The switch notice is queued when the PATCH commits and delivered with the next sealing
    // (spec/03 §8 "switching"). Which batch seals first depends on where the runner was when the
    // PATCH committed — it may still be waiting for this call to execute, or already sealing the
    // following one — but it is always delivered exactly once, before the model makes another
    // drafting call.
    let sealed: Vec<String> = calls
        .iter()
        .map(|call| call.result_text.clone().unwrap_or_default())
        .collect();
    let carriers = sealed
        .iter()
        .filter(|text| text.contains("作者调整了写作粒度"))
        .count();
    assert_eq!(
        carriers, 1,
        "writer.notice.granularity is delivered exactly once: {sealed:?}"
    );
    let notice = sealed
        .iter()
        .find(|text| text.contains("作者调整了写作粒度"))
        .expect("the notice");
    assert!(
        notice.contains("当前写作粒度：逐句"),
        "the notice renders the NEW mode's rule: {notice}"
    );
}

// ───────────────────────────── each tool's errors ─────────────────────────────

/// `replace_text` errors: `text_not_found` (with the paragraph text), `text_ambiguous`.
#[tokio::test]
async fn replace_text_reports_a_missing_and_an_ambiguous_old_text() {
    let runtime = runtime(with_finish(vec![
        insert_step("剑光一闪，剑光又起，剑光不止。", json!({})),
        ScriptStep::new().tool(
            "replace_text",
            json!({ "pid": "p1", "old": "不存在", "new": "x" }),
        ),
        ScriptStep::new().tool(
            "replace_text",
            json!({ "pid": "p1", "old": "剑光", "new": "刀光" }),
        ),
        ScriptStep::new().tool(
            "replace_text",
            json!({ "pid": "p1", "old": "一闪", "new": "一暗" }),
        ),
    ]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let replaces: Vec<_> = calls
        .iter()
        .filter(|call| call.name == "replace_text")
        .collect();
    assert_eq!(replaces.len(), 3);
    assert!(replaces[0].is_error);
    let missing = replaces[0].raw_result_text.clone().unwrap_or_default();
    assert!(
        missing.contains("剑光一闪，剑光又起，剑光不止。"),
        "text_not_found shows the paragraph: {missing}"
    );
    assert!(replaces[1].is_error, "three occurrences are ambiguous");
    assert!(
        replaces[1]
            .raw_result_text
            .clone()
            .unwrap_or_default()
            .contains("3"),
        "text_ambiguous carries the count"
    );
    assert!(!replaces[2].is_error);
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs[0].text, "剑光一暗，剑光又起，剑光不止。");
}

/// `delete_paragraphs`: an unknown pid deletes nothing at all (spec/05 §4).
#[tokio::test]
async fn delete_paragraphs_with_an_unknown_pid_deletes_nothing() {
    let runtime = runtime(with_finish(vec![
        insert_step("第一段。", json!({})),
        insert_step("第二段。", json!({})),
        ScriptStep::new().tool("delete_paragraphs", json!({ "pids": ["p1", "p99"] })),
    ]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let delete = calls
        .iter()
        .find(|call| call.name == "delete_paragraphs")
        .expect("the call");
    assert!(delete.is_error);
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 2, "nothing was deleted");
}

/// `read_draft` returns the pids and the target, and names a page's successor.
#[tokio::test]
async fn read_draft_renders_paragraphs_with_pids_and_the_target() {
    let runtime = runtime(with_finish(vec![
        insert_step("第一段。", json!({})),
        insert_step("第二段。", json!({})),
        ScriptStep::new().tool("read_draft", json!({})),
    ]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let read = calls
        .iter()
        .find(|call| call.name == "read_draft")
        .expect("the call");
    assert!(!read.is_error);
    let text = read.raw_result_text.clone().unwrap_or_default();
    assert!(text.contains("[p1] 第一段。"), "{text}");
    assert!(text.contains("[p2] 第二段。"), "{text}");
    assert!(text.contains("目标"), "the target length is shown: {text}");
}

/// The `ui` payload of every writer call carries data only (spec/05 §1.6, D-054).
#[tokio::test]
async fn writer_tool_ui_payloads_carry_data_not_labels() {
    let runtime = runtime(vec![ScriptStep::new().tool(
        "submit_outline",
        json!({ "outline": "场景一", "reference_notes": "- 要点" }),
    )])
    .await;
    // `submit_outline` belongs to the outline session; its UI payload is pinned there.
    let session_id = start_planner(&runtime).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let ui = call_ui(&runtime, session_id, "submit_outline").await;
    assert!(
        ui.get("outline_chars").is_some(),
        "ui carries data only: {ui}"
    );
    assert!(ui.get("notes_chars").is_some(), "{ui}");
}

// ───────────────────────────── short completion (spec/03 §8) ─────────────────────────────

/// spec/03 §15 test 19: a `chapter_end` below 60% of the target keeps the text but withholds the
/// completion once; the following `finish_draft` completes.
#[tokio::test]
async fn test_19_a_short_chapter_end_is_withheld_once_and_the_text_is_kept() {
    let runtime = runtime(with_finish(vec![
        // 3,000 target words: 1,000 characters is far below 60%.
        insert_step("很短的一段。", json!({ "chapter_end": true })),
    ]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let insert = calls
        .iter()
        .find(|call| call.name == "insert_text")
        .expect("insert");
    let text = insert.raw_result_text.clone().unwrap_or_default();
    assert!(
        text.contains("字数明显少于目标下限"),
        "the result says the chapter is short: {text}"
    );
    // The text was kept, not rolled back.
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 1, "the withheld text stays");
    // The counter records the warning exactly once.
    let row = runtime.session_row(session_id).await;
    assert!(
        row.counters().finish_warned,
        "counters.finish_warned is set"
    );
    // The completion was withheld: `finish_draft` then completes the session.
    let (status, _, _) = chapter_row(&runtime, runtime.chapter_id).await;
    assert_eq!(status, "in_review");
    let finish = calls
        .iter()
        .find(|call| call.name == "finish_draft")
        .expect("finish");
    assert!(!finish.is_error);
    assert!(
        finish
            .raw_result_text
            .clone()
            .unwrap_or_default()
            .contains("本章初稿完成"),
        "the second attempt goes through: {:?}",
        finish.raw_result_text
    );
}

/// `chapter` granularity implies `chapter_end`, so a short one is withheld once as well.
#[tokio::test]
async fn a_short_chapter_mode_write_is_withheld_once() {
    let runtime = runtime(with_finish(vec![
        insert_step("只有一句。", json!({})),
        // The demo keeps writing; the second `chapter` write goes through.
        insert_step("补足一些内容。", json!({})),
    ]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Chapter, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let inserts: Vec<_> = calls
        .iter()
        .filter(|call| call.name == "insert_text")
        .collect();
    assert_eq!(inserts.len(), 2);
    let first = inserts[0].raw_result_text.clone().unwrap_or_default();
    assert!(first.contains("字数明显少于目标下限"), "{first}");
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 2, "both texts were kept");
    assert_eq!(runtime.version_kinds().await, vec!["draft".to_owned()]);
}

/// A completion above the threshold is never withheld (no warning, immediate completion).
#[tokio::test]
async fn a_long_enough_chapter_end_completes_at_once() {
    // The chapter needs ≥ 1,800 words: 2,000 short paragraphs would be slow, so a single long
    // paragraph of 2,000 characters is used (each character counts one word in Chinese).
    let long_text: String = "字".repeat(2_000);
    let runtime = runtime(with_finish(vec![insert_step(
        &long_text,
        json!({ "chapter_end": true }),
    )]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let row = runtime.session_row(session_id).await;
    assert!(!row.counters().finish_warned, "no warning was needed");
    let (status, _, _) = chapter_row(&runtime, runtime.chapter_id).await;
    assert_eq!(status, "in_review");
}

/// A paragraph that exists but holds no text is still an empty draft (R10-03).
#[tokio::test]
async fn finish_draft_on_an_empty_paragraph_is_an_error() {
    let runtime = runtime(with_finish(vec![
        ScriptStep::new().tool("finish_draft", json!({})),
        insert_step("现在有内容了。", json!({})),
    ]))
    .await;
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET content = '{{\"paragraphs\":[{{\"pid\":\"p1\",\"text\":\"\"}}],\"next_pid\":2,\"tail_open\":false}}' WHERE id = {}",
            runtime.chapter_id
        ))
        .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let finishes: Vec<_> = calls
        .iter()
        .filter(|call| call.name == "finish_draft")
        .collect();
    assert_eq!(finishes.len(), 3);
    assert!(finishes[0].is_error);
    assert!(
        finishes[0]
            .raw_result_text
            .clone()
            .unwrap_or_default()
            .contains("还没有任何正文")
    );
}

/// `finish_draft` on an empty chapter is `error.draft_empty`, and the session keeps working.
#[tokio::test]
async fn finish_draft_on_an_empty_chapter_is_an_error() {
    let runtime = runtime(with_finish(vec![
        ScriptStep::new().tool("finish_draft", json!({})),
        insert_step("现在有内容了。", json!({})),
    ]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let finishes: Vec<_> = calls
        .iter()
        .filter(|call| call.name == "finish_draft")
        .collect();
    assert_eq!(
        finishes.len(),
        3,
        "the empty attempt, the withheld one, and the one that completes"
    );
    assert!(finishes[0].is_error);
    assert!(
        finishes[0]
            .raw_result_text
            .clone()
            .unwrap_or_default()
            .contains("还没有任何正文")
    );
    assert!(
        finishes[1]
            .raw_result_text
            .clone()
            .unwrap_or_default()
            .contains("明显少于目标下限"),
        "the first non-empty attempt is withheld as short (spec/03 §8)"
    );
    assert!(!finishes[2].is_error);
    let (status, _, _) = chapter_row(&runtime, runtime.chapter_id).await;
    assert_eq!(status, "in_review");
}

/// The `chunk_text` helper is part of the mock provider; keep the import used so a future test can
/// stream a long argument without re-discovering it.
#[test]
fn chunking_helper_is_available() {
    assert_eq!(
        chunk_text("abcd", 2),
        vec!["ab".to_owned(), "cd".to_owned()]
    );
}

/// A held write is staged, not committed; approving it commits it exactly once (D-139).
///
/// This is the bug the fix removes: the tool used to write the chapter inside `execute`, so a call
/// that held for approval had already changed the chapter, and approve/reject changed nothing about
/// the text.
#[tokio::test]
async fn a_write_call_holds_under_auto_pause_and_its_content_is_committed_on_approval() {
    let runtime = runtime(with_finish(vec![insert_step(
        "自动暂停时写入的一段。",
        json!({}),
    )]))
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, true, false).await;
    let row = runtime.wait_status(session_id, SessionStatus::Held).await;
    let calls = runtime.calls(session_id).await;
    let insert = calls
        .iter()
        .find(|call| call.name == "insert_text")
        .expect("insert");
    assert_eq!(insert.status_enum(), ToolCallStatus::Held);
    assert_eq!(row.held_tool_call_id, Some(insert.id));
    assert!(!insert.sealed, "a held call is not sealed yet");
    assert_eq!(phase_of(&runtime, session_id).await, Phase::Drafting);

    // The chapter is untouched while the call is held (spec/03 §5.2, §5.4).
    let (_, _, version) = chapter_row(&runtime, runtime.chapter_id).await;
    let word_count = agent_write_harness::query_i64(
        &runtime,
        "SELECT word_count AS value FROM chapters WHERE id = ?",
        vec![runtime.chapter_id.into()],
    )
    .await;
    assert_eq!(word_count, 0, "a held write leaves the word count alone");
    let held_content = chapter_content(&runtime, runtime.chapter_id).await;
    assert!(
        held_content.paragraphs.is_empty(),
        "the held write is not in the chapter: {:?}",
        held_content.paragraphs
    );

    let (status, body) = agent_write_harness::resume_raw(&runtime, session_id, None).await;
    assert_eq!(status, 200, "approve the held write: {body}");
    agent_write_harness::resume_until_done(&runtime, session_id, None).await;

    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(
        content.paragraphs.len(),
        1,
        "approving applied the staged write exactly once: {:?}",
        content.paragraphs
    );
    assert_eq!(content.paragraphs[0].text, "自动暂停时写入的一段。");
    assert_eq!(content.paragraphs[0].pid, "p1", "the staged pid was kept");
    let (_, _, after_version) = chapter_row(&runtime, runtime.chapter_id).await;
    let after_word_count = agent_write_harness::query_i64(
        &runtime,
        "SELECT word_count AS value FROM chapters WHERE id = ?",
        vec![runtime.chapter_id.into()],
    )
    .await;
    assert!(
        after_word_count > word_count,
        "the applied content's word count is stored: {after_word_count}"
    );
    assert_eq!(
        after_version,
        version + 1,
        "one approval moves content_version exactly once"
    );
    assert_ne!(after_word_count, word_count);
    // The call is finalized and its effects are spent, so nothing is left to apply again.
    let insert = runtime.call_by_id(insert.id).await;
    assert_eq!(insert.status_enum(), ToolCallStatus::Done);
    assert_eq!(
        insert.effects().pending_content,
        None,
        "the applied write is cleared from the call's effects"
    );
}

/// spec/08 §1.1: the `draft` version's note is the `finish_draft` note.
///
/// The note is what the author reads next to a version in the history list, and it is the only
/// place the writer's own summary of the chapter survives: the call's result is
/// `result.writer_control`, which does not repeat it (spec/05 §4).
#[tokio::test]
async fn finish_draft_stores_its_note_on_the_draft_version() {
    let runtime = runtime(vec![
        insert_step("第一段。", json!({})),
        ScriptStep::new().tool(
            "finish_draft",
            json!({ "note": "本章写了城门口的一场对峙。" }),
        ),
        ScriptStep::new().tool(
            "finish_draft",
            json!({ "note": "本章写了城门口的一场对峙。" }),
        ),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let notes = draft_version_notes(&runtime, runtime.chapter_id).await;
    assert_eq!(
        notes,
        vec!["本章写了城门口的一场对峙。".to_owned()],
        "the draft version carries the finish_draft note"
    );
}

/// The `note` of every `draft` version of a chapter, oldest first.
async fn draft_version_notes(
    runtime: &agent_write_harness::Runtime,
    chapter_id: i32,
) -> Vec<String> {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    runtime
        .app
        .db()
        .conn()
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT note FROM chapter_versions WHERE chapter_id = ? AND kind = 'draft' ORDER BY id",
            vec![chapter_id.into()],
        ))
        .await
        .expect("query versions")
        .into_iter()
        .map(|row| row.try_get::<String>("", "note").expect("note"))
        .collect()
}

// ─────────────────── D-139: validation still fails without writing ───────────────────

/// A granularity rejection happens in `execute` and must leave the chapter byte-identical: the
/// validation stayed where it was, only the write moved (spec/03 §5.2, D-139).
#[tokio::test]
async fn a_granularity_rejection_leaves_the_chapter_byte_identical() {
    let runtime = runtime(with_finish(vec![
        insert_step("第一段。", json!({})),
        // Rejected: a line break under `paragraph` granularity.
        insert_step("一段。\n另一段。", json!({})),
    ]))
    .await;
    // `auto_pause` off: the accepted write is applied by its own call, so the rejected one has a
    // non-empty chapter to leave untouched.
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let (_, _, version) = chapter_row(&runtime, runtime.chapter_id).await;
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(
        content.paragraphs.len(),
        1,
        "only the accepted write is there"
    );
    let calls = runtime.calls(session_id).await;
    let rejected = calls
        .iter()
        .filter(|call| call.name == "insert_text")
        .find(|call| call.is_error)
        .expect("the rejected insert");
    assert!(
        rejected.effects().pending_content.is_none(),
        "a failed call stages nothing, so a later approval cannot apply its text"
    );
    assert_eq!(version, 1, "one accepted write, one version");
}

/// The non-held path is unchanged: with `auto_pause` off a write applies where it always did, with
/// one `chapter.content` event and one version bump (spec/03 §5.2, D-139).
#[tokio::test]
async fn a_non_held_write_applies_in_its_own_call_with_one_event() {
    let runtime = runtime(with_finish(vec![
        insert_step("第一段。", json!({})),
        insert_step("第二段。", json!({})),
    ]))
    .await;
    let mut receiver = runtime.app.events().subscribe();
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 2, "both writes applied");
    let (_, _, version) = chapter_row(&runtime, runtime.chapter_id).await;
    assert_eq!(version, 2, "one version bump per applied write");

    // Two writes, two version bumps: one `chapter.content` per applied write, in order. The
    // completion handler's final `chapter.content` re-states the settled version with no ops, so
    // only the events that carry ops are the writes themselves.
    let events = drain(&mut receiver).await;
    let versions: Vec<i32> = events
        .iter()
        .filter_map(|event| match event {
            ServerEvent::ChapterContentChanged {
                version,
                ops,
                actor,
                ..
            } if *actor == nekowriter::events::ContentActor::Agent && !ops.is_empty() => {
                Some(*version)
            }
            _ => None,
        })
        .collect();
    assert_eq!(versions, vec![1, 2], "one event per write, in order");

    // No call kept a staged write: every one applied in its own transaction.
    for call in runtime.calls(session_id).await {
        assert_eq!(
            call.effects().pending_content,
            None,
            "a finalized call holds no pending content: {}",
            call.name
        );
    }
}

/// The shared M4 harness starts the shipped writer path: `drafting`, the 19-tool list, and no
/// `submit_outline`.
#[tokio::test]
async fn the_harness_uses_the_shipped_drafting_writer_path() {
    let runtime = runtime(vec![]).await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    let (status, snapshot) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{snapshot}");
    assert_eq!(snapshot["session"]["phase"], "drafting");
    assert_eq!(snapshot["session"]["kind"], "chapter_write");
    let names: Vec<&str> = snapshot["tool_names"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|name| name.as_str().unwrap_or_default())
        .collect();
    assert_eq!(names.len(), 19, "{names:?}");
    assert!(!names.contains(&"submit_outline"), "{names:?}");
    assert!(names.contains(&"insert_text"), "{names:?}");
}
