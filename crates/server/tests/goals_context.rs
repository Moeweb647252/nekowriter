//! `block.goals` in the agent context (PLAN M13 13.6, spec/03 §10, spec/06 §11.5).

mod agent_harness;

use std::time::Duration;

use agent_harness::Runtime;
use nekowriter::db::json::{Granularity, SessionKind};
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter_llm::mock::ScriptStep;
use sea_orm::EntityTrait;
use serde_json::{Value, json};

fn idle() -> Vec<ScriptStep> {
    vec![ScriptStep::new().delay(Duration::from_secs(60))]
}

struct Fixture {
    runtime: Runtime,
    target_id: i32,
}

async fn fixture() -> Fixture {
    let runtime = agent_harness::runtime(idle()).await;
    let target = runtime.app.create_chapter(runtime.track_id, "Target").await;
    let target_id = target["id"].as_i64().expect("target") as i32;
    Fixture { runtime, target_id }
}

async fn goal(app: &nekowriter::testing::TestApp, book_id: i32, body: Value) -> Value {
    let (status, body) = app.post(&format!("/api/books/{book_id}/goals"), body).await;
    assert_eq!(status, 201, "goal: {body}");
    body
}

async fn kickoff_session(fixture: &Fixture, kind: SessionKind) -> (i32, String) {
    let (session_id, _) = sessions::create(
        fixture.runtime.app.state(),
        CreateSession {
            kind,
            book_id: Some(fixture.runtime.book_id),
            chapter_id: Some(fixture.target_id),
            granularity: Some(Granularity::Paragraph),
            ..CreateSession::default()
        },
    )
    .await
    .expect("session");
    let messages = fixture.runtime.messages(session_id).await;
    (session_id, messages.first().expect("kickoff").text())
}

async fn kickoff(fixture: &Fixture, kind: SessionKind) -> String {
    kickoff_session(fixture, kind).await.1
}

async fn add_tree(fixture: &Fixture) -> (Value, Value, Value) {
    let app = &fixture.runtime.app;
    let book = fixture.runtime.book_id;
    let goal_node = goal(
        app,
        book,
        json!({ "level": "goal", "title": "目标标题", "purpose": "目标目的", "chapter_from": 1 }),
    )
    .await;
    let plan = goal(
        app,
        book,
        json!({ "level": "plan", "parent_id": goal_node["id"], "title": "计划标题", "purpose": "计划目的", "chapter_from": 1 }),
    )
    .await;
    let task = goal(
        app,
        book,
        json!({ "level": "task", "parent_id": plan["id"], "title": "任务标题", "purpose": "任务目的", "chapter_from": 1 }),
    )
    .await;
    (goal_node, plan, task)
}

#[tokio::test]
async fn planner_write_and_review_kickoffs_carry_task_with_both_ancestor_purposes() {
    let fixture = fixture().await;
    let (goal_node, plan, task) = add_tree(&fixture).await;
    let app = &fixture.runtime.app;
    let book = fixture.runtime.book_id;
    goal(
        app,
        book,
        json!({ "level": "task", "parent_id": plan["id"], "title": "未来任务", "purpose": "未来目的", "chapter_from": 3 }),
    )
    .await;
    goal(
        app,
        book,
        json!({ "level": "task", "parent_id": plan["id"], "title": "等待任务", "purpose": "等待目的", "status": "waiting",
                "conditions": [{ "kind": "chapter_at_least", "chapter": 1 }] }),
    )
    .await;
    let waiting_plan = goal(
        app,
        book,
        json!({ "level": "plan", "parent_id": goal_node["id"], "title": "等待计划", "purpose": "等待计划目的", "status": "waiting",
                "conditions": [{ "kind": "chapter_at_least", "chapter": 1 }] }),
    )
    .await;
    goal(
        app,
        book,
        json!({ "level": "task", "parent_id": waiting_plan["id"], "title": "等待计划任务", "purpose": "等待计划任务目的", "chapter_from": 1 }),
    )
    .await;
    let closed_goal = goal(
        app,
        book,
        json!({ "level": "goal", "title": "已关闭", "purpose": "已关闭目的", "status": "done" }),
    )
    .await;
    let closed_plan = goal(
        app,
        book,
        json!({ "level": "plan", "parent_id": closed_goal["id"], "title": "关闭计划", "purpose": "关闭计划目的", "status": "done" }),
    )
    .await;
    goal(
        app,
        book,
        json!({ "level": "task", "parent_id": closed_plan["id"], "title": "关闭任务", "purpose": "关闭任务目的", "chapter_from": 1 }),
    )
    .await;
    for kind in [
        SessionKind::ChapterOutline,
        SessionKind::ChapterWrite,
        SessionKind::ChapterReview,
    ] {
        let (session_id, text) = kickoff_session(&fixture, kind).await;
        for value in ["任务标题", "任务目的", "计划目的", "目标目的"] {
            assert!(text.contains(value), "{kind:?} missing {value}:\n{text}");
        }
        for value in [
            "未来任务",
            "未来目的",
            "等待任务",
            "等待目的",
            "关闭任务",
            "关闭任务目的",
        ] {
            assert!(!text.contains(value), "{kind:?} leaked {value}:\n{text}");
        }
        assert_eq!(
            text.matches(&format!("goal id=\"{}\"", goal_node["id"]))
                .count(),
            1,
            "{kind:?}"
        );
        assert_eq!(
            text.matches(&format!("plan id=\"{}\"", plan["id"])).count(),
            1,
            "{kind:?}"
        );
        assert_eq!(
            text.matches(&format!("task id=\"{}\"", task["id"])).count(),
            1,
            "{kind:?}"
        );
        assert!(
            text.contains("等待计划任务目的"),
            "{kind:?} missing the active task under a waiting plan: {text}"
        );
        assert!(
            !text.contains(&format!("<plan id=\"{}\"", waiting_plan["id"])),
            "{kind:?} rendered the waiting plan as an ancestor: {text}"
        );
        assert!(
            !text.contains("等待计划目的"),
            "{kind:?} leaked the waiting plan purpose: {text}"
        );
        sessions::abandon(fixture.runtime.app.state(), session_id)
            .await
            .expect("abandon");
    }
}

