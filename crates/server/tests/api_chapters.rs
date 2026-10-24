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
    // spec/10 §2.2: `{ chapter, maintenance_session_id?, warnings }`. A bare `ChapterDto` would
    // lose the warnings the UI localizes.
    let approved = body["chapter"].clone();
    assert_eq!(approved["status"], "official");
    assert_eq!(approved["canon_status"], "pending");
    // `auto_maintenance` defaults to on, but no archivist model is configured here: the chapter
    // is still approved and its canon stays `pending`, with the warning the UI localizes
    // (spec/08 §1.1).
    assert_eq!(
        body["warnings"],
        json!(["model.missing_role"]),
        "no archivist model means a warning, not a failed approval"
    );
    assert!(
        body.get("maintenance_session_id").is_none(),
        "the field is omitted when no session starts"
    );
    assert_eq!(
        approved["canon_status"], "pending",
        "canon stays pending for a manual Retry"
    );
    let official_version_id = approved["official_version_id"]
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

/// Applies the `ContentOp`s of a content response to a starting document, the way the web
/// client and the server's own `apply_ops` do (spec/10 §3).
fn replay_ops(
    mut paragraphs: Vec<(String, String)>,
    ops: &[serde_json::Value],
) -> Vec<(String, String)> {
    for op in ops {
        match op["op"].as_str().expect("op kind") {
            "insert" => {
                let index = match op["after"].as_str() {
                    None => 0,
                    Some(after) => {
                        paragraphs
                            .iter()
                            .position(|(pid, _)| pid == after)
                            .expect("insert anchor exists")
                            + 1
                    }
                };
                paragraphs.insert(
                    index,
                    (
                        op["pid"].as_str().expect("pid").to_owned(),
                        op["text"].as_str().expect("text").to_owned(),
                    ),
                );
            }
            "update" => {
                let pid = op["pid"].as_str().expect("pid");
                let paragraph = paragraphs
                    .iter_mut()
                    .find(|(candidate, _)| candidate == pid)
                    .expect("updated pid exists");
                paragraph.1 = op["text"].as_str().expect("text").to_owned();
            }
            "delete" => {
                let pid = op["pid"].as_str().expect("pid");
                paragraphs.retain(|(candidate, _)| candidate != pid);
            }
            "tail" => {}
            other => panic!("unknown op {other}"),
        }
    }
    paragraphs
}

