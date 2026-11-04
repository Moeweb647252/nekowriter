//! Timeline tree and foreshadowing tools (PLAN M22 22.4, spec/05 §2/§3/§6, spec/07 §2,
//! D-170): query/list rendering, the new `update_foreshadowing`, the three timeline error
//! branches, search docs, and the frozen archivist list.

mod agent_write_harness;

use agent_write_harness::base::runtime;
use nekowriter::agent::tools::registry::{ARCHIVIST_TOOLS, ToolRegistry, tools_for_session};
use nekowriter::db::json::{CanonSystems, SessionKind};
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter_llm::mock::{ScriptStep, ScriptedProvider};
use serde_json::{Value, json};

async fn create_event(app: &nekowriter::testing::TestApp, book_id: i32, body: Value) -> Value {
    let (status, event) = app
        .post(&format!("/api/books/{book_id}/timeline"), body)
        .await;
    assert_eq!(status, axum::http::StatusCode::CREATED, "{event}");
    event
}

async fn patch_event(
    app: &nekowriter::testing::TestApp,
    event_id: i32,
    body: Value,
) -> (axum::http::StatusCode, Value) {
    app.patch(&format!("/api/timeline-events/{event_id}"), body)
        .await
}

async fn start_archivist(runtime: &agent_write_harness::Runtime) -> i32 {
    let (session_id, _warnings) = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterMaintenance,
            book_id: Some(runtime.book_id),
            chapter_id: Some(runtime.chapter_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create archivist session");
    session_id
}

async fn calls(
    runtime: &agent_write_harness::Runtime,
    session_id: i32,
) -> Vec<(String, String, Value, bool)> {
    let session = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await
        .1;
    session["tool_calls"]
        .as_array()
        .expect("tool calls")
        .iter()
        .map(|call| {
            (
                call["name"].as_str().expect("name").to_owned(),
                call["result_text"].as_str().unwrap_or_default().to_owned(),
                call["ui"].clone(),
                call["is_error"].as_bool().expect("is_error"),
            )
        })
        .collect()
}

async fn call<'a>(
    calls: &'a [(String, String, Value, bool)],
    name: &str,
) -> &'a (String, String, Value, bool) {
    calls
        .iter()
        .find(|(call_name, _, _, _)| call_name == name)
        .unwrap_or_else(|| panic!("{name} ran; calls: {calls:?}"))
}

/// `add_timeline_event` creates a planned child under an existing entry, leaves it without a
/// chapter even in a chapter session, and reports the new record (spec/05 §6, D-170).
#[tokio::test]
async fn add_timeline_event_places_a_planned_child_under_its_parent() {
    let mut runtime = runtime(Vec::new()).await;
    // The script is built after the parent id exists.
    let parent = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "Phase", "status": "planned" }),
    )
    .await;
    let parent_id = parent["id"].as_i64().expect("id") as i32;
    runtime
        .app
        .set_provider(ScriptedProvider::new(vec![ScriptStep::new().tool(
            "add_timeline_event",
            json!({ "title": "Scene", "status": "planned", "parent": parent_id }),
        )]));
    let session_id = start_archivist(&runtime).await;
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let call = call(&calls, "add_timeline_event").await;
    assert!(!call.3, "{}", call.1);
    assert!(call.1.contains("已加入时间线"), "{}", call.1);

    let (_, events) = runtime
        .app
        .get(&format!("/api/books/{}/timeline", runtime.book_id))
        .await;
    let scene = events
        .as_array()
        .expect("events")
        .iter()
        .find(|event| event["title"] == "Scene")
        .expect("created scene");
    assert_eq!(scene["parent_id"], parent_id);
    assert_eq!(scene["status"], "planned");
    assert_eq!(scene["chapter_id"], Value::Null);
}

