//! Kickoff context and tiered memory (PLAN M4.1, spec/03 §10, spec/06 §4).
//!
//! The snapshots pin the rendered kickoff of a first chapter and of a later chapter: every block
//! (`block.settings`, `block.memory`, `writer.kickoff.chapter`), the empty-state wording of a book
//! with no canon yet, and the summary/tail/protagonist lines once earlier chapters are official.

mod agent_harness;

use agent_harness::runtime;
use nekowriter::db::json::{Granularity, SessionKind};
use nekowriter::services::sessions::{self, CreateSession};

/// Renders a writer kickoff through the production `assemble_kickoff` path and returns the joined
/// text of its blocks, so a snapshot covers every block a real session would carry.
async fn kickoff_text(runtime: &agent_harness::Runtime, chapter_id: i32) -> String {
    let (session_id, _warnings) = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(chapter_id),
            granularity: Some(Granularity::Paragraph),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create writer session");
    let messages = runtime.messages(session_id).await;
    let kickoff = messages.first().expect("a kickoff message");
    assert_eq!(kickoff.kind, "kickoff");
    assert_eq!(kickoff.seq, 0);
    // One text block per kickoff block (spec/03 §10): the settings block plus the memory block
    // plus the task.
    let blocks = kickoff.blocks();
    assert_eq!(blocks.len(), 3, "writer kickoffs carry three blocks");
    kickoff.text()
}

/// Seeds the book's setting materials so the snapshot shows a realistic `block.settings`.
async fn seed_settings(runtime: &agent_harness::Runtime) {
    runtime
        .app
        .put(
            &format!("/api/books/{}/setting-docs/outline", runtime.book_id),
            serde_json::json!({ "content": "# 全书大纲\n第一卷：入城", "base_revision": 0 }),
        )
        .await;
    runtime
        .app
        .put(
            &format!("/api/books/{}/setting-docs/world", runtime.book_id),
            serde_json::json!({ "content": "# 世界观\n剑气可斩断寻常兵刃。", "base_revision": 0 }),
        )
        .await;
    runtime
        .app
        .put(
            &format!("/api/books/{}/setting-docs/protagonist", runtime.book_id),
            serde_json::json!({ "content": "# 主角\n沈砚，寡言的游侠。", "base_revision": 0 }),
        )
        .await;
}

/// A side-track chapter renders `block.anchor` inside its task (spec/03 §10). Side tracks arrive
/// with M10, so this checks the include machinery on the book-level state the block needs: a main
/// track never renders the anchor block.
#[tokio::test]
async fn a_main_track_kickoff_has_no_anchor_block() {
    let runtime = runtime(Vec::new()).await;
    let text = kickoff_text(&runtime, runtime.chapter_id).await;
    assert!(
        !text.contains("番外"),
        "a main-track kickoff never renders block.anchor:\n{text}"
    );
}

/// The first chapter of a book: `block.memory` says there is no earlier text, and the outline is
/// rendered by the task, not twice (spec/06 §4.2, §4.5).
#[tokio::test]
async fn block_memory_of_a_first_chapter_says_there_is_no_earlier_text() {
    let runtime = runtime(Vec::new()).await;
    seed_settings(&runtime).await;
    let text = kickoff_text(&runtime, runtime.chapter_id).await;
    assert!(
        text.contains("这是本书（本轨道）的第一章，暂无前文。"),
        "the first chapter is told it is the first:\n{text}"
    );
    assert!(
        !text.contains("### 本章章纲"),
        "a first chapter has no outline to repeat:\n{text}"
    );
}

#[tokio::test]
async fn the_first_chapter_kickoff_snapshot() {
    let runtime = runtime(Vec::new()).await;
    seed_settings(&runtime).await;
    // No debug session here: it is a writer session on the same chapter, and invariant 5 allows
    // only one open writer session per chapter (spec/02 §5). The kickoff under test is the
    // production writer one, which `kickoff_text` renders through the production path.
    let text = kickoff_text(&runtime, runtime.chapter_id).await;
    insta::assert_snapshot!("writer_kickoff_first_chapter", text);
}

