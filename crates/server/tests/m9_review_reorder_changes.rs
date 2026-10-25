//! M9 review finding: a character reorder is a conception write and records `change_log` rows
//! (`actor: agent`, spec/05 §7).

#[path = "conception_harness/mod.rs"]
mod harness;

use harness::*;
use nekowriter::db::json::SessionStatus;
use serde_json::json;

/// Every conception write records `change_log` rows (`actor: agent`, spec/05 §7); a reorder is a
/// write to the character designs, so it records the rank changes it made.
#[tokio::test]
async fn reordering_characters_records_its_changes() {
    let runtime = runtime(vec![
        tools_step(&[
            ("upsert_character", json!({ "name": "沈砚" })),
            ("upsert_character", json!({ "name": "柳照" })),
        ]),
        tool_step("reorder_characters", json!({ "names": ["柳照", "沈砚"] })),
        text_step("好了。"),
    ])
    .await;
    let book = create_conceiving_book(&runtime, "Reorder Changes").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("session") as i32;
    let _ = runtime.wait_status(session_id, SessionStatus::Idle).await;

    assert_eq!(
        characters(&runtime, book_id).await,
        vec![("柳照".to_owned(), 1), ("沈砚".to_owned(), 2)],
        "the reorder landed"
    );
    let changes = session_changes(&runtime, session_id).await;
    assert_eq!(
        changes.iter().filter(|row| row.1 == "create").count(),
        2,
        "two designs were created: {changes:?}"
    );
    let updates = changes
        .iter()
        .filter(|row| row.0 == "character_design" && row.1 == "update")
        .count();
    assert_eq!(
        updates, 2,
        "the two designs whose rank changed each record their update: {changes:?}"
    );
}