/// `update_timeline_event` promotes a planned entry with the session chapter by default, resolves
/// `chapter` as a main-track number when given, and moves an entry with its subtree.
#[tokio::test]
async fn update_timeline_event_promotes_and_moves() {
    let mut runtime = runtime(Vec::new()).await;
    let promoted_default = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "Phase default", "status": "planned" }),
    )
    .await;
    let default_id = promoted_default["id"].as_i64().expect("id") as i32;
    let promoted_with_chapter = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "Phase chapter", "status": "planned" }),
    )
    .await;
    let chapter_event_id = promoted_with_chapter["id"].as_i64().expect("id") as i32;
    let move_parent = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "Move parent", "status": "planned" }),
    )
    .await;
    let move_parent_id = move_parent["id"].as_i64().expect("id") as i32;
    let child = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "Scene", "status": "planned", "parent_id": move_parent_id }),
    )
    .await;
    let child_id = child["id"].as_i64().expect("id") as i32;
    let second = runtime
        .app
        .create_chapter(runtime.track_id, "Chapter Two")
        .await;
    let second_id = second["id"].as_i64().expect("id") as i32;
    assert_eq!(second["number"], json!(2));

    runtime.app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new().tool(
            "update_timeline_event",
            json!({ "event_id": default_id, "status": "occurred", "note": "promote" }),
        ),
        ScriptStep::new().tool(
            "update_timeline_event",
            json!({ "event_id": chapter_event_id, "status": "occurred", "chapter": 2, "note": "promote" }),
        ),
        ScriptStep::new().tool(
            "update_timeline_event",
            json!({ "event_id": child_id, "top_level": true, "note": "top" }),
        ),
    ]));
    let session_id = start_archivist(&runtime).await;
    runtime.wait_any_status(session_id).await;

    let (_, promoted) = runtime
        .app
        .get(&format!("/api/timeline-events/{default_id}"))
        .await;
    assert_eq!(promoted["status"], "occurred");
    assert_eq!(promoted["chapter_id"], runtime.chapter_id);
    let (_, promoted_child) = runtime
        .app
        .get(&format!("/api/timeline-events/{child_id}"))
        .await;
    assert_eq!(promoted_child["parent_id"], Value::Null);
    let (_, promoted_chapter) = runtime
        .app
        .get(&format!("/api/timeline-events/{chapter_event_id}"))
        .await;
    assert_eq!(promoted_chapter["chapter_id"], second_id);
}

/// The three timeline error branches the tools expose (D-170): `occurred_ancestor`,
/// `planned_descendant`, `occurred_descendant`, `backward`, and `timeline_has_children`.
#[tokio::test]
async fn timeline_tool_errors_render_conflict_reasons() {
    let mut runtime = runtime(vec![]).await;
    let occurred = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "Happened" }),
    )
    .await;
    let occurred_id = occurred["id"].as_i64().expect("id") as i32;
    let plan = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "Plan", "status": "planned" }),
    )
    .await;
    let plan_id = plan["id"].as_i64().expect("id") as i32;
    let scene = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "Scene", "status": "planned", "parent_id": plan_id }),
    )
    .await;
    let scene_id = scene["id"].as_i64().expect("id") as i32;
    let occurred_child = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "Happened child", "parent_id": plan_id }),
    )
    .await;
    let occurred_child_id = occurred_child["id"].as_i64().expect("id") as i32;

    runtime.app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new().tool(
            "add_timeline_event",
            json!({ "title": "Bad plan", "status": "planned", "parent": occurred_id }),
        ),
        ScriptStep::new().tool(
            "update_timeline_event",
            json!({ "event_id": plan_id, "status": "occurred", "note": "promote" }),
        ),
        ScriptStep::new().tool(
            "update_timeline_event",
            json!({ "event_id": plan_id, "status": "abandoned", "note": "drop" }),
        ),
        ScriptStep::new().tool(
            "update_timeline_event",
            json!({ "event_id": occurred_id, "status": "abandoned", "note": "drop" }),
        ),
    ]));
    let session_id = start_archivist(&runtime).await;
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let add = call(&calls, "add_timeline_event").await;
    assert!(add.3);
    assert!(
        add.1.contains("计划中的事件不能放在已发生的事件"),
        "{}",
        add.1
    );
    let updates: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "update_timeline_event")
        .collect();
    assert_eq!(updates.len(), 3);
    assert!(
        updates[0].1.contains("计划中的下级事件"),
        "{}",
        updates[0].1
    );
    assert!(updates[1].1.contains("已经发生"), "{}", updates[1].1);
    assert!(updates[2].1.contains("只能从计划中"), "{}", updates[2].1);

    // A parent with children cannot be deleted through the book-agent route; the API DELETE
    // returns the same branch the tool maps (D-170 T9).
    let (status, body) = runtime
        .app
        .delete(&format!("/api/timeline-events/{plan_id}"))
        .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT);
    nekowriter::testing::assert_error(&body, "timeline.has_children");
    let _ = scene_id;
    let _ = occurred_child_id;
}

