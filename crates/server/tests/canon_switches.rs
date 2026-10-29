//! Canon-system switches in the runtime (PLAN M13 task 13.4, spec/03 §1.2, §4, §9–§10, §15
//! tests 27–29).
//!
//! Every decision below reads `agent_sessions.canon_systems`, copied from the book at creation:
//! the tool list, the kickoff blocks and memory fragments, `search_canon`'s kinds and
//! `finish_maintenance`'s required steps. The fixture gives every system unique data so a
//! disabled one can be asserted absent by name.

mod agent_harness;

use std::time::Duration;

use agent_harness::Runtime;
use nekowriter::db::json::{CanonSystems, Granularity, SessionKind, SessionStatus};
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter_llm::mock::ScriptStep;
use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement};
use serde_json::json;

fn idle_provider() -> Vec<ScriptStep> {
    vec![ScriptStep::new().delay(Duration::from_secs(60))]
}

struct Fixture {
    runtime: Runtime,
    target_id: i32,
}

/// A book with one official prior chapter and one target chapter, plus one unique row for each
/// canon system and one relevant goal node. The names never appear in the packed prompts.
async fn fixture() -> Fixture {
    let runtime = agent_harness::runtime(idle_provider()).await;
    let first_id = runtime.chapter_id;
    let target = runtime
        .app
        .create_chapter(runtime.track_id, "第二章 雨夜")
        .await;
    let target_id = target["id"].as_i64().expect("target id") as i32;
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/chapters/{target_id}"),
            json!({
                "guidance": "写雨夜对峙。",
                "outline": "本章章纲：独孤影在渡口拦住沈砚。",
            }),
        )
        .await;
    assert_eq!(status, 200, "target patch: {body}");

    runtime.app.put_text(first_id, 0, "旧章正文。").await;
    runtime.app.set_chapter_status(first_id, "official").await;
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET summary = '第一章摘要：沈砚抵达渡口。' WHERE id = {first_id}"
        ))
        .await;

    runtime
        .app
        .execute(&format!(
            r#"
INSERT INTO entities(id, book_id, track_id, kind, name, aliases, summary, description, state,
                     attributes, importance, status, is_protagonist, first_chapter_id,
                     last_chapter_id, created_at, updated_at) VALUES
  (1, {book}, NULL, 'character', '萧无咎', '[]', '独有实体摘要', '独有实体描述。',
   '旧伤未愈的独有状态', '{{"修为":"筑基后期"}}', 5, 'active', 1, {first}, {first},
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO appearances(chapter_id, entity_id, role, note) VALUES ({first}, 1, 'pov', '独有出场');
INSERT INTO timeline_events(id, book_id, track_id, position, story_time, title, description,
                            chapter_id, importance, created_at, updated_at) VALUES
  (1, {book}, NULL, 1.0, '', '独有时间线事件', '独有时间线描述。', {first}, 4,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO memory_notes(id, book_id, track_id, kind, content, chapter_id, importance, status,
                         created_at, updated_at) VALUES
  (1, {book}, NULL, 'fact', '独有记忆笔记内容。', {first}, 5, 'active',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO foreshadowing(id, book_id, track_id, title, description, planned_payoff, status,
                          planted_chapter_id, resolved_chapter_id, resolution, importance,
                          created_at, updated_at) VALUES
  (1, {book}, NULL, '独有伏笔标题', '独有伏笔描述。', '', 'open', {first}, NULL, '', 4,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO goal_nodes(book_id, track_id, level, parent_id, title, purpose, status, chapter_from,
                       chapter_to, conditions, created_at, updated_at) VALUES
  ({book}, NULL, 'task', NULL, '独有目标节点', '独有目标目的。', 'active', 1, NULL, '[]',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
"#,
            book = runtime.book_id,
            first = first_id,
        ))
        .await;
    Fixture { runtime, target_id }
}

async fn set_systems(fixture: &Fixture, systems: &CanonSystems) {
    let (_, book) = fixture
        .runtime
        .app
        .get(&format!("/api/books/{}", fixture.runtime.book_id))
        .await;
    let mut settings = book["settings"].clone();
    settings["canon_systems"] = serde_json::to_value(systems).expect("systems");
    let (status, body) = fixture
        .runtime
        .app
        .patch(
            &format!("/api/books/{}", fixture.runtime.book_id),
            json!({ "settings": settings }),
        )
        .await;
    assert_eq!(status, 200, "patch systems: {body}");
}

async fn create(fixture: &Fixture, kind: SessionKind) -> i32 {
    let (session_id, _warnings) = sessions::create(
        fixture.runtime.app.state(),
        CreateSession {
            kind,
            book_id: Some(fixture.runtime.book_id),
            chapter_id: Some(fixture.target_id),
            granularity: Some(Granularity::Paragraph),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create session");
    session_id
}

async fn kickoff_text(fixture: &Fixture, session_id: i32) -> String {
    let messages = fixture.runtime.messages(session_id).await;
    let kickoff = messages.first().expect("kickoff");
    assert_eq!(kickoff.kind, "kickoff");
    kickoff.text()
}

async fn frozen_tools(fixture: &Fixture, session_id: i32) -> Vec<String> {
    let row = nekowriter::db::entities::agent_sessions::Entity::find_by_id(session_id)
        .one(fixture.runtime.app.db().conn())
        .await
        .expect("query session")
        .expect("session exists");
    row.tools
        .into_inner()
        .into_iter()
        .map(|tool| tool.name)
        .collect()
}

async fn scalar_i64(fixture: &Fixture, sql: &str) -> i64 {
    let row = fixture
        .runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .expect("query")
        .expect("row");
    row.try_get::<i64>("", "value").expect("count")
}

async fn count(fixture: &Fixture, table: &str) -> i64 {
    scalar_i64(fixture, &format!("SELECT count(*) AS value FROM {table}")).await
}

fn assert_tool(tools: &[String], tool: &str, present: bool) {
    assert_eq!(
        tools.iter().any(|name| name == tool),
        present,
        "tool {tool} frozen={present}; got {tools:?}"
    );
}

#[tokio::test]
async fn each_disabled_system_removes_its_data_tools_and_steps() {
    let cases: [(CanonSystems, &[&str], &[&str]); 5] = [
        (
            CanonSystems {
                entities: false,
                ..Default::default()
            },
            &["萧无咎", "旧伤未愈", "独有出场", "主要实体"],
            &["get_entity", "record_appearances"],
        ),
        (
            CanonSystems {
                timeline: false,
                ..Default::default()
            },
            &["独有时间线事件", "独有时间线描述", "时间线事件"],
            &["query_timeline", "add_timeline_event"],
        ),
        (
            CanonSystems {
                memory: false,
                ..Default::default()
            },
            &[
                "独有记忆笔记内容",
                "第一章摘要",
                "核心事实与世界规则",
                "事实 1",
            ],
            &["add_memory_note", "set_chapter_summary"],
        ),
        (
            CanonSystems {
                foreshadowing: false,
                ..Default::default()
            },
            &["独有伏笔标题", "未回收的伏笔"],
            &["list_foreshadowing", "resolve_foreshadowing"],
        ),
        (
            CanonSystems {
                goals: false,
                ..Default::default()
            },
            &["独有目标节点", "独有目标目的"],
            &["propose_goal_change"],
        ),
    ];
    for (systems, absent_texts, absent_tools) in cases {
        let fixture = fixture().await;
        let baseline_counts = [
            ("entities", count(&fixture, "entities").await),
            ("timeline_events", count(&fixture, "timeline_events").await),
            ("memory_notes", count(&fixture, "memory_notes").await),
            ("foreshadowing", count(&fixture, "foreshadowing").await),
            ("goal_nodes", count(&fixture, "goal_nodes").await),
        ];
        set_systems(&fixture, &systems).await;
        let session_id = create(&fixture, SessionKind::ChapterMaintenance).await;
        let kickoff = kickoff_text(&fixture, session_id).await;
        let tools = frozen_tools(&fixture, session_id).await;
        for text in absent_texts {
            assert!(
                !kickoff.contains(text),
                "disabled system data leaked into the kickoff: {text}\n{kickoff}"
            );
        }
        for tool in absent_tools {
            assert_tool(&tools, tool, false);
        }
        if !systems.entities {
            assert!(
                !kickoff.contains("record_appearances："),
                "Entities step leaked"
            );
        }
        if !systems.memory {
            assert!(
                !kickoff.contains("set_chapter_summary："),
                "Memory step leaked"
            );
        }
        if !systems.timeline {
            assert!(
                !kickoff.contains("时间线：用 add_timeline_event"),
                "Timeline step leaked"
            );
        }
        if !systems.foreshadowing {
            assert!(
                !kickoff.contains("伏笔：用 add_foreshadowing"),
                "Foreshadowing step leaked"
            );
        }
        if !systems.goals {
            assert!(
                !kickoff.contains("propose_goal_change"),
                "Goals step leaked"
            );
        }
        // Creating the session did not touch any canon row.
        assert_eq!(count(&fixture, "entities").await, baseline_counts[0].1);
        assert_eq!(
            count(&fixture, "timeline_events").await,
            baseline_counts[1].1
        );
        assert_eq!(count(&fixture, "memory_notes").await, baseline_counts[2].1);
        assert_eq!(count(&fixture, "foreshadowing").await, baseline_counts[3].1);
        assert_eq!(count(&fixture, "goal_nodes").await, baseline_counts[4].1);
    }
}

#[tokio::test]
async fn memory_off_keeps_the_write_outline() {
    let fixture = fixture().await;
    set_systems(
        &fixture,
        &CanonSystems {
            memory: false,
            ..Default::default()
        },
    )
    .await;
    let session_id = create(&fixture, SessionKind::ChapterWrite).await;
    let kickoff = kickoff_text(&fixture, session_id).await;
    assert!(
        kickoff.contains("本章章纲：独孤影在渡口拦住沈砚。"),
        "the chapter draft block must still carry the outline with Memory off:\n{kickoff}"
    );
    assert!(
        !kickoff.contains("第一章摘要"),
        "a summary leaked with Memory off"
    );
}

#[tokio::test]
async fn turning_a_system_back_on_restores_everything() {
    let fixture = fixture().await;
    set_systems(
        &fixture,
        &CanonSystems {
            entities: false,
            ..Default::default()
        },
    )
    .await;
    let off_id = create(&fixture, SessionKind::ChapterMaintenance).await;
    let off_kickoff = kickoff_text(&fixture, off_id).await;
    assert!(!off_kickoff.contains("萧无咎"));
    assert_tool(&frozen_tools(&fixture, off_id).await, "get_entity", false);
    fixture.runtime.app.clear_sessions().await;

    set_systems(&fixture, &CanonSystems::default()).await;
    let on_id = create(&fixture, SessionKind::ChapterMaintenance).await;
    let on_kickoff = kickoff_text(&fixture, on_id).await;
    assert!(on_kickoff.contains("萧无咎"));
    assert!(on_kickoff.contains("旧伤未愈"));
    assert_tool(&frozen_tools(&fixture, on_id).await, "get_entity", true);
}

#[tokio::test]
async fn a_mid_session_flip_changes_nothing_and_rollover_inherits() {
    let fixture = fixture().await;
    let parent_id = create(&fixture, SessionKind::ChapterWrite).await;
    let parent_tools = frozen_tools(&fixture, parent_id).await;
    let parent_kickoff = kickoff_text(&fixture, parent_id).await;
    set_systems(
        &fixture,
        &CanonSystems {
            entities: false,
            timeline: false,
            memory: false,
            foreshadowing: false,
            goals: false,
        },
    )
    .await;
    let parent = nekowriter::agent::session::load(fixture.runtime.app.db().conn(), parent_id)
        .await
        .expect("parent");
    nekowriter::agent::context::rollover(fixture.runtime.app.state(), &parent)
        .await
        .expect("rollover");
    let child_id = scalar_i64(
        &fixture,
        &format!("SELECT id AS value FROM agent_sessions WHERE parent_session_id = {parent_id}"),
    )
    .await as i32;
    assert_eq!(frozen_tools(&fixture, child_id).await, parent_tools);
    let child_kickoff = kickoff_text(&fixture, child_id).await;
    for text in ["萧无咎", "独有记忆笔记内容", "独有伏笔标题", "独有目标节点"]
    {
        assert!(
            child_kickoff.contains(text),
            "the rollover child must inherit the parent's all-on systems: missing {text}\n{child_kickoff}"
        );
    }
    assert!(parent_kickoff.contains("萧无咎"));
}

/// With Memory and Entities off, `finish_maintenance` must not demand appearances or a summary;
/// with all five off it still completes and leaves the chapter `synced` (spec/06 §6, AC15).
#[tokio::test]
async fn canon_counts_and_the_memory_view_report_the_book_systems() {
    let fixture = fixture().await;
    let (status, counts) = fixture
        .runtime
        .app
        .get(&format!(
            "/api/books/{}/canon/counts",
            fixture.runtime.book_id
        ))
        .await;
    assert_eq!(status, 200, "canon counts: {counts}");
    assert_eq!(counts["entities"]["entities"], 1);
    assert_eq!(counts["timeline"]["events"], 1);
    assert_eq!(counts["memory"]["notes"], 1);
    assert_eq!(counts["foreshadowing"]["items"], 1);
    assert_eq!(counts["goals"]["nodes"], 1);

    set_systems(
        &fixture,
        &CanonSystems {
            memory: false,
            ..Default::default()
        },
    )
    .await;
    let (status, memory) = fixture
        .runtime
        .app
        .get(&format!(
            "/api/books/{}/memory?chapter_id={}",
            fixture.runtime.book_id, fixture.target_id
        ))
        .await;
    assert_eq!(status, 200, "memory view: {memory}");
    assert_eq!(memory["systems"]["memory"], false);
    assert!(
        !memory["rendered"]
            .as_str()
            .unwrap_or_default()
            .contains("独有记忆笔记内容")
    );
}

#[tokio::test]
async fn maintenance_completes_when_required_systems_are_off() {
    for systems in [
        CanonSystems {
            entities: false,
            memory: false,
            ..Default::default()
        },
        CanonSystems {
            entities: false,
            timeline: false,
            memory: false,
            foreshadowing: false,
            goals: false,
        },
    ] {
        let runtime = agent_harness::runtime(vec![ScriptStep::new().tool(
            "finish_maintenance",
            json!({ "report": "没有需要维护的系统。" }),
        )])
        .await;
        let runtime_book = runtime.book_id;
        let (status, body) = runtime
            .app
            .patch(
                &format!("/api/books/{runtime_book}"),
                json!({ "settings": { "canon_systems": systems } }),
            )
            .await;
        assert_eq!(status, 200, "settings: {body}");
        let chapter_id = runtime.chapter_id;
        let (session_id, _warnings) = sessions::create(
            runtime.app.state(),
            CreateSession {
                kind: SessionKind::ChapterMaintenance,
                book_id: Some(runtime_book),
                chapter_id: Some(chapter_id),
                ..CreateSession::default()
            },
        )
        .await
        .expect("maintenance session");
        let row = runtime
            .wait_status(session_id, SessionStatus::Completed)
            .await;
        assert_eq!(row.status, "completed", "maintenance did not complete");
        let row = runtime
            .app
            .db()
            .conn()
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT canon_status AS value FROM chapters WHERE id = ?",
                vec![chapter_id.into()],
            ))
            .await
            .expect("query chapter")
            .expect("chapter");
        let status: Option<String> = row.try_get("", "value").expect("canon status");
        assert_eq!(status.as_deref(), Some("synced"));
    }
}
