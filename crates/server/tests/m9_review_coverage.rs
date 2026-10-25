//! M9 review coverage: the spec paths the M9 suites did not pin.
//!
//! These are not defect reproductions; they are the spec/05 §9 obligations ("each tool: a happy
//! path, each documented error, and the `ui` payload shape") and the locking matrix of spec/08
//! §1.4 that the milestone's own tests left implicit.

#[path = "conception_harness/mod.rs"]
mod harness;

use std::time::Duration;

use harness::*;
use nekowriter::db::json::SessionStatus;
use nekowriter_llm::mock::ScriptStep;
use serde_json::json;

/// spec/03 §5.4: feedback on a held call whose effects complete the session revokes the
/// completion — the session keeps working instead of ending, the chapter stays out of review and
/// the call's result carries the revoke notice before the feedback.
#[tokio::test]
async fn feedback_revokes_a_held_completion() {
    let runtime = runtime(vec![
        tool_step(
            "test_write",
            json!({ "text": "a paragraph", "complete": true }),
        ),
        text_step("the session should never ask again"),
    ])
    .await;
    let session_id = runtime
        .start_debug_with(nekowriter::db::json::Granularity::Paragraph, true, false)
        .await;
    let _held = runtime.wait_status(session_id, SessionStatus::Held).await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/resume"),
            json!({ "feedback": "继续写，不要结束。" }),
        )
        .await;
    assert_eq!(status, 200, "resume with feedback: {body}");

    // The session keeps working (not completed) and the completion's domain effects did not run.
    assert_eq!(
        body["status"], "running",
        "resume revokes, it does not end: {body}"
    );
    let call = wait_for_sealed(&runtime, session_id, "test_write").await;
    assert!(
        runtime.version_kinds().await.is_empty(),
        "no draft version was saved by the revoked completion"
    );
    let (chapter_status, _, _) = runtime.chapter_status().await;
    assert_ne!(
        chapter_status, "in_review",
        "the chapter stays out of review"
    );
    assert_ne!(
        runtime.session_row(session_id).await.status,
        "completed",
        "the revoked completion did not end the session"
    );
    let text = call.result_text.clone().unwrap_or_default();
    assert_eq!(call.status, "done");
    assert!(
        text.contains("【系统通知】作者在你完成之前给出了反馈"),
        "the revoke notice is appended: {text}"
    );
    assert!(
        text.contains("继续写，不要结束"),
        "the feedback follows: {text}"
    );
    assert!(
        call.effects().complete.is_none(),
        "the revoked completion is not applied"
    );
}

