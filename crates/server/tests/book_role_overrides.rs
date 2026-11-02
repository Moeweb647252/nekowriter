//! Per-book agent-role model overrides (PROJECT §8.12 "model overrides per role", spec/03 §4
//! step 1, spec/10 §2.2).
//!
//! The resolution chain is `book.model_overrides[role]` → `app_settings["roles.defaults"][role]`
//! → `409 model.missing_role`. Book settings renders this mapping and PATCHes it as one object, so
//! what the UI sends has to be exactly what the server stores and resolves.
//!
//! What already existed: `agent_creation.rs::the_book_override_wins_over_the_role_default` proves
//! the first hop (an override beats the global default) and
//! `agent_creation.rs::a_missing_role_model_is_model_missing_role` proves the last one (no model
//! for the role → `model.missing_role` with `details.role`). Neither covers the **reverse**
//! transition — clearing an override must fall back to the global default — nor the patch's
//! object semantics (a partial object replaces the whole column, so a client that sends only the
//! edited role silently clears its siblings), nor the round trip through `GET /books/:id`. Those
//! are the cases a per-role editor can get wrong, so they are pinned here.

use axum::http::StatusCode;
use nekowriter::db::json::SessionKind;
use nekowriter::testing::TestApp;
use serde_json::{Value, json};

/// The roles the editor lists, in the order the settings pages use (spec/02 §3).
const ROLES: [&str; 6] = [
    "conception",
    "researcher",
    "planner",
    "writer",
    "reviewer",
    "archivist",
];

/// Two model profiles on the Demo provider: a "global" one and a per-book one.
struct Models {
    global: i64,
    book: i64,
}

/// Creates the Demo provider and two model profiles.
async fn demo_models(app: &TestApp) -> Models {
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
    assert_eq!(status, StatusCode::CREATED, "create provider: {provider}");
    let provider_id = provider["id"].as_i64().expect("provider id");

    let mut ids = Vec::new();
    for (display_name, model) in [
        ("Global model", "global-model"),
        ("Book model", "book-model"),
    ] {
        let (status, profile) = app
            .post(
                "/api/model-profiles",
                json!({
                    "provider_id": provider_id,
                    "display_name": display_name,
                    "model": model,
                    "context_window": 200_000,
                    "max_output_tokens": 8_192,
                    "options": {},
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "create profile: {profile}");
        ids.push(profile["id"].as_i64().expect("profile id"));
    }
    Models {
        global: ids[0],
        book: ids[1],
    }
}

/// Assigns one model to every global role default (`PATCH /app-settings`, spec/10 §2.1).
async fn assign_role_defaults(app: &TestApp, model: i64) {
    let roles: Value = ROLES
        .iter()
        .map(|role| ((*role).to_owned(), json!(model)))
        .collect::<serde_json::Map<String, Value>>()
        .into();
    let (status, body) = app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, StatusCode::OK, "assign roles: {body}");
}

/// Creates a book and returns its id and the id of its main track.
async fn book_and_track(app: &TestApp) -> (i32, i32) {
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (status, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    assert_eq!(status, StatusCode::OK, "{tracks}");
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    (book_id, track_id)
}

/// The label of the model a `chapter_write` session resolved for the writer role.
///
/// Each call creates a fresh chapter: starting a session moves its chapter to `drafting`, so a
/// second session on the same chapter is `chapter.session_exists` (spec/08 §1.1), and these tests
/// resolve the model several times for one book.
async fn writer_model_label(app: &TestApp, book_id: i32, track_id: i32) -> Result<String, String> {
    let chapter = app.create_chapter(track_id, "A chapter").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    match nekowriter::services::sessions::create(
        app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(book_id),
            chapter_id: Some(chapter_id),
            ..Default::default()
        },
    )
    .await
    {
        Ok((session_id, _warnings)) => {
            let (status, body) = app.get(&format!("/api/sessions/{session_id}")).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            Ok(body["session"]["model_label"]
                .as_str()
                .expect("model label")
                .to_owned())
        }
        Err(error) => Err(error.code.as_str().to_owned()),
    }
}

/// `PATCH /books/:id` stores the mapping and `GET /books/:id` returns it unchanged (spec/10 §2.2).
#[tokio::test]
async fn the_patch_round_trips_every_role_through_the_book_dto() {
    let app = TestApp::new().await;
    let models = demo_models(&app).await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id");

    // A new book has no overrides at all.
    let (status, body) = app.get(&format!("/api/books/{book_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for role in ROLES {
        assert!(
            body["model_overrides"][role].is_null(),
            "a new book has no {role} override: {body}"
        );
    }

    let overrides = json!({
        "conception": models.book,
        "researcher": models.global,
        "planner": null,
        "writer": models.book,
        "reviewer": null,
        "archivist": null,
        // Old rows and requests without efforts read as the empty choice set (D-162).
        "efforts": {},
    });
    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "model_overrides": overrides }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["model_overrides"], overrides, "the response echoes it");

    let (status, fetched) = app.get(&format!("/api/books/{book_id}")).await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["model_overrides"], overrides, "and it is stored");
    // The rest of the book is untouched by the patch.
    assert_eq!(fetched["title"], "Book");
    assert_eq!(fetched["status"], "active");
}

