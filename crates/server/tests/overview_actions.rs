//! The book overview: next action, attention list and usage (PLAN 11.1, spec/08 §7).
//!
//! Each test drives the real router and seeds only the state a later workflow produces (an
//! official chapter, an open session, a pending proposal), so the assertions cover the exact
//! first-match order spec/08 §7 defines.

use axum::http::StatusCode;
use nekowriter::testing::TestApp;
use serde_json::{Value, json};

/// A migrated app with an active book, its main track and one chapter.
async fn app_with_book() -> (TestApp, i32, i32, i32) {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    (app, book_id, track_id, chapter_id)
}

async fn overview(app: &TestApp, book_id: i32) -> Value {
    let (status, body) = app.get(&format!("/api/books/{book_id}/overview")).await;
    assert_eq!(status, StatusCode::OK, "overview: {body}");
    body
}

/// Makes the chapter official with the given canon status.
async fn official(app: &TestApp, chapter_id: i32, canon: &str) {
    app.execute(&format!(
        "UPDATE chapters SET status = 'official', canon_status = '{canon}' WHERE id = {chapter_id}"
    ))
    .await;
}

#[tokio::test]
async fn a_conceiving_book_continues_its_conception() {
    let (app, book_id, _, _) = app_with_book().await;
    app.execute(&format!(
        "UPDATE books SET status = 'conceiving' WHERE id = {book_id}"
    ))
    .await;
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "continue_conception");
}

#[tokio::test]
async fn a_book_with_designs_but_no_canon_seeds_it() {
    let (app, book_id, _, _) = app_with_book().await;
    let (status, character) = app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "林凡", "role_label": "主角", "summary": "the protagonist" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "character: {character}");
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "seed_canon");
}

#[tokio::test]
async fn a_canon_seed_that_completed_is_not_offered_again() {
    let (app, book_id, _, _) = app_with_book().await;
    let (status, _) = app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "林凡" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    // A completed seed session means the canonical next step is no longer seeding.
    app.execute(&format!(
        "INSERT INTO agent_sessions(book_id, role, kind, model_snapshot, system_prompt, tools, \
         status, created_at, updated_at) VALUES ({book_id}, 'archivist', 'canon_seed', '{{}}', '', \
         '[]', 'completed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    let body = overview(&app, book_id).await;
    assert_ne!(
        body["next_action"]["kind"], "seed_canon",
        "a completed seed is not repeated: {body}"
    );
}

#[tokio::test]
async fn a_chapter_in_review_is_the_next_action() {
    let (app, book_id, track_id, chapter_id) = app_with_book().await;
    app.execute(&format!(
        "UPDATE chapters SET status = 'in_review' WHERE id = {chapter_id}"
    ))
    .await;
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "review_chapter");
    assert_eq!(body["next_action"]["chapter"]["id"], chapter_id);
    assert_eq!(body["next_action"]["track_id"], track_id);
    assert_eq!(
        body["next_action"]["chapter"]["number"], 1,
        "the UI composes the label from the number (D-054)"
    );
}

#[tokio::test]
async fn a_drafting_chapter_continues_with_its_open_writer_session() {
    let (app, book_id, _, chapter_id) = app_with_book().await;
    app.set_chapter_status(chapter_id, "drafting").await;
    app.insert_writer_session(chapter_id, "held").await;
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "continue_chapter");
    assert_eq!(body["next_action"]["chapter"]["id"], chapter_id);
    let session_id = body["next_action"]["session_id"]
        .as_i64()
        .expect("the open writer session is named");
    // The session row the fixture created is the one the action points at.
    let (status, session) = app.get(&format!("/api/sessions/{session_id}")).await;
    assert_eq!(status, StatusCode::OK, "session: {session}");
}

#[tokio::test]
async fn an_unsynced_official_chapter_is_the_next_action() {
    let (app, book_id, track_id, chapter_id) = app_with_book().await;
    official(&app, chapter_id, "pending").await;
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "sync_canon");
    assert_eq!(body["next_action"]["chapter"]["id"], chapter_id);
    assert_eq!(body["next_action"]["track_id"], track_id);
}

#[tokio::test]
async fn a_finished_chapter_points_at_writing_the_next_one() {
    let (app, book_id, track_id, chapter_id) = app_with_book().await;
    official(&app, chapter_id, "synced").await;
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "write_next");
    assert_eq!(body["next_action"]["track_id"], track_id);
    assert_eq!(
        body["next_action"].get("chapter"),
        None,
        "write_next names the track, not a chapter: {body}"
    );
}

