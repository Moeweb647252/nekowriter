//! Goal service/API tests (PLAN M13 13.5, spec/06 §11, spec/10 §2.4).

use nekowriter::testing::{TestApp, assert_error};
use serde_json::{Value, json};

async fn app_and_track() -> (TestApp, i32) {
    let app = TestApp::new().await;
    let book = app.create_book("Goals").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    (app, track_id)
}

async fn get(app: &TestApp, book_id: i32) -> Vec<Value> {
    let (status, body) = app.get(&format!("/api/books/{book_id}/goals")).await;
    assert_eq!(status, 200, "list goals: {body}");
    body.as_array().cloned().unwrap_or_default()
}

async fn create_node(app: &TestApp, book_id: i32, body: Value) -> Value {
    let (status, body) = app.post(&format!("/api/books/{book_id}/goals"), body).await;
    assert_eq!(status, 201, "create goal: {body}");
    body
}

#[tokio::test]
async fn a_tree_is_created_and_parent_rules_are_enforced() {
    let (app, _) = app_and_track().await;
    let book = app.create_book("Tree").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let goal = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "目标", "purpose": "目的" }),
    )
    .await;
    assert_eq!(goal["ready"], false);
    assert_eq!(goal["closable"], false);

    let plan = create_node(
        &app,
        book_id,
        json!({ "level": "plan", "parent_id": goal["id"], "title": "计划", "purpose": "计划目的" }),
    )
    .await;
    let task = create_node(
        &app,
        book_id,
        json!({ "level": "task", "parent_id": plan["id"], "title": "任务", "purpose": "任务目的",
                "chapter_from": 1, "chapter_to": 2 }),
    )
    .await;
    assert_eq!(task["parent_id"], plan["id"]);

    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/goals"),
            json!({ "level": "task", "parent_id": goal["id"], "title": "坏任务", "purpose": "坏" }),
        )
        .await;
    assert_eq!(status, 400, "task under goal: {body}");
    assert_error(&body, "validation.failed");

    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/goals"),
            json!({ "level": "plan", "parent_id": task["id"], "title": "坏计划", "purpose": "坏" }),
        )
        .await;
    assert_eq!(status, 400, "plan under task: {body}");
    assert_error(&body, "validation.failed");

    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/goals"),
            json!({ "level": "goal", "title": "没有目的", "purpose": "  " }),
        )
        .await;
    assert_eq!(status, 400, "blank purpose: {body}");
    assert_error(&body, "validation.failed");

    let (status, body) = app.delete(&format!("/api/goals/{}", goal["id"])).await;
    assert_eq!(status, 409, "delete goal with children: {body}");
    assert_error(&body, "goal.has_children");
    assert_eq!(body["error"]["details"]["children_count"], 1);
}

#[tokio::test]
async fn closing_a_node_needs_confirmation_for_open_descendants() {
    let (app, _) = app_and_track().await;
    let book = app.create_book("Close").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let goal = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "G", "purpose": "P" }),
    )
    .await;
    let plan = create_node(
        &app,
        book_id,
        json!({ "level": "plan", "parent_id": goal["id"], "title": "P", "purpose": "P" }),
    )
    .await;
    let task = create_node(
        &app,
        book_id,
        json!({ "level": "task", "parent_id": plan["id"], "title": "T", "purpose": "P" }),
    )
    .await;

    let (status, body) = app
        .patch(
            &format!("/api/goals/{}", goal["id"]),
            json!({ "status": "done" }),
        )
        .await;
    assert_eq!(status, 409, "close with open descendant: {body}");
    assert_error(&body, "goal.open_children");
    let children = body["error"]["details"]["children"]
        .as_array()
        .expect("children");
    assert!(
        children.iter().any(|child| child["id"] == task["id"]),
        "open descendants must name the task: {body}"
    );

    let (status, body) = app
        .patch(
            &format!("/api/goals/{}", goal["id"]),
            json!({ "status": "done", "confirm_open_children": true }),
        )
        .await;
    assert_eq!(status, 200, "confirmed close: {body}");
    assert_eq!(body["status"], "done");
}

