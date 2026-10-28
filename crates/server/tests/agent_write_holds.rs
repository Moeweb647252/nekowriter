//! Holds, author participation and edit reports (PLAN M4.3, spec/03 §5.2–§5.4, §6–§8).
//!
//! Runtime tests 1, 2, 3, 6, 9, 10 and 18 of spec/03 §15, driven through the real router with the
//! `ScriptedProvider` and a temporary database.

mod agent_write_harness;

use agent_write_harness::{
    chapter_column, chapter_content, chapter_row, outline_step, phase_of, runtime,
    start_legacy_writer, start_writer,
};
use axum::http::StatusCode;
use nekowriter::db::json::{Granularity, Phase, SessionStatus, ToolCallStatus};
use nekowriter::events::ServerEvent;
use nekowriter_llm::mock::{ScriptStep, chunk_text};
use serde_json::json;
use tokio::sync::broadcast::error::TryRecvError;

/// Two `finish_draft` steps, so a short chapter's withdrawal is followed by a real completion.
fn finish_pair() -> [ScriptStep; 2] {
    [
        ScriptStep::new().tool("finish_draft", json!({})),
        ScriptStep::new().tool("finish_draft", json!({})),
    ]
}

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

fn insert_step(text: &str) -> ScriptStep {
    ScriptStep::new().tool("insert_text", json!({ "text": text }))
}

/// `POST /api/sessions/:id/resume` with optional feedback.
async fn resume(
    runtime: &agent_write_harness::Runtime,
    session_id: i32,
    feedback: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let body = match feedback {
        Some(text) => json!({ "feedback": text }),
        None => json!({}),
    };
    runtime
        .app
        .post(&format!("/api/sessions/{session_id}/resume"), body)
        .await
}

/// The held call of a session (spec/03 §5.2 `held_tool_call_id`).
async fn held_call(
    runtime: &agent_write_harness::Runtime,
    session_id: i32,
) -> (
    agent_write_harness::CallRow,
    agent_write_harness::SessionRow,
) {
    let row = runtime.wait_status(session_id, SessionStatus::Held).await;
    let call_id = row.held_tool_call_id.expect("a held call id");
    (runtime.call_by_id(call_id).await, row)
}

// ───────────────────────────── test 1 ─────────────────────────────

