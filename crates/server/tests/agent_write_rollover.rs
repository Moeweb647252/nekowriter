//! Rollover (PLAN M4.4, spec/03 §9).
//!
//! Runtime test 13: a writer near the context limit is superseded by a child session whose kickoff
//! carries the current draft, the author's earlier feedback (including feedback given on the
//! parent's last held call), and the unconsumed inbox items.

mod agent_write_harness;

use agent_write_harness::{chapter_content, outline_step, runtime, start_writer, start_writer_on};
use nekowriter::db::json::{Granularity, Phase, SessionStatus};
use nekowriter_llm::mock::ScriptStep;
use serde_json::json;

fn insert_step(text: &str) -> ScriptStep {
    ScriptStep::new().tool("insert_text", json!({ "text": text }))
}

/// Puts a session just below the rollover threshold, as a long drafting run would (spec/03 §9).
async fn push_usage_near_the_limit(runtime: &agent_write_harness::Runtime, session_id: i32) {
    // The Demo model has a 200,000-token window; 0.85 × 200,000 = 170,000, and the model's
    // max_output_tokens is 8,192 (the test harness profile), so a `last_prompt_tokens` of 165,000
    // already exceeds the threshold once the output budget is added.
    runtime
        .app
        .execute(&format!(
            "UPDATE agent_sessions SET usage = '{{\"requests\":120,\"input_tokens\":165000,\
             \"cached_input_tokens\":0,\"cache_write_tokens\":0,\"output_tokens\":9000,\
             \"reasoning_tokens\":0,\"last_prompt_tokens\":165000,\"estimated\":false}}' \
             WHERE id = {session_id}"
        ))
        .await;
}