async fn changes_for(app: &TestApp, book_id: i32, target_id: i32) -> Vec<Value> {
    let (status, body) = app.get(&format!("/api/books/{book_id}/changes")).await;
    assert_eq!(status, 200, "changes: {body}");
    body["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|change| change["target_id"] == target_id)
        .cloned()
        .collect()
}

#[tokio::test]
async fn goal_create_update_and_delete_revert() {
    let app = TestApp::new().await;
    let book = app.create_book("Reverts").await;
    let book_id = book["id"].as_i64().expect("book") as i32;

    // Create-revert deletes the node.
    let created = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "A", "purpose": "P" }),
    )
    .await;
    let create_change = changes_for(&app, book_id, created["id"].as_i64().unwrap() as i32)
        .await
        .into_iter()
        .find(|change| change["op"] == "create")
        .expect("create change");
    let (status, body) = app
        .post(
            &format!("/api/changes/{}/revert", create_change["id"]),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "create revert: {body}");
    let list = get(&app, book_id).await;
    assert!(!list.iter().any(|row| row["id"] == created["id"]));

    // Update-revert restores the before title.
    let node = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "Before", "purpose": "P" }),
    )
    .await;
    let (status, _) = app
        .patch(
            &format!("/api/goals/{}", node["id"]),
            json!({ "title": "After" }),
        )
        .await;
    assert_eq!(status, 200);
    let update_change = changes_for(&app, book_id, node["id"].as_i64().unwrap() as i32)
        .await
        .into_iter()
        .find(|change| change["op"] == "update")
        .expect("update change");
    let (status, body) = app
        .post(
            &format!("/api/changes/{}/revert", update_change["id"]),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "update revert: {body}");
    let list = get(&app, book_id).await;
    let restored = list
        .iter()
        .find(|row| row["id"] == node["id"])
        .expect("restored node");
    assert_eq!(restored["title"], "Before");

    // Delete-revert recreates with a new id.
    let (status, _) = app.delete(&format!("/api/goals/{}", node["id"])).await;
    assert_eq!(status, 204);
    let delete_change = changes_for(&app, book_id, node["id"].as_i64().unwrap() as i32)
        .await
        .into_iter()
        .find(|change| change["op"] == "delete")
        .expect("delete change");
    let (status, body) = app
        .post(
            &format!("/api/changes/{}/revert", delete_change["id"]),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "delete revert: {body}");
    let list = get(&app, book_id).await;
    let recreated = list
        .iter()
        .find(|row| row["title"] == "Before")
        .expect("recreated node");
    assert_ne!(recreated["id"], node["id"]);
}

#[tokio::test]
async fn create_revert_of_a_parent_with_children_is_refused() {
    let app = TestApp::new().await;
    let book = app.create_book("Revert Parent").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let goal = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "G", "purpose": "P" }),
    )
    .await;
    create_node(
        &app,
        book_id,
        json!({ "level": "plan", "parent_id": goal["id"], "title": "P", "purpose": "P" }),
    )
    .await;
    let create_change = changes_for(&app, book_id, goal["id"].as_i64().unwrap() as i32)
        .await
        .into_iter()
        .find(|change| change["op"] == "create")
        .expect("create change");
    let (status, body) = app
        .post(
            &format!("/api/changes/{}/revert", create_change["id"]),
            json!({}),
        )
        .await;
    assert_eq!(status, 409, "create revert with child: {body}");
    assert_error(&body, "goal.has_children");
}

#[tokio::test]
async fn update_revert_drops_links_to_vanished_targets_and_keeps_broken_conditions() {
    let app = TestApp::new().await;
    let book = app.create_book("Revert Broken").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let entity = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "消失", "summary": "" }),
        )
        .await
        .1;
    let entity_id = entity["id"].as_i64().unwrap() as i32;
    let goal = create_node(
        &app,
        book_id,
        json!({
            "level": "goal", "title": "Before", "purpose": "P", "status": "waiting",
            "conditions": [{ "kind": "entity_status_is", "entity_id": entity_id, "status": "active" }],
            "entity_ids": [entity_id]
        }),
    )
    .await;
    let (status, _) = app
        .patch(
            &format!("/api/goals/{}", goal["id"]),
            json!({ "title": "After" }),
        )
        .await;
    assert_eq!(status, 200);
    app.execute(&format!("DELETE FROM entities WHERE id = {entity_id}"))
        .await;
    let update_change = changes_for(&app, book_id, goal["id"].as_i64().unwrap() as i32)
        .await
        .into_iter()
        .find(|change| change["op"] == "update")
        .expect("update change");
    let (status, body) = app
        .post(
            &format!("/api/changes/{}/revert", update_change["id"]),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "revert update: {body}");
    let list = get(&app, book_id).await;
    let row = list
        .iter()
        .find(|row| row["id"] == goal["id"])
        .expect("goal");
    assert_eq!(row["title"], "Before");
    assert!(row["entities"].as_array().unwrap().is_empty());
    assert_eq!(row["conditions"][0]["broken"], true);
}

