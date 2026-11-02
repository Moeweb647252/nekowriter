//! Retrieval tool tests (PLAN M5.4, spec/05 §3).
//!
//! Every tool is driven through a real agent session: the `ScriptedProvider` emits the call and
//! the assertions read the persisted tool-call result, so what is checked is the text the model
//! would actually see, plus the `ui` payload (spec/05 §1.6, §9).

mod agent_write_harness;

use agent_write_harness::base::runtime;
use nekowriter_llm::mock::ScriptStep;
use serde_json::{Value, json};

/// A runtime with one scripted step, run through a real writer session.
async fn call_tool_with(steps: Vec<ScriptStep>) -> agent_write_harness::Runtime {
    runtime(steps).await
}

/// The calls of the finished session, as `(name, result_text, ui, is_error)`.
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

/// Seeds one entity through the canon API.
async fn entity(
    runtime: &agent_write_harness::Runtime,
    kind: &str,
    name: &str,
    summary: &str,
    importance: i32,
) -> i32 {
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{}/entities", runtime.book_id),
            json!({
                "kind": kind,
                "name": name,
                "summary": summary,
                "state": format!("{name} is here"),
                "description": format!("{name} described"),
                "attributes": { "rank": "3" },
                "importance": importance,
                "aliases": [format!("alias-of-{name}")],
            }),
        )
        .await;
    assert_eq!(status, 201, "create entity: {body}");
    body["id"].as_i64().expect("id") as i32
}

// ───────────────────────────── get_entity ─────────────────────────────

/// `get_entity` renders the entity card with its state, attributes, relations, changes and
/// appearances, and returns `{entity_id, name, kind}` (spec/05 §3).
#[tokio::test]
async fn get_entity_renders_the_card_and_its_ui() {
    let runtime = call_tool_with(vec![
        ScriptStep::new().tool("get_entity", json!({ "entity": "#1" })),
    ])
    .await;
    let target = entity(&runtime, "character", "Lin Fan", "an outer disciple", 5).await;
    let other = entity(&runtime, "character", "Zhang San", "a senior", 4).await;
    let (status, _) = runtime
        .app
        .post(
            &format!("/api/books/{}/relations", runtime.book_id),
            json!({
                "source_id": target,
                "target_id": other,
                "kind": "师兄",
                "inverse_kind": "师弟",
                "description": "same sect",
            }),
        )
        .await;
    assert_eq!(status, 201);
    // An agent-style change note on the first chapter.
    let (status, _) = runtime
        .app
        .patch(
            &format!("/api/entities/{target}"),
            json!({ "state": "in the sword tomb", "note": "修为 筑基初期→筑基中期" }),
        )
        .await;
    assert_eq!(status, 200);
    runtime
        .app
        .execute(&format!(
            "UPDATE change_log SET chapter_id = {} WHERE target_type = 'entity' AND \
             target_id = {target} AND note != ''",
            runtime.chapter_id
        ))
        .await;

    let session_id = agent_write_harness::start_planner(&runtime).await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let (name, text, ui, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "get_entity")
        .expect("the get_entity call");
    assert_eq!(name, "get_entity");
    assert!(!is_error, "the call succeeded: {text}");
    assert!(
        text.contains("【人物】Lin Fan(#"),
        "the card names the kind and the entity: {text}"
    );
    assert!(text.contains("当前状态：in the sword tomb"), "{text}");
    assert!(text.contains("rank=3"), "the attributes: {text}");
    assert!(
        text.contains("Lin Fan(#") && text.contains("师兄"),
        "the relation is phrased from this entity: {text}"
    );
    assert!(
        text.contains("修为 筑基初期→筑基中期"),
        "the change note: {text}"
    );
    assert_eq!(ui["entity_id"], json!(target));
    assert_eq!(ui["name"], json!("Lin Fan"));
    assert_eq!(ui["kind"], json!("character"));
}

