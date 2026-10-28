//! The granularity matrix of spec/03 §8, cell by cell (PLAN M4.2, M4.3).
//!
//! This file is the independent cross-check of the writer tools: it builds the full table of
//! **mode × tail state × `paragraph_end` × `chapter_end` × line breaks × each length boundary**
//! and compares every cell against what spec/03 §8 says the cell must be. It deliberately does
//! not reuse the assertions of `agent_write_tools.rs`; a cell that only one of the two files
//! covers is a cell the other one would miss.
//!
//! Cells covered here:
//!
//! | Mode | Cell | Spec/03 §8 |
//! |---|---|---|
//! | `sentence` | 400 / 401 characters | ≤ 400 |
//! | `sentence` | a line break inside the segment | rejected |
//! | `sentence` | `paragraph_end=true/false` | closes / keeps the tail open |
//! | `paragraph` | 3,000 / 3,001 characters | ≤ 3,000 |
//! | `paragraph` | `paragraph_end=false` | ignored (always closed) |
//! | `free` | a line break; `paragraph_end` | separates paragraphs; state of the last segment |
//! | `free` | 60,000 / 60,001 characters | ≤ 60,000 |
//! | `chapter` | 60,000 / 60,001 characters, `chapter_end` absent | implied true |
//! | `chapter` | an open tail from an earlier call | continued, then closed |
//! | `revising` | `chapter_end=true` | ignored, plus the `finish_revision` hint |
//! | all | an empty or whitespace-only segment | `error.granularity` (`empty`), nothing applied |
//! | all | the rules notice with an open tail | mentions the open tail and its pid |
//! | `sentence`/`paragraph` | `insert_paragraphs`/`rewrite_paragraph` limits | one paragraph |
//! | `free` | the same tools | several paragraphs |
//! | the rule's source | `request_meta`, not the session row | an in-flight call keeps its mode |

mod agent_write_harness;

use agent_write_harness::{
    chapter_content, chapter_row, runtime, start_legacy_writer, start_writer, start_writer_on,
};
use nekowriter::db::json::{Granularity, SessionStatus};
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter_llm::mock::ScriptStep;
use serde_json::json;

/// One `insert_text` step under the given arguments.
fn insert(text: &str, extra: serde_json::Value) -> ScriptStep {
    let mut arguments = json!({ "text": text });
    if let Some(object) = extra.as_object() {
        for (key, value) in object {
            arguments[key] = value.clone();
        }
    }
    ScriptStep::new().tool("insert_text", arguments)
}

/// The two `finish_draft` calls a short chapter needs (the first is withheld once).
fn finish_pair() -> [ScriptStep; 2] {
    [
        ScriptStep::new().tool("finish_draft", json!({})),
        ScriptStep::new().tool("finish_draft", json!({})),
    ]
}

/// Runs `steps` in a fresh non-holding writer session (the harness pre-accepts the outline), then
/// finishes the chapter, and returns the runtime with the finished session id.
async fn run_scripted(
    granularity: Granularity,
    steps: Vec<ScriptStep>,
) -> (agent_write_harness::Runtime, i32) {
    let mut script = steps;
    script.extend(finish_pair());
    let runtime = runtime(script).await;
    let session_id = start_writer(&runtime, granularity, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    (runtime, session_id)
}

/// Every `insert_text` call of a session, in order.
async fn inserts_of(
    runtime: &agent_write_harness::Runtime,
    session_id: i32,
) -> Vec<agent_write_harness::CallRow> {
    runtime
        .calls(session_id)
        .await
        .into_iter()
        .filter(|call| call.name == "insert_text")
        .collect()
}

fn text_of(call: &agent_write_harness::CallRow) -> String {
    call.raw_result_text.clone().unwrap_or_default()
}

// ───────────────────────────── sentence ─────────────────────────────

/// `sentence`: exactly one sentence, no line breaks, ≤ 400 characters (spec/03 §8).
#[tokio::test]
async fn sentence_mode_takes_one_line_of_at_most_400_characters() {
    let at_limit = "字".repeat(400);
    let over_limit = "字".repeat(401);
    let (runtime, session_id) = run_scripted(
        Granularity::Sentence,
        vec![
            // Exactly 400 characters: accepted.
            insert(&at_limit, json!({})),
            // 401 characters: `too_long`.
            insert(&over_limit, json!({})),
            // A line break anywhere inside the segment is `newline`.
            insert("甲。\n乙。", json!({})),
            insert("甲。\n", json!({})),
        ],
    )
    .await;

    let inserts = inserts_of(&runtime, session_id).await;
    assert_eq!(inserts.len(), 4, "every insert is executed in order");
    assert!(!inserts[0].is_error, "400 characters are within the limit");
    assert!(inserts[1].is_error, "401 characters exceed it");
    assert!(inserts[2].is_error, "an inner line break is rejected");
    // Edges are trimmed before validation (spec/03 §8 "Joining"), so a trailing break is not an
    // inner break: the segment is one sentence after trimming.
    assert!(
        !inserts[3].is_error,
        "leading/trailing whitespace is trimmed, not an inner break: {}",
        text_of(&inserts[3])
    );

    let too_long = text_of(&inserts[1]);
    assert!(
        too_long.contains("过长"),
        "the reason is `too_long`: {too_long}"
    );
    assert!(too_long.contains("400"), "the limit is named: {too_long}");
    assert!(too_long.contains("逐句"), "the mode is named: {too_long}");
    let newline = text_of(&inserts[2]);
    assert!(
        newline.contains("换行"),
        "the reason is `newline`: {newline}"
    );
    assert!(newline.contains("一句"), "the unit is named: {newline}");

    // Only the accepted sentences were written, and `paragraph_end` defaulted to true.
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 2, "{:?}", content.paragraphs);
    assert_eq!(content.paragraphs[0].text, at_limit);
    assert_eq!(content.paragraphs[1].text, "甲。");
    assert!(
        !content.tail_open,
        "`paragraph_end` defaults to true in `sentence`"
    );
}

