//! Outline-session concurrency and locking rules (PLAN M12.5, spec/03 §14, spec/08 §1.4, D-146).
//!
//! R6: at most one open session across the planner and writer roles per chapter, so outline,
//! write and revise sessions exclude each other; chapters never block each other. The creation
//! check is re-run inside the transaction (D-121), so two concurrent `POST /outline` calls start
//! exactly one session.

use std::time::Duration;

use axum::http::StatusCode;
use nekowriter::testing::TestApp;
use nekowriter_llm::mock::{ScriptStep, ScriptedProvider};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde_json::json;

/// A migrated app with an active book, its main track and one chapter.
async fn app_with_book() -> (TestApp, i32, i32) {
    let app = TestApp::new().await;
    let (book_id, _track_id, chapter_id) = create_book(&app).await;
    (app, book_id, chapter_id)
}

async fn create_book(app: &TestApp) -> (i32, i32, i32) {
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Chapter One").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    (book_id, track_id, chapter_id)
}

/// Assigns one Demo model to every role so the endpoints can resolve a model.
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
                "effort_levels": [],
                "default_effort": null,
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
                "writer": profile_id,
                "reviewer": profile_id,
                "archivist": profile_id,
            }}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Inserts an open planner session for a chapter, without a runner (a state fixture).
async fn insert_outline_session(app: &TestApp, chapter_id: i32, status: &str) -> i32 {
    app.execute(&format!(
        "INSERT INTO agent_sessions(book_id, role, kind, chapter_id, track_id, model_snapshot, \
         system_prompt, tools, status, phase, counters, usage, created_at, updated_at) \
         SELECT book_id, 'planner', 'chapter_outline', id, track_id, '{{\"provider_id\":0,\"protocol\":\"mock\",\"base_url\":\"\",\"extra_headers\":{{}},\"model\":\"\",\"context_window\":0,\"max_output_tokens\":0,\"options\":null}}', '', '[]', \
         '{status}', 'none', '{{}}', '{{}}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z' \
         FROM chapters WHERE id = {chapter_id}"
    ))
    .await;
    let row = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT max(id) AS id FROM agent_sessions".to_owned(),
        ))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i32>("", "id").expect("id")
}

/// One concurrent `POST /outline` wins; the loser answers `chapter.session_exists` with its
/// `session_id` (D-121 style).
#[tokio::test]
async fn concurrent_outline_creations_start_one_session() {
    let mut app = TestApp::new().await;
    // Hold the winner's model request open long enough for both requests to overlap.
    let _scripted = app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new()
            .tool(
                "submit_outline",
                json!({ "outline": "细纲", "reference_notes": "- 要点" }),
            )
            .delay(Duration::from_secs(2)),
    ]));
    assign_models(&app).await;
    let (_book_id, _track_id, chapter_id) = create_book(&app).await;

    let path = format!("/api/chapters/{chapter_id}/outline");
    let (a, b) = tokio::join!(app.post(&path, json!({})), app.post(&path, json!({})));
    let mut created = 0;
    let mut refused = None;
    for (status, body) in [&a, &b] {
        match *status {
            StatusCode::CREATED => created += 1,
            StatusCode::CONFLICT => refused = Some(body.clone()),
            other => panic!("unexpected status {other}: {body}"),
        }
    }
    assert_eq!(created, 1, "exactly one outline may start: {a:?} {b:?}");
    let refused = refused.expect("the loser answers 409");
    assert_eq!(
        refused["error"]["code"], "chapter.session_exists",
        "{refused}"
    );
    assert!(
        refused["error"]["details"]["session_id"].is_number(),
        "{refused}"
    );
}

/// An open outline session blocks starting a write session on the same chapter.
#[tokio::test]
async fn an_open_outline_session_blocks_a_write_session() {
    let (app, _book_id, chapter_id) = app_with_book().await;
    assign_models(&app).await;
    app.execute(&format!(
        "UPDATE chapters SET outline = '细纲' WHERE id = {chapter_id}"
    ))
    .await;
    insert_outline_session(&app, chapter_id, "held").await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/write"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "chapter.session_exists");
}

