//! The pre-goals baseline (PLAN M13 task 13.2, AC13, spec/03 §10).
//!
//! Every chapter-kind kickoff and every role's frozen tool list is snapshotted here **before**
//! D-150's code changes. The fixture book has all canon systems on (the only state that exists
//! before 0006) and no goal nodes; every later M13 task must leave these snapshots byte-identical.
//!
//! The fixture deliberately fills every `block.memory` fragment the pre-0006 pack can render:
//! short-term summaries, the previous tail, the protagonist's state and attributes, mid-term
//! changes and appearances, an absent important character, key notes, open foreshadowing, the cast
//! and the counts row. Without that, a conditional-template drift in 13.4 could pass unnoticed.
//!
//! Nothing in production code changed for this test. It drives the real `sessions::create` and
//! `context::rollover` paths over the Demo provider, exactly like the runtime tests do.

mod agent_harness;

use std::time::Duration;

use agent_harness::runtime;
use nekowriter::db::json::{Granularity, SessionKind};
use nekowriter::services::sessions::{self, CreateSession};
use nekowriter_llm::mock::ScriptStep;
use serde_json::json;

/// A provider step that never answers during the test: the kickoff is persisted in the session's
/// creation transaction, so the test reads it while the runner is still waiting on the model.
fn idle_provider() -> Vec<ScriptStep> {
    vec![ScriptStep::new().delay(Duration::from_secs(60))]
}

/// The fixture book: chapter ids 1–3 (1 and 2 official, 3 the target), canon data of every kind,
/// and no goal rows (the tables do not exist before 0006).
struct Baseline {
    runtime: agent_harness::Runtime,
    target_id: i32,
}

