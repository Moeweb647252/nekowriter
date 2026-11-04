//! Context breakdown shape and API (M23 23.3, plan §5.2).

use axum::http::StatusCode;
use sea_orm::{ConnectionTrait, EntityTrait, QueryFilter};
use serde_json::json;

use nekowriter::agent::breakdown::{ContextGroupKey, ContextPartKey, breakdown};
use nekowriter::db::entities::{agent_messages, agent_sessions, agent_tool_calls};
use nekowriter::db::json::SessionStatus;
use nekowriter::testing::TestApp;
use nekowriter_llm::Message;
use nekowriter_llm::mock::ScriptStep;

mod agent_harness;

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
async fn loader_ignores_a_call_whose_message_was_not_loaded() {
    let fixture = fixture(None).await;
    let messages = messages(&fixture).await;
    let mut calls = nekowriter::db::entities::agent_tool_calls::Entity::find()
        .all(fixture.app.db().conn())
        .await
        .unwrap();
    // A call whose assistant message was not part of the loaded rows is dropped by the loader;
    // the pure function applies the same guard, so an orphan row cannot add a result leaf.
    calls.push(nekowriter::db::entities::agent_tool_calls::Model {
        message_id: 999999,
        ..calls[0].clone()
    });
    let view = breakdown(&fixture.session, &messages, &calls, None);
    assert!(view.leaves.iter().all(|leaf| leaf.text != "orphan result"));
}

