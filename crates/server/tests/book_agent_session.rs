//! The book-agent session lifecycle and approval hold (plan §5.2, D-169).

use axum::http::StatusCode;
use nekowriter::testing::TestApp;
use nekowriter_llm::mock::{ScriptStep, ScriptedProvider};
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

async fn setup_models(app: &TestApp, assign_assistant: bool) -> i64 {
    let (status, provider) = app
        .post(
            "/api/providers",
            json!({"name":"Demo","protocol":"mock","base_url":"mock://demo","extra_headers":{}}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let provider_id = provider["id"].as_i64().unwrap();
    let (status, profile) = app
        .post(
            "/api/model-profiles",
            json!({"provider_id": provider_id, "display_name":"Demo model", "model":"demo-model",
                   "context_window":200000, "max_output_tokens":8192, "options":{},
                   "effort_levels":[], "default_effort":null}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{profile}");
    let model = profile["id"].as_i64().unwrap();
    let mut roles = serde_json::Map::new();
    roles.insert("writer".to_owned(), json!(model));
    if assign_assistant {
        roles.insert("assistant".to_owned(), json!(model));
    }
    let (status, body) = app
        .patch("/api/app-settings", json!({"roles_defaults": roles}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    model
}

async fn start_agent(app: &TestApp, book_id: i32, body: Value) -> (StatusCode, Value) {
    app.post(&format!("/api/books/{book_id}/agent"), body).await
}

async fn wait_status(app: &TestApp, session_id: i64, wanted: &str) -> Value {
    for _ in 0..500 {
        let (status, body) = app.get(&format!("/api/sessions/{session_id}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if body["session"]["status"].as_str() == Some(wanted) {
            return body;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("session {session_id} never reached {wanted}");
}

async fn setting_content(app: &TestApp, book_id: i32, kind: &str) -> String {
    let (status, body) = app
        .get(&format!("/api/books/{book_id}/settings-materials/{kind}"))
        .await;
    if status == StatusCode::OK {
        return body["content"].as_str().unwrap_or_default().to_owned();
    }
    // The test can read the row directly; the API route above is not part of this fixture.
    let row = app
        .state()
        .db()
        .conn()
        .query_one_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT content FROM setting_docs WHERE book_id = ? AND kind = ?",
            [book_id.into(), kind.into()],
        ))
        .await
        .expect("query")
        .expect("doc");
    row.try_get::<String>("", "content").expect("content")
}

#[tokio::test]
async fn creates_a_book_agent_with_review_mode_and_rejects_a_second_one() {
    let mut app = TestApp::new().await;
    let _model = setup_models(&app, true).await;
    let _provider = app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new().text("Ready."),
    ]));
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;

    let (status, body) = start_agent(&app, book_id, json!({"message": "Read the outline."})).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().unwrap();
    let snapshot = wait_status(&app, session_id, "idle").await;
    assert_eq!(snapshot["session"]["kind"], "book_agent");
    assert_eq!(snapshot["session"]["role"], "assistant");
    assert_eq!(snapshot["session"]["write_mode"], "review");
    let kickoff = snapshot["messages"][0]["message"]["blocks"]
        .as_array()
        .expect("blocks")
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(kickoff.contains("Read the outline."), "{kickoff}");
    assert!(kickoff.contains("全书索引"), "{kickoff}");

    let (status, body) = start_agent(&app, book_id, json!({"message": "Again."})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "session.exists", "{body}");
    assert_eq!(body["error"]["details"]["session_id"], session_id);

    let (status, body) = start_agent(&app, book_id, json!({"message": "   "})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
}

#[tokio::test]
async fn the_assistant_falls_back_to_the_writer_model_and_mode_can_change_while_running() {
    let mut app = TestApp::new().await;
    let _model = setup_models(&app, false).await;
    let _provider = app.set_provider(ScriptedProvider::new(vec![ScriptStep::new().text("hi")]));
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let (status, body) = start_agent(&app, book_id, json!({"message": "Hello"})).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().unwrap();

    // The patch is allowed while the session is still running.
    let (status, body) = app
        .patch(
            &format!("/api/sessions/{session_id}"),
            json!({"write_mode": "full"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["write_mode"], "full");
    let snapshot = wait_status(&app, session_id, "idle").await;
    assert_eq!(snapshot["session"]["write_mode"], "full");
}

#[tokio::test]
async fn a_text_only_reply_goes_idle_and_end_completes_a_book_agent() {
    let mut app = TestApp::new().await;
    let _model = setup_models(&app, true).await;
    let _provider = app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new().text("Waiting."),
    ]));
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let (status, body) = start_agent(&app, book_id, json!({"message": "Hello"})).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().unwrap();
    let _ = wait_status(&app, session_id, "idle").await;

    let (status, body) = app
        .post(&format!("/api/sessions/{session_id}/end"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "completed");

    let (status, body) = app
        .post(&format!("/api/sessions/{session_id}/end"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn review_mode_holds_an_action_until_approval_and_request_changes_keeps_it_unapplied() {
    let mut app = TestApp::new().await;
    let _model = setup_models(&app, true).await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    app.execute(&format!(
        "UPDATE setting_docs SET content = 'OLD' WHERE book_id = {book_id} AND kind = 'world'"
    ))
    .await;
    let _provider = app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new().tool(
            "edit_setting_doc",
            json!({"doc":"world","edits":[{"find":"OLD","replace":"NEW"}]}),
        ),
        ScriptStep::new().text("Changed."),
    ]));
    let (status, body) =
        start_agent(&app, book_id, json!({"message": "Change the world doc."})).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().unwrap();
    let snapshot = wait_status(&app, session_id, "held").await;
    assert_eq!(setting_content(&app, book_id, "world").await, "OLD");
    let held = snapshot["tool_calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|call| call["status"] == "held")
        .expect("held call");
    assert_eq!(held["ui"]["preview"]["kind"], "diff");
    assert_eq!(held["ui"]["preview"]["before"], "OLD");

    let (status, body) = app
        .post(&format!("/api/sessions/{session_id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let _ = wait_status(&app, session_id, "idle").await;
    assert_eq!(setting_content(&app, book_id, "world").await, "NEW");

    // A second book exercises request-changes.
    let book2 = app.create_book("Book 2").await;
    let book2_id = book2["id"].as_i64().unwrap() as i32;
    app.execute(&format!(
        "UPDATE setting_docs SET content = 'OLD' WHERE book_id = {book2_id} AND kind = 'world'"
    ))
    .await;
    let _provider = app.set_provider(ScriptedProvider::new(vec![ScriptStep::new().tool(
        "edit_setting_doc",
        json!({"doc":"world","edits":[{"find":"OLD","replace":"NEW"}]}),
    )]));
    let (status, body) = start_agent(&app, book2_id, json!({"message": "Change it."})).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session2 = body["session_id"].as_i64().unwrap();
    let _ = wait_status(&app, session2, "held").await;
    let (status, body) = app
        .post(
            &format!("/api/sessions/{session2}/resume"),
            json!({"feedback":"Please keep OLD."}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let _ = wait_status(&app, session2, "idle").await;
    assert_eq!(setting_content(&app, book2_id, "world").await, "OLD");
    let snapshot = app.get(&format!("/api/sessions/{session2}")).await.1;
    let call = snapshot["tool_calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|call| call["name"] == "edit_setting_doc")
        .expect("declined call");
    assert_eq!(call["status"], "done");
    assert_eq!(call["is_error"], false);
    assert!(
        call["result_text"]
            .as_str()
            .unwrap_or_default()
            .contains("修改未应用"),
        "{call}"
    );
}

#[tokio::test]
async fn a_preview_error_is_an_error_and_is_never_held() {
    let mut app = TestApp::new().await;
    let _model = setup_models(&app, true).await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    app.execute(&format!(
        "UPDATE setting_docs SET content = 'OLD' WHERE book_id = {book_id} AND kind = 'world'"
    ))
    .await;
    let _provider = app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new().tool(
            "edit_setting_doc",
            json!({"doc":"world","edits":[{"find":"MISSING","replace":"NEW"}]}),
        ),
        ScriptStep::new().text("The edit failed."),
    ]));
    let (status, body) = start_agent(&app, book_id, json!({"message": "Try."})).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().unwrap();
    let snapshot = wait_status(&app, session_id, "idle").await;
    let call = snapshot["tool_calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|call| call["name"] == "edit_setting_doc")
        .expect("call");
    assert_eq!(call["status"], "error");
    assert_eq!(call["is_error"], true);
    assert_eq!(setting_content(&app, book_id, "world").await, "OLD");
}

#[tokio::test]
async fn full_mode_applies_an_action_without_holding() {
    let mut app = TestApp::new().await;
    let _model = setup_models(&app, true).await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    app.execute(&format!(
        "UPDATE setting_docs SET content = 'OLD' WHERE book_id = {book_id} AND kind = 'world'"
    ))
    .await;
    let _provider = app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new().tool(
            "edit_setting_doc",
            json!({"doc":"world","edits":[{"find":"OLD","replace":"NEW"}]}),
        ),
        ScriptStep::new().text("Changed."),
    ]));
    let (status, body) = start_agent(
        &app,
        book_id,
        json!({"message": "Change it.", "write_mode": "full"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().unwrap();
    let snapshot = wait_status(&app, session_id, "idle").await;
    assert_eq!(snapshot["session"]["write_mode"], "full");
    assert_eq!(setting_content(&app, book_id, "world").await, "NEW");
    let call = snapshot["tool_calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|call| call["name"] == "edit_setting_doc")
        .expect("call");
    assert_eq!(call["status"], "done");
}

#[tokio::test]
async fn a_reused_canon_action_holds_in_review_mode_and_writes_after_approval() {
    let mut app = TestApp::new().await;
    let _model = setup_models(&app, true).await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let _provider = app.set_provider(ScriptedProvider::new(vec![
        ScriptStep::new().tool(
            "create_entity",
            json!({"kind":"character","name":"Held canon","summary":"one"}),
        ),
        ScriptStep::new().text("Done."),
    ]));
    let (status, body) = start_agent(&app, book_id, json!({"message": "Create a person."})).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().unwrap();
    let snapshot = wait_status(&app, session_id, "held").await;
    let call = snapshot["tool_calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|call| call["name"] == "create_entity")
        .expect("held canon call");
    assert_eq!(call["status"], "held");
    let count = app
        .state()
        .db()
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DbBackend::Sqlite,
            format!("SELECT count(*) AS n FROM entities WHERE book_id = {book_id}"),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "n")
        .unwrap();
    assert_eq!(count, 0, "nothing is written while held");

    let (status, body) = app
        .post(&format!("/api/sessions/{session_id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let _ = wait_status(&app, session_id, "idle").await;
    let count = app
        .state()
        .db()
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DbBackend::Sqlite,
            format!("SELECT count(*) AS n FROM entities WHERE book_id = {book_id}"),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "n")
        .unwrap();
    assert_eq!(count, 1, "approval applies the canon write");
}

#[tokio::test]
async fn end_accepts_every_documented_status_and_rejects_running_and_other_kinds() {
    let app = TestApp::new().await;
    let _model = setup_models(&app, true).await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let snapshot = r#"{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}"#;
    for (id, status) in [
        (101, "idle"),
        (102, "held"),
        (103, "paused"),
        (104, "stopped"),
        (105, "failed"),
        (106, "interrupted"),
    ] {
        app.execute(&format!(
            "INSERT INTO agent_sessions(id, book_id, role, kind, model_snapshot, system_prompt, tools, status, write_mode, created_at, updated_at) \
             VALUES ({id}, {book_id}, 'assistant', 'book_agent', '{snapshot}', '', '[]', '{status}', 'review', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        )).await;
        let (code, body) = app
            .post(&format!("/api/sessions/{id}/end"), json!({}))
            .await;
        assert_eq!(code, StatusCode::OK, "{status}: {body}");
        assert_eq!(body["status"], "completed", "{status}");
    }
    app.execute(&format!(
        "INSERT INTO agent_sessions(id, book_id, role, kind, model_snapshot, system_prompt, tools, status, write_mode, created_at, updated_at) \
         VALUES (107, {book_id}, 'assistant', 'book_agent', '{snapshot}', '', '[]', 'running', 'review', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    )).await;
    let (code, body) = app.post("/api/sessions/107/end", json!({})).await;
    assert_eq!(code, StatusCode::CONFLICT, "{body}");
    app.execute(&format!(
        "INSERT INTO agent_sessions(id, book_id, role, kind, model_snapshot, system_prompt, tools, status, created_at, updated_at) \
         VALUES (108, {book_id}, 'writer', 'chapter_write', '{snapshot}', '', '[]', 'idle', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    )).await;
    let (code, body) = app.post("/api/sessions/108/end", json!({})).await;
    assert_eq!(code, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn archived_books_start_and_conceiving_books_are_refused() {
    let mut app = TestApp::new().await;
    let _model = setup_models(&app, true).await;
    let _provider = app.set_provider(ScriptedProvider::new(vec![ScriptStep::new().text("ok")]));
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    app.execute(&format!(
        "UPDATE books SET status = 'archived' WHERE id = {book_id}"
    ))
    .await;
    let (code, body) = start_agent(&app, book_id, json!({"message": "hi"})).await;
    assert_eq!(code, StatusCode::CREATED, "{body}");
    app.execute(&format!(
        "UPDATE books SET status = 'conceiving' WHERE id = {book_id}"
    ))
    .await;
    let (code, body) = start_agent(&app, book_id, json!({"message": "hi"})).await;
    assert_eq!(code, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "book.invalid_state");
}

#[tokio::test]
async fn message_boundary_and_missing_model_are_checked() {
    let mut app = TestApp::new().await;
    let _model = setup_models(&app, true).await;
    let _provider = app.set_provider(ScriptedProvider::new(vec![ScriptStep::new().text("ok")]));
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let (status, body) = start_agent(&app, book_id, json!({"message": "x".repeat(20_000)})).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, body) = start_agent(&app, book_id, json!({"message": "x".repeat(20_001)})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    let no_model = TestApp::new().await;
    let book = no_model.create_book("No model").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let (status, body) = start_agent(&no_model, book_id, json!({"message": "hi"})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "model.missing_role", "{body}");
}