/// `sentence`: `paragraph_end` decides whether the paragraph stays open (spec/03 §8).
///
/// The tail state is read from each call's own `result.draft_edit`, because the completing
/// `finish_draft` closes an open tail at the end (spec/05 §4).
#[tokio::test]
async fn sentence_mode_keeps_the_paragraph_open_until_paragraph_end() {
    let (runtime, session_id) = run_scripted(
        Granularity::Sentence,
        vec![
            // Opens p1.
            insert("他抬起头。", json!({ "paragraph_end": false })),
            // Continues p1 (the open tail) and closes it.
            insert("看见了剑光。", json!({})),
            // Opens p2 and leaves it open.
            insert("风停了。", json!({ "paragraph_end": false })),
        ],
    )
    .await;
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 2, "{:?}", content.paragraphs);
    assert_eq!(
        content.paragraphs[0].text, "他抬起头。看见了剑光。",
        "the open tail is continued by the next segment (zh-CN is unspaced)"
    );
    assert_eq!(content.paragraphs[1].text, "风停了。");

    let inserts = inserts_of(&runtime, session_id).await;
    let first = text_of(&inserts[0]);
    assert!(
        first.contains("尚未结束") && first.contains("p1"),
        "the first result reports the open tail and its pid: {first}"
    );
    let second = text_of(&inserts[1]);
    assert!(
        !second.contains("尚未结束"),
        "the second segment closed the paragraph: {second}"
    );
    let third = text_of(&inserts[2]);
    assert!(
        third.contains("尚未结束") && third.contains("p2"),
        "the third result reports the new open tail: {third}"
    );
}

// ───────────────────────────── paragraph ─────────────────────────────

/// `paragraph`: exactly one paragraph, no line breaks, ≤ 3,000 characters (spec/03 §8).
#[tokio::test]
async fn paragraph_mode_takes_one_paragraph_of_at_most_3000_characters() {
    let at_limit = "字".repeat(3_000);
    let over_limit = "字".repeat(3_001);
    let (runtime, session_id) = run_scripted(
        Granularity::Paragraph,
        vec![
            insert(&at_limit, json!({})),
            insert(&over_limit, json!({})),
            insert("第一段。\n第二段。", json!({})),
        ],
    )
    .await;
    let inserts = inserts_of(&runtime, session_id).await;
    assert!(
        !inserts[0].is_error,
        "3,000 characters are within the limit"
    );
    assert!(inserts[1].is_error, "3,001 characters exceed it");
    assert!(inserts[2].is_error, "a line break is rejected");
    let text = text_of(&inserts[1]);
    assert!(text.contains("3000"), "the limit is named: {text}");
    assert!(text.contains("逐段"), "the mode is named: {text}");
    // The unit ("one paragraph") is only named by the `newline` branch of `result.error`.
    let newline = text_of(&inserts[2]);
    assert!(
        newline.contains("一个段落"),
        "a line break names the paragraph unit in `paragraph` mode: {newline}"
    );
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 1);
    assert_eq!(content.paragraphs[0].text, at_limit);
}

/// `paragraph`: `paragraph_end` is ignored and the paragraph is always closed (spec/03 §8).
#[tokio::test]
async fn paragraph_mode_ignores_paragraph_end_and_always_closes() {
    let (runtime, session_id) = run_scripted(
        Granularity::Paragraph,
        vec![
            insert("第一段。", json!({ "paragraph_end": false })),
            insert("第二段。", json!({ "paragraph_end": false })),
        ],
    )
    .await;
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 2, "{:?}", content.paragraphs);
    assert_eq!(content.paragraphs[0].text, "第一段。");
    assert_eq!(content.paragraphs[1].text, "第二段。");
    assert!(
        !content.tail_open,
        "`paragraph` is always closed whatever `paragraph_end` says"
    );
    for call in inserts_of(&runtime, session_id).await {
        let text = text_of(&call);
        assert!(
            !text.contains("尚未结束"),
            "no open-tail notice can appear in `paragraph`: {text}"
        );
    }
}

