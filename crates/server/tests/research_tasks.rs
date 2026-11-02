//! Research task tests (PLAN 8.3, spec/07 §8, spec/10 §2.5, spec/05 §8).
//!
//! The Demo provider drives the runtime path: its steps save the corpus's two knowledge entries
//! and call `finish_research`, exactly the demo behavior of spec/04 §7.2 — no network, no real
//! provider. The helpers are local because the shared `agent_harness` wires a **scripted**
//! provider, which would replace the Demo behavior these tests exist to exercise.

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use nekowriter::testing::{TestApp, assert_error};
use serde_json::{Value, json};

/// An app with a Demo model assigned to the researcher role and **no** provider override, so the
/// Demo provider's own research behavior runs (spec/04 §7.2).
async fn demo_app() -> TestApp {
    let app = TestApp::new().await;
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
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create profile: {profile}");
    let profile_id = profile["id"].as_i64().expect("profile id") as i32;
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": { "researcher": profile_id } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "assign researcher: {body}");
    app
}

/// A book id through the API.
///
/// `start_conception` controls the status of spec/02 §2: `true` leaves the book `conceiving`
/// (a research task of it is a draft), `false` starts it `active`.
async fn book_id(app: &TestApp, title: &str, conceiving: bool) -> i32 {
    let (status, book) = app
        .post(
            "/api/books",
            json!({
                "title": title,
                "idea": "an idea",
                "genre": "xianxia",
                "writing_language": "zh-CN",
                "start_conception": conceiving,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create book: {book}");
    book["book"]["id"].as_i64().expect("book id") as i32
}

/// Waits until the session leaves `running`, then returns its status.
///
/// Read through `GET /api/sessions/:id` rather than SQL: the snapshot is the API the UI polls, and
/// a debug-build Demo step takes seconds, so the deadline is generous.
async fn wait_finished(app: &TestApp, session_id: i32) -> String {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let (_, snapshot) = app.get(&format!("/api/sessions/{session_id}")).await;
        let status = snapshot["session"]["status"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if !status.is_empty() && status != "running" {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "session {session_id} did not finish within 180s"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The kickoff message's concatenated text (the blocks of spec/07 §8 step 2).
async fn kickoff_text(app: &TestApp, session_id: i32) -> String {
    let (status, snapshot) = app.get(&format!("/api/sessions/{session_id}")).await;
    assert_eq!(status, StatusCode::OK, "snapshot: {snapshot}");
    let messages = snapshot["messages"].as_array().cloned().unwrap_or_default();
    let kickoff = messages
        .iter()
        .find(|entry| entry["message"]["kind"] == "kickoff")
        .unwrap_or_else(|| panic!("no kickoff message in {snapshot}"));
    kickoff["message"]["blocks"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|block| block["text"].as_str().map(str::to_owned))
        .collect::<Vec<String>>()
        .join("\n")
}

/// Creates one research task and returns its id.
async fn create_task(app: &TestApp, body: Value) -> i32 {
    let (status, task) = app.post("/api/research-tasks", body).await;
    assert_eq!(status, StatusCode::CREATED, "create task: {task}");
    task["id"].as_i64().expect("task id") as i32
}

/// Runs a task and returns its session id.
async fn run_task(app: &TestApp, task_id: i32) -> i32 {
    let (status, body) = app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "run: {body}");
    body["session_id"].as_i64().expect("session id") as i32
}

async fn task_dto(app: &TestApp, task_id: i32) -> Value {
    let (status, task) = app.get(&format!("/api/research-tasks/{task_id}")).await;
    assert_eq!(status, StatusCode::OK, "get task: {task}");
    task
}

// ───────────────────────────── CRUD ─────────────────────────────

#[tokio::test]
async fn a_task_without_a_book_needs_a_language() {
    let app = TestApp::new().await;
    let (status, body) = app
        .post(
            "/api/research-tasks",
            json!({ "scope_name": "器物考", "topic": "traditional sword care" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_error(&body, "validation.failed");

    let (status, body) = app
        .post(
            "/api/research-tasks",
            json!({
                "scope_name": "器物考",
                "topic": "traditional sword care",
                "language": "zh-CN",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["status"], "ready", "no book means no draft stage");
    assert_eq!(body["language"], "zh-CN");
    assert_eq!(body["entries_created"], 0);
}

#[tokio::test]
async fn a_task_of_a_conceiving_book_is_a_draft_and_ignores_the_language() {
    let app = TestApp::new().await;
    let book = book_id(&app, "Book", false).await;
    // M9 makes `start_conception` produce a `conceiving` book; until then the state is set
    // directly, which is what the rule of spec/10 §2.5 keys on (the status, not the flag).
    app.execute(&format!(
        "UPDATE books SET status = 'conceiving' WHERE id = {book}"
    ))
    .await;
    // A fresh book is `conceiving` (spec/02 §2).
    let (status, body) = app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": book,
                "scope_name": "Background",
                "topic": "gate rules",
                "language": "zh-CN",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["status"], "draft");
    assert_eq!(
        body["language"], "zh-CN",
        "the book's language wins over the request's (spec/10 §2.5)"
    );

    // `status: ready` confirms the draft (spec/10 §2.5).
    let id = body["id"].as_i64().expect("id") as i32;
    let (status, body) = app
        .patch(
            &format!("/api/research-tasks/{id}"),
            json!({ "status": "ready" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ready");

    // Any other status is refused, and a non-draft cannot be confirmed again.
    let (status, body) = app
        .patch(
            &format!("/api/research-tasks/{id}"),
            json!({ "status": "done" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_error(&body, "validation.failed");
    let (status, body) = app
        .patch(
            &format!("/api/research-tasks/{id}"),
            json!({ "status": "ready" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn tasks_list_filter_and_delete() {
    let app = TestApp::new().await;
    let book = book_id(&app, "Book", true).await;
    let (_, first) = app
        .post(
            "/api/research-tasks",
            json!({ "book_id": book, "scope_name": "A", "topic": "one" }),
        )
        .await;
    create_task(
        &app,
        json!({ "scope_name": "B", "topic": "two", "language": "en" }),
    )
    .await;

    let (_, all) = app.get("/api/research-tasks").await;
    assert_eq!(all.as_array().expect("list").len(), 2);
    let (_, mine) = app
        .get(&format!("/api/research-tasks?book_id={book}"))
        .await;
    let mine = mine.as_array().expect("list");
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0]["id"], first["id"]);

    let (status, _) = app
        .delete(&format!("/api/research-tasks/{}", first["id"]))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, after) = app.get("/api/research-tasks").await;
    assert_eq!(after.as_array().expect("list").len(), 1);
}

// ───────────────────────────── run and the session ─────────────────────────────

#[tokio::test]
async fn run_resolves_the_scope_attaches_it_to_the_book_and_drives_the_demo() {
    let app = demo_app().await;
    let book = book_id(&app, "Book", true).await;
    let id = create_task(
        &app,
        json!({ "book_id": book, "scope_name": "Background", "topic": "gate rules" }),
    )
    .await;

    let session_id = run_task(&app, id).await;
    let final_status = wait_finished(&app, session_id).await;
    assert_eq!(final_status, "completed", "the demo researcher finishes");

    // The task is done with its session and its entries counted (spec/07 §8 steps 1, 5).
    let task = task_dto(&app, id).await;
    assert_eq!(task["status"], "done", "{task}");
    assert_eq!(task["session_id"], session_id);
    assert_eq!(task["entries_created"], 2, "{task}");
    assert!(!task["report"].as_str().unwrap_or_default().is_empty());
    let scope_id = task["scope_id"].as_i64().expect("the scope was resolved") as i32;

    // The scope exists by that name (spec/07 §8 step 2).
    let (status, scope) = app.get(&format!("/api/knowledge/scopes/{scope_id}")).await;
    assert_eq!(status, StatusCode::OK, "{scope}");
    assert_eq!(scope["name"], "Background");

    // The two saved entries are in it, attributed to research (spec/07 §1).
    let (_, entries) = app
        .get(&format!("/api/knowledge/scopes/{scope_id}/entries"))
        .await;
    let items = entries["items"].as_array().cloned().unwrap_or_default();
    assert_eq!(items.len(), 2, "{entries}");
    assert!(
        items.iter().all(|entry| entry["source"] == "research"),
        "{entries}"
    );
}

#[tokio::test]
async fn a_second_run_while_a_session_is_open_is_research_invalid_state() {
    let app = demo_app().await;
    let id = create_task(
        &app,
        json!({ "scope_name": "S", "topic": "t", "language": "en" }),
    )
    .await;
    let session_id = run_task(&app, id).await;
    // The demo run can finish before the second request lands, so the open session is made
    // durable: a `running` task with a `running` session is exactly the state `run` refuses
    // (spec/07 §8 step 1). Both columns are what the guard reads, so setting them is faithful.
    app.set_session_status(session_id, "running").await;
    app.execute(&format!(
        "UPDATE research_tasks SET status = 'running', session_id = {session_id} WHERE id = {id}"
    ))
    .await;

    let (status, body) = app
        .post(&format!("/api/research-tasks/{id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "research.invalid_state");
    // The error names the open session (spec/10 §2.5).
    assert_eq!(body["error"]["details"]["session_id"], session_id);

    // PATCH and DELETE are blocked for the same reason (spec/10 §2.5).
    let (status, body) = app
        .patch(
            &format!("/api/research-tasks/{id}"),
            json!({ "topic": "x" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "research.invalid_state");
    let (status, body) = app.delete(&format!("/api/research-tasks/{id}")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // Once the session is done, the task can be run again.
    wait_finished(&app, session_id).await;
    let (status, _) = app
        .post(&format!("/api/research-tasks/{id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn abandoning_the_session_cancels_the_task() {
    let app = demo_app().await;
    let id = create_task(
        &app,
        json!({ "scope_name": "S", "topic": "t", "language": "en" }),
    )
    .await;
    let session_id = run_task(&app, id).await;

    // `POST /sessions/:id/abandon` runs the real transition, which is what couples the task
    // status (spec/07 §8 step 1).
    let (status, body) = app
        .post(&format!("/api/sessions/{session_id}/abandon"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "abandon: {body}");
    let task = task_dto(&app, id).await;
    assert_eq!(task["status"], "cancelled", "{task}");

    // A cancelled task can be run again (spec/07 §8 step 1).
    let (status, _) = app
        .post(&format!("/api/research-tasks/{id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED);
}

// ───────────────────────────── the kickoff ─────────────────────────────

#[tokio::test]
async fn the_kickoff_carries_the_scope_its_description_and_the_existing_titles() {
    let app = demo_app().await;
    let book = book_id(&app, "Book", true).await;
    let (status, scope) = app
        .post(
            "/api/knowledge/scopes",
            json!({ "name": "Background", "description": "the border town and its rules" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{scope}");
    let scope_id = scope["id"].as_i64().expect("id") as i32;
    for index in 0..3 {
        let (status, body) = app
            .post(
                &format!("/api/knowledge/scopes/{scope_id}/entries"),
                json!({ "title": format!("existing {index}"), "content": "content" }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    // `run` attaches the existing scope by name instead of creating a second one.
    let id = create_task(
        &app,
        json!({ "book_id": book, "scope_name": "Background", "topic": "gate rules" }),
    )
    .await;
    let session_id = run_task(&app, id).await;
    wait_finished(&app, session_id).await;

    let task = task_dto(&app, id).await;
    assert_eq!(
        task["scope_id"], scope_id,
        "the scope was reused, not duplicated"
    );

    // The kickoff block carries the scope name, its description and the existing titles
    // (spec/07 §8 step 2).
    let kickoff = kickoff_text(&app, session_id).await;
    assert!(kickoff.contains("Background"), "{kickoff}");
    assert!(
        kickoff.contains("the border town and its rules"),
        "{kickoff}"
    );
    for index in 0..3 {
        assert!(kickoff.contains(&format!("existing {index}")), "{kickoff}");
    }
    // The linked book's brief renders before the researcher block (spec/07 §8 step 2).
    assert!(kickoff.contains("Book"), "{kickoff}");
}

#[tokio::test]
async fn the_kickoff_titles_are_capped_at_two_hundred() {
    let app = demo_app().await;
    let (status, scope) = app
        .post("/api/knowledge/scopes", json!({ "name": "Wide" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{scope}");
    let scope_id = scope["id"].as_i64().expect("id") as i32;
    for index in 0..205 {
        let (status, body) = app
            .post(
                &format!("/api/knowledge/scopes/{scope_id}/entries"),
                json!({ "title": format!("title {index:03}"), "content": "c" }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    let id = create_task(
        &app,
        json!({ "scope_name": "Wide", "topic": "t", "language": "en" }),
    )
    .await;
    let session_id = run_task(&app, id).await;
    wait_finished(&app, session_id).await;

    let kickoff = kickoff_text(&app, session_id).await;
    // The first 200 titles fit; the last five do not (spec/07 §8 step 2: at most 200).
    assert!(kickoff.contains("title 000"), "{kickoff}");
    assert!(kickoff.contains("title 199"), "{kickoff}");
    assert!(!kickoff.contains("title 200"), "{kickoff}");
    assert!(!kickoff.contains("title 204"), "{kickoff}");
}

#[tokio::test]
async fn a_task_without_a_book_has_no_book_brief_block() {
    let app = demo_app().await;
    let id = create_task(
        &app,
        json!({ "scope_name": "Standalone", "topic": "t", "language": "en" }),
    )
    .await;
    let session_id = run_task(&app, id).await;
    wait_finished(&app, session_id).await;

    let kickoff = kickoff_text(&app, session_id).await;
    // The researcher block names the scope (spec/07 §8 step 2), but no book brief renders for a
    // task without a book: the block would be an empty shell (spec/07 §8 step 2, M8).
    assert!(kickoff.contains("Standalone"), "{kickoff}");
    assert!(
        !kickoff.contains("《Book》"),
        "no book exists, so no brief: {kickoff}"
    );
}

// ───────────────────────────── continuation across runs ─────────────────────────────

#[tokio::test]
async fn a_second_run_does_not_duplicate_the_entries() {
    // The demo provider's `saved_entries` hint makes the second run finish immediately, proving a
    // run continues existing material instead of duplicating it (spec/04 §7.2, spec/07 §8).
    let app = demo_app().await;
    let id = create_task(
        &app,
        json!({ "scope_name": "S", "topic": "t", "language": "en" }),
    )
    .await;
    let first = run_task(&app, id).await;
    wait_finished(&app, first).await;
    let task = task_dto(&app, id).await;
    assert_eq!(task["entries_created"], 2, "{task}");

    // The second run: the demo researcher sees the two saved entries and finishes immediately, so
    // the count stays 2 — no duplicates.
    let second = run_task(&app, id).await;
    wait_finished(&app, second).await;
    let task = task_dto(&app, id).await;
    assert_eq!(
        task["entries_created"], 2,
        "the second run added nothing: {task}"
    );
}

// ───────────────────────────── the DTO ─────────────────────────────

#[tokio::test]
async fn the_task_dto_carries_what_the_ui_needs() {
    let app = TestApp::new().await;
    let (_, task) = app
        .post(
            "/api/research-tasks",
            json!({ "scope_name": "S", "topic": "t", "language": "en" }),
        )
        .await;
    for key in [
        "id",
        "scope_name",
        "topic",
        "requirements",
        "language",
        "status",
        "report",
        "entries_created",
        "created_at",
        "updated_at",
    ] {
        assert!(task.get(key).is_some(), "the DTO must carry {key}: {task}");
    }
    // The ts-rs export keeps the frontend typed.
    let generated = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../web/src/api/gen/ResearchTaskDto.ts");
    assert!(generated.exists(), "ResearchTaskDto.ts must be generated");
}