/// A later chapter: earlier chapters are official and summarised, so `block.memory` carries the
/// summary window, the previous chapter's tail and the protagonist's state (spec/06 §4.2).
#[tokio::test]
async fn the_later_chapter_kickoff_snapshot() {
    let runtime = runtime(Vec::new()).await;
    seed_settings(&runtime).await;

    // Chapter one: official, with a summary and text; plus a protagonist entity.
    let first = runtime.app.create_chapter(runtime.track_id, "第一章").await;
    let first_id = first["id"].as_i64().expect("id") as i32;
    runtime
        .app
        .put_text(first_id, 0, "雨落在城门口。\n\n他握紧了旧剑。")
        .await;
    runtime.app.set_chapter_status(first_id, "official").await;
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET summary = '主角入城，遇守门人。' WHERE id = {first_id}"
        ))
        .await;
    runtime
        .app
        .execute(&format!(
            "INSERT INTO entities(book_id, kind, name, summary, state, attributes, importance, \
             status, is_protagonist, created_at, updated_at) VALUES \
             ({}, 'character', '沈砚', '游侠', '刚入城，带着一封旧信', \
              '{{\"修为\":\"筑基中期\"}}', 5, 'active', 1, '2026-01-01T00:00:00Z', \
              '2026-01-01T00:00:00Z')",
            runtime.book_id
        ))
        .await;

    // The chapter under test is the second one.
    let second = runtime.app.create_chapter(runtime.track_id, "第二章").await;
    let second_id = second["id"].as_i64().expect("id") as i32;
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET guidance = '本章要交代旧信的来历。' WHERE id = {second_id}"
        ))
        .await;

    let text = kickoff_text(&runtime, second_id).await;
    assert!(text.contains("主角入城，遇守门人。"), "{text}");
    assert!(
        text.contains("他握紧了旧剑。"),
        "the previous tail:\n{text}"
    );
    assert!(text.contains("沈砚"), "the protagonist:\n{text}");
    insta::assert_snapshot!("writer_kickoff_later_chapter", text);
}

/// A redraft kickoff carries the author's feedback and the previous outline, and uses the redraft
/// task instead of the chapter task (spec/03 §10, spec/08 §1.1).
#[tokio::test]
async fn a_redraft_kickoff_carries_the_feedback_and_the_previous_outline() {
    let runtime = runtime(Vec::new()).await;
    let chapter_id = runtime.chapter_id;
    runtime
        .app
        .patch(
            &format!("/api/chapters/{chapter_id}"),
            serde_json::json!({ "outline": "旧章纲：城门口", "guidance": "写一场雨中的对峙。" }),
        )
        .await;
    runtime.app.put_text(chapter_id, 0, "旧正文的一段。").await;
    runtime
        .app
        .execute(&format!(
            "INSERT INTO chapter_feedback(chapter_id, content_version, text, issues, mode, \
             created_at) VALUES ({chapter_id}, 1, '结尾太仓促，重写。', \
             '[{{\"severity\":\"major\",\"category\":\"pacing\",\"pids\":[\"p1\"],\"quote\":\"\",\
             \"problem\":\"节奏太快\",\"suggestion\":\"补一段过渡\"}}]', 'redraft', \
             '2026-01-01T00:00:00Z')"
        ))
        .await;

    let (session_id, _warnings) = sessions::create(
        runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(runtime.book_id),
            chapter_id: Some(chapter_id),
            granularity: Some(Granularity::Paragraph),
            redraft: true,
            ..CreateSession::default()
        },
    )
    .await
    .expect("create redraft session");
    let messages = runtime.messages(session_id).await;
    let text = messages.first().expect("kickoff").text();
    assert!(text.contains("结尾太仓促，重写。"), "{text}");
    assert!(text.contains("节奏太快"), "the ticked issue:\n{text}");
    assert!(
        text.contains("旧章纲：城门口"),
        "the previous outline:\n{text}"
    );
    assert!(text.contains("重写任务"), "the redraft task:\n{text}");
    assert!(
        !text.contains("写作任务"),
        "not the plain chapter task:\n{text}"
    );
}