#[tokio::test]
async fn attention_lists_sessions_reviews_canon_proposals_and_research() {
    let (app, book_id, _, chapter_id) = app_with_book().await;
    // A held writer session and a failed one both need the author.
    app.insert_writer_session(chapter_id, "held").await;
    app.insert_writer_session(chapter_id, "failed").await;
    // A second chapter in review and an official chapter whose canon is stale.
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let (_, second) = app
        .post(
            &format!("/api/tracks/{track_id}/chapters"),
            json!({ "title": "Chapter Two" }),
        )
        .await;
    let second_id = second["id"].as_i64().expect("chapter id") as i32;
    app.execute(&format!(
        "UPDATE chapters SET status = 'in_review' WHERE id = {second_id}"
    ))
    .await;
    official(&app, chapter_id, "stale").await;
    // One pending proposal and one ready research task.
    app.execute(&format!(
        "INSERT INTO setting_proposals(book_id, target_type, target_kind, target_id, edits, \
         rationale, status, chapter_id, created_at) VALUES ({book_id}, 'setting_doc', 'world', 1, \
         '[]', 'why', 'pending', {chapter_id}, '2026-01-01T00:00:00Z')"
    ))
    .await;
    let (status, task) = app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book_id,
                "scope_name": "背景设定",
                "topic": "边城城防",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "research task: {task}");

    let body = overview(&app, book_id).await;
    let attention = body["attention"].as_array().expect("attention list");
    let kinds: Vec<&str> = attention
        .iter()
        .map(|item| item["kind"].as_str().expect("kind"))
        .collect();
    assert!(
        kinds.contains(&"open_session")
            && kinds.iter().filter(|k| **k == "open_session").count() == 2,
        "both open sessions are listed: {body}"
    );
    assert!(kinds.contains(&"chapter_in_review"), "{body}");
    assert!(kinds.contains(&"canon_unsynced"), "{body}");
    let proposals = attention
        .iter()
        .find(|item| item["kind"] == "pending_proposals")
        .expect("the pending proposal is counted");
    assert_eq!(proposals["count"], 1);
    let research = attention
        .iter()
        .find(|item| item["kind"] == "ready_research")
        .expect("the ready research task is counted");
    assert_eq!(research["count"], 1);
    // The library card's badge is the same number (spec/10 §2.2).
    let (_, books) = app.get("/api/books").await;
    let card = books
        .as_array()
        .expect("book list")
        .iter()
        .find(|book| book["id"] == json!(book_id))
        .expect("the book's card");
    assert_eq!(card["attention_count"], attention.len() as i64);
}

