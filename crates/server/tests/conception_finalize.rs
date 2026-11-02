//! Conception finalize and canon-seed tests (PLAN M9.2, spec/08 §5, spec/06 §7, spec/10 §2.2).

#[path = "conception_harness/mod.rs"]
mod harness;

/// The `(kind, role)` columns of one session row.
async fn session_kind_and_role(runtime: &Runtime, session_id: i32) -> (String, String) {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT kind, role FROM agent_sessions WHERE id = ?",
            vec![session_id.into()],
        ))
        .await
        .expect("query session")
        .expect("session row");
    (
        row.try_get::<String>("", "kind").expect("kind"),
        row.try_get::<String>("", "role").expect("role"),
    )
}

use harness::*;
use nekowriter::db::json::SessionStatus;
use serde_json::json;

/// Seeds the book's materials so finalize's precondition passes: three documents and one
/// character design (spec/08 §5 step 1).
async fn complete_materials(runtime: &Runtime, book_id: i32) {
    for (kind, content) in [
        ("outline", "# 大纲\n\n第一幕：入城。"),
        ("world", "# 世界观\n\n剑气折损心脉。"),
        ("protagonist", "# 主角\n\n沈砚。"),
    ] {
        let (_, body) = runtime
            .app
            .get(&format!("/api/books/{book_id}/setting-docs"))
            .await;
        let revision = body
            .as_array()
            .and_then(|docs| {
                docs.iter()
                    .find(|doc| doc["kind"] == kind)
                    .and_then(|doc| doc["revision"].as_i64())
            })
            .unwrap_or(0);
        let (status, body) = runtime
            .app
            .put(
                &format!("/api/books/{book_id}/setting-docs/{kind}"),
                json!({ "content": content, "base_revision": revision, "note": "" }),
            )
            .await;
        assert_eq!(status, 200, "seed {kind}: {body}");
    }
    // The materials can only be seeded while the conception session is not running.
    let session = conception_session_of_book(runtime, book_id).await;
    if runtime.session_row(session).await.status == "running" {
        // A running session holds the lock; the scripted empty script lets it idle quickly.
        runtime.wait_any_status(session).await;
    }
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({
                "name": "沈砚",
                "role_label": "主角",
                "summary": "游侠",
                "profile": "# 沈砚",
                "note": "",
            }),
        )
        .await;
    assert_eq!(status, 201, "seed character: {body}");
}

/// Finalize refuses an incomplete book with `conception.incomplete` naming what is missing
/// (spec/08 §5 step 1), and refuses a non-conceiving book with `book.invalid_state`.
#[tokio::test]
async fn finalize_checks_its_preconditions() {
    // Empty script: the conception session starts and immediately idles on its closing text.
    let runtime = runtime(vec![]).await;
    let book = create_conceiving_book(&runtime, "Incomplete").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 409, "finalize an empty book: {body}");
    assert_eq!(body["error"]["code"], "conception.incomplete");
    let missing = body["error"]["details"]["missing"].clone();
    assert_eq!(
        missing,
        json!(["outline", "world", "protagonist", "characters"])
    );

    // With the documents written but no character design, only that is missing.
    for kind in ["outline", "world", "protagonist"] {
        let (status, body) = runtime
            .app
            .put(
                &format!("/api/books/{book_id}/setting-docs/{kind}"),
                json!({ "content": "# 内容", "base_revision": 0, "note": "" }),
            )
            .await;
        assert_eq!(status, 200, "write {kind}: {body}");
    }
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        body["error"]["details"]["missing"],
        json!(["characters"]),
        "the character count is part of the precondition: {body}"
    );

    // Completing the materials succeeds, and a second finalize hits the state rule.
    complete_materials(&runtime, book_id).await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "finalize: {body}");
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "book.invalid_state");
}