/// `query_timeline` prunes the root and abandoned entries, marks planned entries and parents,
/// and `under` lists only the descendants (the node itself is excluded).
#[tokio::test]
async fn query_timeline_prunes_and_marks_the_tree() {
    let mut runtime = runtime(Vec::new()).await;
    let alpha = create_event(&runtime.app, runtime.book_id, json!({ "title": "Alpha" })).await;
    let plan = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "Plan", "status": "planned" }),
    )
    .await;
    let plan_id = plan["id"].as_i64().expect("id") as i32;
    let child = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "Child", "status": "planned", "parent_id": plan_id }),
    )
    .await;
    let child_id = child["id"].as_i64().expect("id") as i32;
    let gone = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "Gone", "status": "planned" }),
    )
    .await;
    let gone_id = gone["id"].as_i64().expect("id") as i32;
    let (status, _) = patch_event(
        &runtime.app,
        gone_id,
        json!({ "status": "abandoned", "note": "drop" }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);

    // `under` uses a path id, not a title; use the plan's id in a script with a real value.
    runtime.app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new().tool("query_timeline", json!({})),
        ScriptStep::new().tool("query_timeline", json!({ "under": plan_id })),
        ScriptStep::new().tool("query_timeline", json!({ "under": 999_000 })),
    ]));
    let session_id = start_archivist(&runtime).await;
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let timeline: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "query_timeline")
        .collect();
    let all = &timeline[0];
    assert!(!all.3, "{}", all.1);
    assert!(all.1.contains("Alpha"));
    assert!(all.1.contains("Plan"));
    assert!(all.1.contains("Child"));
    assert!(!all.1.contains("Gone"), "abandoned is pruned: {}", all.1);
    assert!(all.1.contains("（计划中）"), "planned marker: {}", all.1);
    assert!(all.1.contains("↳ #"), "parent marker: {}", all.1);
    assert!(all.1.contains("尚未发生"), "marker explanation: {}", all.1);
    let under = &timeline[1];
    assert!(!under.3, "{}", under.1);
    assert!(under.1.contains("Child"));
    assert!(
        !under.1.contains(&format!("[#{plan_id}]")),
        "under excludes the node itself: {}",
        under.1
    );
    assert!(!under.1.contains("Alpha"));
    let missing = &timeline[2];
    assert!(missing.3);
    assert!(missing.1.contains("找不到时间线事件"), "{}", missing.1);
    let _ = (alpha["id"].as_i64(), child_id);
}