#[tokio::test]
async fn a_side_track_sees_book_wide_goals_but_not_another_tracks_private_nodes() {
    let fixture = fixture().await;
    let app = &fixture.runtime.app;
    let book = fixture.runtime.book_id;
    let (status, side) = app
        .post(
            &format!("/api/books/{book}/tracks"),
            json!({ "title": "Side", "synopsis": "", "canon_mode": "isolated", "anchor_kind": "before_start" }),
        )
        .await;
    assert_eq!(status, 201, "side: {side}");
    let side_track = side["id"].as_i64().unwrap() as i32;
    let (status, other) = app
        .post(
            &format!("/api/books/{book}/tracks"),
            json!({ "title": "Other", "synopsis": "", "canon_mode": "isolated", "anchor_kind": "before_start" }),
        )
        .await;
    assert_eq!(status, 201, "other: {other}");
    let other_track = other["id"].as_i64().unwrap() as i32;
    goal(
        app,
        book,
        json!({ "level": "goal", "title": "书wide目标", "purpose": "书wide目的" }),
    )
    .await;
    goal(
        app,
        book,
        json!({ "level": "task", "title": "本轨任务", "purpose": "本轨目的", "track_id": side_track, "chapter_from": 1 }),
    )
    .await;
    goal(
        app,
        book,
        json!({ "level": "task", "title": "别轨任务", "purpose": "别轨目的", "track_id": other_track, "chapter_from": 1 }),
    )
    .await;
    let side_chapter = app.create_chapter(side_track, "Side Target").await;
    let side_chapter_id = side_chapter["id"].as_i64().unwrap() as i32;
    let (session_id, _) = sessions::create(
        app.state(),
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(book),
            chapter_id: Some(side_chapter_id),
            granularity: Some(Granularity::Paragraph),
            ..CreateSession::default()
        },
    )
    .await
    .expect("side session");
    let messages = fixture.runtime.messages(session_id).await;
    let text = messages.first().unwrap().text();
    assert!(
        text.contains("书wide目的"),
        "book-wide goal missing:\n{text}"
    );
    assert!(text.contains("本轨目的"), "track goal missing:\n{text}");
    assert!(!text.contains("别轨目的"), "other track leaked:\n{text}");
}