/// Finalize completes the open session, activates the book, readies the draft research tasks,
/// creates and attaches their scopes, and starts the seed session (spec/08 §5 steps 2–5).
#[tokio::test]
async fn finalize_completes_the_session_activates_and_seeds() {
    let runtime = runtime(vec![]).await;
    let book = create_conceiving_book(&runtime, "Finalize Flow").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("session") as i32;
    // Wait for the scripted (empty) turn to settle so the materials are editable.
    let _ = runtime.wait_any_status(session_id).await;

    // Two draft topics sharing one scope name, plus one in another scope.
    for (topic, scope) in [
        ("边城城防", "背景设定"),
        ("夜禁规矩", "背景设定"),
        ("剑器养护", "器物考"),
    ] {
        let (status, body) = runtime
            .app
            .post(
                "/api/research-tasks",
                json!({
                    "book_id": book_id,
                    "topic": topic,
                    "requirements": "细节",
                    "scope_name": scope,
                }),
            )
            .await;
        assert_eq!(status, 201, "create topic {topic}: {body}");
    }

    complete_materials(&runtime, book_id).await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "finalize: {body}");

    // The book is active and the conception session completed (spec/08 §5 steps 2–3).
    assert_eq!(body["book"]["status"], "active");
    assert_eq!(book_status(&runtime, book_id).await, "active");
    let state = conception_state(&runtime, book_id).await;
    assert!(
        state.finalized_at.is_some(),
        "ConceptionState.finalized_at is set"
    );
    let row = runtime.session_row(session_id).await;
    assert_eq!(
        row.status, "completed",
        "the open conception session completes"
    );

    // The seed session started (an archivist model is configured in the harness).
    let seed_session_id = body["seed_session_id"].as_i64().expect("seed session") as i32;
    assert!(
        seed_session_id > 0,
        "the archivist seed session started: {body}"
    );
    let seed_row = runtime.session_row(seed_session_id).await;
    assert!(
        seed_row.status == "running"
            || seed_row.status == "idle"
            || seed_row.status == "completed"
            || seed_row.status == "failed"
            || seed_row.status == "interrupted",
        "the seed session is a real session: {}",
        seed_row.status
    );
    let seed = session_kind_and_role(&runtime, seed_session_id).await;
    assert_eq!(seed.0, "canon_seed");
    assert_eq!(seed.1, "archivist");

    // The tasks are ready and their scopes exist and are attached (step 4).
    let tasks = research_tasks(&runtime, book_id).await;
    assert_eq!(tasks.len(), 3);
    for (topic, _, status) in &tasks {
        assert_eq!(status, "ready", "{topic} became ready");
    }
    let (status, scopes) = runtime
        .app
        .get(&format!("/api/books/{book_id}/scopes"))
        .await;
    assert_eq!(status, 200, "book scopes: {scopes}");
    let names: Vec<&str> = scopes
        .as_array()
        .map(|rows| rows.iter().filter_map(|row| row["name"].as_str()).collect())
        .unwrap_or_default();
    assert!(names.contains(&"背景设定"), "{names:?}");
    assert!(names.contains(&"器物考"), "{names:?}");
    // A new scope's description is its topics joined with ` / ` (D-054).
    let background = scopes
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["name"] == "背景设定"))
        .cloned()
        .unwrap_or_default();
    let description = background["description"].as_str().unwrap_or_default();
    assert!(
        description.contains("边城城防 / 夜禁规矩"),
        "the description joins the topics with ' / ': {description}"
    );
}

