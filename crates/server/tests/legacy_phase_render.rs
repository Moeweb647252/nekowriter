//! A historical `chapter_write`/`planning` row still decodes and its transcript renders (plan §6,
//! D-159 §3.4). No production path creates this phase any more.

mod agent_harness;

use agent_harness::runtime;
use nekowriter::agent::tools::{WRITER_TOOLS, is_permitted};
use nekowriter::db::json::{Phase, SessionKind};
use serde_json::json;

#[tokio::test]
async fn a_historical_planning_session_loads_and_renders() {
    let runtime = runtime(vec![]).await;
    let snapshot = r#"{"provider_id":0,"protocol":"mock","base_url":"","extra_headers":{},"model":"","context_window":0,"max_output_tokens":0,"options":null}"#;
    runtime
        .app
        .execute(&format!(
            "INSERT INTO agent_sessions \
             (id, book_id, role, kind, chapter_id, model_snapshot, system_prompt, tools, status, \
              phase, granularity, auto_pause, pause_after_outline, pause_requested, counters, \
              usage, created_at, updated_at) \
             VALUES (900, {}, 'writer', 'chapter_write', {}, '{}', \
              'history: 当前处于【规划阶段】', '[]', 'completed', 'planning', 'paragraph', 0, 0, 0, \
              '{{}}', '{{}}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            runtime.book_id, runtime.chapter_id, snapshot
        ))
        .await;
    runtime
        .app
        .execute(
            "INSERT INTO agent_messages(id, session_id, seq, role, kind, content, created_at) \
             VALUES (900, 900, 0, 'user', 'kickoff', \
             '[{\"type\":\"text\",\"text\":\"当前处于【规划阶段】\"}]', \
             '2026-01-01T00:00:00Z')",
        )
        .await;

    let (status, body) = runtime.app.get("/api/sessions/900").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["session"]["phase"], "planning", "{body}");
    assert!(
        body["system_prompt"]
            .as_str()
            .is_some_and(|text| text.contains("规划阶段")),
        "the frozen prompt still renders: {body}"
    );
    assert_eq!(
        body["messages"][0]["message"]["blocks"][0]["text"],
        json!("当前处于【规划阶段】"),
        "the kickoff transcript still renders"
    );

    // The kept historical decode arm remains a refusal: no session runs in `planning`.
    let frozen: Vec<String> = WRITER_TOOLS.iter().map(|name| (*name).to_owned()).collect();
    assert!(!is_permitted(
        SessionKind::ChapterWrite,
        Phase::Planning,
        "read_draft",
        &frozen
    ));
}
