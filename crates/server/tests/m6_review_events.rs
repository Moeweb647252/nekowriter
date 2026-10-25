//! M6 milestone review regressions for the archivist write tools (spec/05 §6, spec/06 §3,
//! spec/10 §3).
//!
//! `services::canon` returns the `canon.changed` event of every write. The archivist tools used
//! those events only to fill `ui.change_ids` and never published them, so the canon pages and the
//! memory snapshot in the browser never heard about a maintenance run — and the M7 search index,
//! which is driven by the same events, would have missed every maintenance write too.

mod review_harness;

use std::time::Duration;

use nekowriter::db::json::SessionStatus;
use nekowriter::events::ServerEvent;
use nekowriter_llm::mock::ScriptStep;
use review_harness::{prepare_in_review, runtime};
use serde_json::json;

fn step(name: &str, arguments: serde_json::Value) -> ScriptStep {
    ScriptStep::new().tool(name, arguments)
}

/// Every maintenance write publishes its `canon.changed` event, with the right target type.
#[tokio::test]
async fn archivist_canon_writes_publish_canon_changed_events() {
    let runtime = runtime(vec![
        step(
            "create_entity",
            json!({ "kind": "character", "name": "林凡", "summary": "主角" }),
        ),
        step(
            "update_entity",
            json!({ "entity": "林凡", "state": "身处剑冢", "note": "进入剑冢" }),
        ),
        step(
            "record_appearances",
            json!({ "appearances": [ { "entity": "林凡", "role": "pov" } ] }),
        ),
        step(
            "set_chapter_summary",
            json!({ "summary": "林凡进入剑冢。" }),
        ),
        step("finish_maintenance", json!({ "report": "完成" })),
    ])
    .await;
    prepare_in_review(&runtime).await;

    // Subscribe before approval: the auto-maintenance session starts in the approval transaction.
    let mut events = runtime.app.state().events().subscribe();
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/approve", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "approve: {body}");
    let session = body["maintenance_session_id"]
        .as_i64()
        .expect("maintenance session") as i32;
    runtime
        .wait_status_within(session, SessionStatus::Completed, Duration::from_secs(20))
        .await;
    // The events are published right after each commit, so drain until every expected kind has
    // arrived. A fixed settle sleep would be a load-sensitive wait, not a synchronization.
    let mut canon: Vec<serde_json::Value> = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let seen = |canon: &Vec<serde_json::Value>, target: &str| {
        canon.iter().any(|event| event["target_type"] == target)
    };
    loop {
        while let Ok(envelope) = events.try_recv() {
            if let ServerEvent::CanonChanged { .. } = &envelope.event {
                canon.push(serde_json::to_value(&envelope.event).expect("event json"));
            }
        }
        if ["entity", "appearance_set", "chapter_summary"]
            .iter()
            .all(|target| seen(&canon, target))
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the maintenance session did not publish every canon event: {canon:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    canon.sort_by_key(|event| event["change_id"].as_i64().unwrap_or(0));
    let targets: Vec<&str> = canon
        .iter()
        .filter_map(|event| event["target_type"].as_str())
        .collect();
    for expected in ["entity", "appearance_set", "chapter_summary"] {
        assert!(
            targets.contains(&expected),
            "no canon.changed for {expected}: {targets:?}"
        );
    }
    // The entity update is a second change of the same target type.
    assert!(
        targets.iter().filter(|target| **target == "entity").count() >= 2,
        "the create and the update both publish: {targets:?}"
    );
    for event in &canon {
        assert!(
            event["change_id"].as_i64().unwrap_or(0) > 0,
            "every canon.changed carries its change id: {event}"
        );
    }
}
