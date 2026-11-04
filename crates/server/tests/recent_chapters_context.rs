//! `block.recent_chapters` and `block.writing_skills` rendering (design §6–§7).
//!
//! The selection rule must match memory's (spec/06 §4.1): official chapters only, same track plus
//! a side story's main-track top-up up to the anchor, target excluded. The budget is relative to
//! the chapter target, filled newest-first, front-cut at a paragraph boundary keeping the ending,
//! and a front-cut chapter is announced as partial.

use nekowriter::agent::context::assemble_kickoff;
use nekowriter::db::entities::{agent_sessions, chapters, tracks};
use nekowriter::db::json::StoredBlock;
use nekowriter::services::memory::{self, RecentChapter};
use nekowriter::testing::TestApp;
use sea_orm::{ConnectionTrait, EntityTrait};
use serde_json::json;

/// The main track of a fresh book and the book id.
async fn setup(app: &TestApp) -> (i32, i32) {
    let book = app.create_book("Recent Chapters").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    (book_id, track_id)
}

/// Creates a chapter with `text` and makes it official; optionally sets a summary.
async fn official_chapter(
    app: &TestApp,
    track_id: i32,
    title: &str,
    text: &str,
    summary: &str,
) -> i32 {
    let chapter = app.create_chapter(track_id, title).await;
    let id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.put_text(id, 0, text).await;
    app.set_chapter_status(id, "official").await;
    if !summary.is_empty() {
        app.execute(&format!(
            "UPDATE chapters SET summary = '{summary}' WHERE id = {id}"
        ))
        .await;
    }
    id
}

async fn set_recent_settings(app: &TestApp, book_id: i32, count: i32, target_max: i32) {
    let (_, book) = app.get(&format!("/api/books/{book_id}")).await;
    let mut settings = book["settings"].clone();
    settings["recent_chapter_count"] = json!(count);
    settings["target_words_max"] = json!(target_max);
    settings["target_words_min"] = json!(target_max.min(3000));
    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "settings": settings }),
        )
        .await;
    assert_eq!(status, 200, "patch settings: {body}");
}

async fn chapter_model(app: &TestApp, chapter_id: i32) -> chapters::Model {
    chapters::Entity::find_by_id(chapter_id)
        .one(app.db().conn())
        .await
        .expect("query chapter")
        .expect("chapter")
}

async fn track_model(app: &TestApp, track_id: i32) -> tracks::Model {
    tracks::Entity::find_by_id(track_id)
        .one(app.db().conn())
        .await
        .expect("query track")
        .expect("track")
}

/// Calls the production selection service for one target chapter.
async fn recent(app: &TestApp, book_id: i32, track_id: i32, chapter_id: i32) -> Vec<RecentChapter> {
    let prompts = app
        .state()
        .prompts()
        .for_book(app.db(), Some(book_id))
        .await
        .expect("prompts");
    let track = track_model(app, track_id).await;
    let chapter = chapter_model(app, chapter_id).await;
    memory::recent_chapters(app.db().conn(), &prompts, &track, &chapter)
        .await
        .expect("recent chapters")
}

/// Inserts an idle session row and returns its model, so `assemble_kickoff` can be driven without
/// starting a runner (which would need a scripted model step).
async fn session(
    app: &TestApp,
    book_id: i32,
    track_id: i32,
    chapter_id: i32,
    role: &str,
    kind: &str,
) -> agent_sessions::Model {
    app.execute(&format!(
        "INSERT INTO agent_sessions(book_id, role, kind, chapter_id, track_id, model_snapshot, \
         system_prompt, tools, status, phase, created_at, updated_at) VALUES \
         ({book_id}, '{role}', '{kind}', {chapter_id}, {track_id}, '{{\"provider_id\":0,\"protocol\":\"mock\",\"base_url\":\"\",\"extra_headers\":{{}},\"model\":\"\",\"context_window\":0,\"max_output_tokens\":0,\"options\":null}}', '', '[]', 'idle', \
         'none', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    let id = app
        .db()
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT max(id) AS id FROM agent_sessions",
        ))
        .await
        .expect("query id")
        .expect("row")
        .try_get::<i32>("", "id")
        .expect("id");
    agent_sessions::Entity::find_by_id(id)
        .one(app.db().conn())
        .await
        .expect("query session")
        .expect("session")
}

