//! Conception tool tests (PLAN M9.1, spec/05 §7, spec/03 §5.6, §7).
//!
//! The tests drive a real conception session through the production API with a
//! `ScriptedProvider`, so every assertion reads the persisted rows the spec names.

#[path = "conception_harness/mod.rs"]
mod harness;

use harness::*;
use serde_json::json;

/// The M9.1 named flow: ask → answer → write → review → revise (spec/05 §7).
#[tokio::test]
async fn conception_flow_ask_answer_write_review_revise() {
    let runtime = runtime(vec![
        ask_step(),
        tools_step(&[
            (
                "write_setting_doc",
                json!({ "doc": "outline", "content": "# 大纲\n\n第一幕：入城。" }),
            ),
            (
                "write_setting_doc",
                json!({ "doc": "world", "content": "# 世界观\n\n- 剑气折损心脉。" }),
            ),
            (
                "write_setting_doc",
                json!({ "doc": "protagonist", "content": "# 主角\n\n沈砚，寡言。" }),
            ),
            (
                "upsert_character",
                json!({ "name": "沈砚", "role_label": "主角", "summary": "游侠", "profile": "# 沈砚" }),
            ),
            (
                "upsert_character",
                json!({ "name": "柳照", "role_label": "女主", "summary": "副使", "profile": "# 柳照" }),
            ),
            (
                "set_research_topics",
                json!({
                    "topics": [
                        { "topic": "边城城防", "requirements": "城门与轮值", "scope": "背景设定" },
                        { "topic": "剑器养护", "requirements": "剑鞘与缠绳", "scope": "器物考" },
                    ],
                }),
            ),
        ]),
        review_step("请审阅这一版设定。"),
        tool_step(
            "edit_setting_doc",
            json!({ "doc": "world", "edits": [{ "find": "剑气折损心脉", "replace": "剑气折损心脉与寿元" }] }),
        ),
        text_step("设定已经整理好了。"),
    ])
    .await;
    let book = create_conceiving_book(&runtime, "Conception Flow").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("conception session") as i32;
    assert_eq!(book["book"]["status"], "conceiving");

    // Step 0: the agent asks two questions and the session awaits the form.
    let call_id = wait_for_form(&runtime, session_id).await;
    let ask = runtime.call_by_id(call_id).await;
    assert_eq!(ask.name, "ask_user");
    // The call is `awaiting_input` (not `done`): `ask_user` sets both the effect and the
    // outcome's `await_input`, and the latter is what defers the call (spec/05 §7).
    assert_eq!(ask.status, "awaiting_input");
    assert!(!ask.sealed, "the answer is what seals it");
    let ui = call_ui(&runtime, call_id).await;
    let questions = ui["questions"].as_array().cloned().unwrap_or_default();
    assert_eq!(
        questions.len(),
        2,
        "the full question list reaches the form: {ui}"
    );
    assert_eq!(questions[0]["options"].as_array().map(Vec::len), Some(2));

    // A malformed answer is rejected: the ids/labels are validated against the recorded call.
    let (status, body) = answer(
        &runtime,
        session_id,
        call_id,
        json!({ "answers": [{ "id": "tone", "selected": ["不存在的选项"] }] }),
    )
    .await;
    assert_eq!(
        status, 400,
        "an unknown option is a validation error: {body}"
    );
    assert_eq!(body["error"]["code"], "validation.failed");
    assert_eq!(
        runtime.session_row(session_id).await.status,
        "awaiting_input"
    );

    // Step 1: the answers unlock the writes; step 2 then asks for review.
    answer_ok(&runtime, session_id, call_id, ask_answer()).await;
    wait_for_sealed(&runtime, session_id, "ask_user").await;
    let review_call = wait_for_form(&runtime, session_id).await;

    // The three documents, the two ranked designs and the draft topics are persisted.
    for (kind, expected) in [
        ("outline", "# 大纲"),
        ("world", "# 世界观"),
        ("protagonist", "# 主角"),
    ] {
        let (content, revision, _) = setting_doc(&runtime, book_id, kind).await;
        assert!(content.contains(expected), "{kind}: {content}");
        assert_eq!(revision, 1, "{kind} starts at revision 1 after one write");
    }
    assert_eq!(
        characters(&runtime, book_id).await,
        vec![("沈砚".to_owned(), 1), ("柳照".to_owned(), 2)]
    );
    let topics = research_tasks(&runtime, book_id).await;
    assert_eq!(topics.len(), 2);
    assert_eq!(
        topics[0].2, "draft",
        "a conceiving book's topics stay drafts"
    );
    assert_eq!(topics[0].1, "背景设定");
    // Every write recorded its change row with `actor: agent` (spec/05 §7).
    let changes = session_changes(&runtime, session_id).await;
    assert_eq!(
        changes.iter().filter(|row| row.2 == "agent").count(),
        5,
        "three documents and two designs: {changes:?}"
    );
    assert!(changes.iter().any(|row| row.0 == "setting_doc"));
    assert!(changes.iter().any(|row| row.0 == "character_design"));

    // Step 2 already awaited: request_review carries its message for the form.
    let review = runtime.call_by_id(review_call).await;
    assert_eq!(review.name, "request_review");
    assert_eq!(review.status, "awaiting_input");
    assert!(
        review.input.contains("请审阅这一版设定"),
        "the message is recorded for the form: {}",
        review.input
    );

    // The author approves everything but the world document (step 3 revises it).
    answer_ok(
        &runtime,
        session_id,
        review_call,
        json!({
            "docs": {
                "outline": { "approved": true },
                "world": { "approved": false, "feedback": "世界观再具体一些。" },
                "protagonist": { "approved": true },
                "characters": { "approved": true },
                "research_topics": { "approved": true },
            },
            "general": "整体方向可以。",
        }),
    )
    .await;

    // The approval bookkeeping (spec/05 §7): approved documents record their revision.
    let (_, outline_revision, outline_approved) = setting_doc(&runtime, book_id, "outline").await;
    assert_eq!(outline_approved, Some(outline_revision));
    let (_, _, world_approved) = setting_doc(&runtime, book_id, "world").await;
    assert_eq!(world_approved, None, "a rejected document is not approved");
    let state = conception_state(&runtime, book_id).await;
    assert_eq!(state.characters_revision, 2);
    assert_eq!(state.characters_approved_revision, Some(2));
    assert_eq!(state.topics_revision, 1);
    assert_eq!(state.topics_approved_revision, Some(1));

    // The answer's rendered result names the review outcome for the model.
    let sealed = wait_for_sealed(&runtime, session_id, "request_review").await;
    let result = sealed.result_text.clone().unwrap_or_default();
    assert!(
        result.contains("需要修改"),
        "the model sees the verdict: {result}"
    );
    assert!(
        result.contains("世界观再具体一些"),
        "and the feedback: {result}"
    );
    assert!(
        result.contains("整体方向可以"),
        "and the general comment: {result}"
    );

    // Step 3 revises the document, then the closing turn idles the session (spec/03 §5.6).
    let (world, revision, _) = wait_until_setting_doc(&runtime, book_id, "world", 2).await;
    assert!(world.contains("寿元"), "the revision landed: {world}");
    assert_eq!(revision, 2);

    let row = runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Idle)
        .await;
    assert_eq!(row.status, "idle", "a text-only conception turn idles");
    assert!(
        row.error.is_none(),
        "idling is not a failure: {:?}",
        row.error
    );
}