/// An open owner session blocks starting an outline session.
#[tokio::test]
async fn an_open_writer_session_blocks_an_outline_session() {
    let (app, _book_id, chapter_id) = app_with_book().await;
    assign_models(&app).await;
    app.insert_writer_session(chapter_id, "held").await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/outline"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "chapter.session_exists");
}

/// Author edits are `423 chapter.locked` while the outline session runs.
#[tokio::test]
async fn author_edits_are_locked_while_an_outline_session_runs() {
    let (app, _book_id, chapter_id) = app_with_book().await;
    insert_outline_session(&app, chapter_id, "running").await;
    let (status, body) = app
        .patch(
            &format!("/api/chapters/{chapter_id}"),
            json!({ "outline": "作者想改的细纲" }),
        )
        .await;
    assert_eq!(status, StatusCode::LOCKED, "{body}");
    assert_eq!(body["error"]["code"], "chapter.locked");
    let (status, body) = app
        .patch(
            &format!("/api/chapters/{chapter_id}/content"),
            json!({ "base_version": 0, "ops": [] }),
        )
        .await;
    assert_eq!(status, StatusCode::LOCKED, "{body}");
    assert_eq!(body["error"]["code"], "chapter.locked");
}

/// PATCH per kind: an outline session accepts `pause_after_outline` only.
#[tokio::test]
async fn patch_of_an_outline_session_accepts_only_pause_after_outline() {
    let (app, _book_id, chapter_id) = app_with_book().await;
    let session_id = insert_outline_session(&app, chapter_id, "paused").await;

    let (status, body) = app
        .patch(
            &format!("/api/sessions/{session_id}"),
            json!({ "auto_pause": true }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "session.invalid_state");

    let (status, body) = app
        .patch(
            &format!("/api/sessions/{session_id}"),
            json!({ "pause_after_outline": false }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["pause_after_outline"], false);
}

/// The in-transaction creation guard refuses a second open planner session even when the caller
/// bypasses the endpoint's pre-check (F13). This is the lock-group proof; the concurrent HTTP test
/// above only proves the endpoint's pre-check plus the same guard.
#[tokio::test]
async fn a_direct_second_create_is_refused_inside_the_transaction() {
    let (app, book_id, chapter_id) = app_with_book().await;
    assign_models(&app).await;
    insert_outline_session(&app, chapter_id, "held").await;
    let error = nekowriter::services::sessions::create(
        app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: nekowriter::db::json::SessionKind::ChapterOutline,
            book_id: Some(book_id),
            chapter_id: Some(chapter_id),
            pause_after_outline: Some(true),
            ..Default::default()
        },
    )
    .await
    .expect_err("the in-transaction guard must refuse a second session");
    assert_eq!(
        error.code,
        nekowriter::error::ErrorCode::ChapterSessionExists
    );
}

/// Approve is in the lock group: an open planner session refuses it with `chapter.session_exists`
/// (F13, spec/08 §1.1).
#[tokio::test]
async fn approve_refuses_an_open_planner_session() {
    let (app, _book_id, chapter_id) = app_with_book().await;
    app.execute(&format!(
        "UPDATE chapters SET status = 'in_review', content = '{{\"paragraphs\":[{{\"pid\":\"p1\",\"text\":\"正文。\"}}],\"tail_open\":false}}' WHERE id = {chapter_id}"
    ))
    .await;
    insert_outline_session(&app, chapter_id, "held").await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "chapter.session_exists");
}