#[tokio::test]
async fn entities_off_removes_goal_entity_links() {
    let fixture = fixture().await;
    let app = &fixture.runtime.app;
    let book = fixture.runtime.book_id;
    let (_, entity) = app
        .post(
            &format!("/api/books/{book}/entities"),
            json!({ "kind": "character", "name": "链接实体", "summary": "" }),
        )
        .await;
    let entity_id = entity["id"].as_i64().unwrap() as i32;
    goal(
        app,
        book,
        json!({ "level": "goal", "title": "目标", "purpose": "目的", "entity_ids": [entity_id] }),
    )
    .await;
    let (_, book_dto) = app.get(&format!("/api/books/{book}")).await;
    let mut settings = book_dto["settings"].clone();
    settings["canon_systems"]["entities"] = json!(false);
    let (status, body) = app
        .patch(
            &format!("/api/books/{book}"),
            json!({ "settings": settings }),
        )
        .await;
    assert_eq!(status, 200, "settings: {body}");
    let text = kickoff(&fixture, SessionKind::ChapterWrite).await;
    assert!(
        !text.contains("链接实体"),
        "entity link leaked with Entities off:\n{text}"
    );
    assert!(
        text.contains("目的"),
        "the goal itself must still render:\n{text}"
    );
}

async fn frozen_tools(fixture: &Fixture, session_id: i32) -> Vec<String> {
    let row = nekowriter::db::entities::agent_sessions::Entity::find_by_id(session_id)
        .one(fixture.runtime.app.db().conn())
        .await
        .expect("session row")
        .expect("session");
    row.tools
        .into_inner()
        .into_iter()
        .map(|tool| tool.name)
        .collect()
}

#[tokio::test]
async fn the_goal_tool_gate_tracks_relevant_nodes_in_both_directions() {
    let fixture = fixture().await;
    let app = &fixture.runtime.app;
    let book = fixture.runtime.book_id;
    let (no_goal_session, _) = sessions::create(
        app.state(),
        CreateSession {
            kind: SessionKind::ChapterMaintenance,
            book_id: Some(book),
            chapter_id: Some(fixture.target_id),
            granularity: Some(Granularity::Paragraph),
            ..CreateSession::default()
        },
    )
    .await
    .expect("no-goal session");
    let no_goal_tools = frozen_tools(&fixture, no_goal_session).await;
    assert_eq!(no_goal_tools.len(), 22);
    assert!(
        !no_goal_tools
            .iter()
            .any(|tool| tool == "propose_goal_change")
    );
    sessions::abandon(app.state(), no_goal_session)
        .await
        .expect("abandon");

    goal(
        app,
        book,
        json!({ "level": "task", "title": "Relevant", "purpose": "Purpose", "chapter_from": 1 }),
    )
    .await;
    let (goal_session, _) = sessions::create(
        app.state(),
        CreateSession {
            kind: SessionKind::ChapterMaintenance,
            book_id: Some(book),
            chapter_id: Some(fixture.target_id),
            granularity: Some(Granularity::Paragraph),
            ..CreateSession::default()
        },
    )
    .await
    .expect("goal session");
    let goal_tools = frozen_tools(&fixture, goal_session).await;
    assert_eq!(goal_tools.len(), 23);
    assert!(goal_tools.iter().any(|tool| tool == "propose_goal_change"));
}

#[tokio::test]
async fn the_goal_block_names_each_tasks_plan_parent() {
    let fixture = fixture().await;
    let app = &fixture.runtime.app;
    let book = fixture.runtime.book_id;
    let goal_node = goal(
        app,
        book,
        json!({ "level": "goal", "title": "G", "purpose": "G purpose" }),
    )
    .await;
    let plan1 = goal(
        app,
        book,
        json!({ "level": "plan", "parent_id": goal_node["id"], "title": "P1", "purpose": "P1 purpose" }),
    )
    .await;
    let plan2 = goal(
        app,
        book,
        json!({ "level": "plan", "parent_id": goal_node["id"], "title": "P2", "purpose": "P2 purpose" }),
    )
    .await;
    let task1 = goal(
        app,
        book,
        json!({ "level": "task", "parent_id": plan1["id"], "title": "T1", "purpose": "T1 purpose" }),
    )
    .await;
    let task2 = goal(
        app,
        book,
        json!({ "level": "task", "parent_id": plan2["id"], "title": "T2", "purpose": "T2 purpose" }),
    )
    .await;
    let text = kickoff(&fixture, SessionKind::ChapterWrite).await;
    assert!(
        text.contains(&format!("<plan id=\"{}\"", plan1["id"])),
        "{text}"
    );
    assert!(
        text.contains(&format!("<plan id=\"{}\"", plan2["id"])),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "<task id=\"{}\" parent=\"{}\"",
            task1["id"], plan1["id"]
        )),
        "task 1 parent missing:\n{text}"
    );
    assert!(
        text.contains(&format!(
            "<task id=\"{}\" parent=\"{}\"",
            task2["id"], plan2["id"]
        )),
        "task 2 parent missing:\n{text}"
    );
}
