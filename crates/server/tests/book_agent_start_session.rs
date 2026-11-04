//! Book-agent `start_session` (plan §5.5, M21 21.6).

use axum::http::StatusCode;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, TransactionTrait};
use serde_json::{Value, json};

use nekowriter::agent::tools::{Prepared, ToolCtx, ToolErrorKind, Visibility};
use nekowriter::db::entities::{agent_sessions, books, tracks};
use nekowriter::db::json::{Phase, RequestMeta};
use nekowriter::testing::TestApp;

async fn book_session(app: &TestApp, book_id: i32) -> agent_sessions::Model {
    let snapshot = r#"{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}"#;
    app.execute(&format!(
        "INSERT INTO agent_sessions(book_id, role, kind, status, write_mode, model_snapshot, system_prompt, tools, created_at, updated_at) \
         VALUES ({book_id}, 'assistant', 'book_agent', 'idle', 'review', '{snapshot}', '', '[]', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    )).await;
    agent_sessions::Entity::find()
        .one(app.state().db().conn())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn start_session_rejects_bad_actions_and_maps_outline_refusals() {
    let app = TestApp::new().await;
    let (status, _) = app.patch("/api/app-settings", json!({"roles_defaults":{}})).await;
    assert_eq!(status, StatusCode::OK);
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let tracks = app.get(&format!("/api/books/{book_id}/tracks")).await.1;
    let track_id = tracks[0]["id"].as_i64().unwrap() as i32;
    let first = app.create_chapter(track_id, "First").await;
    let first_id = first["id"].as_i64().unwrap() as i32;
    let second = app.create_chapter(track_id, "Second").await;
    let second_id = second["id"].as_i64().unwrap() as i32;
    let session = book_session(&app, book_id).await;
    let before = agent_sessions::Entity::find()
        .count(app.state().db().conn())
        .await
        .unwrap();
    let db = app.state().db();
    let tx = db.conn().begin().await.unwrap();
    let book_row = books::Entity::find_by_id(book_id).one(&tx).await.unwrap();
    let track = tracks::Entity::find_by_id(track_id).one(&tx).await.unwrap();
    let prompts = app
        .state()
        .prompts()
        .for_book(db, Some(book_id))
        .await
        .unwrap();
    let mut ctx = ToolCtx {
        tx: &tx,
        session: &session,
        book: book_row.as_ref(),
        chapter: None,
        track: track.as_ref(),
        visibility: Visibility::Shared,
        request_meta: RequestMeta {
            phase: Phase::None,
            granularity: None,
        },
        prompts: &prompts,
        state: app.state(),
        events: Vec::new(),
    };
    let registry = app.state().tool_registry();
    let bad = registry
        .get("start_session")
        .unwrap()
        .execute(
            &mut ctx,
            json!({"action":"write","chapter":"1"}),
            Prepared::None,
        )
        .await
        .unwrap_err();
    assert_eq!(bad.kind, ToolErrorKind::Validation);
    let refused = registry
        .get("start_session")
        .unwrap()
        .execute(
            &mut ctx,
            json!({"action":"outline","chapter":"3"}),
            Prepared::None,
        )
        .await
        .unwrap_err();
    assert_eq!(refused.kind, ToolErrorKind::StartRefused);
    assert_eq!(
        refused.fields.get("reason").unwrap(),
        "chapter.previous_not_ready"
    );
    assert_eq!(
        agent_sessions::Entity::find().count(&tx).await.unwrap(),
        before
    );
    tx.rollback().await.unwrap();
    let _ = (first_id, second_id);
}

#[tokio::test]
async fn start_session_creates_an_outline_in_the_transaction_and_returns_a_spawn_id() {
    let app = TestApp::new().await;
    let (status, provider) = app
        .post(
            "/api/providers",
            json!({"name":"Demo","protocol":"mock","base_url":"mock://demo","extra_headers":{}}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let (status, profile) = app.post("/api/model-profiles", json!({"provider_id":provider["id"],"display_name":"Demo","model":"demo","context_window":200000,"max_output_tokens":8192,"options":{},"effort_levels":[],"default_effort":null})).await;
    assert_eq!(status, StatusCode::CREATED, "{profile}");
    let model = profile["id"].clone();
    let (status, _) = app
        .patch(
            "/api/app-settings",
            json!({"roles_defaults":{"planner":model,"writer":model}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let tracks = app.get(&format!("/api/books/{book_id}/tracks")).await.1;
    let track_id = tracks[0]["id"].as_i64().unwrap() as i32;
    let first = app.create_chapter(track_id, "First").await;
    let first_id = first["id"].as_i64().unwrap() as i32;
    let second = app.create_chapter(track_id, "Second").await;
    let second_id = second["id"].as_i64().unwrap() as i32;
    app.execute(&format!(
        "UPDATE chapters SET status = 'official', canon_status = 'synced' WHERE id = {first_id}"
    ))
    .await;
    let session = book_session(&app, book_id).await;
    let db = app.state().db();
    let tx = db.conn().begin().await.unwrap();
    let book_row = books::Entity::find_by_id(book_id).one(&tx).await.unwrap();
    let track = tracks::Entity::find_by_id(track_id).one(&tx).await.unwrap();
    let prompts = app
        .state()
        .prompts()
        .for_book(db, Some(book_id))
        .await
        .unwrap();
    let mut ctx = ToolCtx {
        tx: &tx,
        session: &session,
        book: book_row.as_ref(),
        chapter: None,
        track: track.as_ref(),
        visibility: Visibility::Shared,
        request_meta: RequestMeta {
            phase: Phase::None,
            granularity: None,
        },
        prompts: &prompts,
        state: app.state(),
        events: Vec::new(),
    };
    let out = app
        .state()
        .tool_registry()
        .get("start_session")
        .unwrap()
        .execute(
            &mut ctx,
            json!({"action":"outline","chapter":"2","guidance":"outline it"}),
            Prepared::None,
        )
        .await
        .unwrap();
    assert!(!out.is_error);
    assert_eq!(out.spawn.len(), 1);
    let planner = agent_sessions::Entity::find_by_id(out.spawn[0])
        .one(&tx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        planner.kind,
        nekowriter::db::json::SessionKind::ChapterOutline
    );
    assert_eq!(planner.chapter_id, Some(second_id));
    tx.commit().await.unwrap();
    _ = StatusCode::CREATED;
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

#[tokio::test]
async fn approval_spawns_a_started_session_only_after_the_commit() {
    let mut app = TestApp::new().await;
    let (status, provider) = app
        .post(
            "/api/providers",
            json!({"name":"Demo","protocol":"mock","base_url":"mock://demo","extra_headers":{}}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let (status, profile) = app.post("/api/model-profiles", json!({"provider_id":provider["id"],"display_name":"Demo","model":"demo","context_window":200000,"max_output_tokens":8192,"options":{},"effort_levels":[],"default_effort":null})).await;
    assert_eq!(status, StatusCode::CREATED, "{profile}");
    let model = profile["id"].clone();
    app.patch(
        "/api/app-settings",
        json!({"roles_defaults":{"assistant":model,"planner":model,"writer":model}}),
    )
    .await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let tracks = app.get(&format!("/api/books/{book_id}/tracks")).await.1;
    let track_id = tracks[0]["id"].as_i64().unwrap() as i32;
    let first = app.create_chapter(track_id, "First").await;
    let first_id = first["id"].as_i64().unwrap() as i32;
    app.create_chapter(track_id, "Second").await;
    app.execute(&format!(
        "UPDATE chapters SET status = 'official', canon_status = 'synced' WHERE id = {first_id}"
    ))
    .await;
    let _provider = app.set_provider(nekowriter_llm::mock::ScriptedProvider::new(vec![
        nekowriter_llm::mock::ScriptStep::new()
            .tool("start_session", json!({"action":"outline","chapter":"2"})),
        nekowriter_llm::mock::ScriptStep::new().text("Started."),
    ]));
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/agent"),
            json!({"message":"start an outline"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session_id = body["session_id"].as_i64().unwrap();
    let _ = wait_status(&app, session_id, "held").await;
    let before = agent_sessions::Entity::find()
        .filter(agent_sessions::COLUMN.kind.eq("chapter_outline"))
        .count(app.state().db().conn())
        .await
        .unwrap();
    assert_eq!(before, 0);
    let (status, body) = app
        .post(&format!("/api/sessions/{session_id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let _ = wait_status(&app, session_id, "idle").await;
    let after = agent_sessions::Entity::find()
        .filter(agent_sessions::COLUMN.kind.eq("chapter_outline"))
        .count(app.state().db().conn())
        .await
        .unwrap();
    assert_eq!(after, 1);
}

#[tokio::test]
async fn every_action_refusal_with_no_model_creates_nothing() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let tracks = app.get(&format!("/api/books/{book_id}/tracks")).await.1;
    let track_id = tracks[0]["id"].as_i64().unwrap() as i32;
    let a = app.create_chapter(track_id, "A").await;
    let a_id = a["id"].as_i64().unwrap() as i32;
    let b = app.create_chapter(track_id, "B").await;
    let b_id = b["id"].as_i64().unwrap() as i32;
    let c = app.create_chapter(track_id, "C").await;
    let c_id = c["id"].as_i64().unwrap() as i32;
    app.execute(&format!(
        "UPDATE chapters SET status='official', canon_status='pending' WHERE id={a_id}"
    ))
    .await;
    app.execute(&format!(
        "UPDATE chapters SET status='official', canon_status='synced' WHERE id={b_id}"
    ))
    .await;
    app.execute(&format!(
        "UPDATE chapters SET status='official', canon_status='stale' WHERE id={c_id}"
    ))
    .await;
    app.execute(&format!("INSERT INTO research_tasks(book_id,scope_name,topic,kind,origin,requirements,language,status,entries_created,created_at,updated_at) VALUES ({book_id},'Scope','Topic','topic','author','req','zh-CN','ready',0,'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')")).await;
    let session = book_session(&app, book_id).await;
    let db = app.state().db();
    let tx = db.conn().begin().await.unwrap();
    let book_row = books::Entity::find_by_id(book_id).one(&tx).await.unwrap();
    let track = tracks::Entity::find_by_id(track_id).one(&tx).await.unwrap();
    let prompts = app
        .state()
        .prompts()
        .for_book(db, Some(book_id))
        .await
        .unwrap();
    let mut ctx = ToolCtx {
        tx: &tx,
        session: &session,
        book: book_row.as_ref(),
        chapter: None,
        track: track.as_ref(),
        visibility: Visibility::Shared,
        request_meta: RequestMeta {
            phase: Phase::None,
            granularity: None,
        },
        prompts: &prompts,
        state: app.state(),
        events: Vec::new(),
    };
    let registry = app.state().tool_registry();
    let cases = [
        json!({"action":"outline","chapter":"3"}),
        json!({"action":"maintain","chapter":"1"}),
        json!({"action":"resync","chapter":"3"}),
        json!({"action":"research","task":1}),
        json!({"action":"knowledge_check","guidance":"check it"}),
    ];
    let sessions_before = agent_sessions::Entity::find().count(&tx).await.unwrap();
    let versions_before = nekowriter::db::entities::chapter_versions::Entity::find()
        .count(&tx)
        .await
        .unwrap();
    let tasks_before = nekowriter::db::entities::research_tasks::Entity::find()
        .count(&tx)
        .await
        .unwrap();
    for input in cases {
        let error = registry
            .get("start_session")
            .unwrap()
            .execute(&mut ctx, input, Prepared::None)
            .await
            .unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::StartRefused, "{error:?}");
        assert_eq!(error.fields.get("reason").unwrap(), "model.missing_role");
    }
    assert_eq!(
        agent_sessions::Entity::find().count(&tx).await.unwrap(),
        sessions_before
    );
    assert_eq!(
        nekowriter::db::entities::chapter_versions::Entity::find()
            .count(&tx)
            .await
            .unwrap(),
        versions_before
    );
    assert_eq!(
        nekowriter::db::entities::research_tasks::Entity::find()
            .count(&tx)
            .await
            .unwrap(),
        tasks_before
    );
    tx.rollback().await.unwrap();
}