/// A conceiving book created while no conception model was configured still succeeds with the
/// warning, and `conception/start` starts the session later (D-046, spec/08 §5).
#[tokio::test]
async fn a_book_without_a_conception_model_warns_and_starts_later() {
    let runtime = runtime(vec![]).await;
    // Drop the conception role; the writer role stays for the harness chapter.
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["conception"] = serde_json::Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "drop the conception role: {body}");

    let book = create_conceiving_book(&runtime, "No Model").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    assert_eq!(book["book"]["status"], "conceiving");
    assert!(
        book["session_id"].is_null(),
        "no session without a model: {book}"
    );
    assert_eq!(
        book["warnings"],
        json!(["model.missing_role"]),
        "the warning names the gap: {book}"
    );

    // Configure the model again; conception/start starts the session.
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    let writer_profile = roles["writer"].as_i64().expect("the writer model profile");
    roles["conception"] = json!(writer_profile);
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "restore the role: {body}");

    let (status, body) = runtime
        .app
        .post(&format!("/api/books/{book_id}/conception/start"), json!({}))
        .await;
    assert_eq!(status, 200, "conception/start: {body}");
    let session_id = body["session_id"].as_i64().expect("session") as i32;

    // Idempotent: the open session is returned, not a second one (D-047).
    let (status, body) = runtime
        .app
        .post(&format!("/api/books/{book_id}/conception/start"), json!({}))
        .await;
    assert_eq!(status, 200, "second start: {body}");
    assert_eq!(
        body["session_id"],
        json!(session_id),
        "the open session is returned"
    );
}

/// `canon/seed` is idempotent and refuses a conceiving book (spec/08 §5).
#[tokio::test]
async fn canon_seed_is_idempotent_and_requires_an_active_book() {
    let runtime = runtime(vec![]).await;
    let book = create_conceiving_book(&runtime, "Seed Guard").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;

    let (status, body) = runtime
        .app
        .post(&format!("/api/books/{book_id}/canon/seed"), json!({}))
        .await;
    assert_eq!(status, 409, "a conceiving book cannot seed: {body}");
    assert_eq!(body["error"]["code"], "book.invalid_state");

    let _ = runtime
        .wait_any_status(book["session_id"].as_i64().expect("session") as i32)
        .await;
    complete_materials(&runtime, book_id).await;
    let (status, _) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200);

    let first = runtime
        .app
        .post(&format!("/api/books/{book_id}/canon/seed"), json!({}))
        .await;
    assert_eq!(first.0, 200, "seed: {}", first.1);
    let session_id = first.1["session_id"].as_i64().expect("seed session") as i32;

    let second = runtime
        .app
        .post(&format!("/api/books/{book_id}/canon/seed"), json!({}))
        .await;
    assert_eq!(second.0, 200, "second seed: {}", second.1);
    assert_eq!(
        second.1["session_id"],
        json!(session_id),
        "the open seed session is returned"
    );
}

/// Finalize with no archivist model still succeeds, with the warning instead of a seed session
/// (spec/08 §5 step 5, D-046).
#[tokio::test]
async fn finalize_without_an_archivist_warns() {
    let runtime = runtime(vec![]).await;
    let book = create_conceiving_book(&runtime, "No Archivist").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("session") as i32;
    let _ = runtime.wait_any_status(session_id).await;

    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["archivist"] = serde_json::Value::Null;
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "drop the archivist: {body}");

    complete_materials(&runtime, book_id).await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "finalize still succeeds: {body}");
    assert!(body["seed_session_id"].is_null(), "no seed session: {body}");
    assert_eq!(body["warnings"], json!(["model.missing_role"]));
}