#[tokio::test]
async fn a_full_text_save_that_reorders_paragraphs_emits_ops_that_reproduce_the_order() {
    // Alignment keeps both pids (each is only lightly edited) but wants them swapped. `ContentOp`
    // has no move form, so reporting two updates would leave every other tab on the old order —
    // spec/10 §3 says the client applies the ops when `version = local + 1`.
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "ch").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.put_text(
        chapter_id,
        0,
        "The wind blew through the old valley.\n\nA quiet morning over the far hills.",
    )
    .await;
    let (_, before) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    let starting: Vec<(String, String)> = before["content"]["paragraphs"]
        .as_array()
        .expect("paragraphs")
        .iter()
        .map(|paragraph| {
            (
                paragraph["pid"].as_str().expect("pid").to_owned(),
                paragraph["text"].as_str().expect("text").to_owned(),
            )
        })
        .collect();

    let (status, result) = app
        .put(
            &format!("/api/chapters/{chapter_id}/content/text"),
            json!({
                "base_version": before["content_version"],
                "text": "A quiet morning over the far hills, seen from above.\n\nThe wind blew through the old valley, then died.",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let ops = result["ops"].as_array().expect("ops").clone();

    let (_, after) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    let stored: Vec<(String, String)> = after["content"]["paragraphs"]
        .as_array()
        .expect("paragraphs")
        .iter()
        .map(|paragraph| {
            (
                paragraph["pid"].as_str().expect("pid").to_owned(),
                paragraph["text"].as_str().expect("text").to_owned(),
            )
        })
        .collect();

    assert_eq!(
        replay_ops(starting, &ops),
        stored,
        "replaying {ops:?} must produce the stored document"
    );
}

#[tokio::test]
async fn a_malformed_body_returns_the_documented_error_envelope() {
    // spec/10 §1 fixes the shape of every error response to
    // `{ error: { code, message, details } }`, and spec/11 §4.1 has the client parse exactly
    // that envelope. A bare axum 422 with a plain-text body gives the UI no code to localize.
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "ch").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;

    let patch_path = format!("/api/chapters/{chapter_id}/content");
    let put_path = format!("/api/chapters/{chapter_id}/content/text");
    let cases = [
        ("PATCH", &patch_path, json!({ "base_version": 0 })),
        (
            "PATCH",
            &patch_path,
            json!({ "base_version": 0, "ops": [{ "op": "bogus", "pid": "p1" }] }),
        ),
        ("PUT", &put_path, json!({ "base_version": 0, "text": 42 })),
        (
            "PATCH",
            &patch_path,
            json!({ "base_version": "not a number", "ops": [] }),
        ),
        (
            "POST",
            &format!("/api/chapters/{chapter_id}/submit"),
            json!("not an object"),
        ),
    ];
    for (method, path, body) in cases {
        let (status, response) = if method == "PUT" {
            app.put(path, body.clone()).await
        } else if method == "POST" {
            app.post(path, body.clone()).await
        } else {
            app.patch(path, body.clone()).await
        };
        if status.is_success() || status == StatusCode::CONFLICT {
            // Rejected by the service instead of the extractor: already enveloped, and covered
            // by the error-code tests above.
            continue;
        }
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{body} should be a 400 with the error envelope, got {status}: {response}"
        );
        assert_error(&response, "validation.failed");
        assert!(
            response["error"]["details"]["fields"].is_object(),
            "{body} needs details.fields (spec/10 §4), got {response}"
        );
    }
}

