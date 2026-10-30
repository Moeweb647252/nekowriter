//! Goal proposals end to end (PLAN M13 13.6, spec/06 §5, §11.6).

use nekowriter::testing::TestApp;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

async fn setup() -> (TestApp, i32, i32, i32) {
    let app = TestApp::new().await;
    let book = app.create_book("Goal proposals").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track") as i32;
    let chapter = app.create_chapter(track_id, "Target").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter") as i32;
    (app, book_id, track_id, chapter_id)
}

async fn goal(app: &TestApp, book_id: i32, body: Value) -> Value {
    let (status, body) = app.post(&format!("/api/books/{book_id}/goals"), body).await;
    assert_eq!(status, 201, "goal: {body}");
    body
}

async fn insert_proposal(
    app: &TestApp,
    book_id: i32,
    target_type: &str,
    target_id: Option<i32>,
    edits: Value,
    chapter_id: i32,
) -> i32 {
    let edits = serde_json::to_string(&edits)
        .expect("edits")
        .replace('\'', "''");
    let target = target_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "NULL".to_owned());
    let now = chrono::Utc::now().to_rfc3339();
    app.execute(&format!(
        "INSERT INTO setting_proposals(book_id, target_type, target_kind, target_id, edits, \
         rationale, status, chapter_id, session_id, created_at) VALUES \
         ({book_id}, '{target_type}', NULL, {target}, '{edits}', '理由', 'pending', \
          {chapter_id}, NULL, '{now}')"
    ))
    .await;
    let row = app
        .db()
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT max(id) AS value FROM setting_proposals".to_owned(),
        ))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i32>("", "value").expect("id")
}

#[tokio::test]
async fn accepting_complete_changes_the_node_and_conflicts_after_an_edit() {
    let (app, book_id, _track, chapter_id) = setup().await;
    let node = goal(
        &app,
        book_id,
        json!({ "level": "goal", "title": "G", "purpose": "P" }),
    )
    .await;
    let proposal = insert_proposal(
        &app,
        book_id,
        "goal_node",
        Some(node["id"].as_i64().unwrap() as i32),
        json!({ "kind": "set_goal_status", "status": "done" }),
        chapter_id,
    )
    .await;
    let (status, body) = app
        .post(&format!("/api/proposals/{proposal}/accept"), json!({}))
        .await;
    assert_eq!(status, 200, "accept: {body}");
    assert_eq!(body["status"], "accepted");
    let (_, list) = app.get(&format!("/api/books/{book_id}/goals")).await;
    assert_eq!(list[0]["status"], "done");

    let second = goal(
        &app,
        book_id,
        json!({ "level": "goal", "title": "H", "purpose": "P" }),
    )
    .await;
    let proposal = insert_proposal(
        &app,
        book_id,
        "goal_node",
        Some(second["id"].as_i64().unwrap() as i32),
        json!({ "kind": "set_goal_status", "status": "done" }),
        chapter_id,
    )
    .await;
    let (status, _) = app
        .patch(
            &format!("/api/goals/{}", second["id"]),
            json!({ "title": "Changed" }),
        )
        .await;
    assert_eq!(status, 200);
    let (status, body) = app
        .post(&format!("/api/proposals/{proposal}/accept"), json!({}))
        .await;
    assert_eq!(status, 200, "conflict response: {body}");
    assert_eq!(body["status"], "conflict");
}

#[tokio::test]
async fn accepting_a_close_with_open_children_requires_confirmation() {
    let (app, book_id, _track, chapter_id) = setup().await;
    let parent = goal(
        &app,
        book_id,
        json!({ "level": "goal", "title": "Parent", "purpose": "P" }),
    )
    .await;
    goal(
        &app,
        book_id,
        json!({ "level": "plan", "parent_id": parent["id"], "title": "Open", "purpose": "P" }),
    )
    .await;
    let proposal = insert_proposal(
        &app,
        book_id,
        "goal_node",
        Some(parent["id"].as_i64().unwrap() as i32),
        json!({ "kind": "set_goal_status", "status": "done" }),
        chapter_id,
    )
    .await;
    let (status, body) = app
        .post(&format!("/api/proposals/{proposal}/accept"), json!({}))
        .await;
    assert_eq!(status, 409, "open children: {body}");
    assert_eq!(body["error"]["code"], "goal.open_children");
    let (status, body) = app
        .post(
            &format!("/api/proposals/{proposal}/accept"),
            json!({ "confirm_open_children": true }),
        )
        .await;
    assert_eq!(status, 200, "confirmed: {body}");
    assert_eq!(body["status"], "accepted");
}

#[tokio::test]
async fn accepting_a_new_task_creates_it_once_and_drops_vanished_links() {
    let (app, book_id, _track, chapter_id) = setup().await;
    let plan = goal(
        &app,
        book_id,
        json!({ "level": "plan", "title": "Plan", "purpose": "P" }),
    )
    .await;
    let task = json!({
        "parent_id": plan["id"], "title": "Task", "purpose": "Task purpose",
        "chapter_from": 1, "chapter_to": null,
        "entity_ids": [9999], "event_ids": [9999], "foreshadowing_ids": [9999],
        "track_id": null
    });
    let proposal = insert_proposal(
        &app,
        book_id,
        "new_goal_task",
        Some(plan["id"].as_i64().unwrap() as i32),
        json!({ "kind": "create_goal_task", "task": task }),
        chapter_id,
    )
    .await;
    let (status, body) = app
        .post(&format!("/api/proposals/{proposal}/accept"), json!({}))
        .await;
    assert_eq!(status, 200, "accept create: {body}");
    assert_eq!(body["status"], "accepted");
    let (_, list) = app.get(&format!("/api/books/{book_id}/goals")).await;
    let created: Vec<&Value> = list
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["title"] == "Task")
        .collect();
    assert_eq!(created.len(), 1, "exactly one task: {list}");
    assert_eq!(created[0]["parent_id"], plan["id"]);
    assert!(created[0]["entities"].as_array().unwrap().is_empty());

    let (status, body) = app
        .post(&format!("/api/proposals/{proposal}/accept"), json!({}))
        .await;
    assert_eq!(status, 409, "second accept: {body}");
    assert_eq!(body["error"]["code"], "proposal.invalid_state");
}

