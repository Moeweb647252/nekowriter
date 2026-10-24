//! Chapter service/API tests (PLAN M1.2): CRUD, reorder rules, content writes with
//! `base_version`, versions/restore, submit and approve (spec/08 §1.1, §1.2, §1.4).

use axum::http::StatusCode;
use nekowriter::testing::{TestApp, assert_error, assert_error_detail};
use serde_json::json;

/// Creates a book plus its main track id and returns `(book_id, track_id)`.
async fn book_with_track(app: &TestApp) -> (i32, i32) {
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    (book_id, track_id)
}

#[tokio::test]
async fn chapters_are_numbered_contiguously_from_one() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let first = app.create_chapter(track_id, "First").await;
    assert_eq!(first["number"], 1, "the number is the ord (D-037)");
    assert_eq!(first["status"], "planned");
    assert_eq!(first["canon_status"], "none");
    assert_eq!(first["content_version"], 0);
    assert_eq!(first["word_count"], 0);
    assert_eq!(
        first["content"]["paragraphs"]
            .as_array()
            .expect("paragraphs")
            .len(),
        0
    );
    assert_eq!(first["content"]["next_pid"], 1);
    assert_eq!(first["content"]["tail_open"], false);

    let second = app.create_chapter(track_id, "Second").await;
    assert_eq!(second["number"], 2);

    let (status, list) = app.get(&format!("/api/tracks/{track_id}/chapters")).await;
    assert_eq!(status, StatusCode::OK);
    let list = list.as_array().expect("chapters");
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["title"], "First");
    // The list DTO has no content (spec/10 §2.2).
    assert!(list[0].get("content").is_none());
}