/// Waits until a document reaches a revision, returning its content.
async fn wait_until_setting_doc(
    runtime: &Runtime,
    book_id: i32,
    kind: &str,
    revision: i32,
) -> (String, i32, Option<i32>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let doc = setting_doc(runtime, book_id, kind).await;
        if doc.1 >= revision {
            return doc;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{kind} never reached revision {revision}; it is at {}",
            doc.1
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// The M9.1 named edit-report test: the author edits a document and a profile while the review
/// form is waiting, and the answer's sealed result carries `notice.setting_edits` (spec/03 §7).
#[tokio::test]
async fn author_edits_while_awaiting_review_are_reported_in_the_answer_result() {
    let runtime = runtime(vec![
        tools_step(&[
            (
                "write_setting_doc",
                json!({ "doc": "outline", "content": "# 大纲\n\n第一幕：入城。\n第二幕：登楼。" }),
            ),
            (
                "upsert_character",
                json!({ "name": "沈砚", "profile": "# 沈砚\n\n- 佩剑承影。" }),
            ),
        ]),
        review_step("请看这一版。"),
        text_step("好的，我继续。"),
    ])
    .await;
    // The scripted first turn runs without an ask; the session still starts by creating the book.
    let book = create_conceiving_book(&runtime, "Edit Reports").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = conception_session_of_book(&runtime, book_id).await;

    // The first turn writes; the second asks for review.
    let review_call = wait_for_form(&runtime, session_id).await;
    let (_, outline_revision, _) = setting_doc(&runtime, book_id, "outline").await;
    assert_eq!(outline_revision, 1);

    // The author edits the outline and the character profile while the form waits: allowed, and
    // remembered as a view snapshot (spec/08 §1.4, spec/03 §7).
    let (status, body) = runtime
        .app
        .put(
            &format!("/api/books/{book_id}/setting-docs/outline"),
            json!({
                "content": "# 大纲\n\n第一幕：雨夜入城。\n第二幕：登楼。",
                "base_revision": outline_revision,
                "note": "",
            }),
        )
        .await;
    assert_eq!(
        status, 200,
        "an edit while awaiting the form is allowed: {body}"
    );
    let (_, chars) = runtime
        .app
        .get(&format!("/api/books/{book_id}/characters"))
        .await;
    let character_id = chars[0]["id"].as_i64().expect("character id") as i32;
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/characters/{character_id}"),
            json!({ "profile": "# 沈砚\n\n- 佩剑承影，剑鞘有裂痕。", "note": "" }),
        )
        .await;
    assert_eq!(
        status, 200,
        "a profile edit while awaiting the form is allowed: {body}"
    );

    answer_ok(
        &runtime,
        session_id,
        review_call,
        json!({
            "docs": {
                "outline": { "approved": true },
                "world": { "approved": true },
                "protagonist": { "approved": true },
                "characters": { "approved": true },
                "research_topics": { "approved": true },
            },
        }),
    )
    .await;

    // The answer's result carries the edit report: the sealed text of the answered call.
    let sealed = wait_for_sealed(&runtime, session_id, "request_review").await;
    let result = sealed.result_text.clone().unwrap_or_default();
    assert!(
        result.contains("【用户修改了设定】"),
        "the edit report is appended to the answer: {result}"
    );
    assert!(
        result.contains("雨夜入城"),
        "the document diff shows the author's text: {result}"
    );
    assert!(
        result.contains("剑鞘有裂痕"),
        "the changed profile is shown in full: {result}"
    );
    // The reported snapshot is cleared and replaced by a fresh baseline when the turn ends
    // (spec/03 §7): what the agent now "has seen" is the author's edited text, not the old one.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let row = runtime.session_row(session_id).await;
        if row.status == "idle" {
            let snapshot = row
                .view_snapshot
                .as_deref()
                .expect("the ending turn stores what the agent last saw");
            let snapshot: serde_json::Value =
                serde_json::from_str(snapshot).expect("a tagged view snapshot");
            assert_eq!(snapshot["kind"], "conception");
            let outline = snapshot["docs"]["outline"]["content"]
                .as_str()
                .unwrap_or_default();
            assert!(
                outline.contains("雨夜入城"),
                "the new baseline carries the reported edit: {snapshot}"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session never settled; it is {}",
            row.status
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Every error a conception tool returns must render a non-empty, distinctive text for the model
/// (the renderer is strict; a missing template variable would yield an empty error).
#[tokio::test]
async fn conception_tool_errors_render_for_the_model() {
    let runtime = runtime(vec![
        tool_step(
            "write_setting_doc",
            json!({ "doc": "world", "content": "# 世界观\n\n剑气折损心脉。" }),
        ),
        // `find` matches zero times → error.find_failed (doc_label + find + count).
        tool_step(
            "edit_setting_doc",
            json!({ "doc": "world", "edits": [{ "find": "不存在的原文", "replace": "x" }] }),
        ),
        // `find` matches twice → error.find_failed with count 2.
        tool_step(
            "write_setting_doc",
            json!({ "doc": "world", "content": "# 世界观\n\n规则一。\n规则一。" }),
        ),
        tool_step(
            "edit_setting_doc",
            json!({ "doc": "world", "edits": [{ "find": "规则一", "replace": "规则二" }] }),
        ),
        tool_step("remove_character", json!({ "name": "不存在的人物" })),
        tool_step("reorder_characters", json!({ "names": ["张三"] })),
        // The validation branch loops `errors`: it must be a list, never a stringified object.
        tool_step("ask_user", json!({ "questions": [] })),
        text_step("结束。"),
    ])
    .await;
    let book = create_conceiving_book(&runtime, "Tool Errors").await;
    let session_id = book["session_id"].as_i64().expect("conception session") as i32;
    let _ = runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Idle)
        .await;

    let calls = runtime.calls(session_id).await;
    let error_calls: Vec<&CallRow> = calls.iter().filter(|call| call.is_error).collect();
    assert!(
        error_calls.len() >= 4,
        "the scripted errors all ran: {calls:?}"
    );
    for call in &error_calls {
        let text = call.result_text.clone().unwrap_or_default();
        assert!(
            !text.trim().is_empty(),
            "{} rendered an empty error (the strict renderer threw)",
            call.name
        );
        assert!(
            !text.contains("{{"),
            "{} left an unrendered variable: {text}",
            call.name
        );
        assert!(
            !text.contains("内部错误"),
            "{} fell through to the internal-error branch: {text}",
            call.name
        );
    }

    let find_failed: Vec<&CallRow> = error_calls
        .iter()
        .copied()
        .filter(|call| call.name == "edit_setting_doc")
        .collect();
    assert_eq!(find_failed.len(), 2, "{calls:?}");
    let zero = find_failed[0].result_text.clone().unwrap_or_default();
    assert!(
        zero.contains("世界观"),
        "the doc label is localized: {zero}"
    );
    assert!(
        zero.contains("不存在的原文"),
        "the queried text is quoted: {zero}"
    );
    assert!(zero.contains("找不到"), "count 0 says not found: {zero}");
    let twice = find_failed[1].result_text.clone().unwrap_or_default();
    assert!(twice.contains("规则一"), "{twice}");
    assert!(twice.contains('2'), "count 2 is reported: {twice}");

    let reorder = calls
        .iter()
        .find(|call| call.name == "reorder_characters")
        .expect("reorder call")
        .result_text
        .clone()
        .unwrap_or_default();
    assert!(!reorder.trim().is_empty(), "reorder_mismatch renders");

    let validation = calls
        .iter()
        .find(|call| call.name == "ask_user" && call.is_error)
        .expect("the empty questions list is rejected")
        .result_text
        .clone()
        .unwrap_or_default();
    assert!(
        validation.contains("minItems") || validation.contains("questions"),
        "the schema error is listed: {validation}"
    );
}

/// Writes emit `settings.changed` for the studio's live documents (spec/05 §7, spec/10 §3).
#[tokio::test]
async fn conception_writes_emit_settings_events() {
    let runtime = runtime(vec![
        tools_step(&[
            (
                "write_setting_doc",
                json!({ "doc": "outline", "content": "# 大纲\n\n第一幕。" }),
            ),
            (
                "upsert_character",
                json!({ "name": "沈砚", "role_label": "主角" }),
            ),
            (
                "set_research_topics",
                json!({ "topics": [{ "topic": "城门规制", "requirements": "细节", "scope": "背景" }] }),
            ),
        ]),
        text_step("完成。"),
    ])
    .await;
    let mut receiver = runtime.app.events().subscribe();
    let book = create_conceiving_book(&runtime, "Events").await;
    let session_id = book["session_id"].as_i64().expect("conception session") as i32;
    let _ = runtime
        .wait_status(session_id, nekowriter::db::json::SessionStatus::Idle)
        .await;

    let mut kinds = Vec::new();
    while let Ok(envelope) =
        tokio::time::timeout(std::time::Duration::from_millis(50), receiver.recv()).await
    {
        if let Ok(envelope) = envelope
            && let nekowriter::events::ServerEvent::SettingsChanged { kind, .. } = envelope.event
        {
            kinds.push(kind);
        }
    }
    assert!(
        kinds.contains(&nekowriter::events::SettingsKind::SettingDoc),
        "a document write emits settings.changed(setting_doc): {kinds:?}"
    );
    assert!(
        kinds.contains(&nekowriter::events::SettingsKind::Characters),
        "a character write emits settings.changed(characters): {kinds:?}"
    );
    assert!(
        kinds.contains(&nekowriter::events::SettingsKind::ResearchTopics),
        "a topics write emits settings.changed(research_topics): {kinds:?}"
    );
}
