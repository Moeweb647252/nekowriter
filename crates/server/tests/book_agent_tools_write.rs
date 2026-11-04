//! Book-agent write tools (plan §5.4, M21 21.5).

use axum::http::StatusCode;
use sea_orm::{ConnectionTrait, EntityTrait, TransactionTrait};
use serde_json::{Value, json};

use nekowriter::agent::tools::{Prepared, ToolCtx, Visibility};
use nekowriter::db::entities::{agent_sessions, books, chapters, tracks};
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
async fn edit_chapter_text_uses_the_text_service_and_saves_one_backup_per_session() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let tracks = app.get(&format!("/api/books/{book_id}/tracks")).await.1;
    let track_id = tracks[0]["id"].as_i64().unwrap() as i32;
    let chapter = app.create_chapter(track_id, "Chapter").await;
    let chapter_id = chapter["id"].as_i64().unwrap() as i32;
    app.execute(&format!(
        "UPDATE chapters SET content = '{{\"paragraphs\":[{{\"pid\":\"p1\",\"text\":\"old text\"}}],\"next_pid\":2,\"tail_open\":false}}', content_version = 1 WHERE id = {chapter_id}"
    )).await;
    let session = book_session(&app, book_id).await;
    for replacement in ["first", "second"] {
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
        let old = if replacement == "first" {
            "old"
        } else {
            "first"
        };
        app.state()
            .tool_registry()
            .get("edit_chapter_text")
            .unwrap()
            .execute(
                &mut ctx,
                json!({"chapter":"1","edits":[{"find":old,"replace":replacement}]}),
                Prepared::None,
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    let content = app.get(&format!("/api/chapters/{chapter_id}")).await.1["content"]["paragraphs"]
        [0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(content, "second text");
    let count = app.state().db().conn().query_one_raw(sea_orm::Statement::from_string(
        sea_orm::DbBackend::Sqlite,
        format!("SELECT count(*) AS n FROM chapter_versions WHERE chapter_id = {chapter_id} AND kind = 'backup'"),
    )).await.unwrap().unwrap().try_get::<i64>("", "n").unwrap();
    assert_eq!(count, 1);
    let _ = chapters::Entity::find_by_id(chapter_id)
        .one(app.state().db().conn())
        .await;
}

#[tokio::test]
async fn delete_canon_record_writes_a_change_log_and_revert_recreates_the_entity() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let (status, entity) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({"kind":"character","name":"Delete me","summary":"temporary"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{entity}");
    let entity_id = entity["id"].as_i64().unwrap() as i32;
    let session = book_session(&app, book_id).await;
    let db = app.state().db();
    let tx = db.conn().begin().await.unwrap();
    let book_row = books::Entity::find_by_id(book_id).one(&tx).await.unwrap();
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
        track: None,
        visibility: Visibility::Shared,
        request_meta: RequestMeta {
            phase: Phase::None,
            granularity: None,
        },
        prompts: &prompts,
        state: app.state(),
        events: Vec::new(),
    };
    app.state()
        .tool_registry()
        .get("delete_canon_record")
        .unwrap()
        .execute(
            &mut ctx,
            json!({"target_type":"entity","id":entity_id,"note":"test delete"}),
            Prepared::None,
        )
        .await
        .unwrap();
    let change_id = tx.query_one_raw(sea_orm::Statement::from_string(
        sea_orm::DbBackend::Sqlite,
        format!("SELECT id FROM change_log WHERE target_type = 'entity' AND target_id = {entity_id} AND op = 'delete' ORDER BY id DESC LIMIT 1"),
    )).await.unwrap().unwrap().try_get::<i32>("", "id").unwrap();
    tx.commit().await.unwrap();
    let (status, _) = app
        .post(&format!("/api/changes/{change_id}/revert"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = app.get(&format!("/api/books/{book_id}/entities")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.as_array()
            .unwrap()
            .iter()
            .any(|row| row["name"] == "Delete me"),
        "{body}"
    );
}

#[tokio::test]
async fn create_and_edit_chapter_tools_use_the_chapter_services() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let tracks = app.get(&format!("/api/books/{book_id}/tracks")).await.1;
    let track_id = tracks[0]["id"].as_i64().unwrap() as i32;
    let _first = app.create_chapter(track_id, "First").await;
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
    app.state()
        .tool_registry()
        .get("create_chapter")
        .unwrap()
        .execute(
            &mut ctx,
            json!({"track":"main","title":"Second"}),
            Prepared::None,
        )
        .await
        .unwrap();
    app.state()
        .tool_registry()
        .get("edit_chapter")
        .unwrap()
        .execute(
            &mut ctx,
            json!({"chapter":"2","title":"Renamed"}),
            Prepared::None,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let chapters = app.get(&format!("/api/tracks/{track_id}/chapters")).await.1;
    assert_eq!(chapters.as_array().unwrap().len(), 2, "{chapters}");
    assert_eq!(chapters[1]["title"], "Renamed", "{chapters}");
}

#[tokio::test]
async fn goal_research_and_knowledge_tools_write_through_their_services() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let (scope_status, scope) = app
        .post(
            "/api/knowledge/scopes",
            json!({"name":"Book scope","description":"test"}),
        )
        .await;
    assert_eq!(scope_status, StatusCode::CREATED, "{scope}");
    let session = book_session(&app, book_id).await;
    let db = app.state().db();
    let tx = db.conn().begin().await.unwrap();
    let book_row = books::Entity::find_by_id(book_id).one(&tx).await.unwrap();
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
        track: None,
        visibility: Visibility::Shared,
        request_meta: RequestMeta {
            phase: Phase::None,
            granularity: None,
        },
        prompts: &prompts,
        state: app.state(),
        events: Vec::new(),
    };
    app.state()
        .tool_registry()
        .get("create_goal")
        .unwrap()
        .execute(
            &mut ctx,
            json!({"level":"goal","title":"Goal","purpose":"purpose","note":"created"}),
            Prepared::None,
        )
        .await
        .unwrap();
    app.state()
        .tool_registry()
        .get("create_research_task")
        .unwrap()
        .execute(
            &mut ctx,
            json!({"topic":"Topic","requirements":"Find it","scope":"Book scope"}),
            Prepared::None,
        )
        .await
        .unwrap();
    app.state()
        .tool_registry()
        .get("add_knowledge_entry")
        .unwrap()
        .execute(
            &mut ctx,
            json!({"scope":"Book scope","title":"Entry","content":"Content"}),
            Prepared::None,
        )
        .await
        .unwrap();
    let entry_id = tx
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DbBackend::Sqlite,
            "SELECT id FROM knowledge_entries WHERE title = 'Entry'".to_owned(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i32>("", "id")
        .unwrap();
    let preview = app
        .state()
        .tool_registry()
        .get("update_knowledge_entry")
        .unwrap()
        .preview(
            &mut ctx,
            json!({"entry_id":entry_id,"content":"Updated content"}),
        )
        .await
        .unwrap();
    assert_eq!(preview["kind"], "diff");
    assert_eq!(preview["before"], "Content");
    app.state()
        .tool_registry()
        .get("update_knowledge_entry")
        .unwrap()
        .execute(
            &mut ctx,
            json!({"entry_id":entry_id,"content":"Updated content"}),
            Prepared::None,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let goals = app.get(&format!("/api/books/{book_id}/goals")).await.1;
    assert!(
        goals
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["title"] == "Goal"),
        "{goals}"
    );
    let tasks = app
        .get(&format!("/api/research-tasks?book_id={book_id}"))
        .await
        .1;
    assert!(
        tasks
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["topic"] == "Topic"),
        "{tasks}"
    );
    let entries = app
        .get(&format!(
            "/api/knowledge/scopes/{}/entries",
            scope["id"].as_i64().unwrap()
        ))
        .await
        .1;
    let items = entries
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| entries.as_array())
        .unwrap();
    assert!(items.iter().any(|e| e["title"] == "Entry"), "{entries}");
}