/// `get_entity` resolves an exact name, an alias and a unique substring (spec/05 §1.7).
#[tokio::test]
async fn get_entity_resolves_names_aliases_and_substrings() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("get_entity", json!({ "entity": "Lin Fan" })),
        ScriptStep::new().tool("get_entity", json!({ "entity": "alias-of-Lin Fan" })),
        ScriptStep::new().tool("get_entity", json!({ "entity": "Zhang" })),
    ])
    .await;
    entity(&runtime, "character", "Lin Fan", "an outer disciple", 5).await;
    entity(&runtime, "character", "Zhang San", "a senior", 4).await;
    let session_id = agent_write_harness::start_planner(&runtime).await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let get_entities: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "get_entity")
        .collect();
    assert_eq!(get_entities.len(), 3);
    for (index, (_, text, ui, is_error)) in get_entities.iter().enumerate() {
        assert!(!is_error, "call {index} resolved: {text}");
        assert!(
            ui["entity_id"].is_i64(),
            "call {index} has a ui entity_id: {ui}"
        );
    }
    assert_eq!(get_entities[0].2["name"], json!("Lin Fan"));
    assert_eq!(
        get_entities[1].2["name"],
        json!("Lin Fan"),
        "the alias resolves"
    );
    assert_eq!(
        get_entities[2].2["name"],
        json!("Zhang San"),
        "the substring resolves"
    );
}

/// An unknown entity is `error.entity_not_found` with up to 5 suggestions (spec/05 §1.7).
#[tokio::test]
async fn get_entity_reports_entity_not_found_with_suggestions() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("get_entity", json!({ "entity": "Lin Fann" })),
    ])
    .await;
    entity(&runtime, "character", "Lin Fan", "an outer disciple", 5).await;
    let session_id = agent_write_harness::start_planner(&runtime).await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let (_, text, _, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "get_entity")
        .expect("the call");
    assert!(is_error);
    assert!(text.contains("找不到实体"), "{text}");
    assert!(
        text.contains("Lin Fan") && text.contains("你是不是要找"),
        "the suggestions: {text}"
    );
}

/// Several matches are `error.entity_ambiguous` with the candidates (spec/05 §1.7).
#[tokio::test]
async fn get_entity_reports_entity_ambiguous() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("get_entity", json!({ "entity": "Lin" })),
    ])
    .await;
    entity(&runtime, "character", "Lin Fan", "an outer disciple", 5).await;
    entity(&runtime, "character", "Lin Yue", "a swordswoman", 4).await;
    let session_id = agent_write_harness::start_planner(&runtime).await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let (_, text, _, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "get_entity")
        .expect("the call");
    assert!(is_error);
    assert!(
        text.contains("对应多个实体"),
        "the ambiguous wording: {text}"
    );
    assert!(
        text.contains("Lin Fan") && text.contains("Lin Yue"),
        "{text}"
    );
}

// ───────────────────────────── list_entities ─────────────────────────────

/// `list_entities` renders one line per entity sorted by importance desc, and `{count}`
/// (spec/05 §3).
#[tokio::test]
async fn list_entities_renders_one_line_per_entity() {
    let runtime = runtime(vec![ScriptStep::new().tool("list_entities", json!({}))]).await;
    entity(&runtime, "character", "Minor", "a minor", 2).await;
    entity(&runtime, "character", "Central", "the central one", 5).await;
    let session_id = agent_write_harness::start_planner(&runtime).await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let (_, text, ui, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "list_entities")
        .expect("the call");
    assert!(!is_error, "{text}");
    assert!(text.contains("【人物】Central"), "{text}");
    assert!(text.contains("（alias-of-Central）"), "the aliases: {text}");
    let central = text.find("Central").expect("central");
    let minor = text.find("Minor").expect("minor");
    assert!(
        central < minor,
        "sorted by importance desc, so the central entity comes first: {text}"
    );
    assert_eq!(ui["count"], json!(2));
}