#[tokio::test]
async fn reparenting_moves_a_plan_and_level_scope_are_immutable() {
    let app = TestApp::new().await;
    let book = app.create_book("Reparent").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let first = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "一", "purpose": "P" }),
    )
    .await;
    let second = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "二", "purpose": "P" }),
    )
    .await;
    let plan = create_node(
        &app,
        book_id,
        json!({ "level": "plan", "parent_id": first["id"], "title": "计划", "purpose": "P" }),
    )
    .await;
    let (status, body) = app
        .patch(
            &format!("/api/goals/{}", plan["id"]),
            json!({ "parent_id": second["id"] }),
        )
        .await;
    assert_eq!(status, 200, "reparent: {body}");
    assert_eq!(body["parent_id"], second["id"]);

    let (status, body) = app
        .patch(
            &format!("/api/goals/{}", plan["id"]),
            json!({ "level": "task" }),
        )
        .await;
    assert_eq!(status, 400, "level change: {body}");
    assert_error(&body, "validation.failed");
}

#[tokio::test]
async fn goal_list_visibility_is_shared_by_default_and_shared_plus_track_when_filtered() {
    let app = TestApp::new().await;
    let book = app.create_book("Visibility").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let main_track = tracks[0]["id"].as_i64().expect("main track") as i32;
    let (status, isolated) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "title": "Side", "synopsis": "", "canon_mode": "isolated", "anchor_kind": "before_start" }),
        )
        .await;
    assert_eq!(status, 201, "isolated track: {isolated}");
    let side_track = isolated["id"].as_i64().expect("side track") as i32;
    let shared = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "Shared", "purpose": "P" }),
    )
    .await;
    let private = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "Private", "purpose": "P", "track_id": side_track }),
    )
    .await;

    let (status, list) = app.get(&format!("/api/books/{book_id}/goals")).await;
    assert_eq!(status, 200);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], shared["id"]);

    let (status, list) = app
        .get(&format!("/api/books/{book_id}/goals?track_id={side_track}"))
        .await;
    assert_eq!(status, 200);
    let ids: Vec<i64> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_i64().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![
            shared["id"].as_i64().unwrap(),
            private["id"].as_i64().unwrap()
        ]
    );

    // A private node must not use a shared parent.
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/goals"),
            json!({ "level": "plan", "parent_id": shared["id"], "track_id": side_track, "title": "bad", "purpose": "P" }),
        )
        .await;
    assert_eq!(status, 400, "shared parent for private child: {body}");
    assert_error(&body, "validation.failed");
    let _ = main_track;
}

#[tokio::test]
async fn patch_clears_nullable_fields_and_rejects_any_track_id() {
    let app = TestApp::new().await;
    let book = app.create_book("Patch clears").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let goal = create_node(&app, book_id, json!({ "level": "goal", "title": "G", "purpose": "P", "chapter_from": 1, "chapter_to": 2 })).await;
    let plan = create_node(
        &app,
        book_id,
        json!({ "level": "plan", "parent_id": goal["id"], "title": "Plan", "purpose": "P", "chapter_from": 3, "chapter_to": 4 }),
    )
    .await;
    let (status, body) = app
        .patch(
            &format!("/api/goals/{}", plan["id"]),
            json!({ "parent_id": null, "chapter_from": null, "chapter_to": null }),
        )
        .await;
    assert_eq!(status, 200, "clear nullable fields: {body}");
    assert_eq!(body["parent_id"], Value::Null);
    assert_eq!(body["chapter_from"], Value::Null);
    assert_eq!(body["chapter_to"], Value::Null);

    let (status, body) = app
        .patch(
            &format!("/api/goals/{}", plan["id"]),
            json!({ "title": "Only title" }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["title"], "Only title");
    assert_eq!(body["parent_id"], Value::Null);

    let (status, body) = app
        .patch(
            &format!("/api/goals/{}", plan["id"]),
            json!({ "track_id": null }),
        )
        .await;
    assert_eq!(status, 400, "null track id must be rejected: {body}");
    assert_error(&body, "validation.failed");
}

#[tokio::test]
async fn a_broken_condition_target_does_not_block_unrelated_edits() {
    let app = TestApp::new().await;
    let book = app.create_book("Broken edit").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let entity = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "Gone", "summary": "" }),
        )
        .await
        .1;
    let entity_id = entity["id"].as_i64().unwrap() as i32;
    let goal = create_node(
        &app,
        book_id,
        json!({
            "level": "goal", "title": "Before", "purpose": "P", "status": "waiting",
            "conditions": [{ "kind": "entity_status_is", "entity_id": entity_id, "status": "active" }]
        }),
    )
    .await;
    app.execute(&format!("DELETE FROM entities WHERE id = {entity_id}"))
        .await;
    let (status, body) = app
        .patch(
            &format!("/api/goals/{}", goal["id"]),
            json!({ "title": "After", "note": "rename" }),
        )
        .await;
    assert_eq!(status, 200, "unrelated edit after broken target: {body}");
    assert_eq!(body["title"], "After");
    assert_eq!(body["conditions"][0]["broken"], true);
}