#[tokio::test]
async fn chapter_patch_edits_title_outline_and_notes() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id");

    let (status, body) = app
        .patch(
            &format!("/api/chapters/{chapter_id}"),
            json!({
                "title": "The Sword Tomb",
                "guidance": "keep it tense",
                "outline": "1. arrive\n2. duel",
                "reference_notes": "the sword was broken",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["title"], "The Sword Tomb");
    assert_eq!(body["guidance"], "keep it tense");
    assert_eq!(body["outline"], "1. arrive\n2. duel");
    assert_eq!(body["reference_notes"], "the sword was broken");
    assert_eq!(body["summary"], "", "omitted fields are unchanged");
    assert_eq!(
        body["content_version"], 0,
        "metadata edits do not bump the version"
    );
}

#[tokio::test]
async fn content_patch_applies_ops_and_bumps_the_version() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "ch").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;

    let (status, result) = app
        .patch(
            &format!("/api/chapters/{chapter_id}/content"),
            json!({
                "base_version": 0,
                "ops": [
                    { "op": "insert", "after": null, "text": "第一段。" },
                    { "op": "insert", "after": "p1", "text": "第二段。" }
                ]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["version"], 1);
    assert_eq!(
        result["word_count"], 6,
        "第一段。 and 第二段。 are 3 words each"
    );
    let ops = result["ops"].as_array().expect("ops");
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0]["op"], "insert");
    assert_eq!(ops[0]["pid"], "p1");
    assert_eq!(ops[0]["after"], serde_json::Value::Null);
    assert_eq!(ops[1]["pid"], "p2");
    assert_eq!(ops[1]["after"], "p1");

    // The chapter now reports the stored content.
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(chapter["content_version"], 1);
    assert_eq!(chapter["word_count"], 6);
    assert_eq!(chapter["content"]["next_pid"], 3);
    assert_eq!(chapter["content"]["paragraphs"][0]["pid"], "p1");
    assert_eq!(chapter["content"]["paragraphs"][0]["text"], "第一段。");

    // An update and a delete in the next transaction.
    let (status, result) = app
        .patch(
            &format!("/api/chapters/{chapter_id}/content"),
            json!({
                "base_version": 1,
                "ops": [
                    { "op": "update", "pid": "p1", "text": "改写后的第一段。" },
                    { "op": "delete", "pid": "p2" }
                ]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["version"], 2);
    assert_eq!(
        result["word_count"], 7,
        "改写后的第一段。 is seven CJK characters"
    );

    // Pids are never reused: next_pid stayed at 3 even though p2 is gone (spec/02 §5.3).
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(chapter["content"]["next_pid"], 3);
    assert_eq!(
        chapter["content"]["paragraphs"]
            .as_array()
            .expect("paragraphs")
            .len(),
        1
    );
}

#[tokio::test]
async fn stale_base_version_is_conflict_version_with_the_current_version() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "ch").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.put_text(chapter_id, 0, "one").await;

    let (status, body) = app
        .patch(
            &format!("/api/chapters/{chapter_id}/content"),
            json!({
                "base_version": 0,
                "ops": [{ "op": "insert", "after": null, "text": "stale" }]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&body, "conflict.version");
    assert_eq!(body["error"]["details"]["version"], 1);

    // The same rule applies to the full-text save.
    let (status, body) = app
        .put(
            &format!("/api/chapters/{chapter_id}/content/text"),
            json!({ "base_version": 0, "text": "stale" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&body, "conflict.version");
}

#[tokio::test]
async fn unknown_pids_are_rejected_without_changing_the_content() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "ch").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.put_text(chapter_id, 0, "one").await;

    let (status, body) = app
        .patch(
            &format!("/api/chapters/{chapter_id}/content"),
            json!({
                "base_version": 1,
                "ops": [{ "op": "update", "pid": "p99", "text": "x" }]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");
    assert_eq!(body["error"]["details"]["pid"], "p99");

    // The failed transaction rolled back: version and text are unchanged.
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(chapter["content_version"], 1);
    assert_eq!(chapter["content"]["paragraphs"][0]["text"], "one");
}

#[tokio::test]
async fn full_text_save_aligns_pids() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "ch").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;

    let result = app.put_text(chapter_id, 0, "one\n\ntwo\n\nthree").await;
    assert_eq!(result["version"], 1);
    let ops = result["ops"].as_array().expect("ops").clone();
    let inserted: Vec<&str> = ops
        .iter()
        .map(|op| op["pid"].as_str().expect("pid"))
        .collect();
    assert_eq!(inserted, vec!["p1", "p2", "p3"]);

    // Editing one paragraph keeps every pid; the edit is one `update` op.
    let result = app
        .put_text(chapter_id, 1, "one\n\ntwo, slightly edited\n\nthree")
        .await;
    let ops = result["ops"].as_array().expect("ops");
    assert_eq!(ops.len(), 1, "only the edited paragraph changes");
    assert_eq!(ops[0]["op"], "update");
    assert_eq!(ops[0]["pid"], "p2");
    assert_eq!(ops[0]["text"], "two, slightly edited");
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    let pids: Vec<&str> = chapter["content"]["paragraphs"]
        .as_array()
        .expect("paragraphs")
        .iter()
        .map(|paragraph| paragraph["pid"].as_str().expect("pid"))
        .collect();
    assert_eq!(pids, vec!["p1", "p2", "p3"]);

    // A new paragraph in the middle gets a fresh pid; removing one deletes it.
    let result = app
        .put_text(chapter_id, 2, "one\n\nbrand new\n\nthree")
        .await;
    let ops = result["ops"].as_array().expect("ops");
    let kinds: Vec<&str> = ops
        .iter()
        .map(|op| op["op"].as_str().expect("op"))
        .collect();
    assert_eq!(kinds, vec!["insert", "delete"]);
    assert_eq!(ops[0]["after"], "p1");
    assert_eq!(ops[1]["pid"], "p2");
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    let pids: Vec<&str> = chapter["content"]["paragraphs"]
        .as_array()
        .expect("paragraphs")
        .iter()
        .map(|paragraph| paragraph["pid"].as_str().expect("pid"))
        .collect();
    assert_eq!(pids, vec!["p1", "p4", "p3"]);
}

#[tokio::test]
async fn an_author_edit_moves_a_planned_chapter_into_drafting() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "ch").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    assert_eq!(chapter["status"], "planned");

    // `planned ── manual edit ──► drafting` (the lifecycle diagram of spec/08 §1).
    let result = app.put_text(chapter_id, 0, "第一段。").await;
    assert_eq!(result["version"], 1);
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(chapter["status"], "drafting");

    // An empty save does not move the chapter.
    let other = app.create_chapter(track_id, "other").await;
    let other_id = other["id"].as_i64().expect("chapter id") as i32;
    app.put_text(other_id, 0, "   \n\n  ").await;
    let (_, other) = app.get(&format!("/api/chapters/{other_id}")).await;
    assert_eq!(other["status"], "planned", "an empty save changes no text");
}