/// A running planner blocks submit with `chapter.session_exists`; a held one is abandoned by the
/// manual submit, exactly like a writer session (F13, spec/08 §1.1).
#[tokio::test]
async fn submit_uses_the_planner_writer_lock_group() {
    let (app, _book_id, chapter_id) = app_with_book().await;
    app.execute(&format!(
        "UPDATE chapters SET status = 'drafting', content = '{{\"paragraphs\":[{{\"pid\":\"p1\",\"text\":\"正文。\"}}],\"tail_open\":false}}' WHERE id = {chapter_id}"
    ))
    .await;
    let running = insert_outline_session(&app, chapter_id, "running").await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "chapter.session_exists");
    assert_eq!(body["error"]["details"]["session_id"], running);
    app.execute(&format!(
        "UPDATE agent_sessions SET status = 'held' WHERE id = {running}"
    ))
    .await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, session) = app.get(&format!("/api/sessions/{running}")).await;
    assert_eq!(session["session"]["status"], "abandoned");
}

/// Restore is in the lock group: a running planner gives `chapter.locked` (F13, spec/08 §1.2).
#[tokio::test]
async fn restore_refuses_a_running_planner_session() {
    let (app, _book_id, chapter_id) = app_with_book().await;
    app.execute(&format!(
        "UPDATE chapters SET status = 'drafting' WHERE id = {chapter_id}"
    ))
    .await;
    app.execute(&format!(
        "INSERT INTO chapter_versions(chapter_id, version_no, kind, title, content, word_count, note, created_at) \
         VALUES ({chapter_id}, 1, 'manual', 'Chapter One', '{{\"paragraphs\":[],\"tail_open\":false}}', 0, '', '2026-01-01T00:00:00Z')"
    ))
    .await;
    let row = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT max(id) AS id FROM chapter_versions".to_owned(),
        ))
        .await
        .expect("query")
        .expect("row");
    let version_id = row.try_get::<i32>("", "id").expect("version id");
    insert_outline_session(&app, chapter_id, "running").await;
    let (status, body) = app
        .post(
            &format!("/api/chapter-versions/{version_id}/restore"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::LOCKED, "{body}");
    assert_eq!(body["error"]["code"], "chapter.locked");
}