#[tokio::test]
async fn a_deleted_goal_id_is_not_reused() {
    let app = TestApp::new().await;
    let book = app.create_book("Fresh ids").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let first = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "First", "purpose": "P" }),
    )
    .await;
    let first_id = first["id"].as_i64().unwrap();
    let (status, _) = app.delete(&format!("/api/goals/{first_id}")).await;
    assert_eq!(status, 204);
    let second = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "Second", "purpose": "P" }),
    )
    .await;
    assert_ne!(
        second["id"].as_i64().unwrap(),
        first_id,
        "a deleted goal id must not be reused"
    );
}

#[tokio::test]
async fn update_revert_falls_back_when_the_parent_leaves_the_scope() {
    let app = TestApp::new().await;
    let book = app.create_book("Parent fallback").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let main_track = tracks[0]["id"].as_i64().unwrap() as i32;
    let (_, side) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "title": "Side", "synopsis": "", "canon_mode": "isolated", "anchor_kind": "before_start" }),
        )
        .await;
    let side_track = side["id"].as_i64().unwrap() as i32;
    let parent = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "Parent", "purpose": "P" }),
    )
    .await;
    let child = create_node(
        &app,
        book_id,
        json!({ "level": "plan", "parent_id": parent["id"], "title": "Child", "purpose": "P" }),
    )
    .await;
    let (status, _) = app
        .patch(
            &format!("/api/goals/{}", child["id"]),
            json!({ "title": "Changed" }),
        )
        .await;
    assert_eq!(status, 200);
    let _ = main_track;
    app.execute(&format!(
        "UPDATE goal_nodes SET track_id = {side_track} WHERE id = {}",
        parent["id"]
    ))
    .await;
    let update_change = changes_for(&app, book_id, child["id"].as_i64().unwrap() as i32)
        .await
        .into_iter()
        .find(|change| change["op"] == "update")
        .expect("child update");
    let (status, body) = app
        .post(
            &format!("/api/changes/{}/revert", update_change["id"]),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "parent fallback: {body}");
    let list = get(&app, book_id).await;
    let restored = list
        .iter()
        .find(|row| row["id"] == child["id"])
        .expect("child");
    assert_eq!(restored["parent_id"], Value::Null);
}

#[tokio::test]
async fn update_revert_restores_done_without_open_children_confirmation() {
    let app = TestApp::new().await;
    let book = app.create_book("Restore done").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let goal = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "G", "purpose": "P" }),
    )
    .await;
    create_node(
        &app,
        book_id,
        json!({ "level": "plan", "parent_id": goal["id"], "title": "Open", "purpose": "P" }),
    )
    .await;
    let (status, _) = app
        .patch(
            &format!("/api/goals/{}", goal["id"]),
            json!({ "status": "done", "confirm_open_children": true }),
        )
        .await;
    assert_eq!(status, 200);
    let (status, _) = app
        .patch(
            &format!("/api/goals/{}", goal["id"]),
            json!({ "title": "Done title" }),
        )
        .await;
    assert_eq!(status, 200);
    let update_change = changes_for(&app, book_id, goal["id"].as_i64().unwrap() as i32)
        .await
        .into_iter()
        .find(|change| change["op"] == "update")
        .expect("done update");
    let (status, body) = app
        .post(
            &format!("/api/changes/{}/revert", update_change["id"]),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "restore done: {body}");
    let list = get(&app, book_id).await;
    let restored = list
        .iter()
        .find(|row| row["id"] == goal["id"])
        .expect("goal");
    assert_eq!(restored["status"], "done");
}