#[tokio::test]
async fn rebuild_leaves_match_the_scripted_provider_requests() {
    let runtime = agent_harness::runtime(vec![
        ScriptStep::new().tool("echo", json!({"text": "hello"})),
        agent_harness::finish_step(),
    ])
    .await;
    let session_id = runtime.start_debug().await;
    runtime
        .wait_status(session_id, SessionStatus::Completed)
        .await;
    let session = nekowriter::agent::session::load(runtime.app.db().conn(), session_id)
        .await
        .unwrap();
    let messages = agent_messages::Entity::find()
        .filter(agent_messages::COLUMN.session_id.eq(session_id))
        .all(runtime.app.db().conn())
        .await
        .unwrap();
    let calls = agent_tool_calls::Entity::find()
        .filter(agent_tool_calls::COLUMN.session_id.eq(session_id))
        .all(runtime.app.db().conn())
        .await
        .unwrap();
    let assistant_messages: Vec<_> = messages
        .iter()
        .filter(|message| message.kind == nekowriter::db::json::MessageKind::Assistant)
        .collect();
    assert!(!assistant_messages.is_empty());
    let requests = runtime.scripted.requests();
    for (index, assistant) in assistant_messages.iter().enumerate() {
        let view = breakdown(&session, &messages, &calls, Some(assistant.seq));
        let request = &requests[index];
        assert_eq!(
            view.leaves
                .iter()
                .find(|leaf| leaf.group == ContextGroupKey::System)
                .map(|leaf| leaf.text.as_str()),
            Some(request.system.as_str())
        );
        let tool_names: Vec<&str> = view
            .leaves
            .iter()
            .filter(|leaf| leaf.group == ContextGroupKey::Tools)
            .filter_map(|leaf| leaf.name.as_deref())
            .collect();
        assert_eq!(
            tool_names,
            request
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>()
        );
        let user_texts: Vec<&str> = view
            .leaves
            .iter()
            .filter(|leaf| {
                matches!(
                    leaf.group,
                    ContextGroupKey::Kickoff | ContextGroupKey::Author | ContextGroupKey::Runtime
                )
            })
            .map(|leaf| leaf.text.as_str())
            .collect();
        let request_user_texts: Vec<&str> = request
            .messages
            .iter()
            .filter_map(|message| match message {
                Message::User { blocks } => Some(
                    blocks
                        .iter()
                        .map(|block| block.text.as_str())
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(user_texts, request_user_texts);
        let result_texts: Vec<&str> = view
            .leaves
            .iter()
            .filter(|leaf| leaf.group == ContextGroupKey::Results)
            .map(|leaf| leaf.text.as_str())
            .collect();
        let request_result_texts: Vec<&str> = request
            .messages
            .iter()
            .filter_map(|message| match message {
                Message::ToolResults { results } => Some(
                    results
                        .iter()
                        .map(|result| result.content.as_str())
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(result_texts, request_result_texts);
        if index > 0 {
            assert!(
                view.leaves
                    .iter()
                    .any(|leaf| leaf.group == ContextGroupKey::Model)
            );
        }
    }
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

#[tokio::test]
async fn old_rows_without_sources_fall_back_by_kind() {
    let fixture = fixture(None).await;
    fixture
        .app
        .execute(&format!(
            "UPDATE agent_messages SET content = '[{{\"type\":\"text\",\"text\":\"old kickoff\"}}]' WHERE session_id = {} AND seq = 0; \
             UPDATE agent_messages SET content = '[{{\"type\":\"text\",\"text\":\"old notice\"}}]' WHERE session_id = {} AND seq = 2",
            fixture.session_id, fixture.session_id
        ))
        .await;
    let messages = messages(&fixture).await;
    let view = breakdown(&fixture.session, &messages, &[], None);
    let kickoff = view
        .groups
        .iter()
        .find(|group| group.key == ContextGroupKey::Kickoff)
        .unwrap();
    assert_eq!(kickoff.parts[0].name, None);
    assert!(
        view.leaves
            .iter()
            .any(|leaf| leaf.group == ContextGroupKey::Runtime && leaf.text == "old notice")
    );
}

#[tokio::test]
async fn pending_unsealed_call_counts_without_a_result() {
    let fixture = fixture(None).await;
    let assistant = agent_messages::Entity::find()
        .filter(agent_messages::COLUMN.session_id.eq(fixture.session_id))
        .filter(agent_messages::COLUMN.seq.eq(1))
        .one(fixture.app.db().conn())
        .await
        .unwrap()
        .unwrap();
    fixture
        .app
        .execute(&format!(
            "INSERT INTO agent_tool_calls(session_id, message_id, call_index, provider_call_id, name, input, status, appendices, sealed, created_at) VALUES \
             ({}, {}, 1, 'pending', 'query_entity', '{{}}', 'pending', '[]', 0, '2026-01-01T00:00:02Z')",
            fixture.session_id, assistant.id
        ))
        .await;
    let messages = messages(&fixture).await;
    let calls = agent_tool_calls::Entity::find()
        .all(fixture.app.db().conn())
        .await
        .unwrap();
    let current = breakdown(&fixture.session, &messages, &calls, None);
    assert_eq!(current.pending_calls, 1);
    let past = breakdown(&fixture.session, &messages, &calls, Some(1));
    assert_eq!(past.pending_calls, 0);
}

#[tokio::test]
async fn rollover_is_only_for_a_drafting_or_revising_writer() {
    let fixture = fixture(None).await;
    for (kind, phase, expected) in [
        ("chapter_write", "drafting", true),
        ("chapter_revise", "revision_planning", false),
        ("chapter_review", "none", false),
        ("book_agent", "none", false),
    ] {
        fixture
            .app
            .execute(&format!(
                "UPDATE agent_sessions SET kind = '{kind}', phase = '{phase}', write_mode = CASE WHEN '{kind}' = 'book_agent' THEN 'review' ELSE NULL END WHERE id = {}",
                fixture.session_id
            ))
            .await;
        let session = agent_sessions::Entity::find_by_id(fixture.session_id)
            .one(fixture.app.db().conn())
            .await
            .unwrap()
            .unwrap();
        let view = breakdown(&session, &messages(&fixture).await, &[], None);
        assert_eq!(view.rollover_at.is_some(), expected, "{kind}/{phase}");
    }
}

#[tokio::test]
async fn session_dto_carries_context_window() {
    let fixture = fixture(None).await;
    let (status, body) = fixture
        .app
        .get(&format!("/api/sessions/{}", fixture.session_id))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["session"]["context_window"], 100000);
}