// ───────────────────────────── free ─────────────────────────────

/// `free`: line breaks separate paragraphs and `paragraph_end` is the state of the last one
/// (spec/03 §8).
#[tokio::test]
async fn free_mode_splits_on_line_breaks_and_follows_paragraph_end() {
    let (runtime, _session_id) = run_scripted(
        Granularity::Free,
        vec![
            // Three paragraphs, the last one left open.
            insert("甲。\n\n乙。\n\n丙", json!({ "paragraph_end": false })),
            // The next segment continues `丙` and closes it.
            insert("丁。", json!({ "paragraph_end": true })),
        ],
    )
    .await;
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 3, "{:?}", content.paragraphs);
    assert_eq!(content.paragraphs[0].text, "甲。");
    assert_eq!(content.paragraphs[1].text, "乙。");
    assert_eq!(
        content.paragraphs[2].text, "丙丁。",
        "the open tail is continued without a space (zh-CN is unspaced)"
    );
    assert!(!content.tail_open, "the final segment closed the paragraph");
}

/// `free`: 60,000 / 60,001 characters is the boundary (spec/03 §8).
#[tokio::test]
async fn free_mode_accepts_60000_characters_and_rejects_60001() {
    let at_limit = "字".repeat(60_000);
    let over_limit = "字".repeat(60_001);
    let (runtime, session_id) = run_scripted(
        Granularity::Free,
        vec![insert(&at_limit, json!({})), insert(&over_limit, json!({}))],
    )
    .await;
    let inserts = inserts_of(&runtime, session_id).await;
    assert!(
        !inserts[0].is_error,
        "60,000 characters are within the limit"
    );
    assert!(inserts[1].is_error, "60,001 characters exceed it");
    let text = text_of(&inserts[1]);
    assert!(text.contains("60000"), "the limit is named: {text}");
    assert!(text.contains("过长"), "the reason is `too_long`: {text}");
}

// ───────────────────────────── chapter ─────────────────────────────

/// `chapter`: the rest of the chapter in one call, `chapter_end` implied, ≤ 60,000 characters
/// (spec/03 §8).
#[tokio::test]
async fn chapter_mode_implies_chapter_end_and_limits_60000_characters() {
    let over_limit = "字".repeat(60_001);
    let at_limit = "字".repeat(60_000);
    let runtime = runtime(vec![
        // Rejected first, so the session keeps running for the completing call.
        insert(&over_limit, json!({})),
        insert(&at_limit, json!({})),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Chapter, false, false).await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let inserts = inserts_of(&runtime, session_id).await;
    assert_eq!(inserts.len(), 2, "both calls ran: {:?}", inserts.len());
    assert!(inserts[0].is_error, "60,001 characters exceed the limit");
    assert!(
        !inserts[1].is_error,
        "60,000 characters complete the chapter: {}",
        text_of(&inserts[1])
    );
    assert!(
        text_of(&inserts[1]).contains("已标记本章完成"),
        "`chapter` mode implies `chapter_end`: {}",
        text_of(&inserts[1])
    );
    let (status, _, _) = chapter_row(&runtime, runtime.chapter_id).await;
    assert_eq!(status, "in_review", "the completion handler ran");
    assert_eq!(runtime.version_kinds().await, vec!["draft".to_owned()]);
}

/// `chapter`: an open tail left by an earlier call is continued, and the result is closed
/// (spec/03 §8 "open tail" + the table's "ignored (closed)").
///
/// The switch is made deterministically: `auto_pause` holds the first write, the session is not
/// running while it is held, and the `PATCH` happens there.
#[tokio::test]
async fn chapter_mode_continues_an_open_tail_and_closes_it() {
    let runtime = runtime(vec![
        // Left open so the `chapter` call below must continue it, then a completing pair so the
        // session can finish.
        insert("开头的一段，还没有写完", json!({ "paragraph_end": false })),
        insert("，接着写完了。", json!({})),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Free, true, false).await;
    let held = agent_write_harness::wait_for_status(&runtime, session_id, "held").await;
    // A held write is not in the chapter yet (D-139), so approving is what opens the tail.
    assert!(
        chapter_content(&runtime, runtime.chapter_id)
            .await
            .paragraphs
            .is_empty(),
        "the held write is not applied"
    );

    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/sessions/{session_id}"),
            json!({ "granularity": "chapter" }),
        )
        .await;
    assert_eq!(status, 200, "patch granularity on a held session: {body}");
    let (status, body) = agent_write_harness::resume_raw(&runtime, session_id, None).await;
    assert_eq!(status, 200, "approve the held write: {body}");
    let _ = held;
    let open = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(open.paragraphs.len(), 1, "{:?}", open.paragraphs);
    assert!(open.tail_open, "the held write left the paragraph open");
    // `auto_pause` holds a completing write too (it is a WRITE_TOOL), so drain the holds.
    agent_write_harness::resume_until_done(&runtime, session_id, None).await;
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 1, "the open tail was continued");
    assert_eq!(
        content.paragraphs[0].text, "开头的一段，还没有写完，接着写完了。",
        "the `chapter` call appended to the open paragraph"
    );
    assert!(
        !content.tail_open,
        "`chapter` mode always closes the paragraph"
    );
}