/// `list_entities` filters by kind, status and a query, and caps at the requested limit
/// (spec/05 §3).
#[tokio::test]
async fn list_entities_filters_and_caps() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("list_entities", json!({ "kind": "item" })),
        ScriptStep::new().tool("list_entities", json!({ "query": "sword" })),
        ScriptStep::new().tool("list_entities", json!({ "limit": 1 })),
    ])
    .await;
    entity(&runtime, "character", "Lin Fan", "an outer disciple", 5).await;
    entity(&runtime, "item", "Broken Sword", "a hilt", 4).await;
    entity(&runtime, "item", "Old Map", "a map", 2).await;
    let session_id = agent_write_harness::start_planner(&runtime).await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let list: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "list_entities")
        .collect();
    assert_eq!(list.len(), 3);
    // Kind filter.
    assert!(list[0].1.contains("Broken Sword") && list[0].1.contains("Old Map"));
    assert!(
        !list[0].1.contains("Lin Fan"),
        "the kind filter excludes characters"
    );
    assert_eq!(list[0].2["count"], json!(2));
    // Query filter.
    assert!(list[1].1.contains("Broken Sword"));
    assert!(!list[1].1.contains("Old Map"));
    assert_eq!(list[1].2["count"], json!(1));
    // Limit.
    assert_eq!(list[2].2["count"], json!(1));
}

// ───────────────────────────── get_relations ─────────────────────────────

/// `get_relations` phrases each relation from the queried entity's perspective, using the
/// inverse kind when the entity is the target (spec/06 §1.2).
#[tokio::test]
async fn get_relations_phrases_from_the_queried_entity() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("get_relations", json!({ "entity": "Zhang San" })),
    ])
    .await;
    let source = entity(&runtime, "character", "Lin Fan", "an outer disciple", 5).await;
    let target = entity(&runtime, "character", "Zhang San", "a senior", 4).await;
    let (status, _) = runtime
        .app
        .post(
            &format!("/api/books/{}/relations", runtime.book_id),
            json!({
                "source_id": source,
                "target_id": target,
                "kind": "师兄",
                "inverse_kind": "师弟",
                "description": "same sect",
            }),
        )
        .await;
    assert_eq!(status, 201);
    let session_id = agent_write_harness::start_planner(&runtime).await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let (_, text, ui, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "get_relations")
        .expect("the call");
    assert!(!is_error, "{text}");
    assert!(
        text.contains("Lin Fan(#") && text.contains("师弟"),
        "the inverse kind phrases it from Zhang San's side: {text}"
    );
    assert!(text.contains("same sect"), "the description: {text}");
    assert_eq!(ui["entity_id"], json!(target));
    assert_eq!(ui["count"], json!(1));
}

/// `get_relations` hides ended relations unless `include_ended` is set (spec/05 §3).
#[tokio::test]
async fn get_relations_hides_ended_ones_by_default() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("get_relations", json!({ "entity": "Lin Fan" })),
        ScriptStep::new().tool(
            "get_relations",
            json!({ "entity": "Lin Fan", "include_ended": true }),
        ),
    ])
    .await;
    let source = entity(&runtime, "character", "Lin Fan", "an outer disciple", 5).await;
    let target = entity(&runtime, "character", "Zhang San", "a senior", 4).await;
    let (status, relation) = runtime
        .app
        .post(
            &format!("/api/books/{}/relations", runtime.book_id),
            json!({ "source_id": source, "target_id": target, "kind": "enemy" }),
        )
        .await;
    assert_eq!(status, 201);
    let (status, _) = runtime
        .app
        .patch(
            &format!("/api/relations/{}", relation["id"]),
            json!({ "status": "ended" }),
        )
        .await;
    assert_eq!(status, 200);
    let session_id = agent_write_harness::start_planner(&runtime).await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let list: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "get_relations")
        .collect();
    assert_eq!(list[0].2["count"], json!(0), "ended relations are hidden");
    assert_eq!(list[1].2["count"], json!(1), "include_ended shows them");
    assert!(
        list[1].1.contains("已结束"),
        "the ended marker: {}",
        list[1].1
    );
}

// ───────────────────────────── query_timeline ─────────────────────────────