#[tokio::test]
async fn delete_revert_keeps_a_surviving_parent_and_link() {
    let app = TestApp::new().await;
    let book = app.create_book("Delete recreate").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let entity = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "Linked", "summary": "" }),
        )
        .await
        .1;
    let entity_id = entity["id"].as_i64().unwrap() as i32;
    let parent = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "P", "purpose": "P" }),
    )
    .await;
    let child = create_node(
        &app,
        book_id,
        json!({ "level": "plan", "parent_id": parent["id"], "title": "C", "purpose": "P", "entity_ids": [entity_id] }),
    )
    .await;
    let (status, _) = app.delete(&format!("/api/goals/{}", child["id"])).await;
    assert_eq!(status, 204);
    let delete_change = changes_for(&app, book_id, child["id"].as_i64().unwrap() as i32)
        .await
        .into_iter()
        .find(|change| change["op"] == "delete")
        .expect("child delete");
    let (status, body) = app
        .post(
            &format!("/api/changes/{}/revert", delete_change["id"]),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "delete revert: {body}");
    let list = get(&app, book_id).await;
    let recreated = list
        .iter()
        .find(|row| row["title"] == "C")
        .expect("recreated");
    assert_eq!(recreated["parent_id"], parent["id"]);
    assert_eq!(recreated["entities"][0]["id"], entity_id);
}

#[tokio::test]
async fn goal_field_and_condition_limits_are_enforced() {
    let app = TestApp::new().await;
    let book = app.create_book("Limits").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let long_title = "x".repeat(121);
    let long_purpose = "x".repeat(1001);
    for body in [
        json!({ "level": "goal", "title": long_title, "purpose": "P" }),
        json!({ "level": "goal", "title": "T", "purpose": long_purpose }),
        json!({ "level": "goal", "title": "\u{3000}", "purpose": "P" }),
        json!({ "level": "goal", "title": "T", "purpose": "P", "status": "waiting" }),
        json!({ "level": "goal", "title": "T", "purpose": "P", "chapter_from": 3, "chapter_to": 2 }),
        json!({ "level": "goal", "title": "T", "purpose": "P", "conditions": (0..11).map(|i| json!({ "kind": "chapter_at_least", "chapter": i + 1 })).collect::<Vec<_>>() }),
    ] {
        let (status, response) = app.post(&format!("/api/books/{book_id}/goals"), body).await;
        assert_eq!(status, 400, "invalid goal accepted: {response}");
        assert_error(&response, "validation.failed");
    }
    let duplicate = create_node(
        &app,
        book_id,
        json!({
            "level": "goal", "title": "T", "purpose": "P", "status": "waiting",
            "conditions": [
                { "kind": "chapter_at_least", "chapter": 1 },
                { "kind": "chapter_at_least", "chapter": 1 }
            ]
        }),
    )
    .await;
    assert_eq!(duplicate["conditions"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn reverting_a_link_removal_does_not_bind_a_reused_entity_id() {
    let app = TestApp::new().await;
    let book = app.create_book("Link cutoff").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let entity = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "Original", "summary": "" }),
        )
        .await
        .1;
    let entity_id = entity["id"].as_i64().unwrap() as i32;
    let goal = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "Linked", "purpose": "P", "entity_ids": [entity_id] }),
    )
    .await;
    let (status, _) = app
        .patch(
            &format!("/api/goals/{}", goal["id"]),
            json!({ "entity_ids": [] }),
        )
        .await;
    assert_eq!(status, 200);
    let (status, _) = app.delete(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(status, 204);
    let replacement = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "Replacement", "summary": "" }),
        )
        .await
        .1;
    assert_eq!(replacement["id"].as_i64().unwrap() as i32, entity_id);
    let update_change = changes_for(&app, book_id, goal["id"].as_i64().unwrap() as i32)
        .await
        .into_iter()
        .find(|change| change["op"] == "update")
        .expect("link-clearing update");
    let (status, body) = app
        .post(
            &format!("/api/changes/{}/revert", update_change["id"]),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "revert link removal: {body}");
    let list = get(&app, book_id).await;
    let row = list
        .iter()
        .find(|row| row["id"] == goal["id"])
        .expect("goal");
    assert!(row["entities"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn one_sided_range_and_present_track_id_are_guarded() {
    let app = TestApp::new().await;
    let book = app.create_book("One sided").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let goal = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "G", "purpose": "P", "chapter_from": 3, "chapter_to": 4 }),
    )
    .await;
    let (status, body) = app
        .patch(
            &format!("/api/goals/{}", goal["id"]),
            json!({ "chapter_from": null }),
        )
        .await;
    assert_eq!(status, 200, "clear only from: {body}");
    assert_eq!(body["chapter_from"], Value::Null);
    assert_eq!(body["chapter_to"], 4);
    let (status, body) = app
        .patch(
            &format!("/api/goals/{}", goal["id"]),
            json!({ "chapter_to": 0 }),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["error"]["details"]["fields"]["chapter_to"],
        "chapter numbers start at 1"
    );
    let (status, body) = app
        .patch(
            &format!("/api/goals/{}", goal["id"]),
            json!({ "track_id": 1 }),
        )
        .await;
    assert_eq!(status, 400, "present non-null track id: {body}");
    assert_error(&body, "validation.failed");
}

