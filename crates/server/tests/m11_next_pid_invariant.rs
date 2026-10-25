//! spec/02 §5 invariant 3: "Pids are unique within a chapter and never reused (`next_pid` only
//! grows)".
//!
//! Two whole-document replacements can carry fewer pids than the state they replace: a redraft
//! clears the chapter's text (spec/08 §1.1) and a version restore puts an older document back
//! (spec/08 §1.2). Both used to store that document's own counter unchanged, so the counter went
//! **backwards** — after a redraft of a three-paragraph chapter it was 1 again — and the next
//! `insert_text` handed out `p1`, a pid the archived text and the backup version still hold. Both
//! paths now keep the counter monotonic (`text::keep_next_pid`). These tests fail without it.

use axum::http::StatusCode;
use serde_json::json;

use nekowriter::testing::TestApp;

#[path = "agent_write_harness/mod.rs"]
mod agent_write_harness;

/// A book, its main track and one chapter.
async fn app_with_chapter() -> (TestApp, i32) {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    (app, chapter_id)
}

/// The chapter's stored `next_pid`.
async fn next_pid(app: &TestApp, chapter_id: i32) -> i64 {
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    chapter["content"]["next_pid"]
        .as_i64()
        .expect("the content carries next_pid")
}

/// A restore of an older, shorter version keeps the pid counter where it was.
#[tokio::test]
async fn restoring_an_older_version_does_not_lower_the_pid_counter() {
    let (app, chapter_id) = app_with_chapter().await;
    app.put_text(chapter_id, 0, "第一段。\n\n第二段。\n\n第三段。")
        .await;
    let after_write = next_pid(&app, chapter_id).await;
    assert_eq!(after_write, 4, "three paragraphs allocate p1..p3");

    // A manual snapshot of that state.
    let (status, snapshot) = app
        .post(
            &format!("/api/chapters/{chapter_id}/versions"),
            json!({ "note": "snap" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{snapshot}");
    let snapshot_id = snapshot["id"].as_i64().expect("snapshot id");

    // Grow the chapter, then put the snapshot back.
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    let version = chapter["content_version"].as_i64().expect("version");
    let (status, body) = app
        .put(
            &format!("/api/chapters/{chapter_id}/content/text"),
            json!({
                "base_version": version,
                "text": "第一段。\n\n第二段。\n\n第三段。\n\n第四段。",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let grown = next_pid(&app, chapter_id).await;
    assert_eq!(grown, 5, "the fourth paragraph is p4");

    let (status, restored) = app
        .post(
            &format!("/api/chapter-versions/{snapshot_id}/restore"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(
        restored["content"]["next_pid"].as_i64(),
        Some(grown),
        "invariant 3: the counter must not go backwards on restore"
    );
    assert_eq!(next_pid(&app, chapter_id).await, grown);
}

/// A redraft clears the text but keeps the pid counter, so the new draft's first paragraph is not
/// a pid the archived text already used.
#[tokio::test]
async fn a_redraft_keeps_the_pid_counter() {
    // The redraft path starts a writer session, so the harness's model assignment is needed.
    let runtime = agent_write_harness::runtime(Vec::new()).await;
    let chapter_id = runtime.chapter_id;
    runtime
        .app
        .put_text(chapter_id, 0, "第一段。\n\n第二段。\n\n第三段。")
        .await;
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{chapter_id}"))
        .await;
    let before = chapter["content"]["next_pid"]
        .as_i64()
        .expect("the content carries next_pid");
    assert_eq!(before, 4, "three paragraphs allocate p1..p3");

    let (status, body) = runtime
        .app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{chapter_id}/send-back"),
            json!({ "feedback": "重写一遍。", "mode": "redraft" }),
        )
        .await;
    assert!(status.is_success(), "{status} {body}");

    // The session-creation side effect commits with the session, before its runner asks the
    // provider for anything, so the content is observable immediately.
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{chapter_id}"))
        .await;
    assert_eq!(
        chapter["content"]["paragraphs"].as_array().map(Vec::len),
        Some(0),
        "a redraft clears the text: {chapter}"
    );
    assert_eq!(
        chapter["content"]["next_pid"].as_i64(),
        Some(before),
        "invariant 3: a redraft keeps the pid counter"
    );
}