/// `query_timeline` returns events in story order with story time, chapter label and entities,
/// and `{count}` (spec/05 §3).
#[tokio::test]
async fn query_timeline_renders_events_in_story_order() {
    let runtime = runtime(vec![ScriptStep::new().tool("query_timeline", json!({}))]).await;
    let lin = entity(&runtime, "character", "Lin Fan", "an outer disciple", 5).await;
    for (title, time) in [("Third", "Day 3"), ("First", "Day 1"), ("Second", "Day 2")] {
        let (status, _) = runtime
            .app
            .post(
                &format!("/api/books/{}/timeline", runtime.book_id),
                json!({
                    "title": title,
                    "story_time": time,
                    "description": format!("{title} happened"),
                    "entity_ids": [lin],
                }),
            )
            .await;
        assert_eq!(status, 201);
    }
    // Rename so the story order differs from insertion order (a flashback in the middle).
    let (_, events) = runtime
        .app
        .get(&format!("/api/books/{}/timeline", runtime.book_id))
        .await;
    let ids: Vec<i32> = events
        .as_array()
        .expect("events")
        .iter()
        .map(|event| event["id"].as_i64().expect("id") as i32)
        .collect();
    // Insertion order was Third, First, Second (positions 1, 2, 3). Moving Second before First
    // puts it between Third and First, so story order becomes Third → Second → First — different
    // from the insertion order, which is what the tool must return.
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/timeline-events/{}/move", ids[2]),
            json!({ "before_id": ids[1] }),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    let session_id = agent_write_harness::start_planner(&runtime).await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let (_, text, ui, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "query_timeline")
        .expect("the call");
    assert!(!is_error, "{text}");
    assert!(text.contains("时间线（按故事时间先后）"), "{text}");
    let first = text.find("First").expect("first");
    let third = text.find("Third").expect("third");
    let second = text.find("Second").expect("second");
    assert!(
        third < second && second < first,
        "story order follows the positions, not the insertion order: {text}"
    );
    assert!(text.contains("(Day 1)"), "the story time: {text}");
    assert!(text.contains("涉及：Lin Fan(#"), "the entities: {text}");
    assert_eq!(ui["count"], json!(3));
}

/// `query_timeline` supports `entity`, `query`, `order` and `around_event` (spec/05 §3).
#[tokio::test]
async fn query_timeline_filters() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("query_timeline", json!({ "order": "recent_first" })),
        ScriptStep::new().tool("query_timeline", json!({ "query": "battle" })),
        ScriptStep::new().tool("query_timeline", json!({ "around_event": 2 })),
        ScriptStep::new().tool("query_timeline", json!({ "entity": "Lin Fan" })),
    ])
    .await;
    let lin = entity(&runtime, "character", "Lin Fan", "an outer disciple", 5).await;
    let other = entity(&runtime, "character", "Zhang San", "a senior", 4).await;
    for (title, entity_ids) in [
        ("The gate", vec![lin]),
        ("The battle", vec![lin]),
        ("The parting", vec![other]),
    ] {
        let (status, _) = runtime
            .app
            .post(
                &format!("/api/books/{}/timeline", runtime.book_id),
                json!({ "title": title, "entity_ids": entity_ids }),
            )
            .await;
        assert_eq!(status, 201);
    }
    let (_, events) = runtime
        .app
        .get(&format!("/api/books/{}/timeline", runtime.book_id))
        .await;
    let ids: Vec<i32> = events
        .as_array()
        .expect("events")
        .iter()
        .map(|event| event["id"].as_i64().expect("id") as i32)
        .collect();

    let session_id = agent_write_harness::start_planner(&runtime).await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let timeline: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "query_timeline")
        .collect();
    assert_eq!(timeline.len(), 4);
    // recent_first reverses the order.
    assert!(
        timeline[0].1.contains("时间线（由近及远）"),
        "{}",
        timeline[0].1
    );
    assert_eq!(timeline[0].2["count"], json!(3));
    // query narrows to the battle.
    assert_eq!(timeline[1].2["count"], json!(1));
    assert!(timeline[1].1.contains("The battle"));
    // around_event keeps a window.
    assert_eq!(timeline[2].2["count"], json!(3));
    // entity narrows to Lin Fan's events.
    assert_eq!(timeline[3].2["count"], json!(2));
    assert!(!timeline[3].1.contains("The parting"));
    let _ = ids;
}

// ───────────────────────────── list_foreshadowing ─────────────────────────────