mod review_harness;

use std::time::Duration;

use nekowriter::db::json::SessionStatus;
use nekowriter_llm::mock::ScriptStep;
use review_harness::runtime;

fn step(name: &str, arguments: serde_json::Value) -> ScriptStep {
    ScriptStep::new().tool(name, arguments)
}

#[tokio::test]
async fn propose_goal_change_is_available_and_dedupes_a_pending_status_proposal() {
    let runtime = runtime(vec![
        step(
            "propose_goal_change",
            json!({ "action": "complete", "node": 1, "rationale": "本章完成了目标。" }),
        ),
        step(
            "propose_goal_change",
            json!({ "action": "complete", "node": 1, "rationale": "本章完成了目标。" }),
        ),
        step("finish_maintenance", json!({ "report": "完成" })),
    ])
    .await;
    let (_, book) = runtime
        .app
        .get(&format!("/api/books/{}", runtime.book_id))
        .await;
    let mut settings = book["settings"].clone();
    settings["canon_systems"] = json!({
        "entities": false, "timeline": false, "memory": false,
        "foreshadowing": false, "goals": true
    });
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/books/{}", runtime.book_id),
            json!({ "settings": settings }),
        )
        .await;
    assert_eq!(status, 200, "settings: {body}");
    let (status, goal) = runtime
        .app
        .post(
            &format!("/api/books/{}/goals", runtime.book_id),
            json!({ "level": "task", "title": "目标", "purpose": "目的", "chapter_from": 1 }),
        )
        .await;
    assert_eq!(status, 201, "goal: {goal}");
    let goal_id = goal["id"].as_i64().unwrap() as i32;
    review_harness::prepare_in_review(&runtime).await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/approve", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "approve: {body}");
    let session = body["maintenance_session_id"].as_i64().expect("session") as i32;
    runtime
        .wait_status_within(session, SessionStatus::Completed, Duration::from_secs(20))
        .await;
    let calls = runtime.calls(session).await;
    let goal_calls: Vec<_> = calls
        .iter()
        .filter(|call| call.name == "propose_goal_change")
        .collect();
    assert_eq!(goal_calls.len(), 2);
    assert!(
        goal_calls.iter().all(|call| !call.is_error),
        "calls: {calls:?}"
    );
    let (_, pending) = runtime
        .app
        .get(&format!(
            "/api/books/{}/proposals?status=pending",
            runtime.book_id
        ))
        .await;
    assert_eq!(pending.as_array().unwrap().len(), 1, "dedupe: {pending}");
    assert_eq!(pending[0]["target_type"], "goal_node");
    assert_eq!(pending[0]["target_id"], goal_id);
}

#[tokio::test]
async fn propose_goal_change_creates_a_task_proposal_under_a_plan() {
    let runtime = runtime(vec![
        step(
            "propose_goal_change",
            json!({
                "action": "create", "parent": 1, "title": "新任务",
                "purpose": "新任务目的", "chapter_from": 1, "rationale": "新机会。"
            }),
        ),
        step("finish_maintenance", json!({ "report": "完成" })),
    ])
    .await;
    let (_, book) = runtime
        .app
        .get(&format!("/api/books/{}", runtime.book_id))
        .await;
    let mut settings = book["settings"].clone();
    settings["canon_systems"] = json!({
        "entities": false, "timeline": false, "memory": false,
        "foreshadowing": false, "goals": true
    });
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/books/{}", runtime.book_id),
            json!({ "settings": settings }),
        )
        .await;
    assert_eq!(status, 200, "settings: {body}");
    let plan = goal(
        &runtime.app,
        runtime.book_id,
        json!({ "level": "plan", "title": "计划", "purpose": "计划目的" }),
    )
    .await;
    review_harness::prepare_in_review(&runtime).await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/approve", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "approve: {body}");
    let session = body["maintenance_session_id"].as_i64().expect("session") as i32;
    runtime
        .wait_status_within(session, SessionStatus::Completed, Duration::from_secs(20))
        .await;
    let calls = runtime.calls(session).await;
    let call = calls
        .iter()
        .find(|call| call.name == "propose_goal_change")
        .expect("proposal call");
    assert!(!call.is_error, "proposal call: {call:?}");
    let (_, pending) = runtime
        .app
        .get(&format!(
            "/api/books/{}/proposals?status=pending",
            runtime.book_id
        ))
        .await;
    assert_eq!(pending.as_array().unwrap().len(), 1, "pending: {pending}");
    assert_eq!(pending[0]["target_type"], "new_goal_task");
    assert_eq!(pending[0]["target_id"], plan["id"]);
    let proposal_id = pending[0]["id"].as_i64().unwrap() as i32;
    let (status, body) = runtime
        .app
        .post(&format!("/api/proposals/{proposal_id}/accept"), json!({}))
        .await;
    assert_eq!(status, 200, "accept: {body}");
    assert_eq!(body["status"], "accepted");
    let (_, list) = runtime
        .app
        .get(&format!("/api/books/{}/goals", runtime.book_id))
        .await;
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .any(|row| row["title"] == "新任务"),
        "task missing: {list}"
    );
}