/// `search_canon` has no root document, removes an abandoned entry's document, and marks a
/// planned event hit (spec/07 §2, D-170 T8).
#[tokio::test]
async fn search_canon_excludes_root_and_abandoned_and_marks_planned() {
    let runtime = runtime(vec![ScriptStep::new().tool(
        "search_canon",
        json!({ "query": "canary", "kinds": ["event"] }),
    )])
    .await;
    let root_id = {
        use sea_orm::{ConnectionTrait, DbBackend, Statement};
        let row = runtime
            .app
            .db()
            .conn()
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!(
                    "SELECT id AS value FROM timeline_events WHERE book_id = {} AND kind = 'root'",
                    runtime.book_id
                ),
            ))
            .await
            .expect("root query")
            .expect("root row");
        row.try_get::<i32>("", "value").expect("root id")
    };
    runtime
        .app
        .execute(&format!(
            "UPDATE timeline_events SET title = 'canary root' WHERE id = {root_id}"
        ))
        .await;
    let occurred = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "canary happened", "description": "canary body" }),
    )
    .await;
    let _planned = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "canary planned", "description": "canary body", "status": "planned" }),
    )
    .await;
    let gone = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "canary gone", "description": "canary body", "status": "planned" }),
    )
    .await;
    let (status, _) = patch_event(
        &runtime.app,
        gone["id"].as_i64().expect("id") as i32,
        json!({ "status": "abandoned", "note": "drop" }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    // Root has no search doc. An abandoned entry's doc is removed by the same transaction.
    let (status, _) = runtime
        .app
        .get(&format!("/api/timeline-events/{}", occurred["id"]))
        .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let session_id = start_archivist(&runtime).await;
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let search = call(&calls, "search_canon").await;
    assert!(!search.3, "{}", search.1);
    assert!(search.1.contains("canary happened"), "{}", search.1);
    assert!(search.1.contains("canary planned"), "{}", search.1);
    assert!(
        search.1.contains("（计划中）"),
        "planned hit marker: {}",
        search.1
    );
    assert!(
        !search.1.contains("canary root"),
        "root is not indexed: {}",
        search.1
    );
    assert!(
        !search.1.contains("canary gone"),
        "abandoned entry is not indexed: {}",
        search.1
    );
}

/// `list_foreshadowing` renders anchors with the planned marker, an abandoned anchor without its
/// title, and a legacy unanchored row's direction.
#[tokio::test]
async fn list_foreshadowing_renders_anchors() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("list_foreshadowing", json!({})),
    ])
    .await;
    let planned_anchor = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "AnchorPlan", "status": "planned" }),
    )
    .await;
    let planned_id = planned_anchor["id"].as_i64().expect("id") as i32;
    let abandoned_anchor = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "AnchorGone", "status": "planned" }),
    )
    .await;
    let abandoned_id = abandoned_anchor["id"].as_i64().expect("id") as i32;
    for body in [
        json!({ "title": "Anchored planned", "description": "", "planned_payoff": "揭示身世", "payoff_event_id": planned_id, "payoff_position": "after" }),
        json!({ "title": "Anchored abandoned", "description": "", "planned_payoff": "", "payoff_event_id": abandoned_id, "payoff_position": "before" }),
        json!({ "title": "Legacy", "description": "", "planned_payoff": "只写方向" }),
    ] {
        let (status, created) = runtime
            .app
            .post(
                &format!("/api/books/{}/foreshadowing", runtime.book_id),
                body,
            )
            .await;
        assert_eq!(status, axum::http::StatusCode::CREATED, "{created}");
    }
    // The anchor may be abandoned later; the item keeps the pointer and the list hides its title.
    let (status, _) = patch_event(
        &runtime.app,
        abandoned_id,
        json!({ "status": "abandoned", "note": "drop" }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let session_id = start_archivist(&runtime).await;
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let list = call(&calls, "list_foreshadowing").await;
    assert!(!list.3, "{}", list.1);
    assert!(
        list.1.contains(&format!(
            "回收锚点：#{planned_id} AnchorPlan 之后（计划中）"
        )),
        "planned anchor: {}",
        list.1
    );
    assert!(
        list.1.contains("（回收锚点已放弃）") && !list.1.contains("AnchorGone"),
        "abandoned anchor without its title: {}",
        list.1
    );
    assert!(list.1.contains("回收方向：只写方向"), "{}", list.1);
}

/// `add_foreshadowing` / `update_foreshadowing` validate anchors: half an anchor is a validation
/// error, a foreign or abandoned anchor is `not_found`, and re-anchor/clear work through the
/// service. `resolve_foreshadowing` against a planned anchor succeeds and carries the notice.
#[tokio::test]
async fn foreshadowing_anchor_tools_reanchor_and_clear() {
    let mut runtime = runtime(vec![]).await;
    let planned = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "Planned payoff", "status": "planned" }),
    )
    .await;
    let planned_id = planned["id"].as_i64().expect("id") as i32;
    let happened = create_event(
        &runtime.app,
        runtime.book_id,
        json!({ "title": "Happened payoff" }),
    )
    .await;
    let happened_id = happened["id"].as_i64().expect("id") as i32;
    let hook = runtime
        .app
        .post(
            &format!("/api/books/{}/foreshadowing", runtime.book_id),
            json!({ "title": "Hook", "description": "A hook.", "planned_payoff": "旧方向", "payoff_event_id": planned_id, "payoff_position": "after" }),
        )
        .await
        .1;
    let hook_id = hook["id"].as_i64().expect("id") as i32;
    let clear_hook = runtime
        .app
        .post(
            &format!("/api/books/{}/foreshadowing", runtime.book_id),
            json!({ "title": "Clear hook", "description": "x", "payoff_event_id": planned_id, "payoff_position": "at" }),
        )
        .await
        .1;
    let clear_hook_id = clear_hook["id"].as_i64().expect("id") as i32;
    let resolve_hook = runtime
        .app
        .post(
            &format!("/api/books/{}/foreshadowing", runtime.book_id),
            json!({ "title": "Resolve hook", "description": "x", "payoff_event_id": planned_id, "payoff_position": "after" }),
        )
        .await
        .1;
    let resolve_hook_id = resolve_hook["id"].as_i64().expect("id") as i32;

    runtime
        .app
        .set_provider(ScriptedProvider::new(vec![
            ScriptStep::new().tool(
                "add_foreshadowing",
                json!({ "title": "Half", "description": "x", "payoff_event": planned_id }),
            ),
            ScriptStep::new().tool(
                "add_foreshadowing",
                json!({ "title": "Foreign", "description": "x", "payoff_event": 999_000, "payoff_position": "at" }),
            ),
            ScriptStep::new().tool(
                "update_foreshadowing",
                json!({ "foreshadowing_id": hook_id, "note": "re-anchor", "payoff_event": happened_id, "payoff_position": "before" }),
            ),
            ScriptStep::new().tool(
                "update_foreshadowing",
                json!({ "foreshadowing_id": clear_hook_id, "note": "clear", "clear_anchor": true }),
            ),
            ScriptStep::new().tool(
                "resolve_foreshadowing",
                json!({ "foreshadowing_id": resolve_hook_id, "status": "resolved", "resolution": "已回收" }),
            ),
        ]));
    let session_id = start_archivist(&runtime).await;
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let adds: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "add_foreshadowing")
        .collect();
    assert!(adds[0].3 && adds[0].1.contains("payoff"), "{}", adds[0].1);
    assert!(
        adds[1].3 && adds[1].1.contains("找不到时间线事件"),
        "{}",
        adds[1].1
    );
    let updates: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "update_foreshadowing")
        .collect();
    assert_eq!(updates.len(), 2, "{calls:?}");
    assert!(!updates[0].3, "{}", updates[0].1);
    let (_, reanchored) = runtime
        .app
        .get(&format!("/api/foreshadowing/{hook_id}"))
        .await;
    assert_eq!(reanchored["payoff_event_id"], happened_id);
    assert_eq!(reanchored["payoff_position"], "before");
    assert!(!updates[1].3, "{}", updates[1].1);
    let (_, cleared) = runtime
        .app
        .get(&format!("/api/foreshadowing/{clear_hook_id}"))
        .await;
    assert_eq!(cleared["payoff_event_id"], Value::Null);
    assert_eq!(cleared["payoff_position"], Value::Null);
    let resolve = call(&calls, "resolve_foreshadowing").await;
    assert!(!resolve.3, "{}", resolve.1);
    assert!(resolve.1.contains("仍是计划中"), "{}", resolve.1);
}