/// Creates the baseline fixture on a fresh temporary database.
///
/// The book's short-term window is 2 chapters and its mid-term window 1, so the fixture can fill
/// both the summary list and the "absent important character" fragment (a character seen in the
/// older prior chapter only).
async fn baseline() -> Baseline {
    let runtime = runtime(idle_provider()).await;

    // Chapter one is created by the harness; give every chapter a stable, meaningful title.
    let first_id = runtime.chapter_id;
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/chapters/{first_id}"),
            json!({ "title": "第一章 入城" }),
        )
        .await;
    assert_eq!(status, 200, "chapter one title: {body}");
    let second = runtime
        .app
        .create_chapter(runtime.track_id, "第二章 剑冢")
        .await;
    let second_id = second["id"].as_i64().expect("second chapter id") as i32;
    let third = runtime
        .app
        .create_chapter(runtime.track_id, "第三章 夜雨")
        .await;
    let target_id = third["id"].as_i64().expect("target chapter id") as i32;

    // The target chapter is a planned chapter with an accepted outline.
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/chapters/{target_id}"),
            json!({
                "guidance": "本章写雨夜中的对峙。",
                "outline": "细纲：沈砚在剑冢外遇见黑衣人，阿青在城外等候。",
                "reference_notes": "参考要点：旧剑不能出鞘。",
            }),
        )
        .await;
    assert_eq!(status, 200, "target chapter patch: {body}");

    // Setting materials: a full settings block for every kickoff.
    for (kind, content) in [
        ("outline", "# 全书大纲\n第一卷：入城\n第二卷：剑冢"),
        ("world", "# 世界观\n剑气可斩断寻常兵刃。\n剑冢内无法御剑。"),
        ("protagonist", "# 主角\n沈砚，寡言的游侠，佩一柄旧剑。"),
    ] {
        let (status, body) = runtime
            .app
            .put(
                &format!("/api/books/{}/setting-docs/{kind}", runtime.book_id),
                json!({ "content": content, "base_revision": 0 }),
            )
            .await;
        assert_eq!(status, 200, "setting doc {kind}: {body}");
    }
    for (rank, (name, role_label, summary)) in [
        ("沈砚", "主角", "寡言的游侠"),
        ("阿青", "配角", "青云城药铺的学徒"),
    ]
    .into_iter()
    .enumerate()
    {
        let (status, body) = runtime
            .app
            .post(
                &format!("/api/books/{}/characters", runtime.book_id),
                json!({
                    "name": name,
                    "role_label": role_label,
                    "summary": summary,
                    "profile": format!("人物小传 {rank}：{name}。"),
                }),
            )
            .await;
        assert_eq!(status, 201, "character {name}: {body}");
    }

    // Two official prior chapters with text and summaries.
    let first_version = runtime
        .app
        .put_text(first_id, 0, "雨落在城门口。\n\n沈砚握紧了旧剑。")
        .await;
    assert_eq!(first_version["version"], 1);
    runtime.app.set_chapter_status(first_id, "official").await;
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET summary = '沈砚入城，阿青在城门口等他。' WHERE id = {first_id}"
        ))
        .await;
    let second_version = runtime
        .app
        .put_text(second_id, 0, "剑冢深处传来低语。\n\n阿青没有跟来。")
        .await;
    assert_eq!(second_version["version"], 1);
    runtime.app.set_chapter_status(second_id, "official").await;
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET summary = '两人闯入剑冢，发现黑衣人的踪迹。' WHERE id = {second_id}"
        ))
        .await;

    // The windows: two short-term summaries, one mid-term chapter.
    let (_, book) = runtime
        .app
        .get(&format!("/api/books/{}", runtime.book_id))
        .await;
    let mut settings = book["settings"].clone();
    settings["recent_summary_count"] = json!(2);
    settings["midterm_window"] = json!(1);
    let (status, body) = runtime
        .app
        .patch(
            &format!("/api/books/{}", runtime.book_id),
            json!({ "settings": settings }),
        )
        .await;
    assert_eq!(status, 200, "book settings: {body}");

    // Canon data of every kind (spec/06 §4). Raw rows keep the fixture deterministic and let it
    // live before the canon APIs of later milestones exist.
    runtime
        .app
        .execute(&format!(
            r#"
INSERT INTO entities(id, book_id, track_id, kind, name, aliases, summary, description, state,
                     attributes, importance, status, is_protagonist, first_chapter_id,
                     last_chapter_id, created_at, updated_at) VALUES
  (1, {book}, NULL, 'character', '沈砚', '["沈公子"]', '寡言的游侠', '早年离开师门。',
   '刚入剑冢，左臂有伤', '{{"修为":"筑基中期","佩剑":"旧剑"}}', 5, 'active', 1, {first}, {second},
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, {book}, NULL, 'character', '阿青', '[]', '青云城药铺的学徒', '与沈砚同门。',
   '在城外等候', '{{}}', 4, 'active', 0, {first}, {first},
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (3, {book}, NULL, 'character', '黑衣人', '[]', '身份不明的刺客', '', '下落不明', '{{}}',
   4, 'unknown', 0, NULL, NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (4, {book}, NULL, 'organization', '青云宗', '[]', '青云城第一大宗', '', '山门紧闭', '{{}}',
   3, 'active', 0, NULL, NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (5, {book}, NULL, 'location', '剑冢', '[]', '青云宗后山的墓葬', '剑气森然。', '深处有异动',
   '{{}}', 4, 'active', 0, NULL, {second}, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (6, {book}, NULL, 'item', '旧剑', '[]', '沈砚父亲留下的旧剑', '', '剑鞘已有裂纹', '{{}}',
   3, 'active', 0, NULL, NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO relations(id, book_id, track_id, source_id, target_id, kind, inverse_kind,
                      description, status, since_chapter_id, until_chapter_id, created_at,
                      updated_at) VALUES
  (1, {book}, NULL, 1, 2, '师兄', '师妹', '同门之谊', 'active', {first}, NULL,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

INSERT INTO appearances(chapter_id, entity_id, role, note) VALUES
  ({first}, 1, 'pov', '入城'),
  ({first}, 2, 'major', '在城门口等候'),
  ({second}, 1, 'pov', '进入剑冢'),
  ({second}, 5, 'minor', '剑冢深处');

INSERT INTO timeline_events(id, book_id, track_id, position, story_time, title, description,
                            chapter_id, importance, created_at, updated_at) VALUES
  (1001, {book}, NULL, 1.0, '天元历三千年春', '沈砚入城', '沈砚抵达青云城。', {first}, 4,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (1002, {book}, NULL, 2.0, '天元历三千年夏', '剑冢异动', '剑冢深处传来低语。', {second}, 3,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO timeline_event_entities(event_id, entity_id) VALUES (1001, 1), (1002, 1);

INSERT INTO memory_notes(id, book_id, track_id, kind, content, chapter_id, importance, status,
                         created_at, updated_at) VALUES
  (1, {book}, NULL, 'fact', '旧剑是沈砚父亲留下的唯一遗物。', {first}, 4, 'active',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, {book}, NULL, 'world_rule', '剑冢内无法御剑。', {second}, 3, 'active',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (3, {book}, NULL, 'fact', '阿青的身世将在第二卷揭晓。', NULL, 5, 'retired',
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO memory_note_entities(note_id, entity_id) VALUES (1, 1), (2, 5);

INSERT INTO foreshadowing(id, book_id, track_id, title, description, planned_payoff, status,
                          planted_chapter_id, resolved_chapter_id, resolution, importance,
                          created_at, updated_at) VALUES
  (1, {book}, NULL, '旧剑的来历', '剑鞘上的裂纹似乎藏着秘密。', '第二卷由沈砚之父的旧友说明',
   'open', {first}, NULL, '', 4, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
  (2, {book}, NULL, '阿青的身世', '阿青提及自己的父母。', '第一卷末揭晓',
   'resolved', {first}, {second}, '阿青是青云宗长老的遗孤。', 3,
   '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO foreshadowing_entities(foreshadowing_id, entity_id) VALUES (1, 1), (2, 2);

-- Ids are auto-assigned because creating the setting documents and characters above already
-- wrote change rows. Two of these form a revert pair, so the fragments must skip the reverted one.
INSERT INTO change_log(book_id, track_id, chapter_id, session_id, actor, target_type,
                       target_id, op, before, after, note, reverts_id, reverted_by_id,
                       created_at) VALUES
  ({book}, NULL, {second}, NULL, 'agent', 'entity', 1, 'update', NULL, NULL,
   '修为：炼气后期→筑基中期', NULL, NULL, '2026-01-02T00:00:00Z'),
  ({book}, NULL, {second}, NULL, 'agent', 'relation', 1, 'update', NULL, NULL,
   '关系：同门→生死之交', NULL, NULL, '2026-01-02T00:00:00Z'),
  ({book}, NULL, {first}, NULL, 'agent', 'entity', 2, 'update', NULL, NULL,
   '阿青在城外与沈砚道别', NULL, NULL, '2026-01-01T00:00:00Z'),
  ({book}, NULL, {second}, NULL, 'user', 'entity', 1, 'update', NULL, NULL,
   '误改的状态', NULL, NULL, '2026-01-03T00:00:00Z');
INSERT INTO change_log(book_id, track_id, chapter_id, session_id, actor, target_type,
                       target_id, op, before, after, note, reverts_id, reverted_by_id,
                       created_at)
  SELECT {book}, NULL, {second}, NULL, 'user', 'entity', 1, 'update', NULL, NULL, '',
         (SELECT id FROM change_log WHERE note = '误改的状态'), NULL, '2026-01-03T00:01:00Z';
UPDATE change_log SET reverted_by_id = (SELECT max(id) FROM change_log)
WHERE note = '误改的状态';
"#,
            book = runtime.book_id,
            first = first_id,
            second = second_id,
        ))
        .await;

    Baseline { runtime, target_id }
}

/// Adds drafted text to the target chapter (review, revise, maintenance, re-sync and rollover
/// kickoffs carry `block.chapter_draft`).
async fn draft_target(fixture: &Baseline) {
    let body = fixture
        .runtime
        .app
        .put_text(
            fixture.target_id,
            0,
            "夜色压下来。\n\n沈砚在剑冢外停住脚步。",
        )
        .await;
    assert_eq!(body["version"], 1, "target draft");
}

/// The kickoff text of a session through the production path.
async fn kickoff_text(fixture: &Baseline, session_id: i32) -> String {
    let messages = fixture.runtime.messages(session_id).await;
    let kickoff = messages.first().expect("the kickoff is the first message");
    assert_eq!(kickoff.kind, "kickoff");
    assert_eq!(kickoff.seq, 0);
    kickoff.text()
}

/// Creates a session of one kind and returns its id.
async fn create(fixture: &Baseline, kind: SessionKind) -> i32 {
    let (session_id, _warnings) = sessions::create(
        fixture.runtime.app.state(),
        CreateSession {
            kind,
            book_id: Some(fixture.runtime.book_id),
            chapter_id: Some(fixture.target_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create session");
    session_id
}

// ───────────────────────────── chapter-kind kickoffs ─────────────────────────────

/// `chapter_outline`: settings · memory · recent chapters · planner task.
#[tokio::test]
async fn outline_kickoff_is_pinned() {
    let fixture = baseline().await;
    let session_id = create(&fixture, SessionKind::ChapterOutline).await;
    insta::assert_snapshot!("outline_kickoff", kickoff_text(&fixture, session_id).await);
}

/// `chapter_write`: settings · skills (empty, dropped) · memory · recent chapters · draft · writer
/// task.
#[tokio::test]
async fn write_kickoff_is_pinned() {
    let fixture = baseline().await;
    let session_id = create(&fixture, SessionKind::ChapterWrite).await;
    insta::assert_snapshot!("write_kickoff", kickoff_text(&fixture, session_id).await);
}

/// `chapter_revise`: the same writing blocks with the revision task and its feedback.
#[tokio::test]
async fn revise_kickoff_is_pinned() {
    let fixture = baseline().await;
    draft_target(&fixture).await;
    fixture
        .runtime
        .app
        .execute(&format!(
            "INSERT INTO chapter_feedback(chapter_id, content_version, text, issues, mode, \
             created_at) VALUES ({}, 1, '对峙的节奏要放慢，补一段雨声。', \
             '[{{\"severity\":\"major\",\"category\":\"pacing\",\"pids\":[\"p2\"],\
             \"quote\":\"沈砚在剑冢外停住脚步。\",\"problem\":\"转折太快\",\
             \"suggestion\":\"先写雨，再写脚步\"}}]', 'revise', '2026-01-04T00:00:00Z')",
            fixture.target_id
        ))
        .await;
    let session_id = create(&fixture, SessionKind::ChapterRevise).await;
    insta::assert_snapshot!("revise_kickoff", kickoff_text(&fixture, session_id).await);
}

/// `chapter_review`: the writing blocks with the reviewer task.
#[tokio::test]
async fn review_kickoff_is_pinned() {
    let fixture = baseline().await;
    draft_target(&fixture).await;
    fixture
        .runtime
        .app
        .set_chapter_status(fixture.target_id, "in_review")
        .await;
    let session_id = create(&fixture, SessionKind::ChapterReview).await;
    insta::assert_snapshot!("review_kickoff", kickoff_text(&fixture, session_id).await);
}

/// `chapter_maintenance`: settings · memory · cast index · draft · archivist task.
#[tokio::test]
async fn maintenance_kickoff_is_pinned() {
    let fixture = baseline().await;
    draft_target(&fixture).await;
    fixture
        .runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET status = 'official', canon_status = 'pending' WHERE id = {}",
            fixture.target_id
        ))
        .await;
    let session_id = create(&fixture, SessionKind::ChapterMaintenance).await;
    insta::assert_snapshot!(
        "maintenance_kickoff",
        kickoff_text(&fixture, session_id).await
    );
}

/// `canon_seed`: settings · archivist seed task, with no chapter.
#[tokio::test]
async fn seed_kickoff_is_pinned() {
    let fixture = baseline().await;
    let (session_id, _warnings) = sessions::create(
        fixture.runtime.app.state(),
        CreateSession {
            kind: SessionKind::CanonSeed,
            book_id: Some(fixture.runtime.book_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create seed session");
    insta::assert_snapshot!("seed_kickoff", kickoff_text(&fixture, session_id).await);
}

/// `chapter_resync`: settings · memory · cast index · chapter diff · archivist task.
#[tokio::test]
async fn resync_kickoff_is_pinned() {
    let fixture = baseline().await;
    draft_target(&fixture).await;
    // The re-sync diff compares the last synced official version with the edited text.
    fixture
        .runtime
        .app
        .execute(&format!(
            r#"
INSERT INTO chapter_versions(chapter_id, version_no, kind, content, word_count, created_at)
VALUES ({target}, 1, 'official',
        '{{"paragraphs":[{{"pid":"p1","text":"夜色降临。"}},{{"pid":"p2","text":"沈砚停在剑冢外。"}}],"next_pid":3,"tail_open":false}}',
        14, '2026-01-03T00:00:00Z');
INSERT INTO chapter_versions(chapter_id, version_no, kind, content, word_count, created_at)
VALUES ({target}, 2, 'official',
        '{{"paragraphs":[{{"pid":"p1","text":"夜色压下来。"}},{{"pid":"p2","text":"沈砚在剑冢外停住脚步。"}}],"next_pid":3,"tail_open":false}}',
        18, '2026-01-04T00:00:00Z');
UPDATE chapters SET status = 'official', canon_status = 'stale',
       official_version_id = (SELECT id FROM chapter_versions WHERE chapter_id = {target} AND version_no = 2),
       synced_version_id = (SELECT id FROM chapter_versions WHERE chapter_id = {target} AND version_no = 1)
WHERE id = {target};
"#,
            target = fixture.target_id,
        ))
        .await;
    let session_id = create(&fixture, SessionKind::ChapterResync).await;
    insta::assert_snapshot!("resync_kickoff", kickoff_text(&fixture, session_id).await);
}

/// `rollover`: the continuation task of a writer child, with the draft and `block.memory`'s
/// previous tail but without a `block.recent_chapters` block.
#[tokio::test]
async fn rollover_kickoff_is_pinned() {
    let fixture = baseline().await;
    draft_target(&fixture).await;
    let (parent_id, _warnings) = sessions::create(
        fixture.runtime.app.state(),
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(fixture.runtime.book_id),
            chapter_id: Some(fixture.target_id),
            granularity: Some(Granularity::Paragraph),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create parent writer session");
    let parent = nekowriter::agent::session::load(fixture.runtime.app.db().conn(), parent_id)
        .await
        .expect("load parent session");
    nekowriter::agent::context::rollover(fixture.runtime.app.state(), &parent)
        .await
        .expect("roll over the writer session");

    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = fixture
        .runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT id FROM agent_sessions WHERE parent_session_id = ?",
            vec![parent_id.into()],
        ))
        .await
        .expect("query child")
        .expect("the rollover child exists");
    let child_id: i32 = row.try_get("", "id").expect("child id");
    insta::assert_snapshot!("rollover_kickoff", kickoff_text(&fixture, child_id).await);
}

/// The stored sources of a session's kickoff blocks, in kickoff order.
async fn kickoff_sources(fixture: &Baseline, session_id: i32) -> Vec<Option<String>> {
    let messages = fixture.runtime.messages(session_id).await;
    messages[0]
        .blocks()
        .into_iter()
        .map(|block| match block {
            nekowriter::db::json::StoredBlock::Text { source, .. } => source,
            _ => panic!("a kickoff carries only text blocks"),
        })
        .collect()
}

/// The generic kickoff loop carries the prompt key for every session kind, not only a writer.
#[tokio::test]
async fn maintenance_and_book_agent_kickoffs_carry_their_sources() {
    let fixture = baseline().await;
    draft_target(&fixture).await;
    fixture
        .runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET status = 'official', canon_status = 'pending' WHERE id = {}",
            fixture.target_id
        ))
        .await;
    let maintenance = create(&fixture, SessionKind::ChapterMaintenance).await;
    let sources = kickoff_sources(&fixture, maintenance).await;
    assert!(sources.iter().all(Option::is_some), "{sources:?}");
    assert!(
        sources
            .iter()
            .any(|source| source.as_deref() == Some("archivist.kickoff.maintenance")),
        "{sources:?}"
    );

    let (agent_id, _) = sessions::create(
        fixture.runtime.app.state(),
        CreateSession {
            kind: SessionKind::BookAgent,
            book_id: Some(fixture.runtime.book_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create book agent");
    let agent_sources = kickoff_sources(&fixture, agent_id).await;
    assert!(
        agent_sources.iter().all(Option::is_some),
        "{agent_sources:?}"
    );
    assert_eq!(
        agent_sources.last().and_then(|source| source.as_deref()),
        Some("assistant.kickoff")
    );
}

#[tokio::test]
async fn conception_kickoff_carries_its_source() {
    let fixture = baseline().await;
    let (session_id, _) = sessions::create(
        fixture.runtime.app.state(),
        CreateSession {
            kind: SessionKind::Conception,
            book_id: Some(fixture.runtime.book_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("create conception");
    assert_eq!(
        kickoff_sources(&fixture, session_id).await,
        vec![Some("conception.kickoff".to_owned())]
    );
}

// ───────────────────────────── frozen tool lists ─────────────────────────────

/// The frozen tool list of one session, read from the `agent_sessions.tools` column.
async fn frozen_tool_names(fixture: &Baseline, session_id: i32) -> Vec<String> {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let row = fixture
        .runtime
        .app
        .db()
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT tools FROM agent_sessions WHERE id = ?",
            vec![session_id.into()],
        ))
        .await
        .expect("query tools")
        .expect("session row");
    let raw: String = row.try_get("", "tools").expect("tools column");
    let tools: Vec<nekowriter_llm::ToolDef> =
        serde_json::from_str(&raw).expect("frozen tool definitions");
    tools.into_iter().map(|tool| tool.name).collect()
}

/// Every role's frozen tool list, in the order the session stores it. Six sessions cover the six
/// roles: a planner outline, a writer draft, a reviewer review, an archivist maintenance, a
/// conception session and a researcher session.
#[tokio::test]
async fn frozen_tool_lists_of_every_role_are_pinned() {
    let fixture = baseline().await;
    let app = fixture.runtime.app.state();

    // Planner, writer, reviewer and archivist sessions use separate chapters so the one-open-
    // writer-per-chapter invariant never interferes with the fixture.
    let planner_chapter = fixture
        .runtime
        .app
        .create_chapter(fixture.runtime.track_id, "Planner")
        .await;
    let planner_chapter_id = planner_chapter["id"].as_i64().expect("planner chapter") as i32;
    let (planner_session, _) = sessions::create(
        app,
        CreateSession {
            kind: SessionKind::ChapterOutline,
            book_id: Some(fixture.runtime.book_id),
            chapter_id: Some(planner_chapter_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("planner session");

    let writer_chapter = fixture
        .runtime
        .app
        .create_chapter(fixture.runtime.track_id, "Writer")
        .await;
    let writer_chapter_id = writer_chapter["id"].as_i64().expect("writer chapter") as i32;
    let (writer_session, _) = sessions::create(
        app,
        CreateSession {
            kind: SessionKind::ChapterWrite,
            book_id: Some(fixture.runtime.book_id),
            chapter_id: Some(writer_chapter_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("writer session");

    let reviewer_chapter = fixture
        .runtime
        .app
        .create_chapter(fixture.runtime.track_id, "Reviewer")
        .await;
    let reviewer_chapter_id = reviewer_chapter["id"].as_i64().expect("reviewer chapter") as i32;
    fixture
        .runtime
        .app
        .set_chapter_status(reviewer_chapter_id, "in_review")
        .await;
    let (reviewer_session, _) = sessions::create(
        app,
        CreateSession {
            kind: SessionKind::ChapterReview,
            book_id: Some(fixture.runtime.book_id),
            chapter_id: Some(reviewer_chapter_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("reviewer session");

    let (archivist_session, _) = sessions::create(
        app,
        CreateSession {
            kind: SessionKind::ChapterMaintenance,
            book_id: Some(fixture.runtime.book_id),
            chapter_id: Some(fixture.target_id),
            ..CreateSession::default()
        },
    )
    .await
    .expect("archivist session");

    // Conception: a separate conceiving book starts its session.
    let (status, conception_book) = fixture
        .runtime
        .app
        .post(
            "/api/books",
            json!({
                "title": "Conception Book",
                "idea": "an idea",
                "genre": "xianxia",
                "writing_language": "zh-CN",
                "start_conception": true,
            }),
        )
        .await;
    assert_eq!(status, 201, "conception book: {conception_book}");
    let conception_session = conception_book["session_id"]
        .as_i64()
        .expect("conception session") as i32;

    // Research: create a task on the fixture book and run it.
    let (status, task) = fixture
        .runtime
        .app
        .post(
            "/api/research-tasks",
            json!({
                "book_id": fixture.runtime.book_id,
                "scope_name": "Background",
                "topic": "sword care",
                "requirements": "how to oil a blade",
            }),
        )
        .await;
    assert_eq!(status, 201, "research task: {task}");
    let task_id = task["id"].as_i64().expect("task id") as i32;
    let (status, run) = fixture
        .runtime
        .app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, 201, "run research: {run}");
    let research_session = run["session_id"].as_i64().expect("research session") as i32;

    let mut roles = std::collections::BTreeMap::new();
    roles.insert(
        "archivist",
        frozen_tool_names(&fixture, archivist_session).await,
    );
    roles.insert(
        "conception",
        frozen_tool_names(&fixture, conception_session).await,
    );
    roles.insert(
        "planner",
        frozen_tool_names(&fixture, planner_session).await,
    );
    roles.insert(
        "researcher",
        frozen_tool_names(&fixture, research_session).await,
    );
    roles.insert(
        "reviewer",
        frozen_tool_names(&fixture, reviewer_session).await,
    );
    roles.insert("writer", frozen_tool_names(&fixture, writer_session).await);
    insta::assert_snapshot!(
        "frozen_tool_lists",
        serde_json::to_string_pretty(&roles).expect("tools JSON")
    );
}