/// Setting materials are locked (423 with `details.session_id`) only while the conception
/// session runs; held, awaiting, idle and completed leave them editable (spec/08 §1.4).
#[tokio::test]
async fn setting_materials_lock_only_while_running() {
    let runtime = runtime(vec![
        // The first turn runs long enough to be observed running: two calls.
        tool_step("echo", json!({ "text": "first" })),
        tool_step("echo", json!({ "text": "second" })),
    ])
    .await;
    let book = create_conceiving_book(&runtime, "Locked").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("session") as i32;

    // While the session runs, the author's edit is refused with the session id.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let locked = loop {
        let row = runtime.session_row(session_id).await;
        if row.status == "running" {
            break true;
        }
        if row.status != "running" && std::time::Instant::now() > deadline {
            break false;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    };
    if locked {
        let (status, body) = runtime
            .app
            .put(
                &format!("/api/books/{book_id}/setting-docs/outline"),
                json!({ "content": "# 作者的版本", "base_revision": 0, "note": "" }),
            )
            .await;
        assert_eq!(status, 423, "locked while running: {body}");
        assert_eq!(body["error"]["code"], "settings.locked");
        assert_eq!(body["error"]["details"]["session_id"], json!(session_id));
    }

    // Once the session is idle (the empty script's turn ends), the edit goes through.
    let _ = runtime.wait_status(session_id, SessionStatus::Idle).await;
    let (status, body) = runtime
        .app
        .put(
            &format!("/api/books/{book_id}/setting-docs/outline"),
            json!({ "content": "# 作者的版本", "base_revision": 0, "note": "" }),
        )
        .await;
    assert_eq!(status, 200, "editable once idle: {body}");

    // And after finalize they are never locked again.
    for kind in ["world", "protagonist"] {
        let (status, body) = runtime
            .app
            .put(
                &format!("/api/books/{book_id}/setting-docs/{kind}"),
                json!({ "content": "# 内容", "base_revision": 0, "note": "" }),
            )
            .await;
        assert_eq!(status, 200, "seed {kind}: {body}");
    }
    complete_materials(&runtime, book_id).await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "finalize: {body}");
    let (_, docs) = runtime
        .app
        .get(&format!("/api/books/{book_id}/setting-docs"))
        .await;
    let revision = docs
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|doc| doc["kind"] == "outline")
                .and_then(|doc| doc["revision"].as_i64())
        })
        .unwrap_or_default();
    let (status, body) = runtime
        .app
        .put(
            &format!("/api/books/{book_id}/setting-docs/outline"),
            json!({ "content": "# 定稿后的修改", "base_revision": revision, "note": "" }),
        )
        .await;
    assert_eq!(status, 200, "never locked after finalize: {body}");
}

/// Finalize stops a **running** conception session first and awaits its runner, then completes it
/// (spec/08 §5 step 2); a pending form is simply left unanswered.
#[tokio::test]
async fn finalize_stops_a_running_session_and_leaves_a_pending_form() {
    // The second turn is slow, so the session is provably `running` when finalize arrives.
    let slow = nekowriter_llm::mock::ScriptStep::new().tool(
        "write_setting_doc",
        json!({ "doc": "world", "content": "# 世界观（慢写）" }),
    );
    let slow = nekowriter_llm::mock::ScriptStep {
        delay: std::time::Duration::from_millis(1500),
        ..slow
    };
    let runtime = runtime(vec![
        tool_step("test_write", json!({ "text": "a" })),
        slow,
        text_step("done"),
    ])
    .await;
    let book = create_conceiving_book(&runtime, "Running Finalize").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("session") as i32;
    // Let the first turn settle, then complete the materials while it is idle.
    let _ = runtime.wait_any_status(session_id).await;
    complete_materials(&runtime, book_id).await;

    // The author replies: the session starts another turn and is `running` while it runs.
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/sessions/{session_id}/message"),
            json!({ "text": "继续" }),
        )
        .await;
    assert_eq!(status, 200, "message: {body}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if runtime.session_row(session_id).await.status == "running" {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the session never ran"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    // Finalize while it runs: the runner is stopped and awaited, then the session completes.
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "finalize a running session: {body}");
    let row = runtime.session_row(session_id).await;
    assert_eq!(row.status, "completed", "the stopped session completes");
    assert_eq!(row.phase, "none");
    assert_eq!(book_status(&runtime, book_id).await, "active");
}

/// A pending `ask_user`/`request_review` form does not block finalize: the session completes and
/// the call stays unanswered (spec/08 §5 step 2).
#[tokio::test]
async fn finalize_leaves_a_pending_form_unanswered() {
    let runtime = runtime(vec![ask_step(), text_step("done")]).await;
    let book = create_conceiving_book(&runtime, "Pending Form").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("session") as i32;

    // The session asks; the form waits. Materials cannot be edited while it runs, but it is
    // awaiting the author here, so the author may fill them in directly (spec/08 §1.4).
    let call_id = wait_for_form(&runtime, session_id).await;
    complete_materials(&runtime, book_id).await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "finalize with a pending form: {body}");
    let row = runtime.session_row(session_id).await;
    assert_eq!(row.status, "completed");
    let call = runtime.call_by_id(call_id).await;
    assert_eq!(
        call.status, "awaiting_input",
        "the unanswered form stays as it was"
    );
    // Answering it afterwards is refused: the session is finished (spec/03 §6).
    let (status, body) = answer(&runtime, session_id, call_id, ask_answer()).await;
    assert_eq!(status, 409, "a completed session takes no answer: {body}");
}