// ───────────────────────────── revising ─────────────────────────────

/// `revising`: `chapter_end` is ignored and the result appends the `finish_revision` hint
/// (spec/03 §8).
#[tokio::test]
async fn revising_ignores_chapter_end_and_hints_at_finish_revision() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("submit_revision_plan", json!({ "plan": "改 p1。" })),
        insert("这一段是追加的内容。", json!({ "chapter_end": true })),
        // `finish_revision` closes the session; M6.2 owns its completion handler.
        ScriptStep::new().tool("finish_revision", json!({ "summary": "按意见修改。" })),
    ])
    .await;
    // A revise session needs existing text.
    runtime
        .app
        .put_text(runtime.chapter_id, 0, "第一段正文。\n\n第二段正文。")
        .await;
    let (session_id, _warnings) = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: nekowriter::db::json::SessionKind::ChapterRevise,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            research_task_id: None,
            granularity: Some(Granularity::Paragraph),
            auto_pause: Some(false),
            pause_after_outline: Some(false),
            redraft: false,
            guidance: None,
            kickoff_extra: None,
            pending_comment: None,
            legacy_planning_fixture: false,
            legacy_system_prompt: None,
            legacy_kickoff_text: None,
            chapter_guard: false,
            force_previous: false,
        },
    )
    .await
    .expect("create the revise session");
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    let calls = runtime.calls(session_id).await;
    let insert_call = calls
        .iter()
        .find(|call| call.name == "insert_text")
        .expect("the insert");
    assert!(
        !insert_call.is_error,
        "the revision insert is accepted: {}",
        text_of(insert_call)
    );
    let text = text_of(insert_call);
    assert!(
        text.contains("修订阶段的 chapter_end 无效"),
        "the result names the ignored chapter_end: {text}"
    );
    assert!(
        text.contains("finish_revision"),
        "and hints at the tool that completes a revision: {text}"
    );
    assert!(
        !text.contains("已标记本章完成"),
        "nothing was completed: {text}"
    );
    // The text was written, and the chapter is not in review: `chapter_end` did nothing.
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert!(
        content
            .paragraphs
            .iter()
            .any(|paragraph| paragraph.text.contains("追加的内容")),
        "the text was written: {:?}",
        content.paragraphs
    );
    let (status, _, _) = chapter_row(&runtime, runtime.chapter_id).await;
    assert_eq!(
        status, "in_review",
        "a completed revision waits for the author's review (spec/08 §1.1)"
    );
    // A `backup` version exists from the creation side effect of a revise session
    // (spec/03 §4); M6.2's completion handler adds the `revision` version.
    assert_eq!(
        runtime.version_kinds().await,
        vec!["backup".to_owned(), "revision".to_owned()],
        "the revision version is saved on completion (spec/08 §1.1)"
    );
}

// ───────────────────────────── empty segments ─────────────────────────────

/// Every mode rejects an empty or whitespace-only segment with `error.granularity` and applies
/// nothing (spec/03 §8: the reason is `empty`, "without applying anything").
#[tokio::test]
async fn an_empty_segment_is_a_granularity_error_in_every_mode() {
    for granularity in [
        Granularity::Sentence,
        Granularity::Paragraph,
        Granularity::Free,
        Granularity::Chapter,
    ] {
        let (runtime, session_id) = run_scripted(
            granularity,
            vec![
                insert("", json!({})),
                insert("   \n\n\t ", json!({})),
                insert("真正的正文。", json!({})),
            ],
        )
        .await;
        let inserts = inserts_of(&runtime, session_id).await;
        assert!(
            inserts[0].is_error,
            "{granularity:?}: an empty segment is rejected"
        );
        let text = text_of(&inserts[0]);
        assert!(
            text.contains("为空"),
            "{granularity:?}: the reason is `empty`: {text}"
        );
        assert!(
            inserts[1].is_error,
            "{granularity:?}: whitespace-only is the same as empty"
        );
        // The rejected calls applied nothing: only the real segment was written.
        let content = chapter_content(&runtime, runtime.chapter_id).await;
        assert_eq!(
            content.paragraphs.len(),
            1,
            "{granularity:?}: {:?}",
            content.paragraphs
        );
        assert_eq!(content.paragraphs[0].text, "真正的正文。");
    }
}

// ───────────────────────────── the notice names the tail ─────────────────────────────

