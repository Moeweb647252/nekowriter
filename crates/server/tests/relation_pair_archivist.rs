//! The archivist cannot double a relation by re-wording it (spec/06 §1.2, spec/05 §6, D-143).
//!
//! This is the author's actual bug, and it lives on the **agent** path, not the author's UI: the
//! archivist recorded a pair, then re-created the same pair with the bond re-worded ("补记…视角…",
//! "重建为清晰方向…"), and because `kind` was part of the uniqueness key every re-wording left
//! another active row. The author then saw one entity's relation list with several near-identical
//! lines.
//!
//! Two things must hold, and they are not symmetrical:
//!
//! 1. `set_relation` with a re-worded kind **updates** the pair's existing row. This tool is the
//!    archivist's only way to write a relation, so making a re-wording an error would leave it no
//!    way to correct a wording it regrets — it would be forced to `end_relation` and re-create,
//!    which loses history and is exactly what produced the duplicates.
//! 2. An explicit second **create** for a pair is refused (`conflict.exists`), so the author's API
//!    and any future code path cannot duplicate a pair either.
//!
//! The first test drives a real `chapter_maintenance` session over the scripted provider, so what it
//! asserts is what the model would actually have seen.

mod review_harness;

use std::time::Duration;

use nekowriter::db::json::RelationStatus;
use nekowriter::db::json::{SessionKind, SessionStatus};
use nekowriter_llm::mock::ScriptStep;
use review_harness::{Runtime, create_entity, prepare_in_review, runtime, start_archivist};
use serde_json::json;

/// Every relation row of the book, as `(id, kind, status)`.
async fn relation_rows(runtime: &Runtime) -> Vec<(i32, String, RelationStatus)> {
    use nekowriter::db::entities::relations;
    use sea_orm::EntityTrait;
    let mut rows: Vec<(i32, String, RelationStatus)> = relations::Entity::find()
        .all(runtime.app.db().conn())
        .await
        .expect("relations")
        .into_iter()
        .map(|row| (row.id, row.kind, row.status))
        .collect();
    rows.sort_by_key(|row| row.0);
    rows
}

/// A maintenance script that sets one pair twice with different wording, then finishes.
fn reworded_relation_script() -> Vec<ScriptStep> {
    vec![
        ScriptStep::new().tool(
            "set_relation",
            json!({ "source": "林凡", "target": "苏婉", "kind": "盟友", "note": "剑冢结盟" }),
        ),
        // The same pair, re-worded: what the archivist did to the author's canon.
        ScriptStep::new().tool(
            "set_relation",
            json!({ "source": "林凡", "target": "苏婉", "kind": "同门盟友",
                    "note": "补记：两人同为青云宗门人" }),
        ),
        ScriptStep::new().tool(
            "record_appearances",
            json!({ "appearances": [
                { "entity": "林凡", "role": "pov" },
                { "entity": "苏婉", "role": "major" }
            ] }),
        ),
        ScriptStep::new().tool(
            "set_chapter_summary",
            json!({ "summary": "林凡与苏婉在剑冢结盟，两人同为青云宗门人。" }),
        ),
        ScriptStep::new().tool("finish_maintenance", json!({ "report": "记录了关系" })),
    ]
}

/// Runs the re-wording script in a real maintenance session and returns the session id.
async fn run_reworded_relation_session() -> (Runtime, i32) {
    let runtime = runtime(reworded_relation_script()).await;
    prepare_in_review(&runtime).await;
    let _ = create_entity(&runtime, "林凡", false).await;
    let _ = create_entity(&runtime, "苏婉", false).await;
    let session_id = start_archivist(
        &runtime,
        runtime.chapter_id,
        SessionKind::ChapterMaintenance,
    )
    .await;
    runtime
        .wait_status_within(
            session_id,
            SessionStatus::Completed,
            Duration::from_secs(20),
        )
        .await;
    (runtime, session_id)
}

/// Re-wording a pair through `set_relation` rewrites the row it already has: the pair keeps exactly
/// one active relation, and the second call is not an error.
#[tokio::test]
async fn a_reworded_set_relation_updates_the_pair_instead_of_adding_a_row() {
    let (runtime, session_id) = run_reworded_relation_session().await;

    let session = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await
        .1;
    let calls: Vec<(String, String, bool)> = session["tool_calls"]
        .as_array()
        .expect("tool calls")
        .iter()
        .map(|call| {
            (
                call["name"].as_str().expect("name").to_owned(),
                call["result_text"].as_str().unwrap_or_default().to_owned(),
                call["is_error"].as_bool().expect("is_error"),
            )
        })
        .collect();
    for (name, text, is_error) in &calls {
        assert!(
            !is_error,
            "the re-wording is an edit, not a failure: {name} → {text}"
        );
    }

    let rows = relation_rows(&runtime).await;
    assert_eq!(
        rows.len(),
        1,
        "one relation row for the pair, not two: {rows:?}"
    );
    assert_eq!(rows[0].1, "同门盟友", "the new wording wins: {rows:?}");
    assert_eq!(
        rows[0].2,
        RelationStatus::Active,
        "and the row stays active"
    );
}

/// The `ui` payload of `set_relation` names the same relation both times, so the web UI invalidates
/// one row rather than rendering two.
#[tokio::test]
async fn the_two_calls_report_the_same_relation_id() {
    let (runtime, session_id) = run_reworded_relation_session().await;

    let session = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await
        .1;
    let ids: Vec<i64> = session["tool_calls"]
        .as_array()
        .expect("tool calls")
        .iter()
        .filter(|call| call["name"] == json!("set_relation"))
        .map(|call| {
            call["ui"]["target_id"]
                .as_i64()
                .expect("set_relation ui carries target_id")
        })
        .collect();
    assert_eq!(ids.len(), 2, "two set_relation calls: {session}");
    assert_eq!(
        ids[0], ids[1],
        "both calls address the same relation row: {ids:?}"
    );
}

/// An explicit second create for a pair is still `conflict.exists`, with `details.id` naming the
/// existing row: the API contract, separate from the tool's upsert.
#[tokio::test]
async fn a_second_create_for_a_pair_is_a_conflict() {
    let (runtime, _session) = run_reworded_relation_session().await;
    let rows = relation_rows(&runtime).await;
    let (existing_id, _, _) = rows[0].clone();
    // The two entities the session related.
    let first = entity_id(&runtime, "林凡").await;
    let second = entity_id(&runtime, "苏婉").await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{}/relations", runtime.book_id),
            json!({ "source_id": first, "target_id": second, "kind": "生死之交" }),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], json!("conflict.exists"));
    assert_eq!(
        body["error"]["details"]["id"],
        json!(existing_id),
        "the conflict names the relation to edit: {body}"
    );

    let after = relation_rows(&runtime).await;
    assert_eq!(after.len(), 1, "and nothing was written: {after:?}");
}

/// The id of an entity by name.
async fn entity_id(runtime: &Runtime, name: &str) -> i32 {
    let (status, list) = runtime
        .app
        .get(&format!("/api/books/{}/entities", runtime.book_id))
        .await;
    assert_eq!(status, 200, "{list}");
    list.as_array()
        .expect("entities")
        .iter()
        .find(|row| row["name"] == json!(name))
        .and_then(|row| row["id"].as_i64())
        .unwrap_or_else(|| panic!("entity {name} not found: {list}")) as i32
}