/// spec/03 §15 test 13: a rollover creates a child with the draft in its kickoff, moves the
/// unconsumed inbox, supersedes the parent, and carries the feedback given on the parent's last
/// held call in `author_notes`.
#[tokio::test]
async fn test_13_rollover_carries_the_draft_author_notes_and_the_inbox() {
    let runtime = runtime(vec![
        outline_step(),
        insert_step("第一章开头的一段。"),
        insert_step("暂停前写入的一段。"),
        // The child keeps writing; the pair lets it complete.
        ScriptStep::new().tool("finish_draft", json!({})),
        ScriptStep::new().tool("finish_draft", json!({})),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, true, false).await;

    // Approve the first insert so the second one is written; the second holds in turn.
    let first = agent_write_harness::wait_for_status(&runtime, session_id, "held").await;
    let first_id = first.held_tool_call_id.expect("held id");
    let (status, body) = agent_write_harness::resume_raw(&runtime, session_id, None).await;
    assert_eq!(status, 200, "approve the first insert: {body}");
    let second = agent_write_harness::wait_for_new_hold(&runtime, session_id, first_id).await;
    let second_id = second.held_tool_call_id.expect("held id");
    assert_ne!(second_id, first_id, "the second insert is its own call");
    assert_eq!(
        chapter_content(&runtime, runtime.chapter_id)
            .await
            .paragraphs
            .len(),
        2,
        "both inserts were written before the second held"
    );

    // While the second call is held: queue a steering message that stays unconsumed (it must move
    // to the child, spec/03 §9 step 2), and push the session near the context limit so the next
    // boundary rolls over instead of asking the model.
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/message"),
            json!({ "text": "别忘了城楼上的那个人。" }),
        )
        .await;
    assert_eq!(status, 200, "steering while held: {body}");
    assert_eq!(
        runtime.inbox_pending(session_id).await,
        1,
        "the steering message is still unconsumed"
    );
    push_usage_near_the_limit(&runtime, session_id).await;

    // Feedback on the parent's LAST held call (spec/03 §9: "not lost"): it must reach the child's
    // `author_notes`.
    let (status, body) = agent_write_harness::resume_raw(
        &runtime,
        session_id,
        Some("这一段要写出雨声，不要写风声。"),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    // The parent is superseded and exposes its child (spec/03 §9 step 3).
    runtime
        .wait_status(session_id, SessionStatus::Superseded)
        .await;
    let parent = agent_write_harness::session_dto(&runtime, session_id).await;
    let child_id = parent["child_session_id"]
        .as_i64()
        .expect("the parent exposes its child") as i32;
    assert_ne!(child_id, session_id);

    // The child is a writer session of the same kind and phase, linked to the parent.
    let child = agent_write_harness::session_dto(&runtime, child_id).await;
    assert_eq!(child["kind"], "chapter_write");
    assert_eq!(child["phase"], Phase::Drafting.as_str());
    assert_eq!(child["parent_session_id"], session_id);

    // Its kickoff holds the current draft (`block.chapter_draft`) and the author's notes.
    let messages = runtime.messages(child_id).await;
    let kickoff = &messages[0];
    assert_eq!(kickoff.kind, "kickoff");
    let kickoff_text = kickoff.text();
    assert!(
        kickoff_text.contains("第一章开头的一段。"),
        "the child kickoff carries the draft: {kickoff_text}"
    );
    assert!(
        kickoff_text.contains("暂停前写入的一段。"),
        "all of the draft, not just the tail: {kickoff_text}"
    );
    assert!(
        kickoff_text.contains("继续写作"),
        "the rollover task is rendered: {kickoff_text}"
    );
    assert!(
        kickoff_text.contains("这一段要写出雨声，不要写风声。"),
        "feedback on the parent's last held call appears in author_notes: {kickoff_text}"
    );
    // The steering message moved as an inbox item instead of being duplicated in `author_notes`
    // (spec/03 §9 step 2). The child consumes it at its first sealing, so it appears in the
    // child's message stream (a user message carrying `notice.user_message`) and never in the
    // kickoff's `author_notes`.
    assert!(
        !kickoff_text.contains("别忘了城楼上的那个人。"),
        "an unconsumed item is not also copied into author_notes: {kickoff_text}"
    );
    assert_eq!(
        runtime.inbox_pending(session_id).await,
        0,
        "the parent has no inbox items left"
    );
    let child_messages = runtime.messages(child_id).await;
    let delivered = child_messages
        .iter()
        .skip(1)
        .any(|message| message.text().contains("别忘了城楼上的那个人。"));
    assert!(
        delivered,
        "the moved item is delivered to the child: {:?}",
        child_messages
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>()
    );

    // The child runs to completion and keeps writing.
    agent_write_harness::resume_until_done(&runtime, child_id, None).await;
    let content = chapter_content(&runtime, runtime.chapter_id).await;
    assert!(
        !content.paragraphs.is_empty(),
        "the child kept writing: {:?}",
        content.paragraphs
    );
}

/// A non-eligible session that overflows fails with `provider.context_overflow` instead of
/// rolling over (spec/03 §9 last paragraph).
#[tokio::test]
async fn an_ineligible_session_overflowing_fails_without_rolling_over() {
    use nekowriter_llm::retry::LlmError;
    // The provider rejects every request with a context overflow, so a session that is not
    // eligible fails rather than creating a child.
    let app = agent_write_harness::runtime_failing(LlmError::ContextOverflow(
        "context length exceeded".to_owned(),
    ))
    .await;
    let chapter = app.app.create_chapter(app.track_id, "第二章").await;
    let chapter_id = chapter["id"].as_i64().expect("id") as i32;
    let session_id = start_writer_on(
        &app,
        chapter_id,
        Granularity::Paragraph,
        false,
        false,
        false,
    )
    .await;
    let row = agent_write_harness::wait_for_status(&app, session_id, "failed").await;
    let error = row.error().expect("a session error");
    assert_eq!(error.code, "provider.context_overflow");
    let dto = agent_write_harness::session_dto(&app, session_id).await;
    assert!(
        dto["child_session_id"].is_null(),
        "no child was created: {dto}"
    );
}

/// An eligible session whose request fails with a context overflow rolls over exactly once
/// (spec/03 §9 "or the previous request failed with context overflow"). The pre-request trigger and
/// the failure path must not both fire: one overflow means one child.
#[tokio::test]
async fn one_context_overflow_creates_exactly_one_child() {
    use nekowriter_llm::retry::LlmError;

    // The outline is accepted (the one scripted step), then every further request overflows.
    let app = agent_write_harness::runtime_with_failure(
        vec![outline_step()],
        LlmError::ContextOverflow("too long".to_owned()),
    )
    .await;
    let session_id = start_writer(&app, Granularity::Paragraph, false, false).await;

    // The session reaches drafting (the outline was accepted) and then exceeds the limit.
    agent_write_harness::wait_for_status(&app, session_id, "superseded").await;
    let parent = agent_write_harness::session_dto(&app, session_id).await;
    let child_id = parent["child_session_id"].as_i64().expect("a child") as i32;

    // Exactly one child of the parent, and the child is not itself superseded yet.
    let children = agent_write_harness::query_i64(
        &app,
        "SELECT count(*) AS value FROM agent_sessions WHERE parent_session_id = ?",
        vec![session_id.into()],
    )
    .await;
    assert_eq!(children, 1, "one overflow creates exactly one child");
    let child = agent_write_harness::session_dto(&app, child_id).await;
    assert_eq!(child["kind"], "chapter_write");
    assert_eq!(child["phase"], "drafting");
    // The child's kickoff carries the draft state, and the overflow text never became its kickoff.
    let messages = app.messages(child_id).await;
    assert_eq!(messages[0].kind, "kickoff");
}

/// spec/03 §10 marks `block.settings` with `cache: true`; the other blocks store no cache flag at
/// all (`StoredBlock::cache` stays absent), because only the settings block places a breakpoint.
///
/// The rollover kickoff wrote `cache: Some(block_is_cached(key))` for every block, so the three
/// non-settings blocks stored `cache: false` — a flag that says "no breakpoint here" instead of
/// saying nothing, and that a reader of the stored kickoff cannot distinguish from a deliberate
/// one.
#[tokio::test]
async fn only_the_settings_block_of_a_rollover_kickoff_carries_a_cache_flag() {
    let runtime = runtime(vec![
        outline_step(),
        insert_step("第一章开头的一段。"),
        insert_step("暂停前写入的一段。"),
        ScriptStep::new().tool("finish_draft", json!({})),
        ScriptStep::new().tool("finish_draft", json!({})),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, true, false).await;
    let first = agent_write_harness::wait_for_status(&runtime, session_id, "held").await;
    let first_id = first.held_tool_call_id.expect("held id");
    let (status, body) = agent_write_harness::resume_raw(&runtime, session_id, None).await;
    assert_eq!(status, 200, "{body}");
    agent_write_harness::wait_for_new_hold(&runtime, session_id, first_id).await;
    push_usage_near_the_limit(&runtime, session_id).await;
    let (status, body) = agent_write_harness::resume_raw(&runtime, session_id, None).await;
    assert_eq!(status, 200, "{body}");
    runtime
        .wait_status(session_id, SessionStatus::Superseded)
        .await;
    let parent = agent_write_harness::session_dto(&runtime, session_id).await;
    let child_id = parent["child_session_id"].as_i64().expect("a child") as i32;

    let messages = runtime.messages(child_id).await;
    let flags: Vec<Option<bool>> = messages[0]
        .blocks()
        .into_iter()
        .map(|block| match block {
            nekowriter::db::json::StoredBlock::Text { cache, .. } => cache,
            _ => None,
        })
        .collect();
    assert_eq!(
        flags,
        vec![Some(true), None, None, None],
        "only `block.settings` carries a cache flag: {flags:?}"
    );
}

/// spec/03 §9: `author_notes` holds "the author's earlier feedback and steering messages from the
/// parent chain **in order** (`author_notes`: `user_feedback` appendices, sealed or not, and
/// consumed `user_message` inbox items; newest last)".
///
/// The two sources interleave chronologically: the author can steer, then give feedback on the
/// next hold, then steer again. Collecting every feedback appendix of an ancestor before every
/// steering message of that ancestor reorders them, so the model reads the author's requests in an
/// order the author never wrote them in.
#[tokio::test]
async fn author_notes_keep_the_order_the_author_wrote_them_in() {
    let runtime = runtime(vec![
        outline_step(),
        insert_step("第一段。"),
        insert_step("第二段。"),
        insert_step("第三段。"),
        insert_step("第四段。"),
        ScriptStep::new().tool("finish_draft", json!({})),
        ScriptStep::new().tool("finish_draft", json!({})),
    ])
    .await;
    let session_id = start_writer(&runtime, Granularity::Paragraph, true, false).await;

    // 1. Steering while the first insert is held; the resume's sealing consumes it.
    let first = agent_write_harness::wait_for_status(&runtime, session_id, "held").await;
    let mut held_id = first.held_tool_call_id.expect("held id");
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/message"),
            json!({ "text": "第一条指令：多写雨。" }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = agent_write_harness::resume_raw(&runtime, session_id, None).await;
    assert_eq!(status, 200, "{body}");

    // 2. Feedback on the second hold: stored in that call's appendices.
    let second = agent_write_harness::wait_for_new_hold(&runtime, session_id, held_id).await;
    held_id = second.held_tool_call_id.expect("held id");
    let (status, body) =
        agent_write_harness::resume_raw(&runtime, session_id, Some("第二条意见：少写风。")).await;
    assert_eq!(status, 200, "{body}");

    // 3. Steering again on the third hold, consumed by that resume's sealing.
    let third = agent_write_harness::wait_for_new_hold(&runtime, session_id, held_id).await;
    held_id = third.held_tool_call_id.expect("held id");
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/message"),
            json!({ "text": "第三条指令：写城楼。" }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = agent_write_harness::resume_raw(&runtime, session_id, None).await;
    assert_eq!(status, 200, "{body}");

    // 4. Push the session near the limit and let the next boundary roll over.
    agent_write_harness::wait_for_new_hold(&runtime, session_id, held_id).await;
    push_usage_near_the_limit(&runtime, session_id).await;
    let (status, body) = agent_write_harness::resume_raw(&runtime, session_id, None).await;
    assert_eq!(status, 200, "{body}");
    runtime
        .wait_status(session_id, SessionStatus::Superseded)
        .await;
    let parent = agent_write_harness::session_dto(&runtime, session_id).await;
    let child_id = parent["child_session_id"].as_i64().expect("a child") as i32;

    let kickoff = runtime.messages(child_id).await[0].text();
    let first_at = kickoff.find("第一条指令：多写雨。");
    let second_at = kickoff.find("第二条意见：少写风。");
    let third_at = kickoff.find("第三条指令：写城楼。");
    let (Some(first_at), Some(second_at), Some(third_at)) = (first_at, second_at, third_at) else {
        panic!("all three notes are present: {kickoff}");
    };
    assert!(
        first_at < second_at && second_at < third_at,
        "author_notes keep the author's order (first {first_at}, second {second_at}, \
         third {third_at}): {kickoff}"
    );
}