/// The granularity notice mentions the open tail and its pid whenever there is one (spec/03 §8
/// "the notices say so whenever the tail is open").
///
/// `auto_pause` holds the write that leaves the tail open; the `PATCH` happens while the session
/// is held, so it is not running and the notice is queued deterministically.
#[tokio::test]
async fn the_granularity_notice_mentions_an_open_tail_in_every_unspaced_mode() {
    for granularity in [Granularity::Sentence, Granularity::Free] {
        let runtime = runtime(vec![
            insert("这一段的末尾", json!({ "paragraph_end": false })),
            // The next call continues the open tail and closes it.
            insert("，补完。", json!({})),
            finish_pair()[0].clone(),
            finish_pair()[1].clone(),
        ])
        .await;
        let session_id = start_writer(&runtime, granularity, true, false).await;
        let held = agent_write_harness::wait_for_status(&runtime, session_id, "held").await;
        // A held write is staged, not applied (D-139), so the tail is still closed here.
        assert!(
            chapter_content(&runtime, runtime.chapter_id)
                .await
                .paragraphs
                .is_empty(),
            "{granularity:?}: the held write is not applied"
        );

        let (status, body) = runtime
            .app
            .patch(
                &format!("/api/sessions/{session_id}"),
                json!({ "granularity": granularity.as_str() }),
            )
            .await;
        assert_eq!(status, 200, "{granularity:?}: patch: {body}");
        let (status, body) = agent_write_harness::resume_raw(&runtime, session_id, None).await;
        assert_eq!(status, 200, "{granularity:?}: resume: {body}");
        // Approving is what applies the write, including the open tail it left behind.
        let open = chapter_content(&runtime, runtime.chapter_id).await;
        assert!(
            open.tail_open,
            "{granularity:?}: the approved write left it open"
        );
        agent_write_harness::resume_until_done(&runtime, session_id, None).await;

        // The notice is an inbox item sealed into a tool result; find it among the sealed texts.
        let sealed: Vec<String> = runtime
            .calls(session_id)
            .await
            .into_iter()
            .filter_map(|call| call.result_text.clone())
            .collect();
        let notice = sealed
            .iter()
            .find(|text| text.contains("作者调整了写作粒度"))
            .unwrap_or_else(|| {
                panic!("{granularity:?}: the switch notice was delivered: {sealed:?}")
            });
        assert!(
            notice.contains("尚未写完"),
            "{granularity:?}: the notice mentions the open tail: {notice}"
        );
        assert!(
            notice.contains(&open.paragraphs[0].pid),
            "{granularity:?}: the notice names the open pid {}: {notice}",
            open.paragraphs[0].pid
        );
        let _ = held;
    }
}

// ───────────────────────────── which rule applies ─────────────────────────────

/// The rule comes from the assistant message's `request_meta`: a switch while the call is in
/// flight does not reject it (spec/03 §8 "which rule applies").
///
/// The distinguishing cell is a segment of 500 characters: valid under `paragraph` (≤ 3,000) and
/// invalid under `sentence` (≤ 400). The call is generated under `paragraph`, the switch lands
/// while its arguments stream, and the call must still be accepted.
#[tokio::test]
async fn a_call_generated_under_paragraph_survives_a_switch_to_sentence() {
    let in_flight = "字".repeat(500);
    let after_switch = "新的句子。";
    let mut script = vec![
        // Streams slowly, so the PATCH lands after `request_meta` was written but before the
        // call executes.
        insert(&in_flight, json!({})).delay_per_event(std::time::Duration::from_millis(40)),
        insert(after_switch, json!({})),
    ];
    script.extend(finish_pair());
    let runtime = runtime(script).await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, false, false).await;
    // After the split, request 1 is the 500-character insert itself (there is no outline request);
    // wait for *that* request, not the pre-split request 2 (R2-07). A PATCH after the call sealed
    // would make `inserts[0]` vacuous, so assert the call is still streaming when it lands.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while runtime.scripted.requests().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the drafting request never started"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let requests_at_patch = runtime.scripted.requests().len();
    let calls_at_patch = runtime.calls(session_id).await.len();
    assert_eq!(requests_at_patch, 1, "one model request must be in flight");
    assert_eq!(
        calls_at_patch, 0,
        "the distinguishing call must not have sealed"
    );
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

    let inserts = inserts_of(&runtime, session_id).await;
    assert_eq!(inserts.len(), 2, "{:?}", inserts.len());
    assert!(
        !inserts[0].is_error,
        "a 500-character call generated under `paragraph` stays valid: {}",
        text_of(&inserts[0])
    );
    // `request_meta` is what the call was generated under: 500 characters are still there.
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs[0].text, in_flight);
    // The next call runs under `sentence`, so the same 500 characters would now be rejected; the
    // short sentence is accepted, which proves the new mode is in effect.
    assert!(!inserts[1].is_error, "{}", text_of(&inserts[1]));
}

// ───────────────────────────── the paragraph-edit tools ─────────────────────────────