/// spec/03 §15 test 1: plan → `submit_outline` → (pause_after_outline) held → approve → the
/// drafting notice contains the granularity rule → `insert_text` ×3 → `chapter_end` → the chapter
/// is `in_review` with a draft version.
#[tokio::test]
async fn test_01_plan_outline_hold_approve_draft_and_complete() {
    let runtime = runtime(vec![
        outline_step(),
        insert_step("第一句。"),
        insert_step("第二句。"),
        insert_step("第三句。"),
        ScriptStep::new().tool(
            "insert_text",
            json!({ "text": "收尾。", "chapter_end": true }),
        ),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_legacy_writer(&runtime, Granularity::Paragraph, false, true).await;

    // The outline held because `pause_after_outline` is on (spec/03 §5.2).
    let (outline, row) = held_call(&runtime, session_id).await;
    assert_eq!(outline.name, "submit_outline");
    assert_eq!(row.phase, Phase::Planning.as_str());
    assert_eq!(row.held_tool_call_id, Some(outline.id));
    // Planning ran no insertion yet.
    assert_eq!(
        chapter_content(&runtime, runtime.chapter_id)
            .await
            .paragraphs
            .len(),
        0
    );

    let (status, body) = resume(&runtime, session_id, None).await;
    assert_eq!(status, StatusCode::OK, "approve the outline: {body}");
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    // The drafting notice rode with the approved outline and carried the granularity rules.
    let outline = runtime.call_by_id(outline.id).await;
    let notice = outline.result_text.clone().unwrap_or_default();
    assert!(notice.contains("章纲已确认"), "{notice}");
    assert!(notice.contains("当前写作粒度：逐段"), "{notice}");

    // Three inserts and the completing one wrote the text; the completion produced the version.
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 4, "{:?}", content.paragraphs);
    assert!(!content.tail_open);
    let (status, _, _) = chapter_row(&runtime, runtime.chapter_id).await;
    assert_eq!(status, "in_review");
    assert_eq!(runtime.version_kinds().await, vec!["draft".to_owned()]);
    assert_eq!(phase_of(&runtime, session_id).await, Phase::Drafting);
}

// ───────────────────────────── test 2 ─────────────────────────────

/// spec/03 §15 test 2: a held outline + feedback → the phase stays `planning`, the result holds
/// the rejection notice and the feedback, and a resubmission works.
#[tokio::test]
async fn test_02_feedback_on_a_held_outline_keeps_planning_and_allows_resubmission() {
    let runtime = runtime(vec![
        outline_step(),
        ScriptStep::new().tool(
            "submit_outline",
            json!({ "outline": "改过的章纲：场景一、场景二", "reference_notes": "- 改了" }),
        ),
        insert_step("正文。"),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_legacy_writer(&runtime, Granularity::Paragraph, false, true).await;
    let (first_outline, _row) = held_call(&runtime, session_id).await;

    let (status, body) = resume(&runtime, session_id, Some("章纲再细一点，加一场雨中的戏。")).await;
    assert_eq!(status, StatusCode::OK, "feedback: {body}");
    // The rejection is immediate: the phase never left planning.
    assert_eq!(phase_of(&runtime, session_id).await, Phase::Planning);
    // The resubmitted outline holds again (`pause_after_outline`), so approve it too.
    agent_write_harness::resume_until_done(&runtime, session_id, None).await;

    let first_outline = runtime.call_by_id(first_outline.id).await;
    let text = first_outline.result_text.clone().unwrap_or_default();
    assert!(
        text.contains("作者没有通过这份章纲"),
        "the rejection notice: {text}"
    );
    assert!(
        text.contains("加一场雨中的戏"),
        "the author's feedback follows the notice: {text}"
    );
    // The canonical appendix order: effect notice, then author feedback (spec/03 §5.3).
    let notice_at = text.find("作者没有通过这份章纲").expect("notice");
    let feedback_at = text.find("加一场雨中的戏").expect("feedback");
    assert!(
        notice_at < feedback_at,
        "the revoke notice comes first: {text}"
    );

    // The resubmission was accepted and the chapter now carries the new outline.
    assert_eq!(
        chapter_column(&runtime, runtime.chapter_id, "outline").await,
        "改过的章纲：场景一、场景二"
    );
    assert_eq!(
        runtime.version_kinds().await,
        vec!["draft".to_owned()],
        "the chapter was completed afterwards"
    );
}

// ───────────────────────────── test 3 ─────────────────────────────

/// spec/03 §15 test 3: with `auto_pause` every insert holds; approving continues; feedback is
/// appended to that call's result only.
#[tokio::test]
async fn test_03_auto_pause_holds_each_insert_and_feedback_sticks_to_its_call() {
    let runtime = runtime(vec![
        insert_step("第一段。"),
        insert_step("第二段。"),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, true, false).await;

    // First insert: approve with no feedback.
    let (first, _) = held_call(&runtime, session_id).await;
    assert_eq!(first.name, "insert_text");
    assert!(
        chapter_content(&runtime, runtime.chapter_id)
            .await
            .paragraphs
            .is_empty(),
        "a held write is not in the chapter (D-139)"
    );
    let (status, body) = resume(&runtime, session_id, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        chapter_content(&runtime, runtime.chapter_id)
            .await
            .paragraphs
            .len(),
        1,
        "approving the held call applied its write"
    );

    // Second insert: approve WITH feedback; only this call's result carries it.
    let (second, _) = held_call(&runtime, session_id).await;
    assert_ne!(second.id, first.id);
    assert_eq!(
        chapter_content(&runtime, runtime.chapter_id)
            .await
            .paragraphs
            .len(),
        1,
        "the second held write is not applied yet"
    );
    let (status, body) = resume(&runtime, session_id, Some("第二段写得太急。")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The remaining insert holds again; approving it finishes the script.
    agent_write_harness::resume_until_done(&runtime, session_id, None).await;

    let first = runtime.call_by_id(first.id).await;
    let second = runtime.call_by_id(second.id).await;
    let first_text = first.result_text.clone().unwrap_or_default();
    let second_text = second.result_text.clone().unwrap_or_default();
    assert!(
        !first_text.contains("第二段写得太急"),
        "feedback does not leak into an earlier call: {first_text}"
    );
    assert!(
        second_text.contains("第二段写得太急"),
        "the feedback rides with its own call: {second_text}"
    );
    assert!(
        second_text.starts_with("已写入"),
        "the raw result comes first (spec/03 §5.3): {second_text}"
    );
}

// ───────────────────────────── test 6 ─────────────────────────────

/// spec/03 §15 test 6: the author edits the text while held (modify, delete, insert) → the exact
/// edit report; a large edit → the compact report.
#[tokio::test]
async fn test_06_author_edits_while_held_are_reported_exactly() {
    let runtime = runtime(vec![
        insert_step("第一段原文。"),
        insert_step("第二段原文。"),
        insert_step("第三段原文。"),
        insert_step("第四段。"),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    // The author already wrote a paragraph, so there is text to edit while the agent's write is
    // held: a held write is staged and not in the chapter (D-139), so the only paragraphs the
    // author can touch are the ones that were there before the session started.
    runtime
        .app
        .put_text(runtime.chapter_id, 0, "作者原先写的第一段。")
        .await;
    // Pause after the first insert so the author can edit the text the agent already saw.
    let session_id = start_writer(&runtime, Granularity::Paragraph, true, false).await;
    let (first, _) = held_call(&runtime, session_id).await;
    assert_eq!(first.name, "insert_text");
    let version = chapter_row(&runtime, runtime.chapter_id).await.2;

    // Modify p1, delete nothing yet, insert a paragraph at the start: three change kinds.
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/chapters/{}/content", runtime.chapter_id),
            json!({
                "base_version": version,
                "ops": [
                    { "op": "update", "pid": "p1", "text": "第一段被作者改过。" },
                    { "op": "insert", "after": null, "text": "作者在最前面加的一段。" },
                ],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "author edit while held: {body}");

    agent_write_harness::resume_until_done(&runtime, session_id, None).await;

    // The next sealed call carries the edit report (spec/03 §7).
    let calls = runtime.calls(session_id).await;
    let mut report = None;
    for call in &calls {
        if let Some(text) = &call.result_text
            && text.contains("作者在暂停期间直接修改了本章")
        {
            report = Some(text.clone());
            break;
        }
    }
    let report = report.expect("an edit report was delivered");
    assert!(
        report.contains("修改 p1"),
        "the modified paragraph: {report}"
    );
    assert!(
        report.contains("作者原先写的第一段。"),
        "the old text is quoted: {report}"
    );
    assert!(
        report.contains("第一段被作者改过。"),
        "the new text is shown in full: {report}"
    );
    assert!(
        report.contains("在开头新增"),
        "an insertion at the start has no predecessor: {report}"
    );
    assert!(
        report.contains("作者在最前面加的一段。"),
        "the inserted text is shown: {report}"
    );
}

/// spec/03 §7: deletions are reported with their old text, and the 120-character truncation holds.
#[tokio::test]
async fn the_edit_report_reports_deletions_and_truncates_old_text() {
    let long = "旧".repeat(200);
    let runtime = runtime(vec![
        insert_step(&long),
        insert_step("第二段。"),
        insert_step("第三段。"),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    // The long paragraph pre-exists the session, so it is `p1` and the author can delete it while
    // the agent's own write is held (D-139: a held write is not in the chapter).
    runtime.app.put_text(runtime.chapter_id, 0, &long).await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, true, false).await;
    let _first = held_call(&runtime, session_id).await;
    let version = chapter_row(&runtime, runtime.chapter_id).await.2;
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/chapters/{}/content", runtime.chapter_id),
            json!({
                "base_version": version,
                "ops": [
                    { "op": "delete", "pid": "p1" },
                    { "op": "insert", "after": null, "text": "改过的第二段。" },
                ],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    agent_write_harness::resume_until_done(&runtime, session_id, None).await;

    let report = runtime
        .calls(session_id)
        .await
        .into_iter()
        .filter_map(|call| call.result_text.clone())
        .find(|text| text.contains("作者在暂停期间直接修改了本章"))
        .expect("an edit report");
    assert!(report.contains("删除 p1"), "the deletion: {report}");
    // The old text of p1 was 200 characters; the report quotes 120 plus an ellipsis.
    let old_quotes = report.matches('旧').count();
    assert!(
        old_quotes <= 121,
        "the old text is truncated to 120 characters (+ ellipsis), got {old_quotes}:\n{report}"
    );
    assert!(report.contains("…"), "the truncation is marked: {report}");
}

/// spec/03 §7: above 4,000 characters of paragraph report the compact variant replaces it with
/// counts and the instruction to call `read_draft`.
#[tokio::test]
async fn a_large_edit_while_held_produces_the_compact_report() {
    let runtime = runtime(vec![
        insert_step("写手新增的一段。"),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    // The author already wrote twenty long paragraphs before starting the session, so the agent's
    // view snapshot holds them and a rewrite of all of them is a large edit.
    let existing: String = (1..=20)
        .map(|index| format!("原文第{index}段：{}", "旧".repeat(250)))
        .collect::<Vec<_>>()
        .join("\n\n");
    runtime.app.put_text(runtime.chapter_id, 0, &existing).await;

    let session_id = start_writer(&runtime, Granularity::Paragraph, true, false).await;
    // The outline holds first (`auto_pause` covers write tools only, `pause_after_outline` is off,
    // so this hold is the outline's own phase change under auto_pause off...). Either way, keep
    // approving until the chapter already holds an insert.
    let mut guarded = 0;
    loop {
        let row = runtime.session_row(session_id).await;
        if row.status == "held" {
            let call = runtime
                .call_by_id(row.held_tool_call_id.expect("held id"))
                .await;
            if call.name == "insert_text" {
                break;
            }
            let (status, body) = agent_write_harness::resume_raw(&runtime, session_id, None).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        } else if row.status == "running" {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        } else {
            panic!("the session finished before an insert held: {}", row.status);
        }
        guarded += 1;
        assert!(guarded < 100, "never reached a held insert");
    }

    // Rewrite every paragraph with a long text: the paragraph part exceeds 4,000 characters.
    let version = chapter_row(&runtime, runtime.chapter_id).await.2;
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    let mut ops = Vec::new();
    for paragraph in &content.paragraphs {
        ops.push(json!({
            "op": "update",
            "pid": paragraph.pid,
            "text": "改".repeat(300),
        }));
    }
    let rewritten = ops.len();
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/chapters/{}/content", runtime.chapter_id),
            json!({ "base_version": version, "ops": ops }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    agent_write_harness::resume_until_done(&runtime, session_id, None).await;
    let report = runtime
        .calls(session_id)
        .await
        .into_iter()
        .filter_map(|call| call.result_text.clone())
        .find(|text| text.contains("作者在暂停期间直接修改了本章"))
        .expect("an edit report");
    assert!(
        report.contains("改动较多，请先调用 read_draft"),
        "the compact variant tells the agent to call read_draft: {report}"
    );
    assert!(
        report.contains(&format!("修改 {rewritten} 段")),
        "the compact variant carries the counts: {report}"
    );
    assert!(
        !report.contains("改改改改改改改改"),
        "the compact variant does not print the paragraphs: {report}"
    );
}

// ───────────────────────────── test 9 ─────────────────────────────

/// spec/03 §15 test 9: a retrieval tool during drafting is an error result with no effect.
#[tokio::test]
async fn test_09_a_retrieval_tool_during_drafting_is_rejected() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("get_entity", json!({ "entity": "林凡" })),
        insert_step("正文。"),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let calls = runtime.calls(session_id).await;
    let retrieval = calls
        .iter()
        .find(|call| call.name == "get_entity")
        .expect("the call");
    assert!(retrieval.is_error, "retrieval is not permitted in drafting");
    let text = retrieval.raw_result_text.clone().unwrap_or_default();
    assert!(
        text.contains("正文阶段"),
        "error.tool_not_allowed names the phase: {text}"
    );
    assert!(
        text.contains("insert_text"),
        "it lists the permitted tools: {text}"
    );
    // No effect: the chapter has only the drafted paragraph.
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 1);
    assert_eq!(content.paragraphs[0].text, "正文。");
}

// ───────────────────────────── test 10 ─────────────────────────────

/// spec/03 §15 test 10: a granularity switch while the model generates → the call validates
/// against the old mode, and the next result carries the switch notice.
#[tokio::test]
async fn test_10_a_switch_does_not_invalidate_an_in_flight_call() {
    // The drafting call streams slowly, so the PATCH lands while it is generated.
    let runtime = runtime(vec![
        insert_step("这一段按逐段粒度写入，是一个完整的段落。")
            .delay_per_event(std::time::Duration::from_millis(40)),
        ScriptStep::new()
            .tool("insert_text", json!({ "text": "后续一句。" }))
            .delay_per_event(std::time::Duration::from_millis(40)),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while phase_of(&runtime, session_id).await != Phase::Drafting {
        assert!(
            std::time::Instant::now() < deadline,
            "never reached drafting"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    // Wait until the first drafting call has sealed, so the patch lands while the second call
    // streams. A write session is already `Drafting` at creation (D-146), so the phase alone is
    // no longer a proxy for "a call is in flight".
    while runtime.calls(session_id).await.is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the first call never sealed"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/sessions/{session_id}"),
            json!({ "granularity": "sentence" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
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
        "the in-flight call still validates as one paragraph: {}",
        inserts[0].raw_result_text.clone().unwrap_or_default()
    );
    let notices = calls
        .iter()
        .filter_map(|call| call.result_text.clone())
        .filter(|text| text.contains("作者调整了写作粒度"))
        .count();
    assert_eq!(notices, 1, "the switch notice is delivered exactly once");
}

// ───────────────────────────── test 18 ─────────────────────────────

/// spec/03 §15 test 18: the author edits the outline while `submit_outline` is held, then
/// approves → the result carries the outline-accepted notice and an edit report with the new
/// outline.
#[tokio::test]
async fn test_18_editing_the_outline_while_held_reports_it_with_the_acceptance() {
    let runtime = runtime(vec![
        outline_step(),
        insert_step("正文。"),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_legacy_writer(&runtime, Granularity::Paragraph, false, true).await;
    let (outline, _) = held_call(&runtime, session_id).await;
    assert_eq!(outline.name, "submit_outline");

    // The author edits the outline and the notes while the call is held. That is allowed: the
    // chapter is not locked by a *running* writer (spec/03 §14).
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/chapters/{}", runtime.chapter_id),
            json!({
                "outline": "作者补过的章纲：第一场在雨里。",
                "reference_notes": "- 作者的要点",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "author outline edit: {body}");

    agent_write_harness::resume_until_done(&runtime, session_id, None).await;

    let outline = runtime.call_by_id(outline.id).await;
    let text = outline.result_text.clone().unwrap_or_default();
    assert!(
        text.contains("章纲已确认"),
        "the outline was accepted: {text}"
    );
    assert!(
        text.contains("作者补过的章纲：第一场在雨里。"),
        "the report carries the author's new outline: {text}"
    );
    assert!(
        text.contains("- 作者的要点"),
        "the report carries the new reference notes: {text}"
    );
    // Canonical order: effect notice first, then the edit report (spec/03 §5.3).
    let notice_at = text.find("章纲已确认").expect("notice");
    let report_at = text.find("作者在暂停期间直接修改了本章").expect("report");
    assert!(notice_at < report_at, "notice before edit report: {text}");
}

// ───────────────────────────── feedback revokes deferred effects ─────────────────────────────

/// spec/03 §5.4: feedback on a held call with a `complete` effect revokes it, appends the revoke
/// notice before the feedback, and the session keeps working.
#[tokio::test]
async fn feedback_revokes_a_held_completion_and_keeps_working() {
    let long_text: String = "字".repeat(2_000);
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "insert_text",
            json!({ "text": long_text, "chapter_end": true }),
        ),
        insert_step("作者要求补充的一段。"),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, true, false).await;
    let (completing, row) = held_call(&runtime, session_id).await;
    assert_eq!(completing.name, "insert_text");
    // The deferred completion is stored on the held call, not applied (spec/03 §5.2).
    assert_eq!(
        completing.effects().complete,
        Some(nekowriter::db::json::CompletionKind::Draft)
    );
    assert_eq!(row.phase, Phase::Drafting.as_str());
    assert_eq!(
        chapter_row(&runtime, runtime.chapter_id).await.0,
        "drafting"
    );

    // D-139: the held write is not in the chapter, so rejecting it leaves the chapter untouched.
    let (version_before, word_count_before, content_before) =
        chapter_row(&runtime, runtime.chapter_id).await;
    assert!(
        chapter_content(&runtime, runtime.chapter_id)
            .await
            .paragraphs
            .is_empty(),
        "the held write was not applied"
    );

    let (status, body) = resume(&runtime, session_id, Some("结尾再收一点。")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    agent_write_harness::resume_until_done(&runtime, session_id, None).await;

    // The rejected write never reached the chapter: the follow-up paragraph is p1, and nothing of
    // the 2,000-character rejected text survives.
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(
        content.paragraphs.len(),
        1,
        "only the follow-up write was applied: {:?}",
        content.paragraphs
    );
    assert_eq!(
        content.paragraphs[0].pid, "p1",
        "no pid was consumed by the rejected write"
    );
    assert_eq!(content.paragraphs[0].text, "作者要求补充的一段。");
    assert!(
        !content
            .paragraphs
            .iter()
            .any(|paragraph| paragraph.text.contains('字')),
        "the rejected text is not in the chapter: {:?}",
        content.paragraphs
    );
    let _ = (version_before, word_count_before, content_before);

    let completing = runtime.call_by_id(completing.id).await;
    let text = completing.result_text.clone().unwrap_or_default();
    assert!(
        text.contains("这次“完成”没有生效"),
        "the revoke notice: {text}"
    );
    assert!(text.contains("结尾再收一点"), "the feedback: {text}");
    let notice_at = text.find("这次“完成”没有生效").expect("notice");
    let feedback_at = text.find("结尾再收一点").expect("feedback");
    assert!(
        notice_at < feedback_at,
        "the revoke notice comes first: {text}"
    );
    // The completion was revoked, so the chapter kept drafting and finished later.
    assert_eq!(runtime.version_kinds().await, vec!["draft".to_owned()]);
}

/// spec/03 §5.4: feedback on a held `submit_outline` revokes `phase_to` and keeps the session in
/// planning.
#[tokio::test]
async fn feedback_revokes_a_held_phase_change() {
    let runtime = runtime(vec![
        outline_step(),
        ScriptStep::new().tool(
            "submit_outline",
            json!({ "outline": "第二版章纲", "reference_notes": "" }),
        ),
        insert_step("正文。"),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_legacy_writer(&runtime, Granularity::Paragraph, false, true).await;
    let (first, _) = held_call(&runtime, session_id).await;
    assert_eq!(
        first.effects().phase_to,
        Some(Phase::Drafting),
        "the held outline carries the deferred phase change"
    );

    let (status, body) = resume(&runtime, session_id, Some("先把第二场写细。")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The phase never advanced: the feedback revoked it.
    assert_eq!(phase_of(&runtime, session_id).await, Phase::Planning);
    agent_write_harness::resume_until_done(&runtime, session_id, None).await;
    let first = runtime.call_by_id(first.id).await;
    let text = first.result_text.clone().unwrap_or_default();
    assert!(text.contains("作者没有通过这份章纲"), "{text}");
    assert!(text.contains("先把第二场写细"), "{text}");
    // The resubmission moved the session to drafting; the first call stayed revoked.
    let second_outline = runtime
        .calls(session_id)
        .await
        .into_iter()
        .find(|call| call.name == "submit_outline" && call.id != first.id)
        .expect("the resubmission");
    assert!(!second_outline.is_error);
}

/// D-009: a held session has no live task, so it survives a restart and Continue resumes it.
#[tokio::test]
async fn a_held_session_survives_a_restart_and_continue_resumes_it() {
    let runtime = runtime(vec![
        insert_step("暂停前写入的一段。"),
        insert_step("恢复后写入的一段。"),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, true, false).await;
    let (held, _) = held_call(&runtime, session_id).await;
    assert_eq!(held.name, "insert_text");

    // Simulate a restart: startup recovery runs over the database.
    nekowriter::agent::manager::recover(runtime.app.state())
        .await
        .expect("recover");
    let row = runtime.session_row(session_id).await;
    assert_eq!(
        row.status, "held",
        "a held session has no live task, so recovery leaves it held (D-009)"
    );
    assert_eq!(row.held_tool_call_id, Some(held.id));

    // Continue is `resume` for a held session; the runner respawns and finishes the work.
    agent_write_harness::resume_until_done(&runtime, session_id, None).await;
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 2);
}

/// The granularity notice names the open tail, because the new mode's first segment continues it
/// (spec/03 §8 "open tail").
#[tokio::test]
async fn the_granularity_notice_names_an_open_tail() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "insert_text",
            json!({ "text": "这一段还没有写完", "paragraph_end": false }),
        ),
        ScriptStep::new().tool(
            "insert_text",
            json!({ "text": "，现在写完了。", "paragraph_end": true }),
        ),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Free, false, false).await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    // Wait until the open-tail insert landed.
    loop {
        let content = chapter_content(&runtime, runtime.chapter_id).await;
        if content.tail_open {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the tail never opened"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/sessions/{session_id}"),
            json!({ "granularity": "paragraph" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let notice = runtime
        .calls(session_id)
        .await
        .into_iter()
        .filter_map(|call| call.result_text.clone())
        .find(|text| text.contains("作者调整了写作粒度"))
        .expect("the notice");
    assert!(
        notice.contains("尚未写完"),
        "the notice says the tail is open: {notice}"
    );
    assert!(
        notice.contains("当前写作粒度：逐段"),
        "and renders the new mode's rule: {notice}"
    );
}

/// A pause request during streaming holds the call that was already generated (spec/03 §5.2).
///
/// The pause is requested while the *first* insert streams, so the call is generated, executed and
/// then held; if the request lands at a boundary instead the session pauses, which is the other
/// legal outcome of spec/03 §5.1 and is asserted as such.
#[tokio::test]
async fn a_pause_request_during_a_writer_stream_holds_the_call() {
    let runtime = runtime(vec![
        insert_step("流式写入的一段。").delay_per_event(std::time::Duration::from_millis(30)),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    // Wait until the drafting insert is being streamed, then ask for the pause.
    loop {
        let calls = runtime.calls(session_id).await;
        if calls.iter().any(|call| call.name == "insert_text") {
            break;
        }
        let row = runtime.session_row(session_id).await;
        if row.status != "running" {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the insert call never appeared"
        );
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/pause"), json!({}))
        .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::CONFLICT,
        "pause during streaming: {status} {body}"
    );

    let row = runtime.wait_any_status(session_id).await;
    let calls = runtime.calls(session_id).await;
    let insert = calls
        .iter()
        .find(|call| call.name == "insert_text")
        .expect("the insert call");
    match row.status.as_str() {
        "held" => {
            assert_eq!(row.pause_requested, 0, "the hold consumes the pause");
            assert_eq!(insert.status_enum(), ToolCallStatus::Held);
            assert!(insert.raw_result_text.is_some(), "the call executed");
            assert_eq!(row.held_tool_call_id, Some(insert.id));
        }
        "paused" => {
            // The pause landed at the boundary after the call finished (spec/03 §5.1): the call is
            // already `done` and the next model request never went out. Either way nothing is
            // lost.
            assert!(
                matches!(
                    insert.status_enum(),
                    ToolCallStatus::Pending | ToolCallStatus::Executing | ToolCallStatus::Done
                ),
                "got {:?}",
                insert.status_enum()
            );
            // The transition itself consumes the request (spec/03 §3: leaving `running` clears
            // `pause_requested`), whichever way the pause was taken.
            assert_eq!(row.pause_requested, 0, "the pause was consumed");
        }
        other => panic!("unexpected status after a pause request: {other}"),
    }
}

/// Keep `chunk_text` reachable from this file (the mock provider splits arguments with it, and a
/// future test that streams a huge outline will want it).
#[test]
fn chunk_text_is_available() {
    assert_eq!(chunk_text("ab", 1), vec!["a".to_owned(), "b".to_owned()]);
}

/// spec/03 §5.5: a `refusal` stop reason finishes every call of the refusing response as an error
/// (not executed) and seals the batch, so Continue cannot execute it later.
#[tokio::test]
async fn a_refusal_finishes_its_calls_as_errors_and_never_executes_them() {
    use nekowriter_llm::StopReason;

    let runtime = runtime(vec![
        // A response that carries a call AND refuses: the call must never run.
        ScriptStep::new()
            .tool("insert_text", json!({ "text": "这段文字不应该被写入。" }))
            .stop(StopReason::Refusal {
                category: Some("safety".to_owned()),
                explanation: Some("cannot continue".to_owned()),
            }),
        insert_step("恢复后写入的一段。"),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    let row = runtime.wait_status(session_id, SessionStatus::Failed).await;
    assert_eq!(row.error().expect("error").code, "provider.refusal");

    let calls = runtime.calls(session_id).await;
    let refused = calls
        .iter()
        .find(|call| call.name == "insert_text")
        .expect("the refused call");
    assert_eq!(
        refused.status_enum(),
        ToolCallStatus::Error,
        "a refusal resolves its calls (spec/03 §5.5)"
    );
    assert!(refused.is_error);
    assert!(
        refused.sealed,
        "the batch is sealed so Continue can re-send"
    );
    // Not executed: the chapter is untouched.
    assert!(
        chapter_content(&runtime, runtime.chapter_id)
            .await
            .paragraphs
            .is_empty(),
        "a refused call is never executed"
    );

    // Continue re-sends the request, and the call is not picked up by step A.
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{session_id}/continue"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "continue: {body}");
    agent_write_harness::resume_until_done(&runtime, session_id, None).await;
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(
        content.paragraphs.len(),
        1,
        "only the post-continue paragraph exists: {:?}",
        content.paragraphs
    );
    assert_eq!(content.paragraphs[0].text, "恢复后写入的一段。");
}

// ─────────────────────── staged writes: the D-139 guarantees ───────────────────────

/// The event stream of a held write: no `chapter.content` while held, exactly one on approval.
#[tokio::test]
async fn a_held_write_emits_no_content_event_until_it_is_approved() {
    let runtime = runtime(vec![
        insert_step("第一段。"),
        insert_step("第二段。"),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, true, false).await;
    let (first, _) = held_call(&runtime, session_id).await;

    // The held call published no `chapter.content`: its text is not in the chapter.
    let mut receiver = runtime.app.events().subscribe();
    let held_events = drain(&mut receiver).await;
    let held_content = held_events
        .iter()
        .filter(|event| matches!(event, ServerEvent::ChapterContentChanged { .. }))
        .count();
    assert_eq!(held_content, 0, "a held write emits no chapter.content");

    let (status, body) = resume(&runtime, session_id, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Approving it emits exactly one, naming the agent as the actor.
    let approval_events = drain(&mut receiver).await;
    let content: Vec<_> = approval_events
        .iter()
        .filter_map(|event| match event {
            ServerEvent::ChapterContentChanged {
                version,
                word_count,
                actor,
                ..
            } => Some((*version, *word_count, *actor)),
            _ => None,
        })
        .collect();
    assert_eq!(
        content.len(),
        1,
        "approving applies the staged write once, with one chapter.content"
    );
    assert_eq!(content[0].0, 1, "the applied write is version 1");
    assert!(content[0].1 > 0, "the word count is computed from the text");
    assert_eq!(
        content[0].2,
        nekowriter::events::ContentActor::Agent,
        "the event names the agent as the actor"
    );
    let _ = first;
}

/// Approving one held write and then another must not apply the first one twice (the staged write is
/// cleared from the call's effects when it is applied).
#[tokio::test]
async fn approving_two_held_writes_does_not_apply_either_twice() {
    let runtime = runtime(vec![
        insert_step("第一段。"),
        insert_step("第二段。"),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, true, false).await;
    let (first, _) = held_call(&runtime, session_id).await;
    let (status, body) = resume(&runtime, session_id, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (second, _) = held_call(&runtime, session_id).await;
    assert_ne!(second.id, first.id, "a second, distinct held call");
    let (status, body) = resume(&runtime, session_id, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(
        content.paragraphs.len(),
        2,
        "each held write was applied exactly once: {:?}",
        content.paragraphs
    );
    assert_eq!(content.paragraphs[0].text, "第一段。");
    assert_eq!(content.paragraphs[1].text, "第二段。");
    // One version bump per applied write.
    let (_, _, version) = chapter_row(&runtime, runtime.chapter_id).await;
    assert_eq!(version, 2, "two approvals, two content versions");

    // Both calls spent their staged content.
    for call_id in [first.id, second.id] {
        let call = runtime.call_by_id(call_id).await;
        assert_eq!(call.effects().pending_content, None);
    }
}

/// The pending text of a held write is reachable by the client through the session snapshot, so a
/// draft-preview UI can render it without another endpoint.
#[tokio::test]
async fn a_held_write_exposes_its_pending_text_in_the_session_snapshot() {
    let runtime = runtime(vec![
        insert_step("待确认的一段。"),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, true, false).await;
    let (held, _) = held_call(&runtime, session_id).await;

    let (status, body) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "session snapshot: {body}");
    let calls = body["tool_calls"]
        .as_array()
        .expect("the snapshot carries the tool calls");
    let call = calls
        .iter()
        .find(|call| call["id"].as_i64() == Some(i64::from(held.id)))
        .expect("the held call is in the snapshot");
    let pending = &call["pending_content"];
    assert!(
        !pending.is_null(),
        "the held call exposes its pending content: {call}"
    );
    assert_eq!(
        pending["content"]["paragraphs"][0]["text"], "待确认的一段。",
        "the pending text is the staged write: {pending}"
    );
    assert_eq!(
        pending["content"]["tail_open"], false,
        "the staged tail state travels with it"
    );
    // Nothing of it is in the chapter yet.
    assert!(
        chapter_content(&runtime, runtime.chapter_id)
            .await
            .paragraphs
            .is_empty()
    );
}
