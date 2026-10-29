//! `guard_update` keeps the expected-status guard: a stale expected status is
//! `session.invalid_state` and writes no column (spec/03 §14, D-034).

use nekowriter::agent::session::guard_update;
use nekowriter::db::entities::agent_sessions;
use nekowriter::db::json::SessionStatus;
use nekowriter::error::ErrorCode;
use nekowriter::testing::TestApp;
use sea_orm::{ConnectionTrait, DatabaseBackend, Set, Statement};

#[tokio::test]
async fn a_stale_status_is_session_invalid_state_and_changes_nothing() {
    let app = TestApp::new().await;
    let book = app.create_book("Guard Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.insert_writer_session(chapter_id, "held").await;

    let conn = app.db().conn();
    let row = conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT id, status, pause_requested, updated_at FROM agent_sessions \
             ORDER BY id DESC LIMIT 1",
        ))
        .await
        .expect("query session")
        .expect("session row");
    let session_id: i32 = row.try_get("", "id").expect("session id");
    let before_status: String = row.try_get("", "status").expect("status");
    let before_pause: i32 = row.try_get("", "pause_requested").expect("pause");
    let before_updated: String = row.try_get("", "updated_at").expect("updated_at");

    let error = guard_update(
        app.db().conn(),
        session_id,
        SessionStatus::Running,
        agent_sessions::ActiveModel {
            pause_requested: Set(true),
            ..Default::default()
        },
    )
    .await
    .expect_err("the held row must not match the running guard");
    assert_eq!(error.code, ErrorCode::SessionInvalidState);

    let after = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT status, pause_requested, updated_at FROM agent_sessions WHERE id = ?",
            vec![session_id.into()],
        ))
        .await
        .expect("query session")
        .expect("session row");
    assert_eq!(
        after.try_get::<String>("", "status").expect("status"),
        before_status
    );
    assert_eq!(
        after.try_get::<i32>("", "pause_requested").expect("pause"),
        before_pause
    );
    assert_eq!(
        after
            .try_get::<String>("", "updated_at")
            .expect("updated_at"),
        before_updated
    );
    assert_eq!(before_status, "held");
}