#[tokio::test]
async fn usage_sums_the_token_totals_of_the_books_sessions() {
    let (app, book_id, _, chapter_id) = app_with_book().await;
    let usage = |input: i64, output: i64| {
        format!(
            "{{\"requests\":2,\"input_tokens\":{input},\"cached_input_tokens\":0,\
             \"cache_write_tokens\":0,\"output_tokens\":{output},\"reasoning_tokens\":0,\
             \"last_prompt_tokens\":0,\"estimated\":false}}"
        )
    };
    for (status, values) in [("completed", usage(100, 10)), ("held", usage(200, 20))] {
        app.execute(&format!(
            "INSERT INTO agent_sessions(book_id, role, kind, chapter_id, track_id, model_snapshot, \
             system_prompt, tools, status, usage, created_at, updated_at) VALUES ({book_id}, \
             'writer', 'chapter_write', {chapter_id}, 1, '{{}}', '', '[]', '{status}', '{values}', \
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ))
        .await;
    }
    let body = overview(&app, book_id).await;
    assert_eq!(body["usage"]["requests"], 4, "{body}");
    assert_eq!(body["usage"]["input_tokens"], 300);
    assert_eq!(body["usage"]["output_tokens"], 30);
    assert_eq!(body["usage"]["last_prompt_tokens"], 300);
}

/// Inserts a research task in the given status and returns its id, mirroring what conception
/// leaves behind (PROJECT §8.4 F2 step 6: topics become `ready` tasks at finalize).
async fn research_task(
    app: &TestApp,
    book_id: i32,
    status: &str,
    topic: &str,
    language: &str,
) -> i32 {
    app.execute(&format!(
        "INSERT INTO research_tasks(book_id, scope_name, topic, requirements, language, status, \
         created_at, updated_at) VALUES ({book_id}, 'scope', '{topic}', '', '{language}', \
         '{status}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    let (_, body) = app
        .get(&format!("/api/research-tasks?book_id={book_id}"))
        .await;
    match body
        .as_array()
        .expect("research task list")
        .iter()
        .find(|task| task["topic"] == json!(topic))
    {
        Some(task) => task["id"].as_i64().expect("task id") as i32,
        None => panic!("the inserted task is listed: {body}"),
    }
}

/// The count of `attention[]` rows of one kind.
fn attention_count(body: &Value, kind: &str) -> i32 {
    body["attention"]
        .as_array()
        .expect("attention list")
        .iter()
        .filter(|item| item["kind"] == json!(kind))
        .map(|item| item["count"].as_i64().unwrap_or(0) as i32)
        .sum()
}

#[tokio::test]
async fn ready_research_tasks_are_the_next_action_after_seeding() {
    // The reported gap: conception → canon seed → drafting with an empty knowledge base, and
    // nothing asked for research (PROJECT §8.4 F2 step 6, D-136).
    let (app, book_id, _, chapter_id) = app_with_book().await;
    let (status, _) = app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "林凡" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    // Canon seeded, as the author's book 3 has it (43 entities).
    app.execute(&format!(
        "INSERT INTO agent_sessions(book_id, role, kind, model_snapshot, system_prompt, tools, \
         status, created_at, updated_at) VALUES ({book_id}, 'archivist', 'canon_seed', '{{}}', '', \
         '[]', 'completed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    app.set_chapter_status(chapter_id, "drafting").await;
    // Three confirmed topics, exactly what finalize turns the conception's topics into.
    for topic in ["城防", "宝钞", "宗室"] {
        research_task(&app, book_id, "ready", topic, "zh-CN").await;
    }

    let body = overview(&app, book_id).await;
    assert_eq!(
        body["next_action"]["kind"], "run_research",
        "research outranks writing the next chapter: {body}"
    );
    assert_eq!(
        body["next_action"]["ready_research"], 3,
        "the card names how many tasks are runnable: {body}"
    );
    assert_eq!(
        body["next_action"].get("research_task_id"),
        None,
        "several tasks have no single target; the author chooses on /research: {body}"
    );
    // The attention row stays: the list is the full picture, the next action is the suggestion.
    assert_eq!(attention_count(&body, "ready_research"), 3, "{body}");
}

#[tokio::test]
async fn a_single_ready_task_is_named_so_the_card_can_open_it() {
    let (app, book_id, _, _) = app_with_book().await;
    let task_id = research_task(&app, book_id, "ready", "边城城防", "zh-CN").await;
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "run_research");
    assert_eq!(body["next_action"]["ready_research"], 1);
    assert_eq!(body["next_action"]["research_task_id"], task_id);
}

#[tokio::test]
async fn draft_research_topics_are_not_runnable_and_do_not_block_writing() {
    // A `draft` topic is unconfirmed and Run is not allowed on it (spec/07 §8), so offering it as
    // the next action would be a dead end (D-136).
    let (app, book_id, track_id, chapter_id) = app_with_book().await;
    official(&app, chapter_id, "synced").await;
    research_task(&app, book_id, "draft", "未确认主题", "zh-CN").await;

    let body = overview(&app, book_id).await;
    assert_eq!(
        body["next_action"]["kind"], "write_next",
        "a draft topic never becomes the next action: {body}"
    );
    assert_eq!(body["next_action"]["track_id"], track_id);
    assert_eq!(
        attention_count(&body, "ready_research"),
        0,
        "the attention count is `ready` only as well: {body}"
    );
}

#[tokio::test]
async fn finished_research_tasks_do_not_block_writing() {
    for status in ["done", "failed", "cancelled"] {
        let (app, book_id, _, chapter_id) = app_with_book().await;
        official(&app, chapter_id, "synced").await;
        research_task(&app, book_id, status, "已结束主题", "zh-CN").await;
        let body = overview(&app, book_id).await;
        assert_eq!(
            body["next_action"]["kind"], "write_next",
            "a {status} task is not runnable, so writing may continue: {body}"
        );
        assert_eq!(attention_count(&body, "ready_research"), 0, "{body}");
    }
}

#[tokio::test]
async fn a_book_without_research_tasks_still_writes_its_next_chapter() {
    // The no-regression case: books that never collected topics keep the old answer.
    let (app, book_id, track_id, chapter_id) = app_with_book().await;
    official(&app, chapter_id, "synced").await;
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "write_next");
    assert_eq!(body["next_action"]["track_id"], track_id);
    assert_eq!(body["next_action"].get("ready_research"), None, "{body}");
    assert_eq!(body["next_action"].get("research_task_id"), None, "{body}");
}

#[tokio::test]
async fn research_outranks_a_chapter_waiting_for_review() {
    // The ordering ruling (D-136): `run_research` sits after `seed_canon` and before the chapter
    // rules, matching PROJECT §8.4 F2's sentence order ("Run research tasks" before "Write
    // chapter 1"). A chapter already in review therefore waits until the tasks are settled.
    let (app, book_id, _, chapter_id) = app_with_book().await;
    app.set_chapter_status(chapter_id, "in_review").await;
    research_task(&app, book_id, "ready", "城防", "zh-CN").await;

    let body = overview(&app, book_id).await;
    assert_eq!(
        body["next_action"]["kind"], "run_research",
        "research is ordered before the chapter rules: {body}"
    );
    // Both surfaces still report the waiting review, so nothing is hidden from the author.
    assert_eq!(attention_count(&body, "chapter_in_review"), 1, "{body}");
    assert_eq!(attention_count(&body, "ready_research"), 1, "{body}");
}

#[tokio::test]
async fn research_never_outranks_conception_or_seeding() {
    // Rules 1 and 2 keep priority: research is confirmed topics of a finalized book.
    let (app, book_id, _, _) = app_with_book().await;
    research_task(&app, book_id, "ready", "城防", "zh-CN").await;
    app.execute(&format!(
        "UPDATE books SET status = 'conceiving' WHERE id = {book_id}"
    ))
    .await;
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "continue_conception", "{body}");

    // A book that needs seeding still seeds first, even with runnable tasks.
    let (app, book_id, _, _) = app_with_book().await;
    let (status, _) = app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "林凡" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    research_task(&app, book_id, "ready", "城防", "zh-CN").await;
    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "seed_canon", "{body}");
}

// ───────────────────────────── the deferred status (D-138) ─────────────────────────────

#[tokio::test]
async fn deferred_research_tasks_do_not_block_writing() {
    // The release valve (D-138): `next_action` and `attention` count `ResearchTaskStatus::Ready`
    // only, so a task the author has deferred must drop out of both. Before the status existed the
    // only ways out of the gate were running the task or cancelling it (D-136).
    let (app, book_id, track_id, chapter_id) = app_with_book().await;
    official(&app, chapter_id, "synced").await;
    research_task(&app, book_id, "deferred", "推迟的主题", "zh-CN").await;

    let body = overview(&app, book_id).await;
    assert_eq!(
        body["next_action"]["kind"], "write_next",
        "a deferred task is not runnable, so writing continues: {body}"
    );
    assert_eq!(body["next_action"]["track_id"], track_id);
    assert_eq!(
        body["next_action"].get("ready_research"),
        None,
        "nothing is runnable, so the card names no count: {body}"
    );
    assert_eq!(
        attention_count(&body, "ready_research"),
        0,
        "an attention row demands action, and the author just said 'not now': {body}"
    );
}

#[tokio::test]
async fn only_the_ready_tasks_of_a_mixed_book_are_counted() {
    // One ready and one deferred: the ready one still gates, and the count is 1 — not 2, which is
    // what counting the whole list would have produced.
    let (app, book_id, _, chapter_id) = app_with_book().await;
    official(&app, chapter_id, "synced").await;
    let ready_id = research_task(&app, book_id, "ready", "城防", "zh-CN").await;
    research_task(&app, book_id, "deferred", "宝钞", "zh-CN").await;

    let body = overview(&app, book_id).await;
    assert_eq!(body["next_action"]["kind"], "run_research", "{body}");
    assert_eq!(body["next_action"]["ready_research"], 1, "{body}");
    assert_eq!(
        body["next_action"]["research_task_id"], ready_id,
        "exactly one task is runnable, so the card can open it: {body}"
    );
    assert_eq!(attention_count(&body, "ready_research"), 1, "{body}");
}

#[tokio::test]
async fn deferring_the_last_runnable_task_returns_the_overview_to_writing() {
    // The whole feature, end to end through the real endpoints: the overview asks for research,
    // the author defers the task instead of running it, and the same book now asks for the next
    // chapter. This is the author's own situation (8 ready tasks, a drafting chapter).
    let (app, book_id, track_id, chapter_id) = app_with_book().await;
    official(&app, chapter_id, "synced").await;
    let ids = [
        research_task(&app, book_id, "ready", "城防", "zh-CN").await,
        research_task(&app, book_id, "ready", "宝钞", "zh-CN").await,
    ];
    let before = overview(&app, book_id).await;
    assert_eq!(before["next_action"]["kind"], "run_research", "{before}");
    assert_eq!(before["next_action"]["ready_research"], 2, "{before}");

    for id in ids {
        let (status, body) = app
            .post(&format!("/api/research-tasks/{id}/defer"), json!({}))
            .await;
        assert_eq!(status, StatusCode::OK, "defer {id}: {body}");
        assert_eq!(body["status"], "deferred", "{body}");
    }

    let after = overview(&app, book_id).await;
    assert_eq!(
        after["next_action"]["kind"], "write_next",
        "nothing runnable is left, so the overview prompts writing again: {after}"
    );
    assert_eq!(after["next_action"]["track_id"], track_id);
    assert_eq!(attention_count(&after, "ready_research"), 0, "{after}");
}

#[tokio::test]
async fn a_drafting_chapter_wins_back_a_deferred_book() {
    // The author's book 3 exactly: canon seeded, a chapter `drafting`, 0 knowledge entries and
    // every research task deferred. The overview must answer `continue_chapter`, not research.
    let (app, book_id, _, chapter_id) = app_with_book().await;
    app.execute(&format!(
        "INSERT INTO agent_sessions(book_id, role, kind, model_snapshot, system_prompt, tools, \
         status, created_at, updated_at) VALUES ({book_id}, 'archivist', 'canon_seed', '{{}}', '', \
         '[]', 'completed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    app.set_chapter_status(chapter_id, "drafting").await;
    // Four deferred tasks, as many as the conception proposed in the reported database.
    for topic in [
        "洪武朝大事编年",
        "洪武朝人物关系",
        "人痘接种术",
        "男频连载结构",
    ] {
        research_task(&app, book_id, "deferred", topic, "zh-CN").await;
    }

    let body = overview(&app, book_id).await;
    assert_eq!(
        body["next_action"]["kind"], "continue_chapter",
        "the drafting chapter is the next step once research is deferred: {body}"
    );
    assert_eq!(body["next_action"]["chapter"]["id"], chapter_id);
    assert_eq!(attention_count(&body, "ready_research"), 0, "{body}");
}

#[tokio::test]
async fn a_resumed_task_gates_writing_again() {
    // Resume is the inverse: the task returns to the runnable set, so the overview asks for
    // research again. Without this, "defer" would be indistinguishable from "cancel".
    let (app, book_id, _, chapter_id) = app_with_book().await;
    official(&app, chapter_id, "synced").await;
    let task_id = research_task(&app, book_id, "ready", "城防", "zh-CN").await;
    app.post(&format!("/api/research-tasks/{task_id}/defer"), json!({}))
        .await;
    assert_eq!(
        overview(&app, book_id).await["next_action"]["kind"],
        "write_next"
    );

    let (status, body) = app
        .post(&format!("/api/research-tasks/{task_id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after = overview(&app, book_id).await;
    assert_eq!(after["next_action"]["kind"], "run_research", "{after}");
    assert_eq!(after["next_action"]["ready_research"], 1, "{after}");
}

#[tokio::test]
async fn another_books_ready_tasks_do_not_leak_into_this_book() {
    let (app, book_id, _, chapter_id) = app_with_book().await;
    official(&app, chapter_id, "synced").await;
    let other = app.create_book("Other").await;
    let other_id = other["id"].as_i64().expect("book id") as i32;
    research_task(&app, other_id, "ready", "别的书", "zh-CN").await;

    let body = overview(&app, book_id).await;
    assert_eq!(
        body["next_action"]["kind"], "write_next",
        "research tasks are scoped to their book: {body}"
    );
    assert_eq!(attention_count(&body, "ready_research"), 0, "{body}");
}
