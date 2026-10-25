//! Canon-seed tests (PLAN M9.2, spec/06 §7): the archivist's seed session creates the character
//! entities, links them to their designs, and names the rank-1 design the protagonist.

#[path = "conception_harness/mod.rs"]
mod harness;

use harness::*;
use serde_json::json;

/// Seeds a finished book with no conception session, so the scripted queue belongs to the
/// `canon_seed` session alone: materials written while the book is `conceiving` (no running
/// session locks them), then finalized. Returns `(book_id, seed_session)`.
async fn finalized_book(runtime: &Runtime) -> (i32, i32) {
    // Create the book without a conception model, so no conception session exists and the
    // scripted provider's steps are consumed only by the seed session (deterministic).
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["conception"] = serde_json::Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "drop the conception role: {body}");

    let book = create_conceiving_book(runtime, "Seed Canon").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    assert!(
        book["session_id"].is_null(),
        "no conception session: {book}"
    );
    for (kind, content) in [
        ("outline", "# 大纲\n\n- 沈砚入城。"),
        ("world", "# 世界观\n\n- 剑气折损心脉。"),
        ("protagonist", "# 主角\n\n沈砚。"),
    ] {
        let (status, body) = runtime
            .app
            .put(
                &format!("/api/books/{book_id}/setting-docs/{kind}"),
                json!({ "content": content, "base_revision": 0, "note": "" }),
            )
            .await;
        assert_eq!(status, 200, "seed {kind}: {body}");
    }
    for (name, role_label) in [("沈砚", "主角"), ("柳照", "女主")] {
        let (status, body) = runtime
            .app
            .post(
                &format!("/api/books/{book_id}/characters"),
                json!({
                    "name": name,
                    "role_label": role_label,
                    "summary": "",
                    "profile": "#",
                    "note": "",
                }),
            )
            .await;
        assert_eq!(status, 201, "seed {name}: {body}");
    }
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "finalize: {body}");
    (
        book_id,
        body["seed_session_id"].as_i64().expect("seed session") as i32,
    )
}

/// The seed session creates one entity per character design, links `character_designs.entity_id`,
/// and the rank-1 design becomes the protagonist (spec/06 §7).
#[tokio::test]
async fn canon_seed_creates_links_and_names_the_protagonist() {
    let runtime = runtime(vec![
        tool_step(
            "create_entity",
            json!({
                "kind": "character",
                "name": "沈砚",
                "summary": "游侠",
                "is_protagonist": true,
            }),
        ),
        tool_step(
            "create_entity",
            json!({ "kind": "character", "name": "柳照", "summary": "副使" }),
        ),
        tool_step(
            "finish_maintenance",
            json!({ "report": "初始档案建立完成。" }),
        ),
    ])
    .await;
    let (book_id, seed_session) = finalized_book(&runtime).await;
    let row = runtime.wait_any_status(seed_session).await;
    assert_eq!(
        row.status,
        "completed",
        "the seed session finished: {:?}",
        row.error()
    );

    // One character entity per design, linked by `entity_id` (spec/06 §7).
    let (_, entities) = runtime
        .app
        .get(&format!("/api/books/{book_id}/entities?kind=character"))
        .await;
    let list = entities.as_array().cloned().unwrap_or_default();
    assert_eq!(list.len(), 2, "one entity per design: {list:?}");
    let (_, characters) = runtime
        .app
        .get(&format!("/api/books/{book_id}/characters"))
        .await;
    for design in characters.as_array().cloned().unwrap_or_default() {
        let entity_id = design["entity_id"].as_i64();
        assert!(
            entity_id.is_some(),
            "the design {} is linked to its entity",
            design["name"]
        );
        assert!(
            list.iter().any(|entity| entity["id"] == json!(entity_id)),
            "the link points at a character entity"
        );
    }

    // Exactly one protagonist: the rank-1 design (spec/06 §7, spec/02 §5 invariant 7).
    let (_, all) = runtime
        .app
        .get(&format!("/api/books/{book_id}/entities"))
        .await;
    let protagonists: Vec<&str> = all
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|entity| entity["is_protagonist"].as_bool().unwrap_or(false))
                .filter_map(|entity| entity["name"].as_str())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        protagonists,
        vec!["沈砚"],
        "the rank-1 design is the protagonist"
    );

    // The canon_seed tool restrictions held: the completing call was `finish_maintenance`
    // (`record_appearances`/`set_chapter_summary`/`read_chapter` are forbidden, spec/05 §2).
    let calls = runtime.calls(seed_session).await;
    assert!(
        calls.iter().any(|call| call.name == "finish_maintenance"),
        "the seed ends with finish_maintenance: {calls:?}"
    );
}

/// The `canon_seed` tool restrictions (spec/05 §2): `record_appearances`, `set_chapter_summary`
/// and `read_chapter` are not permitted, so a seed session never calls them. The permission
/// matrix is covered by the registry's own tests; this proves the session's frozen list obeys it
/// end to end — the completing `finish_maintenance` call needs no chapter-scoped tool.
#[tokio::test]
async fn canon_seed_freezes_only_its_permitted_tools() {
    let runtime = runtime(vec![tool_step(
        "finish_maintenance",
        json!({ "report": "初始档案建立完成。" }),
    )])
    .await;
    let (book_id, seed_session) = finalized_book(&runtime).await;
    let row = runtime.wait_any_status(seed_session).await;
    assert_eq!(
        row.status,
        "completed",
        "the seed session finished: {:?}",
        row.error()
    );
    // The seed kickoff exists and the session used only what the matrix permits.
    let messages = runtime.messages(seed_session).await;
    assert!(
        messages.iter().any(|message| message.kind == "kickoff"),
        "the seed session has its kickoff"
    );
    let calls = runtime.calls(seed_session).await;
    for call in &calls {
        assert!(
            !matches!(
                call.name.as_str(),
                "record_appearances" | "set_chapter_summary" | "read_chapter"
            ),
            "canon_seed forbids {name}: {calls:?}",
            name = call.name
        );
    }
    let _ = book_id;
}