#[tokio::test]
async fn canon_mode_tracks_cannot_hold_private_goals_and_cannot_switch_with_them() {
    let app = TestApp::new().await;
    let book = app.create_book("Canon mode goals").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let (status, canon) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "title": "Canon side", "synopsis": "", "canon_mode": "canon", "anchor_kind": "before_start" }),
        )
        .await;
    assert_eq!(status, 201, "canon track: {canon}");
    let canon_track = canon["id"].as_i64().unwrap() as i32;
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/goals"),
            json!({ "level": "goal", "title": "Private", "purpose": "P", "track_id": canon_track }),
        )
        .await;
    assert_eq!(status, 400, "canon-mode private goal: {body}");
    assert_error(&body, "validation.failed");

    let (_, isolated) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "title": "Isolated side", "synopsis": "", "canon_mode": "isolated", "anchor_kind": "before_start" }),
        )
        .await;
    let isolated_track = isolated["id"].as_i64().unwrap() as i32;
    create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "Private goal", "purpose": "P", "track_id": isolated_track }),
    )
    .await;
    let (status, body) = app
        .patch(
            &format!("/api/tracks/{isolated_track}"),
            json!({ "canon_mode": "canon" }),
        )
        .await;
    assert_eq!(status, 409, "switch with private goals: {body}");
    assert_error(&body, "track.mode_locked");
    assert_eq!(body["error"]["details"]["private_goal_nodes"], 1);

    let (_, clean) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "title": "Clean", "synopsis": "", "canon_mode": "isolated", "anchor_kind": "before_start" }),
        )
        .await;
    let clean_id = clean["id"].as_i64().unwrap() as i32;
    let (status, body) = app
        .patch(
            &format!("/api/tracks/{clean_id}"),
            json!({ "canon_mode": "canon" }),
        )
        .await;
    assert_eq!(status, 200, "clean switch: {body}");
    assert_eq!(body["canon_mode"], "canon");
}

#[tokio::test]
async fn writing_a_missing_condition_target_is_rejected() {
    let app = TestApp::new().await;
    let book = app.create_book("Missing target").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let goal = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "G", "purpose": "P" }),
    )
    .await;
    let (status, body) = app
        .patch(
            &format!("/api/goals/{}", goal["id"]),
            json!({ "conditions": [{ "kind": "entity_status_is", "entity_id": 9999, "status": "active" }] }),
        )
        .await;
    assert_eq!(status, 404, "missing condition target: {body}");
    assert_error(&body, "not_found");
}

#[tokio::test]
async fn goal_ids_are_unique_across_levels() {
    let app = TestApp::new().await;
    let book = app.create_book("Cross-level ids").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let goal = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "G", "purpose": "P" }),
    )
    .await;
    let plan = create_node(
        &app,
        book_id,
        json!({ "level": "plan", "parent_id": goal["id"], "title": "P", "purpose": "P" }),
    )
    .await;
    let task = create_node(
        &app,
        book_id,
        json!({ "level": "task", "parent_id": plan["id"], "title": "T", "purpose": "P" }),
    )
    .await;
    let plan_id = plan["id"].as_i64().unwrap();
    let task_id = task["id"].as_i64().unwrap();
    let (status, _) = app.delete(&format!("/api/goals/{task_id}")).await;
    assert_eq!(status, 204);
    let (status, _) = app.delete(&format!("/api/goals/{plan_id}")).await;
    assert_eq!(status, 204);
    let next = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "G2", "purpose": "P" }),
    )
    .await;
    assert_ne!(next["id"].as_i64().unwrap(), plan_id);
    assert_ne!(next["id"].as_i64().unwrap(), task_id);
}