/// `insert_paragraphs` / `rewrite_paragraph` take one paragraph under `sentence` and `paragraph`,
/// and several under `free` (spec/03 §8).
#[tokio::test]
async fn paragraph_tools_follow_the_mode_table() {
    // One paragraph is fine in `sentence`; two are rejected, and so is 3,001 characters.
    let (runtime, session_id) = run_scripted(
        Granularity::Sentence,
        vec![
            insert("甲。", json!({})),
            ScriptStep::new().tool(
                "insert_paragraphs",
                json!({ "after": "p1", "text": "乙。" }),
            ),
            ScriptStep::new().tool(
                "insert_paragraphs",
                json!({ "after": "p1", "text": "丙。\n丁。" }),
            ),
            ScriptStep::new().tool("rewrite_paragraph", json!({ "pid": "p2", "text": "戊。" })),
            ScriptStep::new().tool(
                "rewrite_paragraph",
                json!({ "pid": "p2", "text": "己。\n庚。" }),
            ),
        ],
    )
    .await;
    let paragraph_calls: Vec<_> = runtime
        .calls(session_id)
        .await
        .into_iter()
        .filter(|call| call.name == "insert_paragraphs" || call.name == "rewrite_paragraph")
        .collect();
    assert_eq!(paragraph_calls.len(), 4);
    assert!(
        !paragraph_calls[0].is_error,
        "one paragraph is fine under `sentence`: {}",
        text_of(&paragraph_calls[0])
    );
    assert!(
        paragraph_calls[1].is_error,
        "two paragraphs would bypass the granularity"
    );
    assert!(!paragraph_calls[2].is_error, "one paragraph for rewrite");
    assert!(paragraph_calls[3].is_error, "two paragraphs for rewrite");

    // Under `paragraph` the same tools are limited to 3,000 characters.
    let (runtime, session_id) = run_scripted(
        Granularity::Paragraph,
        vec![
            insert("甲。", json!({})),
            ScriptStep::new().tool(
                "insert_paragraphs",
                json!({ "after": "p1", "text": "字".repeat(3_001) }),
            ),
        ],
    )
    .await;
    let call = runtime
        .calls(session_id)
        .await
        .into_iter()
        .find(|call| call.name == "insert_paragraphs")
        .expect("the call");
    assert!(
        call.is_error,
        "3,001 characters exceed the paragraph limit: {}",
        text_of(&call)
    );

    // Under `free` both tools write several paragraphs.
    let (runtime, session_id) = run_scripted(
        Granularity::Free,
        vec![
            insert("甲。", json!({})),
            ScriptStep::new().tool(
                "insert_paragraphs",
                json!({ "after": "p1", "text": "乙。\n丙。" }),
            ),
            ScriptStep::new().tool(
                "rewrite_paragraph",
                json!({ "pid": "p1", "text": "丁。\n戊。" }),
            ),
        ],
    )
    .await;
    for call in runtime
        .calls(session_id)
        .await
        .into_iter()
        .filter(|call| call.name == "insert_paragraphs" || call.name == "rewrite_paragraph")
    {
        assert!(
            !call.is_error,
            "{} is allowed several paragraphs under free: {}",
            call.name,
            text_of(&call)
        );
    }
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert!(
        content.paragraphs.len() >= 4,
        "both calls added paragraphs: {:?}",
        content.paragraphs
    );
}

/// `insert_paragraphs` after `"start"` and after a pid both work, and an unknown pid is
/// `error.pid_not_found` (spec/05 §4).
#[tokio::test]
async fn insert_paragraphs_locates_its_position() {
    let (runtime, session_id) = run_scripted(
        Granularity::Paragraph,
        vec![
            insert("中间。", json!({})),
            ScriptStep::new().tool(
                "insert_paragraphs",
                json!({ "after": "start", "text": "最前面。" }),
            ),
            ScriptStep::new().tool(
                "insert_paragraphs",
                json!({ "after": "p1", "text": "最后面。" }),
            ),
            ScriptStep::new().tool(
                "insert_paragraphs",
                json!({ "after": "p99", "text": "不存在的位置。" }),
            ),
        ],
    )
    .await;
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    let texts: Vec<&str> = content
        .paragraphs
        .iter()
        .map(|paragraph| paragraph.text.as_str())
        .collect();
    assert_eq!(texts, vec!["最前面。", "中间。", "最后面。"], "{texts:?}");
    let calls: Vec<_> = runtime
        .calls(session_id)
        .await
        .into_iter()
        .filter(|call| call.name == "insert_paragraphs")
        .collect();
    assert!(calls[2].is_error, "an unknown pid is rejected");
    assert!(text_of(&calls[2]).contains("找不到段落"));
}