/// `list_foreshadowing` lists open items by default with the planted chapter and planned payoff,
/// and `{count}` (spec/05 §3).
#[tokio::test]
async fn list_foreshadowing_lists_open_items() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("list_foreshadowing", json!({})),
        ScriptStep::new().tool("list_foreshadowing", json!({ "status": "resolved" })),
    ])
    .await;
    let (status, open) = runtime
        .app
        .post(
            &format!("/api/books/{}/foreshadowing", runtime.book_id),
            json!({
                "title": "The broken hilt",
                "description": "A hilt with no blade.",
                "planned_payoff": "The blade returns.",
                "planted_chapter_id": runtime.chapter_id,
            }),
        )
        .await;
    assert_eq!(status, 201, "{open}");
    let (status, resolved) = runtime
        .app
        .post(
            &format!("/api/books/{}/foreshadowing", runtime.book_id),
            json!({ "title": "The old letter", "description": "Unopened." }),
        )
        .await;
    assert_eq!(status, 201);
    let (status, _) = runtime
        .app
        .patch(
            &format!("/api/foreshadowing/{}", resolved["id"]),
            json!({ "status": "resolved", "resolution": "Opened at last." }),
        )
        .await;
    assert_eq!(status, 200);

    let session_id = agent_write_harness::start_planner(&runtime).await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let list: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "list_foreshadowing")
        .collect();
    assert_eq!(
        list[0].2["count"],
        json!(1),
        "only the open item by default"
    );
    assert!(list[0].1.contains("The broken hilt"));
    assert!(list[0].1.contains("计划回收：The blade returns."));
    assert!(
        list[0].1.contains("埋设于第"),
        "the planted chapter: {}",
        list[0].1
    );
    assert_eq!(list[1].2["count"], json!(1), "the resolved tab");
    assert!(list[1].1.contains("The old letter"));
    assert!(list[1].1.contains("回收：Opened at last."));
}

// ───────────────────────────── read_chapter ─────────────────────────────

/// `read_chapter` returns the official text with pids and `{chapter_id, track_id, number}`
/// (spec/05 §3).
#[tokio::test]
async fn read_chapter_returns_the_official_text_with_pids() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("read_chapter", json!({ "chapter": "1" })),
    ])
    .await;
    runtime
        .app
        .put_text(
            runtime.chapter_id,
            0,
            "First paragraph.\n\nSecond paragraph.",
        )
        .await;
    let session_id = agent_write_harness::start_planner(&runtime).await;
    // Creating a writer session moves the chapter to `drafting`; the tool reads the row at call
    // time, so the chapter is made official after the session exists.
    runtime
        .app
        .set_chapter_status(runtime.chapter_id, "official")
        .await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let (_, text, ui, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "read_chapter")
        .expect("the call");
    assert!(!is_error, "{text}");
    assert!(text.contains("[p1] First paragraph."), "{text}");
    assert!(text.contains("[p2] Second paragraph."), "{text}");
    assert_eq!(ui["chapter_id"], json!(runtime.chapter_id));
    assert_eq!(ui["track_id"], json!(runtime.track_id));
    assert_eq!(ui["number"], json!(1));
}

/// A non-official chapter is `error.chapter_not_official` (spec/05 §3).
#[tokio::test]
async fn read_chapter_rejects_a_non_official_chapter() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("read_chapter", json!({ "chapter": "1" })),
    ])
    .await;
    runtime
        .app
        .put_text(runtime.chapter_id, 0, "Draft text.")
        .await;
    let session_id = agent_write_harness::start_planner(&runtime).await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let (_, text, _, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "read_chapter")
        .expect("the call");
    assert!(is_error);
    assert!(text.contains("尚未定稿"), "{text}");
}

/// An unknown chapter reference is `error.chapter_not_found` (spec/05 §3).
#[tokio::test]
async fn read_chapter_rejects_an_unknown_reference() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("read_chapter", json!({ "chapter": "99" })),
        ScriptStep::new().tool("read_chapter", json!({ "chapter": "not a chapter" })),
    ])
    .await;
    let session_id = agent_write_harness::start_planner(&runtime).await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let reads: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "read_chapter")
        .collect();
    assert_eq!(reads.len(), 2);
    for (_, text, _, is_error) in reads {
        assert!(is_error);
        assert!(text.contains("找不到章节"), "{text}");
        assert!(
            text.contains("\"12\"") || text.contains("main 12"),
            "the hint names the accepted forms: {text}"
        );
    }
}

