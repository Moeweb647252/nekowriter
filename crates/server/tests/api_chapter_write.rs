//! `POST /api/chapters/:id/write` preconditions (PLAN M4.5, spec/08 §1.1).

use axum::http::StatusCode;
use nekowriter::testing::{TestApp, assert_error, assert_error_detail};
use serde_json::json;

/// A book, its main track and one chapter.
async fn book_and_chapter(app: &TestApp) -> (i32, i32, i32) {
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    (book_id, track_id, chapter_id)
}

/// Assigns one Demo model to every role, so a session can be created.
async fn assign_models(app: &TestApp) {
    let (status, provider) = app
        .post(
            "/api/providers",
            json!({
                "name": "Demo",
                "protocol": "mock",
                "base_url": "mock://demo",
                "extra_headers": {},
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let provider_id = provider["id"].as_i64().expect("provider id") as i32;
    let (status, profile) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Demo model",
                "model": "demo-model",
                "context_window": 200_000,
                "max_output_tokens": 8_192,
                "options": {},
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{profile}");
    let profile_id = profile["id"].as_i64().expect("profile id") as i32;
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({
                "roles_defaults": {
                    "conception": profile_id,
                    "researcher": profile_id,
                    "writer": profile_id,
                    "reviewer": profile_id,
                    "archivist": profile_id,
                },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// The write endpoint starts a `chapter_write` session in planning and the response carries its id
/// (spec/08 §1.1, spec/10 §2.2).
#[tokio::test]
async fn write_starts_a_planning_session() {
    let app = TestApp::new().await;
    assign_models(&app).await;
    let (_book_id, _track_id, chapter_id) = book_and_chapter(&app).await;

    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().expect("session id") as i32;
    assert!(body["warnings"].is_array(), "{body}");

    let (status, session) = app.get(&format!("/api/sessions/{session_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["session"]["kind"], "chapter_write");
    assert_eq!(session["session"]["phase"], "planning");
    assert_eq!(session["session"]["chapter_id"], chapter_id);
    // The chapter entered drafting on creation (spec/03 §4 domain side effects).
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(chapter["status"], "drafting");
}

/// The body carries the writer switches; omitted ones come from the book's settings.
#[tokio::test]
async fn write_accepts_the_writer_switches_and_stores_the_guidance() {
    let app = TestApp::new().await;
    assign_models(&app).await;
    let (_book_id, _track_id, chapter_id) = book_and_chapter(&app).await;

    let (status, body) = app
        .post(
            &format!("/api/chapters/{chapter_id}/write"),
            json!({
                "guidance": "Write it in the rain.",
                "granularity": "sentence",
                "auto_pause": true,
                "pause_after_outline": false,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().expect("session id") as i32;
    let (_, session) = app.get(&format!("/api/sessions/{session_id}")).await;
    assert_eq!(session["session"]["granularity"], "sentence");
    assert_eq!(session["session"]["auto_pause"], true);
    assert_eq!(session["session"]["pause_after_outline"], false);
    // The guidance is stored before the session reads it into its kickoff.
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(chapter["guidance"], "Write it in the rain.");
}

/// Only a `planned` or `drafting` chapter can be written (spec/08 §1.1).
#[tokio::test]
async fn write_rejects_a_chapter_in_a_later_status() {
    let app = TestApp::new().await;
    assign_models(&app).await;
    let (_book_id, _track_id, chapter_id) = book_and_chapter(&app).await;
    app.set_chapter_status(chapter_id, "in_review").await;

    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "chapter.invalid_state");
}

/// A chapter with an open writer session cannot start a second one (spec/08 §1.1).
#[tokio::test]
async fn write_rejects_a_chapter_with_an_open_writer_session() {
    let app = TestApp::new().await;
    assign_models(&app).await;
    let (_book_id, _track_id, chapter_id) = book_and_chapter(&app).await;

    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().expect("session id") as i32;

    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "chapter.session_exists");
    assert_error_detail(&body, "chapter.session_exists", "session_id");
    assert_eq!(body["error"]["details"]["session_id"], session_id);
}

/// The previous chapter of the track must be official with synced canon before the next one is
/// written (spec/08 §1.1), and `force` overrides it.
#[tokio::test]
async fn write_requires_the_previous_chapter_to_be_ready_unless_forced() {
    let app = TestApp::new().await;
    assign_models(&app).await;
    let (_book_id, track_id, first_id) = book_and_chapter(&app).await;
    let second = app.create_chapter(track_id, "Chapter Two").await;
    let second_id = second["id"].as_i64().expect("chapter id") as i32;

    // The first chapter is `planned`, so it is not ready: the write is refused with the reason.
    let (status, body) = app
        .post(&format!("/api/chapters/{second_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "chapter.previous_not_ready");
    assert_error_detail(&body, "chapter.previous_not_ready", "chapter_id");
    assert_eq!(body["error"]["details"]["chapter_id"], first_id);
    assert_eq!(body["error"]["details"]["chapter_number"], 1);
    assert_eq!(body["error"]["details"]["status"], "planned");

    // `force` writes anyway (the Write dialog's "Write anyway").
    let (status, body) = app
        .post(
            &format!("/api/chapters/{second_id}/write"),
            json!({ "force": true }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // Once the first chapter is official AND synced, no force is needed.
    let app2 = TestApp::new().await;
    assign_models(&app2).await;
    let (_book_id, track_id, first_id) = book_and_chapter(&app2).await;
    let second = app2.create_chapter(track_id, "Chapter Two").await;
    let second_id = second["id"].as_i64().expect("chapter id") as i32;
    app2.set_chapter_status(first_id, "official").await;
    // Official alone is not enough: the canon must be synced too.
    let (status, body) = app2
        .post(&format!("/api/chapters/{second_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "chapter.previous_not_ready");
    assert_eq!(body["error"]["details"]["canon_status"], "none");

    app2.mark_canon_synced(first_id).await;
    let (status, body) = app2
        .post(&format!("/api/chapters/{second_id}/write"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "official + synced is ready: {body}"
    );
}

/// The first chapter of a track has no predecessor, so no readiness check applies.
#[tokio::test]
async fn write_of_a_first_chapter_needs_no_predecessor() {
    let app = TestApp::new().await;
    assign_models(&app).await;
    let (_book_id, _track_id, chapter_id) = book_and_chapter(&app).await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

/// Writing a chapter that already has text keeps the text: the kickoff carries it and asks the
/// writer to plan the rest (spec/08 §1.1).
#[tokio::test]
async fn write_of_a_chapter_with_text_carries_the_draft_in_the_kickoff() {
    let app = TestApp::new().await;
    assign_models(&app).await;
    let (_book_id, _track_id, chapter_id) = book_and_chapter(&app).await;
    app.put_text(chapter_id, 0, "Already written by the author.")
        .await;

    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().expect("session id") as i32;
    let (status, snapshot) = app.get(&format!("/api/sessions/{session_id}")).await;
    assert_eq!(status, StatusCode::OK);
    let text = snapshot["messages"][0]["message"]["blocks"]
        .as_array()
        .expect("blocks")
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    assert!(
        text.contains("Already written by the author."),
        "the kickoff carries the existing draft: {text}"
    );
    assert!(
        text.contains("从草稿末尾继续写"),
        "and asks the writer to continue from its end: {text}"
    );
}

/// A missing writer model is `409 model.missing_role` (spec/03 §4 step 1).
#[tokio::test]
async fn write_without_a_writer_model_is_a_missing_role_error() {
    let app = TestApp::new().await;
    let (_book_id, _track_id, chapter_id) = book_and_chapter(&app).await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "model.missing_role");
    assert_eq!(body["error"]["details"]["role"], "writer");
}

/// A rejected write stores nothing: the guidance is part of the operation, not a side effect.
///
/// The guidance is written before the session is created so the kickoff can read it (spec/08 §1.1),
/// but the two must commit together — otherwise a `409 model.missing_role` leaves the chapter with
/// guidance from a request that never started anything.
#[tokio::test]
async fn a_rejected_write_does_not_store_the_guidance() {
    let app = TestApp::new().await;
    let (_book_id, _track_id, chapter_id) = book_and_chapter(&app).await;
    let (status, body) = app
        .post(
            &format!("/api/chapters/{chapter_id}/write"),
            json!({ "guidance": "Write it in the rain." }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "model.missing_role");
    let (_, chapter) = app.get(&format!("/api/chapters/{chapter_id}")).await;
    assert_eq!(
        chapter["guidance"], "",
        "a failed write must not store guidance: {chapter}"
    );
}

/// spec/10 §3 names `details.chapter_id` and `details.reason` for `chapter.previous_not_ready`.
///
/// The reason is a code worded by the Write dialog, not a sentence written by the server, so the
/// dialog can say *why* the previous chapter blocks the write (not official yet, or official but
/// its canon is not synced) and offer "Write anyway" with an accurate explanation.
#[tokio::test]
async fn previous_not_ready_reports_a_reason_code() {
    let app = TestApp::new().await;
    assign_models(&app).await;
    let (_book_id, track_id, first_id) = book_and_chapter(&app).await;
    let second = app.create_chapter(track_id, "Chapter Two").await;
    let second_id = second["id"].as_i64().expect("chapter id") as i32;

    // Not official yet.
    let (status, body) = app
        .post(&format!("/api/chapters/{second_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "chapter.previous_not_ready");
    assert_eq!(
        body["error"]["details"]["reason"], "not_official",
        "the reason names the failing condition: {body}"
    );

    // Official, but its canon is not synced.
    app.set_chapter_status(first_id, "official").await;
    let (status, body) = app
        .post(&format!("/api/chapters/{second_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["error"]["details"]["reason"], "canon_not_synced",
        "the reason distinguishes the canon state: {body}"
    );
}
