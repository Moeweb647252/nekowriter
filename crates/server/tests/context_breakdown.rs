//! Context breakdown shape and API (M23 23.3, plan §5.2).

use axum::http::StatusCode;
use sea_orm::{ConnectionTrait, EntityTrait, QueryFilter};
use serde_json::json;

use nekowriter::agent::breakdown::{ContextGroupKey, ContextPartKey, breakdown};
use nekowriter::db::entities::{agent_messages, agent_sessions};
use nekowriter::testing::TestApp;

struct Fixture {
    app: TestApp,
    session_id: i32,
    session: agent_sessions::Model,
}

async fn fixture(usage: Option<serde_json::Value>) -> Fixture {
    let app = TestApp::new().await;
    let book = app.create_book("Context Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    app.execute(&format!(
        "INSERT INTO agent_sessions(book_id, role, kind, status, phase, model_snapshot, system_prompt, tools, created_at, updated_at) VALUES \
         ({book_id}, 'writer', 'chapter_write', 'idle', 'drafting', '{{\"provider_id\":0,\"protocol\":\"mock\",\"base_url\":\"\",\"extra_headers\":{{}},\"model\":\"demo\",\"context_window\":100000,\"max_output_tokens\":8192,\"options\":null}}', 'SYSTEM PROMPT', '[{{\"name\":\"read_chapter\",\"description\":\"Read a chapter\",\"input_schema\":{{\"type\":\"object\"}}}}]', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    let session_id = app
        .db()
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT max(id) AS id FROM agent_sessions",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i32>("", "id")
        .unwrap();
    let session = agent_sessions::Entity::find_by_id(session_id)
        .one(app.db().conn())
        .await
        .unwrap()
        .unwrap();
    let usage_sql = usage
        .as_ref()
        .map(|value| format!("'{}'", value))
        .unwrap_or_else(|| "NULL".to_owned());
    let content0 = json!([
        {"type": "text", "text": "kickoff text", "source": "block.settings"},
        {"type": "text", "text": "task text", "source": "writer.kickoff.chapter"}
    ])
    .to_string();
    let content1 = json!([
        {"type": "text", "text": "answer text"},
        {"type": "tool_call", "call_id": "call-1", "name": "read_chapter", "input_text": "{}"}
    ])
    .to_string();
    let content2 = json!([
        {"type": "text", "text": "runtime notice", "source": "inbox.notice"}
    ])
    .to_string();
    app.execute(&format!(
        "INSERT INTO agent_messages(session_id, seq, role, kind, content, usage, created_at) VALUES \
         ({session_id}, 0, 'user', 'kickoff', '{content0}', NULL, '2026-01-01T00:00:01Z'), \
         ({session_id}, 1, 'assistant', 'assistant', '{content1}', {usage_sql}, '2026-01-01T00:00:02Z'), \
         ({session_id}, 2, 'user', 'notice', '{content2}', NULL, '2026-01-01T00:00:03Z')"
    ))
    .await;
    let assistant_id = app
        .db()
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            format!("SELECT id FROM agent_messages WHERE session_id = {session_id} AND seq = 1"),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i32>("", "id")
        .unwrap();
    let appendices = json!([{"kind": "edit_report", "text": "edit report"}]).to_string();
    app.execute(&format!(
        "INSERT INTO agent_tool_calls(session_id, message_id, call_index, provider_call_id, name, input, status, raw_result_text, appendices, result_text, is_error, sealed, created_at) VALUES \
         ({session_id}, {assistant_id}, 0, 'call-1', 'read_chapter', '{{}}', 'done', 'raw result', '{appendices}', 'raw result\n\nedit report', 0, 1, '2026-01-01T00:00:02Z')"
    ))
    .await;
    Fixture {
        app,
        session_id,
        session,
    }
}

async fn messages(fixture: &Fixture) -> Vec<agent_messages::Model> {
    agent_messages::Entity::find()
        .filter(agent_messages::COLUMN.session_id.eq(fixture.session_id))
        .all(fixture.app.db().conn())
        .await
        .unwrap()
}

#[tokio::test]
async fn shape_classifies_system_tools_kickoff_author_runtime_model_and_results() {
    let fixture = fixture(Some(json!({"requests":1,"input_tokens":100,"cached_input_tokens":30,"cache_write_tokens":0,"output_tokens":5,"reasoning_tokens":0,"last_prompt_tokens":100,"estimated":false}))).await;
    let messages = messages(&fixture).await;
    let calls = nekowriter::db::entities::agent_tool_calls::Entity::find()
        .all(fixture.app.db().conn())
        .await
        .unwrap();
    let view = breakdown(&fixture.session, &messages, &calls, None);
    let keys: Vec<ContextGroupKey> = view.groups.iter().map(|group| group.key).collect();
    assert_eq!(
        keys,
        vec![
            ContextGroupKey::System,
            ContextGroupKey::Tools,
            ContextGroupKey::Kickoff,
            ContextGroupKey::Author,
            ContextGroupKey::Runtime,
            ContextGroupKey::Model,
            ContextGroupKey::Results
        ]
    );
    assert_eq!(
        view.estimate,
        view.groups.iter().map(|group| group.estimate).sum::<i64>()
    );
    let kickoff = view
        .groups
        .iter()
        .find(|group| group.key == ContextGroupKey::Kickoff)
        .unwrap();
    assert_eq!(
        kickoff
            .parts
            .iter()
            .map(|part| part.name.clone())
            .collect::<Vec<_>>(),
        vec![
            Some("block.settings".into()),
            Some("writer.kickoff.chapter".into())
        ]
    );
    let author = view
        .groups
        .iter()
        .find(|group| group.key == ContextGroupKey::Author)
        .unwrap();
    assert!(
        author
            .parts
            .iter()
            .any(|part| part.key == ContextPartKey::EditReport)
    );
    let results = view
        .groups
        .iter()
        .find(|group| group.key == ContextGroupKey::Results)
        .unwrap();
    assert!(
        results
            .parts
            .iter()
            .any(|part| part.key == ContextPartKey::Result
                && part.name.as_deref() == Some("read_chapter"))
    );
    assert_eq!(view.rollover_at, Some(76808));
}

#[tokio::test]
async fn past_seq_reports_measured_scale_and_ignores_later_messages() {
    let fixture = fixture(Some(json!({"requests":1,"input_tokens":100,"cached_input_tokens":30,"cache_write_tokens":0,"output_tokens":5,"reasoning_tokens":0,"last_prompt_tokens":100,"estimated":false}))).await;
    let messages = messages(&fixture).await;
    let calls = nekowriter::db::entities::agent_tool_calls::Entity::find()
        .all(fixture.app.db().conn())
        .await
        .unwrap();
    let view = breakdown(&fixture.session, &messages, &calls, Some(1));
    assert_eq!(view.measured, Some(100));
    assert_eq!(view.cached, Some(30));
    assert!(view.calibrated);
    assert!((view.scale - 100.0 / view.estimate as f64).abs() < 1e-9);
    assert_eq!(view.requests.len(), 1);
    assert_eq!(view.requests[0].seq, 1);
    assert_eq!(view.requests[0].estimate, view.estimate);
}

#[tokio::test]
async fn context_api_returns_the_dto_and_rejects_bad_seq_and_unknown_session() {
    let fixture = fixture(None).await;
    let (status, body) = fixture
        .app
        .get(&format!("/api/sessions/{}/context", fixture.session_id))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["seq"], json!(null));
    assert_eq!(body["context_window"], 100000);
    let (status, body) = fixture
        .app
        .get(&format!(
            "/api/sessions/{}/context?seq=0",
            fixture.session_id
        ))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["error"]["details"]["fields"]["seq"],
        "must be an assistant message of this session"
    );
    let (status, _) = fixture.app.get("/api/sessions/999999/context").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn result_items_are_capped_at_forty_with_the_rest_summed() {
    let fixture = fixture(None).await;
    let assistant = agent_messages::Entity::find()
        .filter(agent_messages::COLUMN.session_id.eq(fixture.session_id))
        .filter(agent_messages::COLUMN.seq.eq(1))
        .one(fixture.app.db().conn())
        .await
        .unwrap()
        .unwrap();
    for index in 1..45 {
        fixture
            .app
            .execute(&format!(
                "INSERT INTO agent_tool_calls(session_id, message_id, call_index, provider_call_id, name, input, status, raw_result_text, appendices, result_text, is_error, sealed, created_at) VALUES \
                 ({}, {}, {}, 'call-{index}', 'read_chapter', '{{}}', 'done', 'x', '[]', 'x', 0, 1, '2026-01-01T00:00:02Z')",
                fixture.session_id, assistant.id, index
            ))
            .await;
    }
    let messages = messages(&fixture).await;
    let calls = nekowriter::db::entities::agent_tool_calls::Entity::find()
        .all(fixture.app.db().conn())
        .await
        .unwrap();
    let view = breakdown(&fixture.session, &messages, &calls, None);
    let results = view
        .groups
        .iter()
        .find(|group| group.key == ContextGroupKey::Results)
        .unwrap();
    let part = results
        .parts
        .iter()
        .find(|part| part.name.as_deref() == Some("read_chapter"))
        .unwrap();
    assert_eq!(part.items.len(), 40);
    assert_eq!(part.rest.as_ref().map(|rest| rest.count), Some(5));
    assert_eq!(
        part.rest.as_ref().map(|rest| rest.estimate),
        Some(part.estimate - part.items.iter().map(|item| item.estimate).sum::<i64>())
    );
}