/// `read_chapter` honours `from_pid` and `max_chars`, with the continuation hint
/// (spec/05 §1.5, §3).
#[tokio::test]
async fn read_chapter_pages_with_from_pid_and_max_chars() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("read_chapter", json!({ "chapter": "1", "from_pid": "p2" })),
        ScriptStep::new().tool("read_chapter", json!({ "chapter": "1", "max_chars": 500 })),
    ])
    .await;
    let mut text = String::new();
    for index in 1..=30 {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&format!(
            "Paragraph {index} has enough words to fill the budget of the reader tool."
        ));
    }
    runtime.app.put_text(runtime.chapter_id, 0, &text).await;
    let session_id = agent_write_harness::start_planner(&runtime).await;
    // Creating a writer session moves the chapter to `drafting`; the tool reads the row at call
    // time, so the chapter is made official after the session exists.
    runtime
        .app
        .set_chapter_status(runtime.chapter_id, "official")
        .await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let reads: Vec<_> = calls
        .iter()
        .filter(|(name, _, _, _)| name == "read_chapter")
        .collect();
    assert!(!reads[0].3, "{}", reads[0].1);
    assert!(
        !reads[0].1.contains("[p1]"),
        "from_pid starts the range: {}",
        reads[0].1
    );
    assert!(reads[0].1.contains("[p2]"));
    assert!(
        reads[1].1.contains("继续阅读请使用 from_pid="),
        "the continuation hint: {}",
        reads[1].1
    );
}

/// An unknown `from_pid` is `error.pid_not_found` naming the tool (spec/05 §1.5).
#[tokio::test]
async fn read_chapter_rejects_an_unknown_pid() {
    let runtime =
        runtime(vec![ScriptStep::new().tool(
            "read_chapter",
            json!({ "chapter": "1", "from_pid": "p99" }),
        )])
        .await;
    runtime
        .app
        .put_text(runtime.chapter_id, 0, "Only paragraph.")
        .await;
    let session_id = agent_write_harness::start_planner(&runtime).await;
    // Creating a writer session moves the chapter to `drafting`; the tool reads the row at call
    // time, so the chapter is made official after the session exists.
    runtime
        .app
        .set_chapter_status(runtime.chapter_id, "official")
        .await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let (_, text, _, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "read_chapter")
        .expect("the call");
    assert!(is_error);
    assert!(text.contains("找不到段落"), "{text}");
    assert!(text.contains("p99"), "{text}");
    assert!(
        text.contains("从头阅读"),
        "a read_chapter pid miss points back to reading the chapter: {text}"
    );
}

// ───────────────────────────── anchor markers (spec/06 §2) ─────────────────────────────