/// `chapters::outline` must recheck the chapter status inside the creation transaction: an
/// approve that commits `official` between the endpoint's read and the insert refuses the planner
/// session (R3-03).
#[tokio::test]
async fn a_concurrent_approve_makes_the_outline_creation_refuse() {
    use nekowriter::db::Db;
    use tokio::sync::oneshot;

    let (app, book_id, chapter_id) = app_with_book().await;
    assign_models(&app).await;
    let (begun_tx, begun_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let db = app.db().clone();
    let updater = tokio::spawn(async move {
        Db::write(&db, move |tx| {
            Box::pin(async move {
                tx.execute_unprepared(&format!(
                    "UPDATE chapters SET status = 'official' WHERE id = {chapter_id}"
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
    begun_rx.await.expect("the updater holds the write lock");

    let state = app.state().clone();
    let create = tokio::spawn(async move {
        nekowriter::services::chapters::outline(
            &state,
            chapter_id,
            nekowriter::api::dto::chapters::ChapterOutlineInput::default(),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    release_tx.send(()).expect("release the updater");
    let error = create
        .await
        .expect("the create task did not panic")
        .expect_err("an official chapter must refuse the outline session");
    assert_eq!(
        error.code,
        nekowriter::error::ErrorCode::ChapterInvalidState
    );
    updater.await.expect("updater task");
    let count = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "SELECT count(*) AS n FROM agent_sessions WHERE chapter_id = {chapter_id}                  AND kind = 'chapter_outline'"
            ),
        ))
        .await
        .expect("query sessions")
        .expect("row")
        .try_get::<i64>("", "n")
        .expect("count");
    assert_eq!(
        count, 0,
        "no planner session exists on the official chapter"
    );
    let _ = book_id;
}

/// `reviews::send_back` must reload and revalidate the chapter inside its write transaction:
/// an approve can commit `official` between its precheck and its feedback/session writes. Both
/// revise and redraft must refuse without clearing the official text or leaving a feedback row
/// (R4-01).
async fn race_send_back(mode: nekowriter::db::json::FeedbackMode) {
    use nekowriter::api::dto::reviews::SendBackInput;
    use nekowriter::db::Db;
    use tokio::sync::oneshot;

    let (app, _book_id, chapter_id) = app_with_book().await;
    assign_models(&app).await;
    app.execute(&format!(
        "UPDATE chapters SET status = 'in_review', outline = '细纲', \
         content = '{{\"paragraphs\":[{{\"pid\":\"p1\",\"text\":\"正文。\"}}],\"tail_open\":false}}' \
         WHERE id = {chapter_id}"
    ))
    .await;
    let (begun_tx, begun_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let db = app.db().clone();
    let updater = tokio::spawn(async move {
        Db::write(&db, move |tx| {
            Box::pin(async move {
                tx.execute_unprepared(&format!(
                    "UPDATE chapters SET status = 'official' WHERE id = {chapter_id}"
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
    begun_rx.await.expect("the updater holds the write lock");

    let state = app.state().clone();
    let mode_label = mode.as_str().to_owned();
    let request = tokio::spawn(async move {
        nekowriter::services::reviews::send_back(
            &state,
            chapter_id,
            SendBackInput {
                feedback: "请按意见修改".to_owned(),
                mode,
                review_report_id: None,
                issue_indexes: Vec::new(),
            },
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    release_tx.send(()).expect("release the updater");
    let error = request
        .await
        .expect("the send-back task did not panic")
        .expect_err("an official chapter must refuse send-back");
    assert_eq!(
        error.code,
        nekowriter::error::ErrorCode::ChapterInvalidState
    );
    updater.await.expect("updater task");

    let row = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("SELECT status, content FROM chapters WHERE id = {chapter_id}"),
        ))
        .await
        .expect("query chapter")
        .expect("chapter");
    assert_eq!(
        row.try_get::<String>("", "status").expect("status"),
        "official"
    );
    assert!(
        row.try_get::<String>("", "content")
            .expect("content")
            .contains("正文。"),
        "the official text is untouched ({mode_label})"
    );
    let sessions = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "SELECT count(*) AS n FROM agent_sessions WHERE chapter_id = {chapter_id} \
                 AND kind IN ('chapter_revise','chapter_write')"
            ),
        ))
        .await
        .expect("query sessions")
        .expect("row")
        .try_get::<i64>("", "n")
        .expect("count");
    assert_eq!(sessions, 0, "no revision session is created ({mode_label})");
    let feedback = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("SELECT count(*) AS n FROM chapter_feedback WHERE chapter_id = {chapter_id}"),
        ))
        .await
        .expect("query feedback")
        .expect("row")
        .try_get::<i64>("", "n")
        .expect("count");
    assert_eq!(feedback, 0, "no feedback row is left ({mode_label})");
}

#[tokio::test]
async fn send_back_redraft_refuses_an_official_chapter() {
    race_send_back(nekowriter::db::json::FeedbackMode::Redraft).await;
}

#[tokio::test]
async fn send_back_revise_refuses_an_official_chapter() {
    race_send_back(nekowriter::db::json::FeedbackMode::Revise).await;
}

/// `write`/`outline` must recheck previous-chapter readiness inside the creation transaction
/// (M5-03): a previous canon change committed after the endpoint's read must refuse the session.
#[tokio::test]
async fn outline_rechecks_previous_readiness_inside_the_transaction() {
    use nekowriter::api::dto::chapters::ChapterOutlineInput;
    use nekowriter::db::Db;
    use tokio::sync::oneshot;

    let (app, _book_id, first_id) = app_with_book().await;
    assign_models(&app).await;
    app.execute(&format!(
        "UPDATE chapters SET status = 'official', canon_status = 'synced' WHERE id = {first_id}"
    ))
    .await;
    let (_, tracks) = app.get("/api/books/1/tracks").await;
    let track_id = tracks[0]["id"].as_i64().expect("track") as i32;
    let second = app.create_chapter(track_id, "Chapter Two").await;
    let second_id = second["id"].as_i64().expect("id") as i32;
    let (begun_tx, begun_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let db = app.db().clone();
    let updater = tokio::spawn(async move {
        Db::write(&db, move |tx| {
            Box::pin(async move {
                tx.execute_unprepared(&format!(
                    "UPDATE chapters SET canon_status = 'stale' WHERE id = {first_id}"
                ))
                .await?;
                let _ = begun_tx.send(());
                let _ = release_rx.await;
                Ok(())
            })
        })
        .await
        .expect("the previous-chapter update commits");
    });
    begun_rx.await.expect("the updater holds the write lock");
    let state = app.state().clone();
    let create = tokio::spawn(async move {
        nekowriter::services::chapters::outline(&state, second_id, ChapterOutlineInput::default())
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    release_tx.send(()).expect("release the updater");
    let error = create
        .await
        .expect("the create task did not panic")
        .expect_err("the previous chapter is no longer ready");
    assert_eq!(
        error.code,
        nekowriter::error::ErrorCode::ChapterPreviousNotReady
    );
    updater.await.expect("updater task");
    let count = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "SELECT count(*) AS n FROM agent_sessions WHERE chapter_id = {second_id} \
                 AND kind = 'chapter_outline'"
            ),
        ))
        .await
        .expect("query sessions")
        .expect("row")
        .try_get::<i64>("", "n")
        .expect("count");
    assert_eq!(count, 0, "no outline session is created");
}

/// The same previous-readiness recheck for the write path (S-4).
#[tokio::test]
async fn write_rechecks_previous_readiness_inside_the_transaction() {
    use nekowriter::api::dto::ChapterWriteInput;
    use nekowriter::db::Db;
    use tokio::sync::oneshot;

    let (app, _book_id, first_id) = app_with_book().await;
    assign_models(&app).await;
    app.execute(&format!(
        "UPDATE chapters SET status = 'official', canon_status = 'synced' WHERE id = {first_id}"
    ))
    .await;
    let (_, tracks) = app.get("/api/books/1/tracks").await;
    let track_id = tracks[0]["id"].as_i64().expect("track") as i32;
    let second = app.create_chapter(track_id, "Chapter Two").await;
    let second_id = second["id"].as_i64().expect("id") as i32;
    app.execute(&format!(
        "UPDATE chapters SET outline = '细纲' WHERE id = {second_id}"
    ))
    .await;
    let (begun_tx, begun_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let db = app.db().clone();
    let updater = tokio::spawn(async move {
        Db::write(&db, move |tx| {
            Box::pin(async move {
                tx.execute_unprepared(&format!(
                    "UPDATE chapters SET canon_status = 'stale' WHERE id = {first_id}"
                ))
                .await?;
                let _ = begun_tx.send(());
                let _ = release_rx.await;
                Ok(())
            })
        })
        .await
        .expect("the previous-chapter update commits");
    });
    begun_rx.await.expect("the updater holds the write lock");
    let state = app.state().clone();
    let create = tokio::spawn(async move {
        nekowriter::services::chapters::write(&state, second_id, ChapterWriteInput::default()).await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    release_tx.send(()).expect("release the updater");
    let error = create
        .await
        .expect("the create task did not panic")
        .expect_err("the previous chapter is no longer ready");
    assert_eq!(
        error.code,
        nekowriter::error::ErrorCode::ChapterPreviousNotReady
    );
    updater.await.expect("updater task");
    let count = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "SELECT count(*) AS n FROM agent_sessions WHERE chapter_id = {second_id} \
                 AND kind = 'chapter_write'"
            ),
        ))
        .await
        .expect("query sessions")
        .expect("row")
        .try_get::<i64>("", "n")
        .expect("count");
    assert_eq!(count, 0, "no write session is created");
}
