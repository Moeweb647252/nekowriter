//! Reviewer-session races against approval (M5-01, M5-02, M5-07).
//!
//! These use the same hold-the-write-lock pattern as `outline_concurrency.rs`.

use std::time::Duration;

use axum::http::StatusCode;
use nekowriter::db::Db;
use nekowriter::testing::TestApp;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde_json::json;
use tokio::sync::oneshot;

async fn app_with_book() -> (TestApp, i32, i32) {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    (app, book_id, chapter_id)
}

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
            json!({ "roles_defaults": {
                "conception": profile_id,
                "researcher": profile_id,
                "planner": profile_id,
                "writer": profile_id,
                "reviewer": profile_id,
                "archivist": profile_id,
            }}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn scalar(app: &TestApp, sql: &str) -> i64 {
    let row = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i64>("", "n").expect("n")
}

/// Puts a chapter in `in_review` with one paragraph.
async fn in_review(app: &TestApp, chapter_id: i32) {
    app.execute(&format!(
        "UPDATE chapters SET status = 'in_review', \
         content = '{{\"paragraphs\":[{{\"pid\":\"p1\",\"text\":\"正文。\"}}],\"tail_open\":false}}' \
         WHERE id = {chapter_id}"
    ))
    .await;
}

async fn completed_reviewer(app: &TestApp, book_id: i32, chapter_id: i32) -> i32 {
    app.execute(&format!(
        "INSERT INTO agent_sessions(book_id, role, kind, chapter_id, track_id, model_snapshot, \
         system_prompt, tools, status, phase, base_content_version, counters, usage, created_at, \
         updated_at) VALUES ({book_id}, 'reviewer', 'chapter_review', {chapter_id}, 1, '{{}}', \
         '', '[]', 'completed', 'none', 0, '{{}}', '{{}}', '2026-01-01T00:00:00Z', \
         '2026-01-01T00:00:00Z')"
    ))
    .await;
    scalar(
        app,
        "SELECT max(id) AS n FROM agent_sessions WHERE kind = 'chapter_review'",
    )
    .await as i32
}

async fn insert_report(
    app: &TestApp,
    chapter_id: i32,
    session_id: i32,
    content_version: i32,
) -> i32 {
    app.execute(&format!(
        "INSERT INTO review_reports(chapter_id, content_version, session_id, attempt, verdict, \
         summary, issues, feedback, created_at) VALUES ({chapter_id}, {content_version}, \
         {session_id}, 1, 'pass', 'ok', '[]', 'fine', '2026-01-01T00:00:00Z')"
    ))
    .await;
    scalar(app, "SELECT max(id) AS n FROM review_reports").await as i32
}

/// Holds an update of `status` in a write transaction until `release` is sent.
async fn hold_status(
    app: &TestApp,
    chapter_id: i32,
    status: &str,
) -> (
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
    oneshot::Receiver<()>,
) {
    let (begun_tx, begun_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let db = app.db().clone();
    let status = status.to_owned();
    let updater = tokio::spawn(async move {
        Db::write(&db, move |tx| {
            Box::pin(async move {
                tx.execute_unprepared(&format!(
                    "UPDATE chapters SET status = '{status}' WHERE id = {chapter_id}"
                ))
                .await?;
                let _ = begun_tx.send(());
                let _ = release_rx.await;
                Ok(())
            })
        })
        .await
        .expect("the status update commits");
    });
    (release_tx, updater, begun_rx)
}

#[tokio::test]
async fn ask_reviewer_refuses_a_concurrently_approved_chapter() {
    let (app, book_id, chapter_id) = app_with_book().await;
    assign_models(&app).await;
    in_review(&app, chapter_id).await;
    let (release, updater, begun) = hold_status(&app, chapter_id, "official").await;
    begun.await.expect("the updater holds the lock");
    let state = app.state().clone();
    let call = tokio::spawn(async move {
        nekowriter::services::reviews::ask_reviewer(&state, chapter_id).await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    release.send(()).expect("release");
    let error = call
        .await
        .expect("task")
        .expect_err("official chapter must refuse a reviewer");
    assert_eq!(
        error.code,
        nekowriter::error::ErrorCode::ChapterInvalidState
    );
    updater.await.expect("updater");
    assert_eq!(
        scalar(
            &app,
            &format!("SELECT count(*) AS n FROM agent_sessions WHERE chapter_id = {chapter_id} AND kind = 'chapter_review'"),
        )
        .await,
        0
    );
    let _ = book_id;
}

#[tokio::test]
async fn rereview_changed_text_refuses_a_concurrently_approved_chapter() {
    let (app, book_id, chapter_id) = app_with_book().await;
    assign_models(&app).await;
    in_review(&app, chapter_id).await;
    app.execute(&format!(
        "UPDATE chapters SET content_version = 1 WHERE id = {chapter_id}"
    ))
    .await;
    let session = completed_reviewer(&app, book_id, chapter_id).await;
    let report = insert_report(&app, chapter_id, session, 0).await;
    let (release, updater, begun) = hold_status(&app, chapter_id, "official").await;
    begun.await.expect("the updater holds the lock");
    let state = app.state().clone();
    let call = tokio::spawn(async move {
        nekowriter::services::reviews::rereview(
            &state,
            report,
            nekowriter::api::dto::RereviewInput {
                comment: "再看一次".to_owned(),
            },
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    release.send(()).expect("release");
    let error = call
        .await
        .expect("task")
        .expect_err("official chapter must refuse rereview");
    assert_eq!(
        error.code,
        nekowriter::error::ErrorCode::ChapterInvalidState
    );
    updater.await.expect("updater");
    assert_eq!(
        scalar(
            &app,
            &format!("SELECT count(*) AS n FROM agent_sessions WHERE chapter_id = {chapter_id} AND kind = 'chapter_review'"),
        )
        .await,
        1,
        "the completed reviewer is not replaced"
    );
}

#[tokio::test]
async fn rereview_same_text_refuses_a_concurrently_approved_chapter() {
    let (app, book_id, chapter_id) = app_with_book().await;
    assign_models(&app).await;
    in_review(&app, chapter_id).await;
    let session = completed_reviewer(&app, book_id, chapter_id).await;
    let report = insert_report(&app, chapter_id, session, 0).await;
    let (release, updater, begun) = hold_status(&app, chapter_id, "official").await;
    begun.await.expect("the updater holds the lock");
    let state = app.state().clone();
    let call = tokio::spawn(async move {
        nekowriter::services::reviews::rereview(
            &state,
            report,
            nekowriter::api::dto::RereviewInput {
                comment: "再看一次".to_owned(),
            },
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    release.send(()).expect("release");
    let error = call
        .await
        .expect("task")
        .expect_err("official chapter must refuse rereview");
    assert_eq!(
        error.code,
        nekowriter::error::ErrorCode::ChapterInvalidState
    );
    updater.await.expect("updater");
    let status = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("SELECT status FROM agent_sessions WHERE id = {session}"),
        ))
        .await
        .expect("query")
        .expect("row")
        .try_get::<String>("", "status")
        .expect("status");
    assert_eq!(status, "completed");
}

/// Inserts an open reviewer session in a write transaction and holds it until released.
async fn hold_insert_reviewer(
    app: &TestApp,
    book_id: i32,
    chapter_id: i32,
) -> (
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
    oneshot::Receiver<()>,
) {
    let (begun_tx, begun_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let db = app.db().clone();
    let updater = tokio::spawn(async move {
        Db::write(&db, move |tx| {
            Box::pin(async move {
                tx.execute_unprepared(&format!(
                    "INSERT INTO agent_sessions(book_id, role, kind, chapter_id, track_id, \
                     model_snapshot, system_prompt, tools, status, phase, base_content_version, \
                     counters, usage, created_at, updated_at) VALUES ({book_id}, 'reviewer', \
                     'chapter_review', {chapter_id}, 1, '{{}}', '', '[]', 'stopped', 'none', 0, \
                     '{{}}', '{{}}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
                ))
                .await?;
                let _ = begun_tx.send(());
                let _ = release_rx.await;
                Ok(())
            })
        })
        .await
        .expect("the reviewer insert commits");
    });
    (release_tx, updater, begun_rx)
}

async fn reviewer_status(app: &TestApp, chapter_id: i32) -> String {
    let row = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "SELECT status FROM agent_sessions WHERE chapter_id = {chapter_id} \
                 AND kind = 'chapter_review' ORDER BY id DESC LIMIT 1"
            ),
        ))
        .await
        .expect("query reviewer")
        .expect("reviewer row");
    row.try_get::<String>("", "status").expect("status")
}

#[tokio::test]
async fn approve_abandons_a_reviewer_created_during_the_gap() {
    let (app, book_id, chapter_id) = app_with_book().await;
    assign_models(&app).await;
    in_review(&app, chapter_id).await;
    let (release, updater, begun) = hold_insert_reviewer(&app, book_id, chapter_id).await;
    begun.await.expect("the reviewer insert holds the lock");
    let state = app.state().clone();
    let call =
        tokio::spawn(
            async move { nekowriter::services::chapters::approve(&state, chapter_id).await },
        );
    tokio::time::sleep(Duration::from_millis(100)).await;
    release.send(()).expect("release");
    call.await
        .expect("approve task")
        .expect("approve succeeds after the reviewer insert");
    updater.await.expect("updater");
    assert_eq!(reviewer_status(&app, chapter_id).await, "abandoned");
}

#[tokio::test]
async fn send_back_abandons_a_reviewer_created_during_the_gap() {
    use nekowriter::api::dto::reviews::SendBackInput;
    let (app, book_id, chapter_id) = app_with_book().await;
    assign_models(&app).await;
    in_review(&app, chapter_id).await;
    let (release, updater, begun) = hold_insert_reviewer(&app, book_id, chapter_id).await;
    begun.await.expect("the reviewer insert holds the lock");
    let state = app.state().clone();
    let call = tokio::spawn(async move {
        nekowriter::services::reviews::send_back(
            &state,
            chapter_id,
            SendBackInput {
                feedback: "请修改".to_owned(),
                mode: nekowriter::db::json::FeedbackMode::Revise,
                review_report_id: None,
                issue_indexes: Vec::new(),
            },
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    release.send(()).expect("release");
    call.await
        .expect("send-back task")
        .expect("send-back succeeds after the reviewer insert");
    updater.await.expect("updater");
    assert_eq!(reviewer_status(&app, chapter_id).await, "abandoned");
}

/// A post-commit cleanup can see a reviewer that completed in the gap; that is success, not an
/// error that would turn the already-committed chapter transition into a failure (S-1).
#[tokio::test]
async fn abandoning_an_already_terminal_reviewer_is_not_an_error() {
    let (app, book_id, chapter_id) = app_with_book().await;
    assign_models(&app).await;
    in_review(&app, chapter_id).await;
    let session = completed_reviewer(&app, book_id, chapter_id).await;
    let state = app.state().clone();
    nekowriter::services::sessions::abandon_if_open(&state, session)
        .await
        .expect("a terminal reviewer is already cleaned up");
}