/// `replace_text` and `delete_paragraphs` are not limited by granularity
/// (spec/03 §8 "`replace_text` and `delete_paragraphs` are not limited").
#[tokio::test]
async fn replace_and_delete_are_not_limited_by_granularity() {
    let long_text = "字".repeat(4_000);
    let (runtime, session_id) = run_scripted(
        Granularity::Sentence,
        vec![
            insert("短句。", json!({})),
            insert("删掉这一段。", json!({})),
            // `replace_text` is not limited: a 4,000-character replacement in one paragraph is
            // fine, even though `insert_text` would reject anything over 400 characters here.
            ScriptStep::new().tool(
                "replace_text",
                json!({ "pid": "p1", "old": "短句。", "new": long_text }),
            ),
            ScriptStep::new().tool("delete_paragraphs", json!({ "pids": ["p2"] })),
        ],
    )
    .await;
    let calls: Vec<_> = runtime
        .calls(session_id)
        .await
        .into_iter()
        .filter(|call| call.name == "replace_text" || call.name == "delete_paragraphs")
        .collect();
    assert_eq!(calls.len(), 2);
    assert!(
        !calls[0].is_error,
        "replace_text is not limited by granularity: {}",
        text_of(&calls[0])
    );
    assert!(
        !calls[1].is_error,
        "delete_paragraphs is not limited by granularity: {}",
        text_of(&calls[1])
    );
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 1, "{:?}", content.paragraphs);
    assert_eq!(content.paragraphs[0].text, long_text);
    let _ = (start_writer_on, chapter_row);
}

// ─────────────────────── notices name the open tail ───────────────────────

/// spec/03 §8: "The notices say so whenever the tail is open."
///
/// The `outline_accepted` notice is rendered by the transaction that finalizes `submit_outline`.
/// The tail state it reads must be the chapter's current tail, not the session's stored
/// `view_snapshot`: that column is only written when a session *leaves* `running` (spec/03 §3), so
/// on this path it is still `NULL` and the hint would silently disappear.
#[tokio::test]
async fn the_outline_accepted_notice_names_an_open_tail() {
    let runtime = runtime(vec![
        // The first session writes one paragraph and leaves it open, then is abandoned.
        insert("开头的一段，还没写完", json!({ "paragraph_end": false })),
        // The second (legacy) session plans first, then drafts the rest of the chapter.
        agent_write_harness::outline_step(),
        insert("，接着写完了。", json!({})),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    // `auto_pause` holds the write so the first session stops with the tail open. The mode must be
    // one that honours `paragraph_end` (`paragraph` always closes, spec/03 §8).
    let first = start_writer(&runtime, Granularity::Free, true, false).await;
    agent_write_harness::wait_for_status(&runtime, first, "held").await;
    // The held write is staged, not applied (D-139), so approving it is what creates the open tail
    // the fixture needs.
    let (status, body) = agent_write_harness::resume_raw(&runtime, first, None).await;
    assert_eq!(status, 200, "approve the first write: {body}");
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert!(content.tail_open, "the fixture has an open tail");
    let pid = content
        .paragraphs
        .last()
        .map(|paragraph| paragraph.pid.clone())
        .unwrap_or_default();
    let (status, body) = runtime
        .app
        .post(&format!("/api/sessions/{first}/abandon"), json!({}))
        .await;
    assert_eq!(status, 200, "abandon the first session: {body}");

    // The second session is the legacy planning branch (the writer acceptance notice it carries
    // exists only there now), with the open tail in front of it.
    let second = start_legacy_writer(&runtime, Granularity::Paragraph, false, false).await;
    runtime.wait_status(second, SessionStatus::Completed).await;

    let notice = runtime
        .calls(second)
        .await
        .into_iter()
        .filter_map(|call| call.result_text.clone())
        .find(|text| text.contains("章纲已确认"))
        .expect("the accepted notice was delivered");
    assert!(
        notice.contains("尚未写完"),
        "the accepted notice says the tail is open: {notice}"
    );
    assert!(
        notice.contains(&pid),
        "and names the open pid {pid}: {notice}"
    );
}

/// spec/03 §8: "In `revising`, `chapter_end` is ignored, and the result appends a hint to call
/// `finish_revision`."
///
/// `chapter` mode implies `chapter_end`, so the implied completion is ignored there too — the hint
/// must appear even though the model never passed the flag explicitly.
#[tokio::test]
async fn revising_ignores_the_implied_chapter_end_of_chapter_mode() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "submit_revision_plan",
            json!({ "plan": "把开头改得更冷一些。" }),
        ),
        // `chapter` mode would complete the chapter in a drafting session; in `revising` it must
        // not, and the result has to say why.
        insert("整章重写的正文。", json!({})),
        ScriptStep::new().tool("finish_revision", json!({ "summary": "改好了。" })),
    ])
    .await;
    // A revise session needs existing text.
    runtime
        .app
        .put_text(runtime.chapter_id, 0, "第一段正文。\n\n第二段正文。")
        .await;
    let (session_id, _warnings) = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: nekowriter::db::json::SessionKind::ChapterRevise,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            research_task_id: None,
            granularity: Some(Granularity::Chapter),
            auto_pause: Some(false),
            pause_after_outline: Some(false),
            redraft: false,
            guidance: None,
            kickoff_extra: None,
            pending_comment: None,
            legacy_planning_fixture: false,
            legacy_system_prompt: None,
            legacy_kickoff_text: None,
            chapter_guard: false,
            force_previous: false,
        },
    )
    .await
    .expect("create the revise session");
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let insert = inserts_of(&runtime, session_id)
        .await
        .into_iter()
        .next()
        .expect("the insert ran");
    assert!(
        !insert.is_error,
        "the insert itself is valid: {}",
        text_of(&insert)
    );
    let text = text_of(&insert);
    assert!(
        text.contains("finish_revision"),
        "the implied chapter_end of `chapter` mode is ignored and the hint says so: {text}"
    );
}