/// The patch replaces the whole object: sending one role is a full replacement, which is why the
/// UI must always send every role. Pinned so a server change cannot make the UI's behaviour
/// silently different from what it renders.
#[tokio::test]
async fn the_patch_replaces_the_whole_object_not_one_key() {
    let app = TestApp::new().await;
    let models = demo_models(&app).await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id");

    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "model_overrides": { "writer": models.book, "reviewer": models.global } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["model_overrides"]["writer"], models.book);
    assert_eq!(body["model_overrides"]["reviewer"], models.global);

    // A partial object clears the roles it omits: this is the bug the editor avoids by always
    // sending the full object, and the reason the UI test asserts the whole payload.
    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "model_overrides": { "writer": models.global } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["model_overrides"]["writer"], models.global);
    assert!(
        body["model_overrides"]["reviewer"].is_null(),
        "an omitted role is cleared: {body}"
    );
}

/// Book override wins; clearing it falls back to the **global** default; with neither, creating the
/// session is `model.missing_role` (spec/03 §4 step 1).
#[tokio::test]
async fn clearing_an_override_falls_back_to_the_global_default() {
    let app = TestApp::new().await;
    let models = demo_models(&app).await;
    assign_role_defaults(&app, models.global).await;
    let (book_id, track_id) = book_and_track(&app).await;

    // 1. No override: the global default resolves.
    let label = writer_model_label(&app, book_id, track_id)
        .await
        .expect("the global default resolves the writer");
    assert_eq!(label, "Global model");

    // 2. The book override wins over the global default.
    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "model_overrides": { "writer": models.book } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let label = writer_model_label(&app, book_id, track_id)
        .await
        .expect("the book override resolves the writer");
    assert_eq!(label, "Book model", "the book override must win");

    // 3. Clearing the override sends `null` and falls back to the global default.
    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "model_overrides": { "writer": null } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["model_overrides"]["writer"].is_null(), "{body}");
    let label = writer_model_label(&app, book_id, track_id)
        .await
        .expect("clearing an override falls back to the global default");
    assert_eq!(
        label, "Global model",
        "a cleared override must fall back to the global default"
    );

    // 4. With neither a book override nor a global default, the session fails.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": { "writer": null } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let error = writer_model_label(&app, book_id, track_id)
        .await
        .expect_err("no model is configured for the writer role any more");
    assert_eq!(error, "model.missing_role");
}

/// The overrides are per book: one book's choice does not leak into another (spec/03 §4 step 1).
#[tokio::test]
async fn overrides_are_scoped_to_their_own_book() {
    let app = TestApp::new().await;
    let models = demo_models(&app).await;
    assign_role_defaults(&app, models.global).await;
    let (overridden_book, overridden_track) = book_and_track(&app).await;
    let (plain_book, plain_track) = book_and_track(&app).await;

    let (status, body) = app
        .patch(
            &format!("/api/books/{overridden_book}"),
            json!({ "model_overrides": { "writer": models.book } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(
        writer_model_label(&app, overridden_book, overridden_track)
            .await
            .expect("the second book resolves"),
        "Book model"
    );
    assert_eq!(
        writer_model_label(&app, plain_book, plain_track)
            .await
            .expect("the first book resolves"),
        "Global model",
        "a book without an override keeps the global default"
    );
}

/// Deleting the overridden profile does not silently re-resolve to the global default: the stored
/// id is what the chain returns, and the session then fails with the documented code. The UI
/// mirrors this by rendering `settings:model_missing` rather than a blank select.
#[tokio::test]
async fn a_deleted_override_does_not_fall_through_to_the_global_default() {
    let app = TestApp::new().await;
    let models = demo_models(&app).await;
    assign_role_defaults(&app, models.global).await;
    let (book_id, track_id) = book_and_track(&app).await;

    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "model_overrides": { "writer": models.book } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = app
        .delete(&format!("/api/model-profiles/{}", models.book))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // The book still names the deleted profile: it is not quietly replaced by the global default.
    let (_, fetched) = app.get(&format!("/api/books/{book_id}")).await;
    assert_eq!(fetched["model_overrides"]["writer"], models.book);

    // Resolving it fails with the documented code rather than running the wrong model.
    let error = writer_model_label(&app, book_id, track_id)
        .await
        .expect_err("the overridden profile is gone");
    assert_eq!(error, "model.missing_role");
}
