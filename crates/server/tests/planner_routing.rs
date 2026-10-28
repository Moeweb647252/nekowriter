//! The planner's model routing (PLAN M12.3, spec/03 §4, D-146).
//!
//! The planner resolves `book.planner → global.planner → book.writer → global.writer`. An unset
//! planner therefore reproduces today's writer behaviour exactly, so no stored setting has to be
//! rewritten at the upgrade. The last case proves the documented `model.missing_role` error names
//! the planner role.

use axum::http::StatusCode;
use nekowriter::db::entities::books;
use nekowriter::db::json::{AgentRole, SessionKind};
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter::testing::TestApp;
use sea_orm::EntityTrait;
use serde_json::json;

/// Creates the Demo provider and four model profiles, returning their ids.
async fn models(app: &TestApp) -> [i64; 4] {
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
    let provider_id = provider["id"].as_i64().expect("provider id");
    let mut ids = Vec::new();
    for name in [
        "global writer",
        "global planner",
        "book writer",
        "book planner",
    ] {
        let (status, profile) = app
            .post(
                "/api/model-profiles",
                json!({
                    "provider_id": provider_id,
                    "display_name": name,
                    "model": name.replace(' ', "-"),
                    "context_window": 200_000,
                    "max_output_tokens": 8_192,
                    "options": {},
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{profile}");
        ids.push(profile["id"].as_i64().expect("profile id"));
    }
    [ids[0], ids[1], ids[2], ids[3]]
}

async fn book_row(app: &TestApp, book_id: i32) -> books::Model {
    books::Entity::find_by_id(book_id)
        .one(app.db().conn())
        .await
        .expect("query book")
        .expect("book row")
}

/// Resolves the planner through the production resolution function.
async fn planner_model(app: &TestApp, book_id: i32) -> Option<i32> {
    let book = book_row(app, book_id).await;
    sessions::try_resolve_model(app.db().conn(), Some(&book), AgentRole::Planner)
        .await
        .expect("resolve planner")
}

#[tokio::test]
async fn the_four_level_precedence_is_book_planner_global_planner_book_writer_global_writer() {
    let app = TestApp::new().await;
    let [global_writer, global_planner, book_writer, book_planner] = models(&app).await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;

    // Nothing set: no planner model.
    assert_eq!(planner_model(&app, book_id).await, None);

    // Global writer: the planner follows it (today's behaviour exactly).
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": { "writer": global_writer } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        planner_model(&app, book_id).await,
        Some(global_writer as i32)
    );

    // A book writer override beats the global writer.
    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "model_overrides": { "writer": book_writer } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(planner_model(&app, book_id).await, Some(book_writer as i32));

    // A global planner beats the book writer.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": { "writer": global_writer, "planner": global_planner } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        planner_model(&app, book_id).await,
        Some(global_planner as i32)
    );

    // A book planner beats everything.
    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "model_overrides": { "writer": book_writer, "planner": book_planner } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        planner_model(&app, book_id).await,
        Some(book_planner as i32)
    );
}

#[tokio::test]
async fn a_missing_planner_reports_model_missing_role_for_the_planner() {
    let app = TestApp::new().await;
    let _models = models(&app).await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;

    // No planner and no writer anywhere.
    let error = sessions::create(
        app.state(),
        CreateSession {
            kind: SessionKind::ChapterOutline,
            book_id: Some(book_id),
            chapter_id: Some(chapter_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect_err("the planner has no model");
    assert_eq!(error.code, nekowriter::error::ErrorCode::ModelMissingRole);
    assert_eq!(
        error.details.as_ref().and_then(|d| d["role"].as_str()),
        Some("planner")
    );
}
