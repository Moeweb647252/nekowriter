//! M9 review findings on the `request_review` answer bookkeeping (spec/05 §7, D-105).

#[path = "conception_harness/mod.rs"]
mod harness;

use harness::*;
use serde_json::json;

/// A `request_review` answer that rejects an item must clear that item's approval: keeping the
/// revision reference makes the studio (and any later session) treat a rejected revision as
/// approved (spec/05 §7, D-105).
#[tokio::test]
async fn a_rejected_item_loses_its_approval() {
    let runtime = runtime(vec![
        tools_step(&[
            (
                "write_setting_doc",
                json!({ "doc": "outline", "content": "# 大纲\n\n第一幕：入城。" }),
            ),
            (
                "write_setting_doc",
                json!({ "doc": "world", "content": "# 世界观\n\n剑气折损心脉。" }),
            ),
            (
                "write_setting_doc",
                json!({ "doc": "protagonist", "content": "# 主角\n\n沈砚。" }),
            ),
            (
                "upsert_character",
                json!({ "name": "沈砚", "role_label": "主角" }),
            ),
        ]),
        review_step("第一版，请看。"),
        review_step("第二版，请看。"),
        text_step("好的。"),
    ])
    .await;
    let book = create_conceiving_book(&runtime, "Approval Staleness").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("session") as i32;

    let first = wait_for_form(&runtime, session_id).await;
    answer_ok(
        &runtime,
        session_id,
        first,
        json!({
            "docs": {
                "outline": { "approved": true },
                "world": { "approved": true },
                "protagonist": { "approved": true },
                "characters": { "approved": true },
            },
        }),
    )
    .await;
    let (_, outline_revision, outline_approved) = setting_doc(&runtime, book_id, "outline").await;
    assert_eq!(outline_approved, Some(outline_revision));
    let approved_state = conception_state(&runtime, book_id).await;
    assert_eq!(
        approved_state.characters_approved_revision,
        Some(approved_state.characters_revision)
    );

    // The next review of the same revisions rejects the outline and the cast.
    let second = wait_for_form(&runtime, session_id).await;
    answer_ok(
        &runtime,
        session_id,
        second,
        json!({
            "docs": {
                "outline": { "approved": false, "feedback": "第一幕还不成立。" },
                "characters": { "approved": false, "feedback": "主角缺少动机。" },
            },
        }),
    )
    .await;

    let (_, revision, approved) = setting_doc(&runtime, book_id, "outline").await;
    assert_eq!(
        approved, None,
        "a rejection clears the approval even when the revision is unchanged ({revision})"
    );
    let state = conception_state(&runtime, book_id).await;
    assert_eq!(
        state.characters_approved_revision, None,
        "a rejected cast is unapproved"
    );
}

/// A `request_review` answer that reviews nothing must not tell the model everything was approved
/// (spec/05 §7): `result.request_review` would otherwise claim an approval that never happened.
#[tokio::test]
async fn a_review_with_no_items_does_not_claim_all_approved() {
    let runtime = runtime(vec![review_step("请审阅。"), text_step("好的。")]).await;
    let book = create_conceiving_book(&runtime, "Empty Review").await;
    let session_id = book["session_id"].as_i64().expect("session") as i32;

    let review = wait_for_form(&runtime, session_id).await;
    answer_ok(&runtime, session_id, review, json!({})).await;
    let sealed = wait_for_sealed(&runtime, session_id, "request_review").await;
    let result = sealed.result_text.clone().unwrap_or_default();
    assert!(
        !result.contains("全部内容都已通过"),
        "nothing was reviewed, so nothing is approved: {result}"
    );
}