async fn kickoff_blocks(
    app: &TestApp,
    book_id: i32,
    track_id: i32,
    chapter_id: i32,
    role: &str,
    kind: &str,
) -> Vec<String> {
    let session = session(app, book_id, track_id, chapter_id, role, kind).await;
    let prompts = app
        .state()
        .prompts()
        .for_book(app.db(), Some(book_id))
        .await
        .expect("prompts");
    let blocks = assemble_kickoff(app.db().conn(), &prompts, &session, false, None)
        .await
        .expect("kickoff");
    blocks
        .into_iter()
        .filter_map(|block| match block {
            StoredBlock::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

async fn kickoff_text(
    app: &TestApp,
    book_id: i32,
    track_id: i32,
    chapter_id: i32,
    role: &str,
    kind: &str,
) -> String {
    kickoff_blocks(app, book_id, track_id, chapter_id, role, kind)
        .await
        .join("\n\n")
}

#[tokio::test]
async fn selection_uses_official_chapters_only_and_excludes_the_target() {
    let app = TestApp::new().await;
    let (book_id, track_id) = setup(&app).await;
    let first = official_chapter(&app, track_id, "第一章", "第一章正文。", "").await;
    let second = official_chapter(&app, track_id, "第二章", "第二章正文。", "").await;
    // A draft/in-review chapter is not approved prose: it must never be used as a top-up source.
    let pending = app.create_chapter(track_id, "第三章").await;
    let pending_id = pending["id"].as_i64().expect("id") as i32;
    app.put_text(pending_id, 0, "第三章草稿。").await;
    let target = app.create_chapter(track_id, "第四章").await;
    let target_id = target["id"].as_i64().expect("id") as i32;
    set_recent_settings(&app, book_id, 2, 4000).await;

    let selected = recent(&app, book_id, track_id, target_id).await;
    let ids: Vec<i32> = selected.iter().map(|entry| entry.chapter_id).collect();
    assert_eq!(ids, vec![first, second]);
    assert_eq!(selected[0].text, "第一章正文。");
    assert_eq!(selected[1].text, "第二章正文。");
    assert!(
        selected.iter().all(|entry| !entry.partial),
        "a short chapter is injected whole"
    );
    assert!(
        !ids.contains(&pending_id) && !ids.contains(&target_id),
        "non-official text and the target never appear"
    );
}

#[tokio::test]
async fn recent_chapter_count_is_a_hard_chapter_limit_not_just_a_budget_multiplier() {
    let app = TestApp::new().await;
    let (book_id, track_id) = setup(&app).await;
    let first = official_chapter(&app, track_id, "第一章", &"甲".repeat(100), "").await;
    let second = official_chapter(&app, track_id, "第二章", &"乙".repeat(100), "").await;
    let third = official_chapter(&app, track_id, "第三章", &"丙".repeat(100), "").await;
    let target = app.create_chapter(track_id, "第四章").await;
    let target_id = target["id"].as_i64().expect("id") as i32;
    // 2 × 6,000 characters is far above three 100-character chapters: only the chapter count can
    // stop the iteration, so a bug that treats the setting as a budget multiplier injects all 3.
    set_recent_settings(&app, book_id, 2, 4000).await;

    let selected = recent(&app, book_id, track_id, target_id).await;
    assert_eq!(
        selected.len(),
        2,
        "recent_chapter_count = 2 must inject at most two chapters, got {:?}",
        selected
            .iter()
            .map(|entry| entry.chapter_id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        selected
            .iter()
            .map(|entry| entry.chapter_id)
            .collect::<Vec<_>>(),
        vec![second, third],
        "the newest two chapters are the ones kept"
    );
    assert!(!selected.iter().any(|entry| entry.chapter_id == first));
}

#[tokio::test]
async fn a_side_track_is_topped_up_from_the_main_track_up_to_its_anchor() {
    let app = TestApp::new().await;
    let (book_id, main_track_id) = setup(&app).await;
    let main_one = official_chapter(&app, main_track_id, "主线一", "主线一正文。", "").await;
    let main_two = official_chapter(&app, main_track_id, "主线二", "主线二正文。", "").await;
    // After the anchor: must not be used as a top-up.
    let main_three = official_chapter(&app, main_track_id, "主线三", "主线三正文。", "").await;

    app.execute(&format!(
        "INSERT INTO tracks(book_id, kind, title, synopsis, canon_mode, anchor_kind, \
         anchor_chapter_id, ord, created_at, updated_at) VALUES \
         ({book_id}, 'side', '剑冢旧事', '师父年轻时的故事。', 'canon', 'after_chapter', \
         {main_two}, 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let side_track_id = tracks
        .as_array()
        .expect("array")
        .iter()
        .find(|track| track["kind"] == "side")
        .expect("side track")["id"]
        .as_i64()
        .expect("id") as i32;
    let side_one = official_chapter(&app, side_track_id, "番外一", "番外一正文。", "").await;
    let side_target = app.create_chapter(side_track_id, "番外二").await;
    let side_target_id = side_target["id"].as_i64().expect("id") as i32;
    set_recent_settings(&app, book_id, 2, 4000).await;

    let selected = recent(&app, book_id, side_track_id, side_target_id).await;
    let ids: Vec<i32> = selected.iter().map(|entry| entry.chapter_id).collect();
    // One own chapter + the last main chapter up to the anchor, in story order.
    assert_eq!(ids, vec![main_two, side_one]);
    assert!(
        !ids.contains(&main_one) && !ids.contains(&main_three),
        "only the missing count is topped up, and nothing past the anchor"
    );

    // The side track's own chapter carries its side label; the main-track top-up keeps the main
    // label, so the chapter tag tells the model which story the prose belongs to (design §7).
    let text = kickoff_text(
        &app,
        book_id,
        side_track_id,
        side_target_id,
        "writer",
        "chapter_write",
    )
    .await;
    assert!(
        text.contains("<chapter_text number=\"2\" label=\"第2章\" title=\"主线二\">"),
        "the main-track top-up keeps its main label:\n{text}"
    );
    assert!(
        text.contains(
            "<chapter_text number=\"1\" label=\"番外《剑冢旧事》第1章\" title=\"番外一\">"
        ),
        "the side chapter carries its side label:\n{text}"
    );
}

#[tokio::test]
async fn an_over_cap_chapter_is_front_cut_at_a_paragraph_boundary_keeping_the_ending() {
    let app = TestApp::new().await;
    let (book_id, track_id) = setup(&app).await;
    let paragraph = "甲".repeat(400);
    let second = "乙".repeat(400);
    let third = "丙".repeat(400);
    let fourth = "丁".repeat(400);
    let text = format!("{paragraph}\n\n{second}\n\n{third}\n\n{fourth}");
    let first = official_chapter(&app, track_id, "长章", &text, "").await;
    let target = app.create_chapter(track_id, "下一章").await;
    let target_id = target["id"].as_i64().expect("id") as i32;
    // 1,000 target words → 1,500-character cap; the chapter is 1,604 characters over four
    // paragraphs, so the front cut keeps the last paragraphs only.
    set_recent_settings(&app, book_id, 1, 1000).await;

    let selected = recent(&app, book_id, track_id, target_id).await;
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].chapter_id, first);
    assert!(selected[0].partial, "a front-cut chapter is marked partial");
    assert!(
        selected[0].text.ends_with(&fourth),
        "the ending is kept: {}",
        selected[0].text
    );
    assert!(
        !selected[0].text.contains(&paragraph),
        "the beginning is dropped"
    );
    assert!(selected[0].text.chars().count() <= 1500);

    // The rendered block announces the omission, and the chapter tag carries `partial="true"`.
    let text = kickoff_text(
        &app,
        book_id,
        track_id,
        target_id,
        "writer",
        "chapter_write",
    )
    .await;
    assert!(text.contains("partial=\"true\""), "{text}");
    assert!(
        text.contains("节选"),
        "the block says the text is partial:\n{text}"
    );
    assert!(text.contains("<chapter_text number=\"1\""));
}

#[tokio::test]
async fn a_single_paragraph_over_cap_is_cut_at_a_sentence_boundary() {
    let app = TestApp::new().await;
    let (book_id, track_id) = setup(&app).await;
    // One paragraph of three 599-character sentences; the 1,500-character cap fits the last two
    // sentences exactly but not the last three, so the cut must start at a sentence boundary.
    let first = format!("{}。", "甲".repeat(598));
    let second = format!("{}。", "乙".repeat(598));
    let third = format!("{}。", "丙".repeat(598));
    let text = format!("{first}{second}{third}");
    official_chapter(&app, track_id, "独段长章", &text, "").await;
    let target = app.create_chapter(track_id, "下一章").await;
    let target_id = target["id"].as_i64().expect("id") as i32;
    set_recent_settings(&app, book_id, 1, 1000).await;

    let selected = recent(&app, book_id, track_id, target_id).await;
    assert_eq!(selected.len(), 1);
    assert!(selected[0].partial, "the front was cut");
    assert!(
        selected[0].text.starts_with(&second),
        "a single over-cap paragraph is cut at a sentence boundary, not mid-sentence:\n{}",
        selected[0].text
    );
    assert_eq!(
        selected[0].text,
        format!("{second}{third}"),
        "the fitting sentence suffix is kept whole"
    );
}

#[tokio::test]
async fn a_chapter_under_the_cap_is_whole_and_the_budget_fills_newest_first() {
    let app = TestApp::new().await;
    let (book_id, track_id) = setup(&app).await;
    let old_text = "旧".repeat(500);
    let middle_text = "中".repeat(500);
    let new_text = "新".repeat(500);
    let old = official_chapter(&app, track_id, "旧章", &old_text, "").await;
    let middle = official_chapter(&app, track_id, "中章", &middle_text, "").await;
    let new = official_chapter(&app, track_id, "新章", &new_text, "").await;
    let target = app.create_chapter(track_id, "目标章").await;
    let target_id = target["id"].as_i64().expect("id") as i32;
    set_recent_settings(&app, book_id, 2, 4000).await;

    // Three official chapters with a wide budget: the count limit, not the budget, must pick the
    // newest two — two chapters could not tell "last N" apart from "fill the budget".
    let selected = recent(&app, book_id, track_id, target_id).await;
    assert_eq!(
        selected
            .iter()
            .map(|entry| entry.chapter_id)
            .collect::<Vec<_>>(),
        vec![middle, new],
        "at most `recent_chapter_count` chapters, newest first"
    );
    assert_eq!(selected.len(), 2);
    assert!(!selected.iter().any(|entry| entry.chapter_id == old));
    assert!(selected.iter().all(|entry| !entry.partial));
    assert_eq!(selected[0].text, middle_text);
    assert_eq!(selected[1].text, new_text);
}

#[tokio::test]
async fn zero_disables_the_block_and_an_empty_book_renders_nothing() {
    let app = TestApp::new().await;
    let (book_id, track_id) = setup(&app).await;
    official_chapter(&app, track_id, "第一章", "第一章正文。", "").await;
    let target = app.create_chapter(track_id, "第二章").await;
    let target_id = target["id"].as_i64().expect("id") as i32;
    set_recent_settings(&app, book_id, 0, 4000).await;

    assert!(recent(&app, book_id, track_id, target_id).await.is_empty());
    let text = kickoff_text(
        &app,
        book_id,
        track_id,
        target_id,
        "writer",
        "chapter_write",
    )
    .await;
    assert!(!text.contains("# 最近章节正文"), "{text}");

    // An empty candidate set (a first chapter) is dropped by `assemble_kickoff` too.
    let empty_book = app.create_book("Empty").await["id"].as_i64().expect("id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{empty_book}/tracks")).await;
    let empty_track = tracks[0]["id"].as_i64().expect("id") as i32;
    let first = app.create_chapter(empty_track, "第一章").await;
    let first_id = first["id"].as_i64().expect("id") as i32;
    set_recent_settings(&app, empty_book, 2, 4000).await;
    let text = kickoff_text(
        &app,
        empty_book,
        empty_track,
        first_id,
        "writer",
        "chapter_write",
    )
    .await;
    assert!(!text.contains("# 最近章节正文"), "{text}");
}

#[tokio::test]
async fn selecting_only_the_empty_builtin_renders_no_skills_block() {
    let app = TestApp::new().await;
    let (book_id, track_id) = setup(&app).await;
    let target = app.create_chapter(track_id, "第一章").await;
    let target_id = target["id"].as_i64().expect("id") as i32;
    let (_, all) = app.get("/api/writing-skills").await;
    let builtin = all
        .as_array()
        .expect("array")
        .iter()
        .find(|row| row["key"] == "basic-writing")
        .expect("the seeded built-in")["id"]
        .as_i64()
        .expect("id") as i32;
    // A blank body injects nothing: selecting the skill keeps the selection but the block is
    // dropped (design §5, D-148). The seeded built-in now ships content (D-156), so blank it here
    // to keep the rule under test.
    app.execute("UPDATE writing_skills SET body = '' WHERE key = 'basic-writing'")
        .await;
    let (status, body) = app
        .put(
            &format!("/api/books/{book_id}/writing-skills"),
            json!({ "skill_ids": [builtin] }),
        )
        .await;
    assert_eq!(status, 200, "select the empty built-in: {body}");

    let text = kickoff_text(
        &app,
        book_id,
        track_id,
        target_id,
        "writer",
        "chapter_write",
    )
    .await;
    assert!(
        !text.contains("# 写作技能"),
        "the heading must be blank:\n{text}"
    );
    assert!(
        !text.contains("<writing_skill"),
        "no empty skill tag:\n{text}"
    );
}

#[tokio::test]
async fn an_empty_recent_chapters_override_keeps_previous_tail() {
    let app = TestApp::new().await;
    let (book_id, track_id) = setup(&app).await;
    official_chapter(
        &app,
        track_id,
        "第一章",
        "雨落在城门口。\n\n他握紧了旧剑。",
        "主角入城。",
    )
    .await;
    let target = app.create_chapter(track_id, "第二章").await;
    let target_id = target["id"].as_i64().expect("id") as i32;
    set_recent_settings(&app, book_id, 2, 4000).await;
    app.execute(&format!(
        "INSERT INTO prompt_overrides(scope, book_id, key, content, updated_at) \
         VALUES ('book', {book_id}, 'block.recent_chapters', '', '2026-01-01T00:00:00Z')"
    ))
    .await;

    let text = kickoff_text(
        &app,
        book_id,
        track_id,
        target_id,
        "writer",
        "chapter_write",
    )
    .await;
    assert!(
        text.contains("### 上一章结尾（第1章）"),
        "an empty recent-chapters override must not suppress previous_tail:\n{text}"
    );
    assert!(
        !text.contains("# 最近章节正文"),
        "the empty recent-chapters block must be dropped:\n{text}"
    );
}

#[tokio::test]
async fn an_empty_chapter_draft_override_keeps_the_outline_in_memory() {
    let app = TestApp::new().await;
    let (book_id, track_id) = setup(&app).await;
    let target = app.create_chapter(track_id, "第二章").await;
    let target_id = target["id"].as_i64().expect("id") as i32;
    let (status, body) = app
        .patch(
            &format!("/api/chapters/{target_id}"),
            json!({ "outline": "这一章要先写雨，再写剑。" }),
        )
        .await;
    assert_eq!(status, 200, "set outline: {body}");
    app.execute(&format!(
        "INSERT INTO prompt_overrides(scope, book_id, key, content, updated_at) \
         VALUES ('book', {book_id}, 'block.chapter_draft', '', '2026-01-01T00:00:00Z')"
    ))
    .await;

    let text = kickoff_text(
        &app,
        book_id,
        track_id,
        target_id,
        "writer",
        "chapter_write",
    )
    .await;
    assert!(
        text.contains("这一章要先写雨，再写剑。"),
        "an empty chapter_draft override must not suppress the outline in memory:\n{text}"
    );
}

#[tokio::test]
async fn previous_tail_is_deduplicated_but_summaries_and_the_memory_block_stay() {
    let app = TestApp::new().await;
    let (book_id, track_id) = setup(&app).await;
    official_chapter(
        &app,
        track_id,
        "第一章",
        "雨落在城门口。\n\n他握紧了旧剑。",
        "主角入城，遇守门人。",
    )
    .await;
    let target = app.create_chapter(track_id, "第二章").await;
    let target_id = target["id"].as_i64().expect("id") as i32;
    set_recent_settings(&app, book_id, 2, 4000).await;

    let text = kickoff_text(
        &app,
        book_id,
        track_id,
        target_id,
        "writer",
        "chapter_write",
    )
    .await;
    assert_eq!(
        text.matches("他握紧了旧剑。").count(),
        1,
        "previous_tail is a subset of the injected chapter and is dropped:\n{text}"
    );
    assert!(
        text.contains("主角入城，遇守门人。"),
        "summaries are kept, not duplicates of the text:\n{text}"
    );
    assert!(text.contains("# 最近章节正文"), "{text}");
}

#[tokio::test]
async fn the_session_kind_table_controls_which_blocks_receive_skills_and_recent_chapters() {
    let app = TestApp::new().await;
    let (book_id, track_id) = setup(&app).await;
    official_chapter(&app, track_id, "第一章", "前一章正文。", "").await;
    let target = app.create_chapter(track_id, "第二章").await;
    let target_id = target["id"].as_i64().expect("id") as i32;
    set_recent_settings(&app, book_id, 2, 4000).await;
    let (_, skill) = app
        .post(
            "/api/writing-skills",
            json!({ "name": "冷硬文风", "category": "文风", "body": "用短句。" }),
        )
        .await;
    let skill_id = skill["id"].as_i64().expect("id") as i32;
    app.put(
        &format!("/api/books/{book_id}/writing-skills"),
        json!({ "skill_ids": [skill_id] }),
    )
    .await;

    // chapter_write: both, skills before memory and recent chapters after memory (design §6.1).
    let write = kickoff_text(
        &app,
        book_id,
        track_id,
        target_id,
        "writer",
        "chapter_write",
    )
    .await;
    let settings_at = write.find("# 设定资料").expect("settings");
    let skills_at = write.find("# 写作技能").expect("skills");
    let memory_at = write.find("# 记忆").expect("memory");
    let recent_at = write.find("# 最近章节正文").expect("recent");
    let draft_at = write.find("# 本章正文").expect("draft");
    assert!(settings_at < skills_at && skills_at < memory_at && memory_at < recent_at);
    assert!(recent_at < draft_at);
    assert!(
        write.contains("<writing_skill name=\"冷硬文风\" category=\"文风\">"),
        "{write}"
    );
    assert!(
        write.contains("指令文本") && write.contains("不是本书的设定资料"),
        "the skills block says it is instruction text, not canon:\n{write}"
    );

    // The planner writes an outline, not prose: recent chapters yes, skills no.
    let outline = kickoff_text(
        &app,
        book_id,
        track_id,
        target_id,
        "planner",
        "chapter_outline",
    )
    .await;
    assert!(outline.contains("# 最近章节正文"), "{outline}");
    assert!(!outline.contains("# 写作技能"), "{outline}");

    // Revision and review grade/produce prose against the book's standard: skills yes, recent no.
    for (role, kind) in [("writer", "chapter_revise"), ("reviewer", "chapter_review")] {
        let text = kickoff_text(&app, book_id, track_id, target_id, role, kind).await;
        assert!(text.contains("# 写作技能"), "{kind}:\n{text}");
        assert!(!text.contains("# 最近章节正文"), "{kind}:\n{text}");
    }

    // The archivist and research kinds get neither.
    for (role, kind) in [
        ("archivist", "chapter_maintenance"),
        ("researcher", "research"),
    ] {
        if kind == "research" {
            // A research session has no chapter; the block list has no recent-chapters entry at
            // all, which the kind-table test in `agent_creation.rs` already pins.
            continue;
        }
        let text = kickoff_text(&app, book_id, track_id, target_id, role, kind).await;
        assert!(!text.contains("# 写作技能"), "{kind}:\n{text}");
        assert!(!text.contains("# 最近章节正文"), "{kind}:\n{text}");
    }
}

/// The real-check fixture: a writer kickoff with **two prior official chapters** and one selected
/// skill. It prints the block order, the XML tags and the character budget (run with
/// `--nocapture`); the assertions pin what the report quotes.
#[tokio::test]
async fn full_writer_kickoff_with_two_prior_chapters_reports_blocks_tags_and_budget() {
    let app = TestApp::new().await;
    let (book_id, track_id) = setup(&app).await;
    // About 2,000 characters each, so both fit the default 4,000-word target's 6,000-character
    // per-chapter cap (total budget 12,000 for two chapters).
    let first_text = "甲".repeat(2000);
    let second_text = "乙".repeat(2000);
    official_chapter(&app, track_id, "第一章", &first_text, "第一章摘要。").await;
    official_chapter(&app, track_id, "第二章", &second_text, "第二章摘要。").await;
    let target = app.create_chapter(track_id, "第三章").await;
    let target_id = target["id"].as_i64().expect("id") as i32;
    set_recent_settings(&app, book_id, 2, 4000).await;
    let (_, skill) = app
        .post(
            "/api/writing-skills",
            json!({ "name": "冷硬文风", "category": "文风", "body": "用短句推进，不写大段心理独白。" }),
        )
        .await;
    let skill_id = skill["id"].as_i64().expect("id") as i32;
    app.put(
        &format!("/api/books/{book_id}/writing-skills"),
        json!({ "skill_ids": [skill_id] }),
    )
    .await;

    let session = session(
        &app,
        book_id,
        track_id,
        target_id,
        "writer",
        "chapter_write",
    )
    .await;
    let prompts = app
        .state()
        .prompts()
        .for_book(app.db(), Some(book_id))
        .await
        .expect("prompts");
    let blocks = assemble_kickoff(app.db().conn(), &prompts, &session, false, None)
        .await
        .expect("kickoff");
    assert_eq!(
        blocks.len(),
        6,
        "settings, skills, memory, recent, draft, task"
    );
    println!("=== writer kickoff block order ===");
    let mut recent_chars = 0usize;
    for (index, block) in blocks.iter().enumerate() {
        let StoredBlock::Text { text, cache, .. } = block else {
            panic!("unexpected non-text block");
        };
        let first_line = text.lines().next().unwrap_or_default();
        let chars = text.chars().count();
        println!("{index}: {first_line} ({chars} chars, cache={cache:?})");
        if first_line == "# 最近章节正文" {
            recent_chars = chars;
        }
        // Only the settings block is cached (`block_is_cached` matches one key only).
        assert_eq!(cache.is_some(), index == 0, "cache on block {index}");
    }
    let joined = blocks
        .iter()
        .filter_map(|block| match block {
            StoredBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    assert!(joined.contains("<writing_skill name=\"冷硬文风\" category=\"文风\">"));
    assert!(joined.contains("<chapter_text number=\"1\" label=\"第1章\" title=\"第一章\">"));
    assert!(joined.contains("<chapter_text number=\"2\" label=\"第2章\" title=\"第二章\">"));
    assert!(
        recent_chars <= 12_000,
        "the recent-chapters block stays under the 12,000-character budget: {recent_chars}"
    );
    println!(
        "recent-chapters block: {recent_chars} chars; total budget = 2 × (4000 × 1.5) = 12,000"
    );
}

/// The Memory page promises to show what the writer receives (PROJECT §6.5): the API names the
/// chapters `block.recent_chapters` injects in full for the selected position (design §7), so the
/// page can state the variant it renders and the full-text chapters.
#[tokio::test]
async fn the_memory_page_names_the_chapters_injected_in_full() {
    let app = TestApp::new().await;
    let (book_id, track_id) = setup(&app).await;
    let first = official_chapter(&app, track_id, "第一章", "甲甲乙乙。", "第一章摘要。").await;
    let second = official_chapter(&app, track_id, "第二章", "丙丙丁丁。", "第二章摘要。").await;
    let target = app.create_chapter(track_id, "第三章").await;
    let target_id = target["id"].as_i64().expect("id") as i32;
    set_recent_settings(&app, book_id, 2, 4000).await;

    let (status, body) = app
        .get(&format!(
            "/api/books/{book_id}/memory?chapter_id={target_id}"
        ))
        .await;
    assert_eq!(status, 200, "memory view: {body}");
    let recent = body["recent_chapters"].as_array().expect("array");
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0]["chapter_id"], first);
    assert_eq!(recent[1]["chapter_id"], second);
    assert_eq!(recent[0]["label"], "第1章");
    assert_eq!(recent[0]["title"], "第一章");
    assert_eq!(recent[0]["characters"], 5);
    assert_eq!(recent[0]["partial"], false);
    // The page renders `block.memory`; the snapshot keeps its summaries.
    assert_eq!(
        body["snapshot"]["recent_summaries"][0]["summary"],
        "第一章摘要。"
    );
}

/// WS-05: the Memory page's `rendered` is the exact `block.memory` a writer kickoff receives, so
/// both apply the same `previous_tail` dedupe when `block.recent_chapters` repeats the chapter.
#[tokio::test]
async fn the_memory_page_rendered_text_matches_the_writers_memory_block() {
    let app = TestApp::new().await;
    let (book_id, track_id) = setup(&app).await;
    official_chapter(
        &app,
        track_id,
        "第一章",
        "雨落在城门口。\n\n他握紧了旧剑。",
        "第一章摘要。",
    )
    .await;
    let target = app.create_chapter(track_id, "第二章").await;
    let target_id = target["id"].as_i64().expect("id") as i32;
    set_recent_settings(&app, book_id, 2, 4000).await;

    let (status, body) = app
        .get(&format!(
            "/api/books/{book_id}/memory?chapter_id={target_id}"
        ))
        .await;
    assert_eq!(status, 200, "memory view: {body}");
    let rendered = body["rendered"].as_str().expect("rendered");
    assert!(
        !rendered.contains("### 上一章结尾"),
        "the page must drop previous_tail exactly like the writer:\n{rendered}"
    );
    assert!(
        body["snapshot"]["previous_tail"].is_null(),
        "the snapshot shown beside the text agrees: {body}"
    );

    let blocks = kickoff_blocks(
        &app,
        book_id,
        track_id,
        target_id,
        "writer",
        "chapter_write",
    )
    .await;
    let memory_block = blocks
        .iter()
        .find(|block| block.lines().next() == Some("# 记忆"))
        .expect("the writer's memory block");
    assert_eq!(
        rendered, memory_block,
        "the Memory page shows the exact block.memory text the writer receives"
    );
}