/// spec/05 §7: the answer payload is validated against the question ids and labels. A bad id, a
/// bad label, several selections on a single-choice question and free text on a question without
/// "Other" are all `validation.failed`, and none of them finalizes the call.
#[tokio::test]
async fn ask_user_rejects_every_malformed_answer() {
    let ask = tool_step(
        "ask_user",
        json!({
            "questions": [
                {
                    "id": "tone",
                    "question": "基调？",
                    "options": [{ "label": "热血" }, { "label": "沉郁" }],
                },
                {
                    "id": "ending",
                    "question": "结局？",
                    "options": [{ "label": "开放" }, { "label": "悲剧" }],
                    "allow_other": false,
                },
            ],
        }),
    );
    let runtime = runtime(vec![ask, text_step("好的。")]).await;
    let book = create_conceiving_book(&runtime, "Answer Validation").await;
    let session_id = book["session_id"].as_i64().expect("session") as i32;
    let call_id = wait_for_form(&runtime, session_id).await;

    for (payload, reason) in [
        (
            json!({ "answers": [{ "id": "nope", "selected": [] }] }),
            "unknown id",
        ),
        (
            json!({ "answers": [{ "id": "tone", "selected": ["不存在"] }] }),
            "unknown label",
        ),
        (
            json!({ "answers": [{ "id": "tone", "selected": ["热血", "沉郁"] }] }),
            "several selections on a single-choice question",
        ),
        (
            json!({ "answers": [{ "id": "ending", "selected": [], "other": "自定义" }] }),
            "free text without allow_other",
        ),
    ] {
        let (status, body) = answer(&runtime, session_id, call_id, payload).await;
        assert_eq!(status, 400, "{reason}: {body}");
        assert_eq!(
            body["error"]["code"], "validation.failed",
            "{reason}: {body}"
        );
        assert!(
            !body["error"]["details"]["errors"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .is_empty(),
            "{reason}: the errors list explains it: {body}"
        );
        assert_eq!(
            runtime.session_row(session_id).await.status,
            "awaiting_input",
            "{reason}: the form still waits"
        );
    }

    // A valid answer finalizes it.
    answer_ok(
        &runtime,
        session_id,
        call_id,
        json!({
            "answers": [
                { "id": "tone", "selected": ["沉郁"] },
                { "id": "ending", "selected": ["开放"] },
            ],
        }),
    )
    .await;
    let _ = runtime.wait_status(session_id, SessionStatus::Idle).await;
}

/// spec/05 §7: "Applies every edit atomically, or none." A failing second edit leaves the
/// document exactly as it was (same revision, same text).
#[tokio::test]
async fn edit_setting_doc_applies_all_edits_or_none() {
    let runtime = runtime(vec![
        tool_step(
            "write_setting_doc",
            json!({ "doc": "world", "content": "# 世界观\n\n规则一。\n规则二。" }),
        ),
        tool_step(
            "edit_setting_doc",
            json!({
                "doc": "world",
                "edits": [
                    { "find": "规则一", "replace": "规则甲" },
                    { "find": "不存在的原文", "replace": "x" },
                ],
            }),
        ),
        tool_step(
            "edit_setting_doc",
            json!({ "doc": "world", "edits": [{ "find": "规则一", "replace": "规则甲" }] }),
        ),
        text_step("完成。"),
    ])
    .await;
    let book = create_conceiving_book(&runtime, "Atomic Edits").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("session") as i32;
    let _ = runtime.wait_status(session_id, SessionStatus::Idle).await;

    let calls = runtime.calls(session_id).await;
    let edits: Vec<&CallRow> = calls
        .iter()
        .filter(|call| call.name == "edit_setting_doc")
        .collect();
    assert_eq!(edits.len(), 2);
    assert!(edits[0].is_error, "the pair with a failed find is an error");
    let (content, revision, _) = setting_doc(&runtime, book_id, "world").await;
    assert_eq!(revision, 2, "only the successful single edit applied");
    assert!(
        content.contains("规则甲") && content.contains("规则二"),
        "the first edit landed: {content}"
    );
}

/// spec/05 §7: `set_research_topics` replaces the book's tasks in status `draft` only; a task the
/// author confirmed as `ready` is kept.
#[tokio::test]
async fn set_research_topics_replaces_drafts_only() {
    let runtime = runtime(vec![
        tool_step(
            "set_research_topics",
            json!({
                "topics": [
                    { "topic": "边城城防", "requirements": "细节", "scope": "背景设定" },
                    { "topic": "剑器养护", "requirements": "细节", "scope": "器物考" },
                ],
            }),
        ),
        text_step("主题已设置。"),
        tool_step(
            "set_research_topics",
            json!({ "topics": [{ "topic": "夜禁规矩", "requirements": "细节", "scope": "背景设定" }] }),
        ),
        text_step("完成。"),
    ])
    .await;
    let book = create_conceiving_book(&runtime, "Topic Replacement").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("session") as i32;
    let _ = runtime.wait_status(session_id, SessionStatus::Idle).await;

    // The author confirms one draft, so it must survive the agent's next replacement.
    let (_, tasks) = runtime
        .app
        .get(&format!("/api/research-tasks?book_id={book_id}"))
        .await;
    let sword = tasks
        .as_array()
        .and_then(|rows| rows.iter().find(|task| task["topic"] == "剑器养护"))
        .cloned()
        .expect("the 剑器养护 topic");
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/research-tasks/{}", sword["id"]),
            json!({ "status": "ready" }),
        )
        .await;
    assert_eq!(status, 200, "confirm a draft: {body}");

    // Continue the conversation: the next tool call replaces the drafts only.
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/message"),
            json!({ "text": "继续" }),
        )
        .await;
    assert_eq!(status, 200, "message: {body}");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let tasks = research_tasks(&runtime, book_id).await;
        if tasks.iter().any(|task| task.0 == "夜禁规矩") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the replacement never ran: {tasks:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let tasks = research_tasks(&runtime, book_id).await;
    let topics: Vec<(&str, &str)> = tasks
        .iter()
        .map(|(topic, _, status)| (topic.as_str(), status.as_str()))
        .collect();
    assert!(
        topics.contains(&("剑器养护", "ready")),
        "a ready task is not a draft: {topics:?}"
    );
    assert!(
        topics.contains(&("夜禁规矩", "draft")),
        "the new draft is there: {topics:?}"
    );
    assert!(
        !topics.iter().any(|(topic, _)| *topic == "边城城防"),
        "the old draft was replaced: {topics:?}"
    );
}