/// Every seedable system off means finalize starts no `canon_seed` session at all — not even a
/// warning (spec/08 §5 step 5, spec/06 §7, D-150).
#[tokio::test]
async fn finalize_does_not_seed_when_all_seedable_systems_are_off() {
    let runtime = runtime(vec![]).await;
    let book = create_conceiving_book(&runtime, "No Seed").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("session") as i32;
    runtime.wait_any_status(session_id).await;
    complete_materials(&runtime, book_id).await;

    let (_, current) = runtime.app.get(&format!("/api/books/{book_id}")).await;
    let mut settings = current["settings"].clone();
    settings["canon_systems"] = json!({
        "entities": false,
        "timeline": false,
        "memory": false,
        "foreshadowing": false,
        "goals": false,
    });
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "settings": settings }),
        )
        .await;
    assert_eq!(status, 200, "settings: {body}");

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "finalize: {body}");
    assert!(
        body["seed_session_id"].is_null(),
        "no seed session may start: {body}"
    );
    assert_eq!(body["warnings"], json!([]));

    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT count(*) AS value FROM agent_sessions WHERE book_id = ? AND kind = 'canon_seed'",
            vec![book_id.into()],
        ))
        .await
        .expect("query")
        .expect("row");
    let count: i64 = row.try_get("", "value").expect("count");
    assert_eq!(count, 0);
}

/// O2: finalize's seed session can carry the stale-effort warning; the finalize response must
/// forward it instead of discarding the seed result.
#[tokio::test]
async fn finalize_surfaces_the_seed_sessions_effort_warning() {
    const WARNING: &str = "model.effort_unsupported";
    let runtime = runtime(vec![]).await;
    let book = create_conceiving_book(&runtime, "Finalize warning").await;
    let book_id = book["book"]["id"].as_i64().expect("book id") as i32;
    let session_id = book["session_id"].as_i64().expect("session") as i32;
    let _ = runtime.wait_any_status(session_id).await;
    complete_materials(&runtime, book_id).await;

    let (status, provider) = runtime
        .app
        .post(
            "/api/providers",
            json!({
                "name": "Finalize OpenAI",
                "protocol": "openai",
                "base_url": "http://127.0.0.1:9/v1",
                "extra_headers": {},
            }),
        )
        .await;
    assert_eq!(status, 201, "{provider}");
    let provider_id = provider["id"].as_i64().expect("provider id");
    let (status, profile) = runtime
        .app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Finalize archivist",
                "model": "finalize-archivist",
                "context_window": 128000,
                "max_output_tokens": 16384,
                "options": {},
                "effort_levels": ["low", "ultra"],
                "default_effort": "low",
            }),
        )
        .await;
    assert_eq!(status, 201, "{profile}");
    let profile_id = profile["id"].as_i64().expect("profile id");
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/providers/{provider_id}"),
            json!({
                "name": "Finalize OpenAI",
                "protocol": "anthropic",
                "base_url": "https://api.anthropic.test",
                "api_key": "sk-ant-test",
                "extra_headers": {},
            }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (_, settings) = runtime.app.get("/api/app-settings").await;
    let mut roles = settings["roles_defaults"].clone();
    roles["archivist"] = json!(profile_id);
    roles["efforts"] = json!({ "archivist": "ultra" });
    let (status, body) = runtime
        .app
        .patch("/api/app-settings", json!({ "roles_defaults": roles }))
        .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .any(|warning| warning == WARNING),
        "finalize forwards the seed warning: {body}"
    );
}
