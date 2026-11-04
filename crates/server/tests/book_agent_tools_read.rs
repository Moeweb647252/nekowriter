//! Book-agent read tools and freeze rules (plan §5.3, D-169).

use sea_orm::{EntityTrait, TransactionTrait};
use serde_json::json;

use nekowriter::agent::tools::{
    ASSISTANT_ACTION_TOOLS, ASSISTANT_TOOLS, Prepared, ToolCtx, ToolRegistry, tools_for_session,
};
use nekowriter::db::entities::{agent_sessions, books, tracks};
use nekowriter::db::json::{CanonSystems, Phase, RequestMeta, SessionKind};
use nekowriter::testing::TestApp;

fn with_system(systems: &CanonSystems, off: &str) -> CanonSystems {
    let mut value = systems.clone();
    match off {
        "entities" => value.entities = false,
        "timeline" => value.timeline = false,
        "memory" => value.memory = false,
        "foreshadowing" => value.foreshadowing = false,
        "goals" => value.goals = false,
        _ => unreachable!(),
    }
    value
}

#[test]
fn the_book_agent_freezes_exactly_the_44_tool_list() {
    let registry = ToolRegistry::new();
    let tools = tools_for_session(
        "assistant",
        SessionKind::BookAgent,
        &CanonSystems::default(),
        false,
        false,
        &registry,
    );
    assert_eq!(tools, ASSISTANT_TOOLS);
    assert_eq!(tools.len(), 44);
    assert_eq!(ASSISTANT_ACTION_TOOLS.len(), 28);
}

#[test]
fn the_canon_switches_remove_exactly_the_documented_rows() {
    let registry = ToolRegistry::new();
    let all = CanonSystems::default();
    let rows = |systems: &CanonSystems| {
        tools_for_session(
            "assistant",
            SessionKind::BookAgent,
            systems,
            false,
            false,
            &registry,
        )
    };
    let cases: [(&str, &[&str]); 5] = [
        (
            "entities",
            &[
                "get_entity",
                "list_entities",
                "get_relations",
                "create_entity",
                "update_entity",
                "set_relation",
                "end_relation",
            ],
        ),
        (
            "timeline",
            &[
                "query_timeline",
                "add_timeline_event",
                "update_timeline_event",
            ],
        ),
        ("memory", &["add_memory_note", "retire_memory_note"]),
        (
            "foreshadowing",
            &[
                "list_foreshadowing",
                "add_foreshadowing",
                "update_foreshadowing",
                "resolve_foreshadowing",
            ],
        ),
        ("goals", &["list_goals", "create_goal", "update_goal"]),
    ];
    for (system, removed) in cases {
        let narrowed = rows(&with_system(&all, system));
        for tool in removed {
            assert!(!narrowed.contains(tool), "{system} must remove {tool}");
        }
        assert!(narrowed.contains(&"book_overview"));
    }
    let all_four_off = CanonSystems {
        entities: false,
        timeline: false,
        memory: false,
        foreshadowing: false,
        goals: true,
    };
    let narrowed = rows(&all_four_off);
    assert!(!narrowed.contains(&"search_canon"));
    assert!(!narrowed.contains(&"delete_canon_record"));
    assert!(narrowed.contains(&"book_overview"));
}