#[tokio::test]
async fn validation_errors_name_the_offending_field() {
    // spec/10 §4 documents `details.fields` for `validation.failed`; without it the UI can only
    // show a generic message instead of pointing at the control.
    let app = TestApp::new().await;
    let (status, body) = app
        .post(
            "/api/books",
            json!({ "title": "   ", "writing_language": "en" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_detail(&body, "validation.failed", "fields");
    assert!(
        body["error"]["details"]["fields"]["title"].is_string(),
        "the empty title is the offending field: {body}"
    );

    let (status, body) = app
        .post(
            "/api/books",
            json!({ "title": "Book", "writing_language": "not a language" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_detail(&body, "validation.failed", "fields");
    assert!(
        body["error"]["details"]["fields"]["writing_language"].is_string(),
        "the bad BCP-47 tag is the offending field: {body}"
    );

    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id");
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "  " }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_detail(&body, "validation.failed", "fields");
    assert!(
        body["error"]["details"]["fields"]["name"].is_string(),
        "the empty name is the offending field: {body}"
    );
}

#[tokio::test]
async fn a_second_main_track_reports_the_existing_track_id() {
    // spec/10 §4: `conflict.exists` carries `details.id`.
    let app = TestApp::new().await;
    let (book_id, track_id) = book_with_track(&app).await;
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "kind": "main", "title": "Another" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error_detail(&body, "conflict.exists", "id");
    assert_eq!(
        body["error"]["details"]["id"].as_i64(),
        Some(i64::from(track_id)),
        "the id points at the existing main track"
    );
}

#[tokio::test]
async fn a_rejected_reorder_reports_the_chapter_status() {
    // spec/10 §4: `chapter.invalid_state` carries `details.status`.
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let first = app.create_chapter(track_id, "one").await;
    let second = app.create_chapter(track_id, "two").await;
    let first_id = first["id"].as_i64().expect("first id") as i32;
    let second_id = second["id"].as_i64().expect("second id") as i32;
    app.put_text(first_id, 0, "第一段。").await;
    app.post(&format!("/api/chapters/{first_id}/submit"), json!({}))
        .await;
    app.post(&format!("/api/chapters/{first_id}/approve"), json!({}))
        .await;

    let (status, body) = app
        .post(
            &format!("/api/tracks/{track_id}/chapters/reorder"),
            json!({ "ids": [second_id, first_id] }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error_detail(&body, "chapter.invalid_state", "status");
    assert_eq!(body["error"]["details"]["status"], "official");
    assert_error_detail(&body, "chapter.invalid_state", "official_chapter_ids");
}

#[tokio::test]
async fn a_no_op_content_save_changes_nothing() {
    // A save that stores the same document is not a change: bumping `content_version` would
    // invalidate every other tab's `base_version`, emit a pointless `chapter.content`, and — on
    // an official chapter with synced canon — wrongly mark the canon stale (spec/08 §1.1).
    let app = TestApp::new().await;
    let (_book_id, track_id) = book_with_track(&app).await;
    let chapter = app.create_chapter(track_id, "ch").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    let (_, initial) = app
        .put(
            &format!("/api/chapters/{chapter_id}/content/text"),
            json!({ "base_version": 0, "text": "one\n\ntwo" }),
        )
        .await;
    let version = initial["version"].as_i64().expect("version");

    // Identical full text.
    let (status, result) = app
        .put(
            &format!("/api/chapters/{chapter_id}/content/text"),
            json!({ "base_version": version, "text": "one\n\ntwo" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["version"], version, "the version must not move");
    assert_eq!(result["ops"], json!([]));
    assert_eq!(result["word_count"], 2);

    // Empty op batch.
    let (status, result) = app
        .patch(
            &format!("/api/chapters/{chapter_id}/content"),
            json!({ "base_version": version, "ops": [] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["version"], version, "the version must not move");
    assert_eq!(result["ops"], json!([]));

    let (_, after) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(after["content_version"], version);

    // On a synced official chapter a no-op save must not mark the canon stale.
    app.post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    app.post(&format!("/api/chapters/{chapter_id}/approve"), json!({}))
        .await;
    app.mark_canon_synced(chapter_id).await;
    let (_, official) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(official["canon_status"], "synced");
    let (status, _) = app
        .put(
            &format!("/api/chapters/{chapter_id}/content/text"),
            json!({ "base_version": official["content_version"], "text": "one\n\ntwo" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, after) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(after["canon_status"], "synced", "a no-op is not an edit");
    assert_eq!(after["content_version"], official["content_version"]);
}

#[tokio::test]
async fn a_no_op_character_save_writes_no_change_row() {
    // spec/06 §3 records one `change_log` row per real change. A form that saves unchanged values
    // is not a change, so it must write no row and emit no `settings.changed`.
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id");
    let (_, character) = app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "Alice", "summary": "The protagonist." }),
        )
        .await;
    let character_id = character["id"].as_i64().expect("character id");
    let (_, before) = app
        .get(&format!(
            "/api/books/{book_id}/changes?target_type=character_design"
        ))
        .await;
    let rows_before = before["items"].as_array().expect("items").len();

    // Re-send the same values (the form sends the whole patch).
    let (status, body) = app
        .patch(
            &format!("/api/characters/{character_id}"),
            json!({ "name": "Alice", "summary": "The protagonist.", "note": "no change" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "Alice");

    let (_, after) = app
        .get(&format!(
            "/api/books/{book_id}/changes?target_type=character_design"
        ))
        .await;
    assert_eq!(
        after["items"].as_array().expect("items").len(),
        rows_before,
        "an unchanged save must not add a change row: {after}"
    );

    // A real change still records one.
    let (status, _) = app
        .patch(
            &format!("/api/characters/{character_id}"),
            json!({ "summary": "The narrator." }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, changed) = app
        .get(&format!(
            "/api/books/{book_id}/changes?target_type=character_design"
        ))
        .await;
    assert_eq!(
        changed["items"].as_array().expect("items").len(),
        rows_before + 1,
        "a real change records a row"
    );
}