/// With Timeline off, an anchor parameter is `error.system_off`; Foreshadowing off removes
/// `update_foreshadowing` from the frozen list (spec/05 §2, D-170 T17).
#[tokio::test]
async fn timeline_off_refuses_anchor_parameters_and_foreshadowing_off_hides_the_tool() {
    let mut runtime = runtime(vec![]).await;
    let anchor = create_event(&runtime.app, runtime.book_id, json!({ "title": "Anchor" })).await;
    let anchor_id = anchor["id"].as_i64().expect("id") as i32;
    let (_, book) = runtime
        .app
        .get(&format!("/api/books/{}", runtime.book_id))
        .await;
    let mut settings = book["settings"].clone();
    settings["canon_systems"] = serde_json::to_value(CanonSystems {
        timeline: false,
        ..CanonSystems::default()
    })
    .expect("systems");
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/books/{}", runtime.book_id),
            json!({ "settings": settings }),
        )
        .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    runtime
        .app
        .set_provider(ScriptedProvider::new(vec![ScriptStep::new().tool(
            "add_foreshadowing",
            json!({ "title": "Anchored", "description": "x", "payoff_event": anchor_id, "payoff_position": "at" }),
        )]));
    let session_id = start_archivist(&runtime).await;
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let add = call(&calls, "add_foreshadowing").await;
    assert!(add.3);
    assert!(
        add.1.contains("时间线") && add.1.contains("已关闭"),
        "{}",
        add.1
    );

    let systems = CanonSystems {
        foreshadowing: false,
        ..CanonSystems::default()
    };
    let tools = tools_for_session(
        "archivist",
        SessionKind::ChapterMaintenance,
        &systems,
        true,
        false,
        &ToolRegistry::new(),
    );
    assert!(!tools.contains(&"update_foreshadowing"), "{tools:?}");
    let all = tools_for_session(
        "archivist",
        SessionKind::ChapterMaintenance,
        &CanonSystems::default(),
        true,
        false,
        &ToolRegistry::new(),
    );
    assert_eq!(all.len(), 23);
    assert_eq!(all[17], "update_foreshadowing");
    assert_eq!(ARCHIVIST_TOOLS.len(), 23);
}

/// A session frozen before M22 still runs `add_timeline_event` with only its old arguments: every
/// new parameter is optional (plan §3.3).
#[tokio::test]
async fn a_pre_m22_frozen_session_accepts_the_old_arguments() {
    let mut runtime = runtime(Vec::new()).await;
    runtime
        .app
        .set_provider(ScriptedProvider::new(vec![ScriptStep::new().tool(
            "add_timeline_event",
            json!({ "title": "Old shape", "description": "no parent, no status" }),
        )]));
    let session_id = start_archivist(&runtime).await;
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let add = call(&calls, "add_timeline_event").await;
    assert!(!add.3, "{}", add.1);
    assert!(add.1.contains("已加入时间线"), "{}", add.1);
}