#[tokio::test]
async fn read_tools_return_the_book_index_chapter_and_setting_doc() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().unwrap() as i32;
    let (goal_status, goal_body) = app
        .post(
            &format!("/api/books/{book_id}/goals"),
            json!({"level":"goal","title":"Shared goal","purpose":"shared","status":"active"}),
        )
        .await;
    assert_eq!(goal_status, axum::http::StatusCode::CREATED, "{goal_body}");
    let (track_status, side_track) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({"kind":"side","title":"Private side","canon_mode":"isolated"}),
        )
        .await;
    assert_eq!(
        track_status,
        axum::http::StatusCode::CREATED,
        "{side_track}"
    );
    let side_track_id = side_track["id"].as_i64().unwrap();
    let (private_status, private_goal) = app.post(
        &format!("/api/books/{book_id}/goals"),
        json!({"level":"goal","title":"Private goal","purpose":"private","track_id":side_track_id}),
    ).await;
    assert_eq!(
        private_status,
        axum::http::StatusCode::CREATED,
        "{private_goal}"
    );
    let tracks = app.get(&format!("/api/books/{book_id}/tracks")).await.1;
    let track_id = tracks[0]["id"].as_i64().unwrap() as i32;
    let chapter = app.create_chapter(track_id, "A chapter").await;
    let chapter_id = chapter["id"].as_i64().unwrap() as i32;
    app.execute(&format!(
        "UPDATE chapters SET content = '{{\"paragraphs\":[{{\"pid\":\"p1\",\"text\":\"Hello\"}},{{\"pid\":\"p2\",\"text\":\"World\"}}],\"next_pid\":3,\"tail_open\":false}}', content_version = 3, word_count = 2, canon_status = 'none' WHERE id = {chapter_id}"
    )).await;
    app.execute(&format!(
        "UPDATE setting_docs SET content = 'abcdefghij' WHERE book_id = {book_id} AND kind = 'world'"
    )).await;
    for rank in 1..=31 {
        app.execute(&format!(
            "INSERT INTO character_designs(book_id, rank, name, role_label, summary, profile, created_at, updated_at) \
             VALUES ({book_id}, {rank}, 'Character {rank}', '角色', '', '', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        )).await;
    }
    let snapshot = r#"{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}"#;
    app.execute(&format!(
        "INSERT INTO agent_sessions(book_id, role, kind, status, write_mode, model_snapshot, system_prompt, tools, created_at, updated_at) \
         VALUES ({book_id}, 'assistant', 'book_agent', 'idle', 'review', '{snapshot}', '', '[]', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    )).await;
    let db = app.state().db();
    let tx = db.conn().begin().await.unwrap();
    let session = agent_sessions::Entity::find()
        .one(&tx)
        .await
        .unwrap()
        .unwrap();
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
        visibility: nekowriter::agent::tools::Visibility::Shared,
        request_meta: RequestMeta {
            phase: Phase::None,
            granularity: None,
        },
        prompts: &prompts,
        state: app.state(),
        events: Vec::new(),
    };
    let registry = app.state().tool_registry();
    let overview = registry
        .get("book_overview")
        .unwrap()
        .execute(&mut ctx, json!({}), Prepared::None)
        .await
        .unwrap();
    assert!(overview.text.contains("全书索引"), "{}", overview.text);
    assert!(
        overview.text.contains("另有 1 位未列出"),
        "{}",
        overview.text
    );
    let open = registry
        .get("open_chapter")
        .unwrap()
        .execute(
            &mut ctx,
            json!({"chapter":"1","part":"text","max_chars":8}),
            Prepared::None,
        )
        .await
        .unwrap();
    assert!(open.text.contains("p1"), "{}", open.text);
    assert!(open.text.contains("from_pid=\"p1\""), "{}", open.text);
    assert_eq!(open.ui["content_version"], 3);
    let doc = registry
        .get("read_setting_doc")
        .unwrap()
        .execute(&mut ctx, json!({"doc":"world","offset":2}), Prepared::None)
        .await
        .unwrap();
    assert!(doc.text.contains("cdefghij"), "{}", doc.text);
    let chars = registry
        .get("read_characters")
        .unwrap()
        .execute(&mut ctx, json!({}), Prepared::None)
        .await
        .unwrap();
    assert!(chars.text.contains("Character 1"), "{}", chars.text);
    let missing = registry
        .get("read_characters")
        .unwrap()
        .execute(&mut ctx, json!({"name":"Nobody"}), Prepared::None)
        .await;
    assert_eq!(
        missing.unwrap_err().kind,
        nekowriter::agent::tools::ToolErrorKind::CharacterNotFound
    );
    let goals = registry
        .get("list_goals")
        .unwrap()
        .execute(&mut ctx, json!({}), Prepared::None)
        .await
        .unwrap();
    assert!(goals.text.contains("Shared goal"), "{}", goals.text);
    assert!(!goals.text.contains("Private goal"), "{}", goals.text);
    let research = registry
        .get("list_research")
        .unwrap()
        .execute(&mut ctx, json!({}), Prepared::None)
        .await
        .unwrap();
    assert!(research.text.contains("没有资料"), "{}", research.text);
    let sessions = registry
        .get("list_sessions")
        .unwrap()
        .execute(&mut ctx, json!({}), Prepared::None)
        .await
        .unwrap();
    assert!(sessions.text.contains("book_agent"), "{}", sessions.text);
    for part in ["outline", "reference_notes", "guidance", "summary"] {
        let view = registry
            .get("open_chapter")
            .unwrap()
            .execute(&mut ctx, json!({"chapter":"1","part":part}), Prepared::None)
            .await
            .unwrap();
        assert!(view.text.contains("状态"), "{}", view.text);
    }
    tx.rollback().await.unwrap();
}
