//! Goal evaluator and scope tests (PLAN M13 13.5, spec/06 §11.3).

use nekowriter::testing::TestApp;
use serde_json::{Value, json};

async fn book_with_main(app: &TestApp) -> (i32, i32) {
    let book = app.create_book("Readiness").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    (book_id, track_id)
}

async fn create(app: &TestApp, book_id: i32, body: Value) -> Value {
    let (status, body) = app.post(&format!("/api/books/{book_id}/goals"), body).await;
    assert_eq!(status, 201, "create goal: {body}");
    body
}

async fn get(app: &TestApp, book_id: i32) -> Vec<Value> {
    let (status, body) = app.get(&format!("/api/books/{book_id}/goals")).await;
    assert_eq!(status, 200, "list goals: {body}");
    body.as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn chapter_at_least_and_foreshadowing_conditions_become_ready_without_refresh() {
    let app = TestApp::new().await;
    let (book_id, track_id) = book_with_main(&app).await;
    let chapter = app.create_chapter(track_id, "One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter") as i32;
    app.execute(&format!(
        "INSERT INTO foreshadowing(id, book_id, track_id, title, description, planned_payoff, 
         status, planted_chapter_id, resolved_chapter_id, resolution, importance, created_at, 
         updated_at) VALUES (1, {book_id}, NULL, '钩子', '', '', 'open', NULL, NULL, '', 3, 
         '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    let goal = create(
        &app,
        book_id,
        json!({
            "level": "goal", "title": "等待", "purpose": "目的", "status": "waiting",
            "conditions": [
                { "kind": "chapter_at_least", "chapter": 1 },
                { "kind": "foreshadowing_resolved", "foreshadowing_id": 1 }
            ]
        }),
    )
    .await;
    let list = get(&app, book_id).await;
    let row = list
        .iter()
        .find(|row| row["id"] == goal["id"])
        .expect("goal");
    assert_eq!(
        row["conditions"][0]["satisfied"], false,
        "a draft chapter is not approved"
    );
    assert_eq!(row["conditions"][1]["broken"], false);
    assert_eq!(row["ready"], false, "the hook is still open");

    app.set_chapter_status(chapter_id, "official").await;
    let list = get(&app, book_id).await;
    let row = list
        .iter()
        .find(|row| row["id"] == goal["id"])
        .expect("goal");
    assert_eq!(row["conditions"][0]["satisfied"], true);

    app.execute("UPDATE foreshadowing SET status = 'resolved' WHERE id = 1")
        .await;
    let list = get(&app, book_id).await;
    let row = list
        .iter()
        .find(|row| row["id"] == goal["id"])
        .expect("goal");
    assert_eq!(row["conditions"][1]["satisfied"], true);
    assert_eq!(row["ready"], true);
    assert_eq!(row["conditions"][1]["target_label"], "钩子");
}

#[tokio::test]
async fn vanished_conditions_are_broken_and_closable_uses_child_statuses() {
    let app = TestApp::new().await;
    let (book_id, _track) = book_with_main(&app).await;
    app.execute(&format!(
        "INSERT INTO entities(id, book_id, track_id, kind, name, aliases, summary, description, 
         state, attributes, importance, status, is_protagonist, first_chapter_id, last_chapter_id, 
         created_at, updated_at) VALUES (1, {book_id}, NULL, 'character', 'A', '[]', '', '', '', 
         '{{}}', 3, 'active', 0, NULL, NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    let goal = create(
        &app,
        book_id,
        json!({
            "level": "goal", "title": "断链", "purpose": "目的", "status": "waiting",
            "conditions": [{ "kind": "entity_status_is", "entity_id": 1, "status": "active" }]
        }),
    )
    .await;
    app.execute("DELETE FROM entities WHERE id = 1").await;
    let list = get(&app, book_id).await;
    let row = list
        .iter()
        .find(|row| row["id"] == goal["id"])
        .expect("goal");
    assert_eq!(row["conditions"][0]["broken"], true);
    assert_eq!(row["conditions"][0]["satisfied"], false);
    assert_eq!(row["ready"], false);

    let parent = create(
        &app,
        book_id,
        json!({ "level": "goal", "title": "父", "purpose": "目的" }),
    )
    .await;
    let child = create(
        &app,
        book_id,
        json!({ "level": "plan", "parent_id": parent["id"], "title": "子", "purpose": "目的" }),
    )
    .await;
    let (status, body) = app
        .patch(
            &format!("/api/goals/{}", child["id"]),
            json!({ "status": "done" }),
        )
        .await;
    assert_eq!(status, 200, "close child: {body}");
    let list = get(&app, book_id).await;
    let row = list
        .iter()
        .find(|row| row["id"] == parent["id"])
        .expect("parent");
    assert_eq!(row["closable"], true);
    assert_eq!(row["status"], "active");
}

#[tokio::test]
async fn entity_merge_moves_goal_links_and_rewrites_conditions_without_goal_changes() {
    let app = TestApp::new().await;
    let (book_id, _) = book_with_main(&app).await;
    let target = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "A", "summary": "" }),
        )
        .await
        .1;
    let source = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "B", "summary": "" }),
        )
        .await
        .1;
    let target_id = target["id"].as_i64().unwrap() as i32;
    let source_id = source["id"].as_i64().unwrap() as i32;
    let goal = create(
        &app,
        book_id,
        json!({
            "level": "goal", "title": "链接", "purpose": "目的", "status": "waiting",
            "conditions": [
                { "kind": "entity_status_is", "entity_id": target_id, "status": "active" },
                { "kind": "entity_status_is", "entity_id": source_id, "status": "active" }
            ],
            "entity_ids": [source_id]
        }),
    )
    .await;
    let goal_changes_before = app
        .get(&format!("/api/books/{book_id}/changes?target_type=goal"))
        .await
        .1["items"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    let (status, body) = app
        .post(
            &format!("/api/entities/{target_id}/merge"),
            json!({ "from_id": source_id }),
        )
        .await;
    assert_eq!(status, 200, "merge: {body}");
    let goal_changes_after = app
        .get(&format!("/api/books/{book_id}/changes?target_type=goal"))
        .await
        .1["items"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    assert_eq!(
        goal_changes_after, goal_changes_before,
        "the merge must not write per-node goal changes"
    );
    let list = get(&app, book_id).await;
    let row = list
        .iter()
        .find(|row| row["id"] == goal["id"])
        .expect("goal");
    assert_eq!(row["entities"][0]["id"], target_id);
    assert_eq!(row["conditions"].as_array().unwrap().len(), 1);
    assert_eq!(row["conditions"][0]["condition"]["entity_id"], target_id);
}

#[tokio::test]
async fn private_scopes_see_shared_rows_but_not_other_tracks() {
    let app = TestApp::new().await;
    let (book_id, _) = book_with_main(&app).await;
    let (_, t1) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "title": "T1", "synopsis": "", "canon_mode": "isolated", "anchor_kind": "before_start" }),
        )
        .await;
    let (_, t2) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "title": "T2", "synopsis": "", "canon_mode": "isolated", "anchor_kind": "before_start" }),
        )
        .await;
    let track1 = t1["id"].as_i64().unwrap() as i32;
    let track2 = t2["id"].as_i64().unwrap() as i32;
    let shared_entity = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "Shared", "summary": "" }),
        )
        .await
        .1;
    let private_entity = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "Private2", "summary": "", "track_id": track2 }),
        )
        .await
        .1;
    let shared_id = shared_entity["id"].as_i64().unwrap() as i32;
    let private_id = private_entity["id"].as_i64().unwrap() as i32;

    // A private-T goal may link shared canon.
    create(
        &app,
        book_id,
        json!({ "level": "goal", "title": "T1", "purpose": "P", "track_id": track1, "entity_ids": [shared_id] }),
    )
    .await;
    // It may not link another track's private canon.
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/goals"),
            json!({ "level": "goal", "title": "Bad", "purpose": "P", "track_id": track1, "entity_ids": [private_id] }),
        )
        .await;
    assert!(
        status == 400 || status == 404,
        "other-track link: {status} {body}"
    );
    // A shared goal may not link a private row.
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/goals"),
            json!({ "level": "goal", "title": "Bad shared", "purpose": "P", "entity_ids": [private_id] }),
        )
        .await;
    assert!(
        status == 400 || status == 404,
        "shared link: {status} {body}"
    );
}