#[tokio::test]
async fn submit_requires_drafting_and_creates_a_version() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "ch").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;

    // `planned` cannot be submitted (spec/08 §1.1).
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error_detail(&body, "chapter.invalid_state", "status");

    // A manual edit moves the chapter into drafting (spec/08 §1 lifecycle).
    app.put_text(chapter_id, 0, "第一段。\n\n第二段。").await;
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(chapter["status"], "drafting");
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "in_review");

    // A `draft` version was created (spec/08 §1.2).
    let (_, versions) = app
        .get(&format!("/api/chapters/{chapter_id}/versions"))
        .await;
    let versions = versions.as_array().expect("versions");
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0]["kind"], "draft");
    assert_eq!(versions[0]["version_no"], 1);
    assert_eq!(versions[0]["word_count"], 6);

    // A second submit is rejected: the chapter already left drafting.
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&body, "chapter.invalid_state");
}

#[tokio::test]
async fn approve_creates_the_official_version_and_pends_canon() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "ch").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;

    // Approve needs `in_review`.
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&body, "chapter.invalid_state");

    app.put_text(chapter_id, 0, "第一段。").await;
    let (status, _) = app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "official");
    assert_eq!(body["canon_status"], "pending");
    let official_version_id = body["official_version_id"]
        .as_i64()
        .expect("official_version_id");

    let (_, versions) = app
        .get(&format!("/api/chapters/{chapter_id}/versions"))
        .await;
    let versions = versions.as_array().expect("versions");
    let kinds: Vec<&str> = versions
        .iter()
        .map(|version| version["kind"].as_str().expect("kind"))
        .collect();
    assert_eq!(kinds, vec!["official", "draft"]);
    assert_eq!(versions[0]["id"], official_version_id);

    // Approving an official chapter is rejected.
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&body, "chapter.invalid_state");
}