// ───────────────────────── short completion ─────────────────────────

/// spec/03 §8: a completion below 60% of `target_words_min` is withheld **once**; the next one goes
/// through even though the chapter is still short, and the withheld text is kept.
///
/// The threshold is a ratio of the minimum, so 60% exactly is not short and 59% is.
#[tokio::test]
async fn a_short_completion_is_withheld_once_at_the_sixtieth_percentile() {
    // The harness book's target is 3,000–4,000, so 60% is 1,800 words.
    let just_under = "字".repeat(1_799);
    let at_threshold = "字".repeat(1_800);
    let first = runtime(vec![
        // 1,799 characters: below the threshold, so the `chapter_end` is withheld.
        insert(&just_under, json!({ "chapter_end": true })),
        // The retry goes through even though the chapter is still short.
        ScriptStep::new().tool("finish_draft", json!({})),
    ])
    .await;
    let session_id = start_writer(&first, Granularity::Free, false, false).await;
    first
        .wait_status(session_id, SessionStatus::Completed)
        .await;

    let inserts = inserts_of(&first, session_id).await;
    let withheld = text_of(&inserts[0]);
    assert!(
        withheld.contains("字数明显少于目标下限"),
        "1,799 of 3,000 is below 60%: {withheld}"
    );
    let content = chapter_content(&first, first.chapter_id).await;
    assert_eq!(
        content.paragraphs.len(),
        1,
        "the withheld text is kept, not rolled back"
    );
    assert_eq!(content.paragraphs[0].text, just_under);
    let row = first.session_row(session_id).await;
    assert!(
        row.counters().finish_warned,
        "counters.finish_warned records the one warning"
    );
    // The completion still happened on the second attempt.
    let (status, _, _) = chapter_row(&first, first.chapter_id).await;
    assert_eq!(status, "in_review");

    // At exactly 60% the completion is not short at all, so the first `chapter_end` completes.
    let exact = runtime(vec![insert(&at_threshold, json!({ "chapter_end": true }))]).await;
    let exact_session = start_writer(&exact, Granularity::Free, false, false).await;
    exact
        .wait_status(exact_session, SessionStatus::Completed)
        .await;
    let inserts = inserts_of(&exact, exact_session).await;
    let completed = text_of(&inserts[0]);
    assert!(
        !completed.contains("字数明显少于目标下限"),
        "1,800 of 3,000 is exactly 60%, which is not short: {completed}"
    );
    assert!(
        completed.contains("已标记本章完成"),
        "the first completion goes through: {completed}"
    );
    let row = exact.session_row(exact_session).await;
    assert!(
        !row.counters().finish_warned,
        "nothing was withheld, so the counter stays clear"
    );
}

/// spec/05 §4: "Inserting after an open tail closes it first."
///
/// A complete paragraph cannot be followed by a still-open one: `tail_open` names the last
/// paragraph, and after the insert the last paragraph is the new, complete one.
#[tokio::test]
async fn inserting_after_the_open_tail_closes_it() {
    // `free` honours `paragraph_end`, so the fixture can leave a real open tail.
    let runtime = runtime(vec![
        // One segment: p1 complete, p2 open (the last paragraph is the open tail).
        insert(
            "第一段。\n\n第二段还没有写完",
            json!({ "paragraph_end": false }),
        ),
        // Insert after p2, the open tail.
        ScriptStep::new().tool(
            "insert_paragraphs",
            json!({ "after": "p2", "text": "插在最后面。" }),
        ),
        finish_pair()[0].clone(),
        finish_pair()[1].clone(),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Free, true, false).await;
    agent_write_harness::resume_until_done(&runtime, session_id, None).await;
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert_eq!(content.paragraphs.len(), 3, "{:?}", content.paragraphs);
    assert_eq!(content.paragraphs[1].text, "第二段还没有写完");
    assert_eq!(content.paragraphs[2].text, "插在最后面。");
    assert!(
        !content.tail_open,
        "the last paragraph is the complete inserted one, so the tail is closed: {:?}",
        content.paragraphs
    );
}
