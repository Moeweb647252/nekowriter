//! Shared fixtures for the canon tests (PLAN M5.1).
//!
//! A book with a main track, plus helpers to create an isolated side track (the visibility
//! fixtures of spec/06 §2, D-014) and to read the canon rows the assertions check.

#![allow(dead_code)]

use axum::http::StatusCode;
use nekowriter::testing::TestApp;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde_json::json;

/// One `i64` from a fixture query (the raw `Value` mapping does not cover SQLite integers).
async fn scalar(app: &TestApp, sql: &str) -> i64 {
    let row = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .expect("fixture query")
        .expect("fixture row");
    row.try_get::<i64>("", "value").expect("fixture value")
}

/// Creates a book with its main track and returns `(book_id, main_track_id)`.
pub async fn book(app: &TestApp) -> (i32, i32) {
    let book = app.create_book("Canon Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (status, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    assert_eq!(status, StatusCode::OK, "{tracks}");
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    (book_id, track_id)
}

/// Creates an isolated side track and returns its id.
///
/// The tracks API cannot create side tracks until M10, so the fixture writes the row directly
/// into the migrated database — the visibility rules still apply because the services read the
/// row, not the route that made it (spec/06 §2, D-014).
pub async fn isolated_track(app: &TestApp, book_id: i32, title: &str) -> i32 {
    let ord = scalar(
        app,
        &format!("SELECT coalesce(max(ord), 0) + 1 AS value FROM tracks WHERE book_id = {book_id}"),
    )
    .await;
    app.execute(&format!(
        "INSERT INTO tracks(book_id, kind, title, synopsis, canon_mode, anchor_kind, ord, \
         created_at, updated_at) VALUES ({book_id}, 'side', '{title}', '', 'isolated', 'none', \
         {ord}, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    scalar(
        app,
        &format!("SELECT max(id) AS value FROM tracks WHERE book_id = {book_id}"),
    )
    .await as i32
}

/// Creates a canon-mode side track (its chapters count for shared entities, D-036).
pub async fn canon_track(app: &TestApp, book_id: i32, title: &str) -> i32 {
    let ord = scalar(
        app,
        &format!("SELECT coalesce(max(ord), 0) + 1 AS value FROM tracks WHERE book_id = {book_id}"),
    )
    .await;
    app.execute(&format!(
        "INSERT INTO tracks(book_id, kind, title, synopsis, canon_mode, anchor_kind, ord, \
         created_at, updated_at) VALUES ({book_id}, 'side', '{title}', '', 'canon', 'none', \
         {ord}, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    scalar(
        app,
        &format!("SELECT max(id) AS value FROM tracks WHERE book_id = {book_id}"),
    )
    .await as i32
}

/// Runs idempotent fixture SQL (the helpers above build rows the API cannot create yet).
pub async fn sql(app: &TestApp, statement: &str) {
    app.execute(statement).await;
}

/// The number of rows of one canon table.
pub async fn count(app: &TestApp, table: &str) -> i64 {
    scalar(app, &format!("SELECT count(*) AS value FROM {table}")).await
}

/// A JSON body for an entity create.
pub fn entity_input(kind: &str, name: &str) -> serde_json::Value {
    json!({ "kind": kind, "name": name })
}
