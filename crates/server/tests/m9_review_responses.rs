//! M9 review finding: the optional response ids are serialized as `null`, matching the generated
//! `number | null` TypeScript types (the studio dialog's landing decision tests against `null`).

#[path = "conception_harness/mod.rs"]
mod harness;

use harness::*;
use serde_json::json;

/// A conceiving book with every material finalize needs, created without a conception model so no
/// session exists and the scripted queue stays deterministic.
async fn finalized_materials(runtime: &Runtime) -> i32 {
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["conception"] = serde_json::Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "drop the conception role: {body}");

    let book = create_conceiving_book(runtime, "Review Findings").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    for (kind, content) in [
        ("outline", "# 大纲\n\n第一幕：入城。"),
        ("world", "# 世界观\n\n剑气折损心脉。"),
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
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({
                "name": "沈砚",
                "role_label": "主角",
                "summary": "游侠",
                "profile": "# 沈砚",
                "note": "",
            }),
        )
        .await;
    assert_eq!(status, 201, "seed character: {body}");
    book_id
}

/// The generated `CreateBookResponse` declares `session_id: number | null` and the finalize
/// response does the same for `seed_session_id`; the wire must carry the key as `null` instead of
/// omitting it, or `response.session_id === null` in the studio dialog never matches (a book
/// without a conception session lands on the studio instead of the overview).
#[tokio::test]
async fn the_create_and_finalize_responses_carry_their_optional_id_as_null() {
    let runtime = runtime(vec![]).await;
    let (status, book) = runtime
        .app
        .post(
            "/api/books",
            json!({
                "title": "Plain Book",
                "writing_language": "zh-CN",
                "start_conception": false,
            }),
        )
        .await;
    assert_eq!(status, 201, "create a plain book: {book}");
    assert!(
        book.get("session_id").is_some(),
        "the create response must carry session_id (null, not absent): {book}"
    );
    assert!(book["session_id"].is_null(), "{book}");

    let book_id = finalized_materials(&runtime).await;
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["archivist"] = serde_json::Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "drop the archivist: {body}");
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "finalize: {body}");
    assert!(
        body.get("seed_session_id").is_some(),
        "the finalize response must carry seed_session_id (null, not absent): {body}"
    );
    assert!(body["seed_session_id"].is_null(), "{body}");
}