/// A side track with an `after_chapter` anchor marks main-story items from later chapters with
/// `（锚点之后）`, and `before_start` marks all of them (spec/06 §2, spec/08 §6).
#[tokio::test]
async fn a_side_track_marks_items_after_its_anchor() {
    // The fixture needs a side track and a chapter on it, which the tracks API cannot create
    // until M10; the anchor marker helper is exercised over a main-track session with the
    // `before_start` anchor set on the main track's row would be wrong, so the test builds the
    // rows directly and drives the tool through a session whose track is the side track.
    let runtime = runtime(vec![
        ScriptStep::new().tool("query_timeline", json!({})),
        ScriptStep::new().tool(
            "submit_outline",
            json!({ "outline": "番外细纲", "reference_notes": "- 要点" }),
        ),
    ])
    .await;
    let book_id = runtime.book_id;
    // Two main-track events, one before and one after the anchor chapter.
    let (status, _) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/timeline"),
            json!({ "title": "Before the anchor" }),
        )
        .await;
    assert_eq!(status, 201);
    runtime
        .app
        .execute(&format!(
            "UPDATE timeline_events SET chapter_id = {} WHERE book_id = {book_id} AND \
             title = 'Before the anchor'",
            runtime.chapter_id
        ))
        .await;
    let second = runtime.app.create_chapter(runtime.track_id, "Second").await;
    let second_id = second["id"].as_i64().expect("id") as i32;
    let (status, _) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/timeline"),
            json!({ "title": "After the anchor" }),
        )
        .await;
    assert_eq!(status, 201);
    runtime
        .app
        .execute(&format!(
            "UPDATE timeline_events SET chapter_id = {second_id} WHERE book_id = {book_id} AND \
             title = 'After the anchor'"
        ))
        .await;

    // The side track anchored after the first chapter.
    runtime
        .app
        .execute(&format!(
            "INSERT INTO tracks(book_id, kind, title, synopsis, canon_mode, anchor_kind, \
             anchor_chapter_id, ord, created_at, updated_at) VALUES ({book_id}, 'side', \
             'Side', '', 'canon', 'after_chapter', {}, 1, '2026-01-01T00:00:00Z', \
             '2026-01-01T00:00:00Z')",
            runtime.chapter_id
        ))
        .await;
    let side_track = max_track_id(&runtime, book_id).await;
    let side_chapter = runtime.app.create_chapter(side_track, "Side one").await;
    let side_chapter_id = side_chapter["id"].as_i64().expect("id") as i32;

    // Start a writer session on the side track's chapter.
    let (session_id, _) = nekowriter::services::sessions::create(
        runtime.app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: nekowriter::db::json::SessionKind::ChapterOutline,
            book_id: Some(book_id),
            chapter_id: Some(side_chapter_id),
            pause_after_outline: Some(false),
            ..nekowriter::services::sessions::CreateSession::default()
        },
    )
    .await
    .expect("side-track session");
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let (_, text, _, is_error) = calls
        .iter()
        .find(|(name, _, _, _)| name == "query_timeline")
        .expect("the call");
    assert!(!is_error, "{text}");
    assert!(
        text.contains("（锚点之后）"),
        "the main-story item after the anchor is marked: {text}"
    );
    let before = text.find("Before the anchor").expect("before");
    let after = text.find("After the anchor").expect("after");
    assert!(
        !text[before..after].contains("（锚点之后）"),
        "the item before the anchor is not marked: {text}"
    );
}

/// The id of the book's newest track (a fixture read; `query_json` does not map SQLite integers).
async fn max_track_id(runtime: &agent_write_harness::Runtime, book_id: i32) -> i32 {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("SELECT max(id) AS value FROM tracks WHERE book_id = {book_id}"),
        ))
        .await
        .expect("fixture query")
        .expect("fixture row");
    row.try_get::<i32>("", "value").expect("track id")
}

// ───────────────────────────── the phase matrix (spec/05 §2) ─────────────────────────────

/// Retrieval tools are rejected in drafting (spec/05 §2), so a drafting-phase retrieval call is
/// `error.tool_not_allowed`.
#[tokio::test]
async fn retrieval_tools_are_rejected_in_drafting() {
    let runtime = runtime(vec![
        ScriptStep::new().tool("get_entity", json!({ "entity": "Lin Fan" })),
    ])
    .await;
    entity(&runtime, "character", "Lin Fan", "an outer disciple", 5).await;
    // A shipped write session starts in `drafting`: retrieval belongs to the planner now.
    let session_id = agent_write_harness::start_writer(
        &runtime,
        nekowriter::db::json::Granularity::Paragraph,
        false,
        false,
    )
    .await;
    // The script ends after the retrieval call, so the session terminates; the tool-call rows are
    // already persisted, which is what these assertions read.
    runtime.wait_any_status(session_id).await;
    let calls = calls(&runtime, session_id).await;
    let get_entity = calls
        .iter()
        .find(|(name, _, _, _)| name == "get_entity")
        .expect("the call");
    assert!(get_entity.3, "a drafting-phase retrieval call is rejected");
    assert!(
        get_entity.1.contains("正文阶段不能使用"),
        "the tool_not_allowed wording: {}",
        get_entity.1
    );
}