/// spec/05 §7: `upsert_character` creates or updates by name and `rank` moves it (the others
/// shift); `remove_character` deletes it and closes the ranking. The `ui` payload names what
/// happened.
#[tokio::test]
async fn character_tools_rank_move_remove_and_report() {
    let runtime = runtime(vec![
        tools_step(&[
            ("upsert_character", json!({ "name": "沈砚" })),
            ("upsert_character", json!({ "name": "柳照" })),
            ("upsert_character", json!({ "name": "张三" })),
        ]),
        tool_step("upsert_character", json!({ "name": "张三", "rank": 1 })),
        tool_step("remove_character", json!({ "name": "沈砚" })),
        text_step("完成。"),
    ])
    .await;
    let book = create_conceiving_book(&runtime, "Character Tools").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("session") as i32;
    let _ = runtime.wait_status(session_id, SessionStatus::Idle).await;

    assert_eq!(
        characters(&runtime, book_id).await,
        vec![("张三".to_owned(), 1), ("柳照".to_owned(), 2)],
        "the rank move and the removal renumber the list"
    );
    let calls = runtime.calls(session_id).await;
    let moved = calls
        .iter()
        .rfind(|call| call.name == "upsert_character" && !call.is_error)
        .expect("the rank-moving upsert");
    let ui = call_ui(&runtime, moved.id).await;
    assert_eq!(ui["name"], "张三");
    assert_eq!(ui["rank"], 1, "the ui payload carries the new rank: {ui}");
    let removed = calls
        .iter()
        .find(|call| call.name == "remove_character")
        .expect("the removal");
    assert!(!removed.is_error);
    assert_eq!(
        call_ui(&runtime, removed.id).await["name"],
        "沈砚",
        "the removal reports the name"
    );
}

/// The author's character writes are locked while the conception session runs (spec/08 §1.4:
/// "character designs ... rejected (423 settings.locked) only while the book's conception session
/// is running"), and editable again while it does not.
#[tokio::test]
async fn author_character_writes_are_locked_while_the_conception_session_runs() {
    let slow = ScriptStep {
        delay: Duration::from_millis(1500),
        ..tool_step("echo", json!({ "text": "thinking" }))
    };
    let runtime = runtime(vec![slow]).await;
    let book = create_conceiving_book(&runtime, "Character Lock").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("session") as i32;
    assert_eq!(
        runtime.session_row(session_id).await.status,
        "running",
        "the slow first turn is running"
    );

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "沈砚", "role_label": "主角", "summary": "", "profile": "", "note": "" }),
        )
        .await;
    assert_eq!(status, 423, "create while running: {body}");
    assert_eq!(body["error"]["code"], "settings.locked");
    assert_eq!(body["error"]["details"]["session_id"], json!(session_id));

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/characters/reorder"),
            json!({ "ids": [] }),
        )
        .await;
    assert_eq!(status, 423, "reorder while running: {body}");
    assert_eq!(body["error"]["code"], "settings.locked");

    // Once the turn settles, the author writes again.
    let _ = runtime.wait_status(session_id, SessionStatus::Idle).await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "沈砚", "role_label": "主角", "summary": "", "profile": "", "note": "" }),
        )
        .await;
    assert_eq!(status, 201, "editable once idle: {body}");
}

/// `upsert_character`'s name-collision error renders the `character_exists` branch (spec/05 §7):
/// renaming a design onto another's name is refused and the model reads the localized text, not
/// an empty result (the strict renderer turns a missing field into an empty error).
#[tokio::test]
async fn renaming_a_character_onto_another_name_renders_character_exists() {
    let runtime = runtime(vec![
        tools_step(&[
            ("upsert_character", json!({ "name": "沈砚" })),
            ("upsert_character", json!({ "name": "柳照" })),
        ]),
        tool_step(
            "upsert_character",
            json!({ "name": "柳照", "new_name": "沈砚" }),
        ),
        text_step("完成。"),
    ])
    .await;
    let book = create_conceiving_book(&runtime, "Character Exists").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("session") as i32;
    let _ = runtime.wait_status(session_id, SessionStatus::Idle).await;

    let calls = runtime.calls(session_id).await;
    let refused = calls
        .iter()
        .rfind(|call| call.name == "upsert_character")
        .expect("the rename");
    assert!(refused.is_error, "the rename is refused");
    let text = refused
        .result_text
        .clone()
        .or(refused.raw_result_text.clone())
        .unwrap_or_default();
    assert!(
        text.contains("人物设定中已有"),
        "the character_exists branch renders: {text}"
    );
    assert!(text.contains("沈砚"), "it names the taken name: {text}");
    assert_eq!(
        characters(&runtime, book_id).await,
        vec![("沈砚".to_owned(), 1), ("柳照".to_owned(), 2)],
        "nothing changed"
    );
}