#[tokio::test]
async fn author_edits_of_a_synced_official_chapter_make_canon_stale() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "ch").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.put_text(chapter_id, 0, "第一段。").await;
    app.post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    app.post(&format!("/api/chapters/{chapter_id}/approve"), json!({}))
        .await;

    // Maintenance marks it synced (M6 owns the endpoint); simulate the row state.
    app.mark_canon_synced(chapter_id).await;

    let (status, body) = app
        .patch(
            &format!("/api/chapters/{chapter_id}/content"),
            json!({
                "base_version": 1,
                "ops": [{ "op": "update", "pid": "p1", "text": "改写过的第一段。" }]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(
        chapter["canon_status"], "stale",
        "an author edit of a synced official chapter makes canon stale (spec/08 §1.1)"
    );
}

#[tokio::test]
async fn versions_can_be_saved_fetched_and_restored() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "ch").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.put_text(chapter_id, 0, "original\n\ntext").await;

    let (status, version) = app
        .post(
            &format!("/api/chapters/{chapter_id}/versions"),
            json!({ "note": "before the rewrite" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{version}");
    assert_eq!(version["kind"], "manual");
    assert_eq!(version["note"], "before the rewrite");
    assert_eq!(version["word_count"], 2, "original text is two words");
    let version_id = version["id"].as_i64().expect("version id");

    // The version endpoint returns the content (spec/10 §2.2).
    let (status, full) = app
        .get(&format!("/api/chapter-versions/{version_id}"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(full["kind"], "manual");
    assert_eq!(
        full["content"]["paragraphs"]
            .as_array()
            .expect("paragraphs")
            .len(),
        2
    );

    // Rewrite, then restore: a backup is created first (spec/08 §1.2).
    app.put_text(chapter_id, 1, "rewritten").await;
    let (status, body) = app
        .post(
            &format!("/api/chapter-versions/{version_id}/restore"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["content"]["paragraphs"][0]["text"], "original");
    assert_eq!(body["content_version"], 3, "the restore counts as an edit");

    let (_, versions) = app
        .get(&format!("/api/chapters/{chapter_id}/versions"))
        .await;
    let kinds: Vec<&str> = versions
        .as_array()
        .expect("versions")
        .iter()
        .map(|version| version["kind"].as_str().expect("kind"))
        .collect();
    assert_eq!(kinds, vec!["backup", "manual"]);

    // Restoring a missing version is not_found.
    let (status, body) = app
        .post("/api/chapter-versions/999/restore", json!({}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&body, "not_found");
}

#[tokio::test]
async fn reorder_moves_only_non_official_chapters() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let first = app.create_chapter(track_id, "one").await;
    let second = app.create_chapter(track_id, "two").await;
    let third = app.create_chapter(track_id, "three").await;
    let third_id = third["id"].as_i64().expect("third id");

    // All planned: any order is legal.
    let (status, list) = app
        .post(
            &format!("/api/tracks/{track_id}/chapters/reorder"),
            json!({ "ids": [third_id, first["id"], second["id"]] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let numbers: Vec<i64> = list
        .as_array()
        .expect("list")
        .iter()
        .map(|chapter| chapter["number"].as_i64().expect("number"))
        .collect();
    assert_eq!(numbers, vec![1, 2, 3], "renumbered contiguously");
    assert_eq!(list[0]["title"], "three");

    // A reorder list that loses a chapter is `validation.failed`.
    let (status, body) = app
        .post(
            &format!("/api/tracks/{track_id}/chapters/reorder"),
            json!({ "ids": [third_id] }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");

    // Make the first chapter official: it may not move, and nothing may move before it.
    let first_id = first["id"].as_i64().expect("first id");
    app.set_chapter_status(first_id as i32, "in_review").await;
    let (status, body) = app
        .post(
            &format!("/api/tracks/{track_id}/chapters/reorder"),
            json!({ "ids": [first_id, third_id, second["id"]] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    app.set_chapter_status(first_id as i32, "official").await;
    let (status, body) = app
        .post(
            &format!("/api/tracks/{track_id}/chapters/reorder"),
            json!({ "ids": [second["id"], first_id, third_id] }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&body, "chapter.invalid_state");

    // Reordering the non-official ones after it is fine.
    let (status, _) = app
        .post(
            &format!("/api/tracks/{track_id}/chapters/reorder"),
            json!({ "ids": [first_id, third_id, second["id"]] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn delete_renumbers_and_refuses_when_a_session_is_open() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let first = app.create_chapter(track_id, "one").await;
    let second = app.create_chapter(track_id, "two").await;
    let third = app.create_chapter(track_id, "three").await;
    let second_id = second["id"].as_i64().expect("second id") as i32;

    let (status, _) = app.delete(&format!("/api/chapters/{second_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, list) = app.get(&format!("/api/tracks/{track_id}/chapters")).await;
    let remaining: Vec<(&str, i64)> = list
        .as_array()
        .expect("list")
        .iter()
        .map(|chapter| {
            (
                chapter["title"].as_str().expect("title"),
                chapter["number"].as_i64().expect("number"),
            )
        })
        .collect();
    assert_eq!(
        remaining,
        vec![("one", 1), ("three", 2)],
        "the track stays contiguous"
    );

    // An open session blocks a delete with `chapter.session_exists` (spec/10 §4). A completed
    // one does not (spec/02 §5.5 defines "open" as not completed/superseded/abandoned).
    let third_id = third["id"].as_i64().expect("third id") as i32;
    app.insert_writer_session(third_id, "completed").await;
    let (status, _) = app.delete(&format!("/api/chapters/{third_id}")).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "a completed session does not block a delete"
    );
    // Restore the chapter for the blocked case.
    let restored = app.create_chapter(track_id, "restored").await;
    let restored_id = restored["id"].as_i64().expect("restored id") as i32;
    app.insert_writer_session(restored_id, "held").await;
    let (status, body) = app.delete(&format!("/api/chapters/{restored_id}")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error_detail(&body, "chapter.session_exists", "session_id");
    let _ = first;
}

#[tokio::test]
async fn content_edits_are_rejected_while_the_writer_is_running() {
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "ch").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.put_text(chapter_id, 0, "one").await;

    app.insert_writer_session(chapter_id, "running").await;

    let (status, body) = app
        .patch(
            &format!("/api/chapters/{chapter_id}/content"),
            json!({
                "base_version": 1,
                "ops": [{ "op": "update", "pid": "p1", "text": "two" }]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::LOCKED);
    assert_error_detail(&body, "chapter.locked", "session_id");

    let (status, body) = app
        .put(
            &format!("/api/chapters/{chapter_id}/content/text"),
            json!({ "base_version": 1, "text": "two" }),
        )
        .await;
    assert_eq!(status, StatusCode::LOCKED);
    assert_error(&body, "chapter.locked");

    // The outline is locked too, but the title is not (spec/08 §1.4).
    let (status, body) = app
        .patch(
            &format!("/api/chapters/{chapter_id}"),
            json!({ "outline": "new outline" }),
        )
        .await;
    assert_eq!(status, StatusCode::LOCKED);
    assert_error(&body, "chapter.locked");
    let (status, _) = app
        .patch(
            &format!("/api/chapters/{chapter_id}"),
            json!({ "title": "renamed" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // A held (not running) writer does not lock the text.
    app.clear_sessions().await;
    app.insert_writer_session(chapter_id, "held").await;
    let (status, _) = app
        .put(
            &format!("/api/chapters/{chapter_id}/content/text"),
            json!({ "base_version": 1, "text": "edited while held" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}