#[tokio::test]
async fn reverting_a_link_removal_does_not_restore_an_invisible_target() {
    let app = TestApp::new().await;
    let book = app.create_book("Link visibility").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
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
    let entity = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "Shared", "summary": "" }),
        )
        .await
        .1;
    let entity_id = entity["id"].as_i64().unwrap() as i32;
    let goal = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "Private", "purpose": "P", "track_id": track1, "entity_ids": [entity_id] }),
    )
    .await;
    let (status, _) = app
        .patch(
            &format!("/api/goals/{}", goal["id"]),
            json!({ "entity_ids": [] }),
        )
        .await;
    assert_eq!(status, 200);
    app.execute(&format!(
        "UPDATE entities SET track_id = {track2} WHERE id = {entity_id}"
    ))
    .await;
    let update_change = changes_for(&app, book_id, goal["id"].as_i64().unwrap() as i32)
        .await
        .into_iter()
        .find(|change| change["op"] == "update")
        .expect("link-clearing update");
    let (status, body) = app
        .post(
            &format!("/api/changes/{}/revert", update_change["id"]),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "revert invisible link: {body}");
    let (status, list) = app
        .get(&format!("/api/books/{book_id}/goals?track_id={track1}"))
        .await;
    assert_eq!(status, 200);
    let row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == goal["id"])
        .expect("goal");
    assert!(row["entities"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn canon_mode_switch_counts_all_private_tables_and_allows_a_clean_track() {
    let app = TestApp::new().await;
    let book = app.create_book("All private rows").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let (_, track) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "title": "Private", "synopsis": "", "canon_mode": "isolated", "anchor_kind": "before_start" }),
        )
        .await;
    let track_id = track["id"].as_i64().unwrap() as i32;
    let a = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "A", "summary": "" }),
        )
        .await
        .1;
    let b = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "B", "summary": "" }),
        )
        .await
        .1;
    let a_id = a["id"].as_i64().unwrap() as i32;
    let b_id = b["id"].as_i64().unwrap() as i32;
    let private_entity = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "Private", "summary": "", "track_id": track_id }),
        )
        .await;
    assert_eq!(
        private_entity.0, 201,
        "private entity: {}",
        private_entity.1
    );
    // Timeline, memory, foreshadowing and relation create endpoints are author/shared-only;
    // private rows are written by an isolated agent session. Insert them directly here so the
    // guard sees every private table.
    app.execute(&format!(
        "INSERT INTO relations(book_id, track_id, source_id, target_id, kind, inverse_kind, \
         description, status, created_at, updated_at) VALUES \
         ({book_id}, {track_id}, {a_id}, {b_id}, 'ally', '', '', 'active', \
          '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    app.execute(&format!(
        "INSERT INTO timeline_events(book_id, track_id, position, title, created_at, updated_at) \
         VALUES ({book_id}, {track_id}, 1.0, 'Private event', \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    app.execute(&format!(
        "INSERT INTO memory_notes(book_id, track_id, kind, content, created_at, updated_at) \
         VALUES ({book_id}, {track_id}, 'fact', 'Private fact', \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    app.execute(&format!(
        "INSERT INTO foreshadowing(book_id, track_id, title, created_at, updated_at) \
         VALUES ({book_id}, {track_id}, 'Private hook', \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "Private goal", "purpose": "P", "track_id": track_id }),
    )
    .await;

    let (status, body) = app
        .patch(
            &format!("/api/tracks/{track_id}"),
            json!({ "canon_mode": "canon" }),
        )
        .await;
    assert_eq!(status, 409, "private rows: {body}");
    assert_error(&body, "track.mode_locked");
    let details = &body["error"]["details"];
    for key in [
        "private_entities",
        "private_relations",
        "private_timeline_events",
        "private_memory_notes",
        "private_foreshadowing",
        "private_goal_nodes",
        "private_goal_changes",
    ] {
        assert_eq!(details[key], 1, "count {key}: {body}");
    }

    let (_, clean) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "title": "Clean", "synopsis": "", "canon_mode": "isolated", "anchor_kind": "before_start" }),
        )
        .await;
    let clean_id = clean["id"].as_i64().unwrap() as i32;
    let (status, body) = app
        .patch(
            &format!("/api/tracks/{clean_id}"),
            json!({ "canon_mode": "canon" }),
        )
        .await;
    assert_eq!(status, 200, "clean switch: {body}");
    assert_eq!(body["canon_mode"], "canon");
}

#[tokio::test]
async fn canon_mode_switch_refuses_private_goal_history_after_the_node_is_deleted() {
    let app = TestApp::new().await;
    let book = app.create_book("Private goal history").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let (_, track) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "title": "Private", "synopsis": "", "canon_mode": "isolated", "anchor_kind": "before_start" }),
        )
        .await;
    let track_id = track["id"].as_i64().unwrap() as i32;
    let goal = create_node(
        &app,
        book_id,
        json!({ "level": "goal", "title": "Private goal", "purpose": "P", "track_id": track_id }),
    )
    .await;
    let (status, _) = app.delete(&format!("/api/goals/{}", goal["id"])).await;
    assert_eq!(status, 204);
    let (status, body) = app
        .patch(
            &format!("/api/tracks/{track_id}"),
            json!({ "canon_mode": "canon" }),
        )
        .await;
    assert_eq!(status, 409, "private goal history: {body}");
    assert_error(&body, "track.mode_locked");
    assert_eq!(body["error"]["details"]["private_goal_nodes"], 0);
    assert_eq!(body["error"]["details"]["private_goal_changes"], 2);
}

#[tokio::test]
async fn canon_mode_switch_refuses_each_private_table_individually() {
    let app = TestApp::new().await;
    let book = app.create_book("Each private table").await;
    let book_id = book["id"].as_i64().expect("book") as i32;
    let a = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "A", "summary": "" }),
        )
        .await
        .1;
    let b = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "B", "summary": "" }),
        )
        .await
        .1;
    let a_id = a["id"].as_i64().unwrap() as i32;
    let b_id = b["id"].as_i64().unwrap() as i32;
    let cases = [
        ("entities", "private_entities"),
        ("relations", "private_relations"),
        ("timeline_events", "private_timeline_events"),
        ("memory_notes", "private_memory_notes"),
        ("foreshadowing", "private_foreshadowing"),
        ("goal_nodes", "private_goal_nodes"),
        ("goal_changes", "private_goal_changes"),
    ];
    for (kind, key) in cases {
        let (_, track) = app
            .post(
                &format!("/api/books/{book_id}/tracks"),
                json!({ "title": kind, "synopsis": "", "canon_mode": "isolated", "anchor_kind": "before_start" }),
            )
            .await;
        let track_id = track["id"].as_i64().unwrap() as i32;
        match kind {
            "entities" => {
                app.post(
                    &format!("/api/books/{book_id}/entities"),
                    json!({ "kind": "character", "name": "Private", "summary": "", "track_id": track_id }),
                )
                .await;
            }
            "relations" => {
                app.execute(&format!(
                    "INSERT INTO relations(book_id, track_id, source_id, target_id, kind, inverse_kind, \
                     description, status, created_at, updated_at) VALUES \
                     ({book_id}, {track_id}, {a_id}, {b_id}, 'ally', '', '', 'active', \
                      '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
                ))
                .await;
            }
            "timeline_events" => {
                app.execute(&format!(
                    "INSERT INTO timeline_events(book_id, track_id, position, title, created_at, updated_at) \
                     VALUES ({book_id}, {track_id}, 1.0, 'E', '2026-01-01T00:00:00Z', \
                             '2026-01-01T00:00:00Z')"
                ))
                .await;
            }
            "memory_notes" => {
                app.execute(&format!(
                    "INSERT INTO memory_notes(book_id, track_id, kind, content, created_at, updated_at) \
                     VALUES ({book_id}, {track_id}, 'fact', 'F', '2026-01-01T00:00:00Z', \
                             '2026-01-01T00:00:00Z')"
                ))
                .await;
            }
            "foreshadowing" => {
                app.execute(&format!(
                    "INSERT INTO foreshadowing(book_id, track_id, title, created_at, updated_at) \
                     VALUES ({book_id}, {track_id}, 'H', '2026-01-01T00:00:00Z', \
                             '2026-01-01T00:00:00Z')"
                ))
                .await;
            }
            "goal_nodes" => {
                create_node(
                    &app,
                    book_id,
                    json!({ "level": "goal", "title": "G", "purpose": "P", "track_id": track_id }),
                )
                .await;
            }
            "goal_changes" => {
                let goal = create_node(
                    &app,
                    book_id,
                    json!({ "level": "goal", "title": "History", "purpose": "P", "track_id": track_id }),
                )
                .await;
                let (status, _) = app.delete(&format!("/api/goals/{}", goal["id"])).await;
                assert_eq!(status, 204);
            }
            _ => unreachable!(),
        }
        let (status, body) = app
            .patch(
                &format!("/api/tracks/{track_id}"),
                json!({ "canon_mode": "canon" }),
            )
            .await;
        assert_eq!(status, 409, "{kind}: {body}");
        let expected = if kind == "goal_changes" { 2 } else { 1 };
        assert_eq!(body["error"]["details"][key], expected, "{kind}: {body}");
    }
}
