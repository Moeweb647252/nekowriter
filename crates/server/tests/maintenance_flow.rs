//! Maintenance and re-sync end to end (PLAN M6.3, spec/06 §6, §8, spec/08 §1.1, spec/03 §14).
//!
//! The tests drive real archivist sessions over the scripted provider and assert the persisted
//! canon: change rows, appearances, summaries, the canon status and the versions.

mod review_harness;

use std::time::Duration;

use nekowriter::db::json::{SessionKind, SessionStatus};
use nekowriter_llm::mock::ScriptStep;
use review_harness::base::*;
use review_harness::{
    chapter_state, count, create_entity, prepare_in_review, runtime, sessions_of_kind,
    start_archivist,
};
use serde_json::json;

fn step(name: &str, arguments: serde_json::Value) -> ScriptStep {
    ScriptStep::new().tool(name, arguments)
}

fn create_character(name: &str, summary: &str) -> ScriptStep {
    step(
        "create_entity",
        json!({ "kind": "character", "name": name, "summary": summary }),
    )
}

fn create_location(name: &str, summary: &str) -> ScriptStep {
    step(
        "create_entity",
        json!({ "kind": "location", "name": name, "summary": summary }),
    )
}

/// A maintenance run that writes every kind of required row: two entities, an update, a
/// relation, appearances, a timeline event, a note, a foreshadowing item, the summary and a
/// setting proposal (spec/06 §6).
fn full_maintenance() -> Vec<ScriptStep> {
    vec![
        create_character("林凡", "身怀残破剑灵的青云宗弟子"),
        create_location("剑冢", "青云宗后山的墓葬"),
        step(
            "update_entity",
            json!({ "entity": "林凡", "state": "身处剑冢第三层，左臂受伤", "note": "进入剑冢后受伤" }),
        ),
        step(
            "set_relation",
            json!({ "source": "林凡", "target": "剑冢", "kind": "身处", "note": "本章进入剑冢" }),
        ),
        step(
            "record_appearances",
            json!({ "appearances": [
                { "entity": "林凡", "role": "pov" },
                { "entity": "剑冢", "role": "major" }
            ] }),
        ),
        step(
            "add_timeline_event",
            json!({ "title": "林凡入剑冢", "description": "林凡闯入剑冢第三层" }),
        ),
        step(
            "add_memory_note",
            json!({ "kind": "world_rule", "content": "剑冢内无法御剑" }),
        ),
        step(
            "add_foreshadowing",
            json!({ "title": "黑衣人的身份", "description": "章末出现的黑衣人", "planned_payoff": "第二卷揭晓" }),
        ),
        step(
            "set_chapter_summary",
            json!({ "summary": "林凡闯入剑冢第三层，遇见黑衣人，得知师父之死另有隐情。" }),
        ),
        step(
            "propose_setting_change",
            json!({
                "target": "world",
                "append": "## 剑冢\n\n剑冢内无法御剑，越深越甚。",
                "rationale": "本章确立了剑冢的这一规则。",
            }),
        ),
        step(
            "finish_maintenance",
            json!({ "report": "记录了本章的档案" }),
        ),
    ]
}

/// Approve the prepared chapter, which starts the maintenance session (`auto_maintenance` is on).
async fn approve_and_start_maintenance(runtime: &Runtime) -> i32 {
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/approve", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "approve: {body}");
    let session_id = body["maintenance_session_id"]
        .as_i64()
        .expect("approval starts maintenance") as i32;
    // The archivist model is configured in the harness, so no warning is expected.
    assert_eq!(body["warnings"], json!([]), "warnings: {body}");
    session_id
}

// ───────────────────────────── approve → maintenance → synced ─────────────────────────────

/// Approval starts `chapter_maintenance`; `finish_maintenance` compares `content_version` with the
/// session's base version and marks the canon synced, with every canon row written through the
/// tools (spec/06 §6, spec/05 §6, spec/03 §14).
#[tokio::test]
async fn maintenance_writes_every_canon_row_and_completes_synced() {
    let runtime = runtime(full_maintenance()).await;
    let version = prepare_in_review(&runtime).await;
    let session_id = approve_and_start_maintenance(&runtime).await;

    let (_, _, official, _, _) = chapter_state(&runtime).await;
    assert!(official.is_some(), "approval creates the official version");

    runtime
        .wait_status_within(
            session_id,
            SessionStatus::Completed,
            Duration::from_secs(20),
        )
        .await;

    let (status, canon, official_id, synced_id, current_version) = chapter_state(&runtime).await;
    assert_eq!(status, "official");
    assert_eq!(canon, "synced", "finish_maintenance synced the canon");
    assert_eq!(
        synced_id, official_id,
        "synced_version_id = official_version_id"
    );
    assert_eq!(
        current_version, version,
        "maintenance never touches the text version"
    );

    // Every call succeeded, and the change log carries the session's writes with actor agent.
    let calls = runtime.calls(session_id).await;
    assert_eq!(calls.len(), 11, "every scripted step ran");
    for call in &calls {
        assert!(
            !call.is_error,
            "{} failed: {:?}",
            call.name, call.raw_result_text
        );
        assert_eq!(call.status, "done", "{} status", call.name);
    }
    let (_, session) = runtime
        .app
        .get(&format!("/api/sessions/{session_id}"))
        .await;
    let appearance = session["tool_calls"]
        .as_array()
        .expect("tool calls")
        .iter()
        .find(|call| call["name"] == "record_appearances")
        .expect("record_appearances ran");
    assert_eq!(
        appearance["ui"]["count"],
        json!(2),
        "record_appearances carries the real appearance count: {appearance}"
    );
    let changes = count(
        &runtime,
        "change_log",
        &format!("session_id = {session_id} AND actor = 'agent'"),
    )
    .await;
    assert!(
        changes >= 9,
        "the canon writes recorded change rows per change: {changes}"
    );
    assert_eq!(
        count(
            &runtime,
            "appearances",
            &format!("chapter_id = {}", runtime.chapter_id)
        )
        .await,
        2,
        "record_appearances replaced the set"
    );
    assert_eq!(
        count(
            &runtime,
            "entities",
            &format!("book_id = {}", runtime.book_id)
        )
        .await,
        2,
        "two entities were created"
    );
    assert_eq!(
        count(
            &runtime,
            "timeline_events",
            &format!("book_id = {}", runtime.book_id)
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &runtime,
            "memory_notes",
            &format!("book_id = {}", runtime.book_id)
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &runtime,
            "foreshadowing",
            &format!("book_id = {}", runtime.book_id)
        )
        .await,
        1
    );

    // The chapter summary is stored; the proposal waits for the author (spec/06 §5).
    let (_, chapter) = runtime
        .app
        .get(&format!("/api/chapters/{}", runtime.chapter_id))
        .await;
    assert!(
        chapter["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("剑冢第三层"),
        "set_chapter_summary stored the summary: {chapter}"
    );
    let (status, proposals) = runtime
        .app
        .get(&format!(
            "/api/books/{}/proposals?status=pending",
            runtime.book_id
        ))
        .await;
    assert_eq!(status, 200, "proposals: {proposals}");
    let proposals = proposals.as_array().expect("proposals");
    assert_eq!(proposals.len(), 1, "one pending proposal: {proposals:?}");
    assert_eq!(proposals[0]["target_type"], "setting_doc");
    assert_eq!(proposals[0]["target_kind"], "world");
    assert_eq!(proposals[0]["session_id"], session_id);
    assert!(
        proposals[0]["preview"]["after"]
            .as_str()
            .unwrap_or_default()
            .contains("剑冢内无法御剑"),
        "the preview shows the appended passage: {proposals:?}"
    );
}

// ───────────────────────────── an author edit during maintenance → stale ─────────────────────────────

/// Author edits are allowed while an archivist works (spec/03 §14); the completion handler compares
/// the versions and marks the canon `stale` (spec/06 §6, D-026).
#[tokio::test]
async fn an_author_edit_during_maintenance_makes_canon_stale() {
    let runtime = runtime(vec![
        ScriptStep::new()
            .tool(
                "create_entity",
                json!({ "kind": "character", "name": "林凡", "summary": "主角" }),
            )
            .delay(Duration::from_millis(400)),
        step(
            "record_appearances",
            json!({ "appearances": [ { "entity": "林凡", "role": "pov" } ] }),
        ),
        step(
            "set_chapter_summary",
            json!({ "summary": "林凡进入剑冢。" }),
        ),
        step("finish_maintenance", json!({ "report": "完成" })),
    ])
    .await;
    let version = prepare_in_review(&runtime).await;
    let session_id = approve_and_start_maintenance(&runtime).await;

    // The author edits the text while maintenance runs: no writer session, so no lock.
    let (status, body) = runtime
        .app
        .put(
            &format!("/api/chapters/{}/content/text", runtime.chapter_id),
            json!({
                "base_version": version,
                "text": "第一段，改过的开头。\n\n第二段，他握紧了手中的旧剑。\n\n第三段，钟声敲了七下。",
            }),
        )
        .await;
    assert_eq!(
        status, 200,
        "an author edit during maintenance is allowed: {body}"
    );
    assert_eq!(
        chapter_state(&runtime).await.0,
        "official",
        "editing an official chapter keeps it official"
    );

    runtime
        .wait_status_within(
            session_id,
            SessionStatus::Completed,
            Duration::from_secs(20),
        )
        .await;
    let (status, canon, official_id, synced_id, current_version) = chapter_state(&runtime).await;
    assert_eq!(status, "official");
    assert_eq!(
        current_version,
        version + 1,
        "the edit advanced the version"
    );
    assert_eq!(
        canon, "stale",
        "a text change during maintenance makes the canon stale (D-026)"
    );
    assert_eq!(
        synced_id, official_id,
        "the canon still records which version it was synced to"
    );
}

// ───────────────────────────── abandon → failed → Retry ─────────────────────────────

/// Abandoning an open maintenance session sets `canon_status = failed` (D-048); Retry starts a new
/// session and can finish (spec/06 §6, spec/08 §1.1).
#[tokio::test]
async fn abandoning_maintenance_fails_canon_and_retry_starts_a_new_session() {
    let runtime = runtime(vec![
        ScriptStep::new()
            .tool(
                "create_entity",
                json!({ "kind": "character", "name": "林凡", "summary": "主角" }),
            )
            .delay(Duration::from_millis(2_000)),
        create_character("林凡", "主角"),
        step(
            "record_appearances",
            json!({ "appearances": [ { "entity": "林凡", "role": "pov" } ] }),
        ),
        step(
            "set_chapter_summary",
            json!({ "summary": "林凡进入剑冢。" }),
        ),
        step("finish_maintenance", json!({ "report": "完成" })),
    ])
    .await;
    runtime
        .app
        .put_text(runtime.chapter_id, 0, "第一段正文。\n\n第二段正文。")
        .await;
    review_harness::mark_official(&runtime, runtime.chapter_id).await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/maintain", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "maintain: {body}");
    let first = body["session_id"].as_i64().expect("session") as i32;
    assert_eq!(
        sessions_of_kind(&runtime, "chapter_maintenance")
            .await
            .len(),
        1
    );
    assert_eq!(chapter_state(&runtime).await.1, "syncing");

    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{first}/abandon"), json!({}))
        .await;
    assert_eq!(status, 200, "abandon");
    runtime
        .wait_status_within(first, SessionStatus::Abandoned, Duration::from_secs(10))
        .await;
    assert_eq!(
        chapter_state(&runtime).await.1,
        "failed",
        "an abandoned maintenance fails the canon (D-048)"
    );

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/maintain", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "retry: {body}");
    let retry = body["session_id"].as_i64().expect("session") as i32;
    assert_ne!(retry, first, "Retry starts a new session");
    assert_eq!(chapter_state(&runtime).await.1, "syncing");
    runtime
        .wait_status_within(retry, SessionStatus::Completed, Duration::from_secs(20))
        .await;
    assert_eq!(chapter_state(&runtime).await.1, "synced");
}

/// Retry continues an open **failed** session instead of starting another one (spec/08 §1.1).
#[tokio::test]
async fn maintain_continues_a_failed_open_session() {
    let runtime = runtime(vec![
        ScriptStep::new()
            .tool(
                "create_entity",
                json!({ "kind": "character", "name": "林凡", "summary": "主角" }),
            )
            .delay(Duration::from_millis(2_000)),
        create_character("林凡", "主角"),
        step(
            "record_appearances",
            json!({ "appearances": [ { "entity": "林凡", "role": "pov" } ] }),
        ),
        step(
            "set_chapter_summary",
            json!({ "summary": "林凡进入剑冢。" }),
        ),
        step("finish_maintenance", json!({ "report": "完成" })),
    ])
    .await;
    runtime
        .app
        .put_text(runtime.chapter_id, 0, "第一段正文。")
        .await;
    review_harness::mark_official(&runtime, runtime.chapter_id).await;
    let session = start_archivist(
        &runtime,
        runtime.chapter_id,
        SessionKind::ChapterMaintenance,
    )
    .await;
    // Stop the run and turn it into a failed session (the state spec/06 §6 names).
    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{session}/stop"), json!({}))
        .await;
    assert_eq!(status, 200, "stop");
    runtime
        .wait_status_within(session, SessionStatus::Stopped, Duration::from_secs(10))
        .await;
    runtime.app.set_session_status(session, "failed").await;
    // A failure leaves canon `failed` (spec/06 §6); the fixture's forced status bypasses the
    // transition that normally does it.
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET canon_status = 'failed' WHERE id = {}",
            runtime.chapter_id
        ))
        .await;

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/maintain", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "maintain: {body}");
    assert_eq!(
        body["session_id"], session,
        "Retry continues the open failed session"
    );
    // M5-04's positive path: the continue transaction must put canon back to `syncing` before
    // the runner completes (the first scripted step is delayed).
    assert_eq!(chapter_state(&runtime).await.1, "syncing");
    runtime
        .wait_status_within(session, SessionStatus::Completed, Duration::from_secs(20))
        .await;
    assert_eq!(chapter_state(&runtime).await.1, "synced");
}

// ───────────────────────────── isolated track ─────────────────────────────

/// In an isolated track the shared protagonist is read-only: the update is refused with
/// `error.isolated_readonly`, appearances may record shared entities, and `finish_maintenance`
/// does not ask for the state confirmation (spec/06 §2, §6, D-060).
#[tokio::test]
async fn isolated_maintenance_cannot_write_shared_canon() {
    let runtime = runtime(vec![
        step(
            "update_entity",
            json!({ "entity": "林凡", "state": "番外里的状态", "note": "试图修改正史" }),
        ),
        step(
            "record_appearances",
            json!({ "appearances": [ { "entity": "林凡", "role": "pov" } ] }),
        ),
        step(
            "set_chapter_summary",
            json!({ "summary": "林凡在番外中出场。" }),
        ),
        step("finish_maintenance", json!({ "report": "完成" })),
    ])
    .await;
    let shared = create_entity(&runtime, "林凡", true).await;
    let (status, body) = runtime.app.get(&format!("/api/entities/{shared}")).await;
    assert_eq!(status, 200, "entity: {body}");
    let original_state = body["state"].as_str().unwrap_or_default().to_owned();

    // The side track API is deferred to M10; the fixture inserts the track directly.
    runtime
        .app
        .execute(&format!(
            "INSERT INTO tracks(id, book_id, kind, title, synopsis, canon_mode, anchor_kind, ord, \
             created_at, updated_at) VALUES (901, {}, 'side', '番外', '', 'isolated', 'none', 1, \
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            runtime.book_id
        ))
        .await;
    let chapter = runtime.app.create_chapter(901, "番外一").await;
    let side_chapter = chapter["id"].as_i64().expect("chapter id") as i32;
    runtime.app.put_text(side_chapter, 0, "番外正文。").await;
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET status = 'official', canon_status = 'pending' WHERE id = {side_chapter}"
        ))
        .await;

    let (session_id, _) = nekowriter::services::sessions::create(
        runtime.app.state(),
        nekowriter::services::sessions::CreateSession {
            kind: SessionKind::ChapterMaintenance,
            book_id: Some(runtime.book_id),
            chapter_id: Some(side_chapter),
            ..Default::default()
        },
    )
    .await
    .expect("create isolated maintenance");
    runtime
        .wait_status_within(
            session_id,
            SessionStatus::Completed,
            Duration::from_secs(20),
        )
        .await;

    let calls = runtime.calls(session_id).await;
    let update = calls
        .iter()
        .find(|call| call.name == "update_entity")
        .expect("the update call");
    assert!(
        update.is_error,
        "a shared entity is read-only in an isolated track"
    );
    let text = update.raw_result_text.clone().unwrap_or_default();
    assert!(
        text.contains("独立番外"),
        "error.isolated_readonly explains the rule: {text:?}"
    );
    // The other calls succeeded and the canon is synced.
    for call in calls.iter().filter(|call| call.name != "update_entity") {
        assert!(
            !call.is_error,
            "{} failed: {:?}",
            call.name, call.raw_result_text
        );
    }
    let (_, side_body) = runtime
        .app
        .get(&format!("/api/chapters/{side_chapter}"))
        .await;
    assert_eq!(side_body["canon_status"], "synced");

    // The shared entity was not modified.
    let (status, after) = runtime.app.get(&format!("/api/entities/{shared}")).await;
    assert_eq!(status, 200);
    assert_eq!(
        after["state"].as_str().unwrap_or_default(),
        original_state,
        "the shared protagonist's state is unchanged"
    );
}

// ───────────────────────────── re-sync ─────────────────────────────

/// Re-sync first saves the edited text as a new `official` version, then starts `chapter_resync`
/// whose kickoff shows only the paragraph diff; completion sets `synced_version_id` (spec/06 §8).
#[tokio::test]
async fn resync_saves_an_official_version_and_shows_the_diff() {
    let mut steps = full_maintenance();
    // The re-sync archivist re-records appearances and the summary, then finishes.
    steps.push(step(
        "record_appearances",
        json!({ "appearances": [ { "entity": "林凡", "role": "pov" } ] }),
    ));
    steps.push(step(
        "set_chapter_summary",
        json!({ "summary": "林凡进入剑冢，遇见了黑衣人。" }),
    ));
    steps.push(step("finish_maintenance", json!({ "report": "已同步" })));
    let runtime = runtime(steps).await;
    let version = prepare_in_review(&runtime).await;
    let maintenance = approve_and_start_maintenance(&runtime).await;
    runtime
        .wait_status_within(
            maintenance,
            SessionStatus::Completed,
            Duration::from_secs(20),
        )
        .await;
    let (_, canon, first_official, first_synced, _) = chapter_state(&runtime).await;
    assert_eq!(canon, "synced");
    assert_eq!(first_synced, first_official);

    // The author edits the official text: canon becomes stale (spec/08 §1.1).
    let (status, body) = runtime
        .app
        .put(
            &format!("/api/chapters/{}/content/text", runtime.chapter_id),
            json!({
                "base_version": version,
                "text": "第一段，改过的开头。\n\n第二段，他握紧了手中的旧剑。\n\n第三段，钟声敲了七下。",
            }),
        )
        .await;
    assert_eq!(status, 200, "edit: {body}");
    assert_eq!(chapter_state(&runtime).await.1, "stale");

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/resync", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "resync: {body}");
    let resync = body["session_id"].as_i64().expect("session") as i32;
    assert_eq!(
        sessions_of_kind(&runtime, "chapter_resync").await,
        vec![(resync, "running".to_owned())]
    );
    let (_, _, second_official, _, _) = chapter_state(&runtime).await;
    assert_ne!(
        second_official, first_official,
        "resync saved the current text as a new official version"
    );

    // The kickoff carries only the diff (spec/06 §8).
    let kickoff = review_harness::kickoff_text(&runtime, resync).await;
    assert!(kickoff.contains("正文变更"), "diff block: {kickoff:?}");
    assert!(
        kickoff.contains("改过的开头"),
        "the inserted/edited paragraph is in the diff: {kickoff:?}"
    );
    assert!(
        !kickoff.contains("记录本章的档案"),
        "the kickoff is not the previous task"
    );

    runtime
        .wait_status_within(resync, SessionStatus::Completed, Duration::from_secs(20))
        .await;
    let (_, canon, official, synced, _) = chapter_state(&runtime).await;
    assert_eq!(canon, "synced");
    assert_eq!(
        synced, official,
        "completion sets synced_version_id to the new official version"
    );
    let versions = runtime.version_kinds().await;
    assert_eq!(
        versions.iter().filter(|kind| *kind == "official").count(),
        2,
        "two official versions: {versions:?}"
    );

    // An identical official version is reused (spec/06 §8, D-046): edit the text, restore the
    // official version (which brings back its exact content, pids included), then re-sync — no
    // third official version is created.
    let (_, _, version_now) = runtime.chapter_status().await;
    runtime
        .app
        .put_text(runtime.chapter_id, version_now, "另一段完全不同的正文。")
        .await;
    let (status, body) = runtime
        .app
        .post(
            &format!(
                "/api/chapter-versions/{}/restore",
                second_official.expect("the second official version")
            ),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "restore the official version: {body}");
    assert_eq!(chapter_state(&runtime).await.1, "stale");
    let (status, reused) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/resync", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "resync with the same content: {reused}");
    let (_, _, official_after_reuse, _, _) = chapter_state(&runtime).await;
    assert_eq!(
        official_after_reuse, second_official,
        "the identical official version is reused instead of a new one"
    );
    assert_eq!(
        runtime
            .version_kinds()
            .await
            .iter()
            .filter(|kind| *kind == "official")
            .count(),
        2,
        "still two official versions"
    );
    let reuse_session = reused["session_id"].as_i64().expect("session") as i32;
    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{reuse_session}/abandon"), json!({}))
        .await;
    assert_eq!(status, 200, "abandon the reuse re-sync");
}

/// Retry after an abandoned re-sync starts `chapter_resync` again, not a maintenance run
/// (spec/08 §1.1, D-044).
#[tokio::test]
async fn retry_after_an_abandoned_resync_starts_a_resync() {
    let mut steps = full_maintenance();
    // First re-sync: one slow step so the test can abandon it while it runs.
    steps.push(
        ScriptStep::new()
            .tool(
                "record_appearances",
                json!({ "appearances": [ { "entity": "林凡", "role": "pov" } ] }),
            )
            .delay(Duration::from_millis(2_000)),
    );
    // Retry re-sync.
    steps.push(step(
        "record_appearances",
        json!({ "appearances": [ { "entity": "林凡", "role": "pov" } ] }),
    ));
    steps.push(step(
        "set_chapter_summary",
        json!({ "summary": "林凡进入剑冢。" }),
    ));
    steps.push(step("finish_maintenance", json!({ "report": "已同步" })));
    let runtime = runtime(steps).await;
    let version = prepare_in_review(&runtime).await;
    let maintenance = approve_and_start_maintenance(&runtime).await;
    runtime
        .wait_status_within(
            maintenance,
            SessionStatus::Completed,
            Duration::from_secs(20),
        )
        .await;

    runtime
        .app
        .put(
            &format!("/api/chapters/{}/content/text", runtime.chapter_id),
            json!({ "base_version": version, "text": "第一段，改过的开头。" }),
        )
        .await;
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/resync", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "resync: {body}");
    let first = body["session_id"].as_i64().expect("session") as i32;
    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{first}/abandon"), json!({}))
        .await;
    assert_eq!(status, 200, "abandon the re-sync");
    runtime
        .wait_status_within(first, SessionStatus::Abandoned, Duration::from_secs(10))
        .await;
    assert_eq!(chapter_state(&runtime).await.1, "failed");

    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/maintain", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "retry: {body}");
    let retry = body["session_id"].as_i64().expect("session") as i32;
    assert_ne!(retry, first);
    let kinds = sessions_of_kind(&runtime, "chapter_resync").await;
    assert!(
        kinds.iter().any(|(id, _)| *id == retry),
        "Retry starts a re-sync again: {kinds:?}"
    );
    runtime
        .wait_status_within(retry, SessionStatus::Completed, Duration::from_secs(20))
        .await;
    assert_eq!(chapter_state(&runtime).await.1, "synced");
}

// ───────────────────────────── cast index ─────────────────────────────

/// `block.cast_index` lists up to 300 visible entities **with their aliases**, so the archivist
/// resolves a name it reads in the prose instead of creating a duplicate (spec/06 §4.5).
#[tokio::test]
async fn the_cast_index_caps_at_three_hundred_entities_and_includes_aliases() {
    let runtime = runtime(Vec::new()).await;
    // 305 entities: the cap must drop five, ordered by importance then id.
    for index in 1..=305 {
        let (status, body) = runtime
            .app
            .post(
                &format!("/api/books/{}/entities", runtime.book_id),
                json!({
                    "kind": "character",
                    "name": format!("角色{index:03}"),
                    "aliases": if index == 1 { json!(["小一", "阿一"]) } else { json!([]) },
                    "summary": "cast index fixture",
                }),
            )
            .await;
        assert_eq!(status, 201, "create entity {index}: {body}");
    }
    runtime.app.put_text(runtime.chapter_id, 0, "正文。").await;
    review_harness::mark_official(&runtime, runtime.chapter_id).await;
    let session = start_archivist(
        &runtime,
        runtime.chapter_id,
        SessionKind::ChapterMaintenance,
    )
    .await;
    let kickoff = review_harness::kickoff_text(&runtime, session).await;
    // Only the cast-index block: the memory block has a `- #` cast section of its own.
    let cast = kickoff
        .split("实体索引")
        .nth(1)
        .unwrap_or_else(|| panic!("the kickoff has a cast index: {kickoff:?}"));
    let entries = cast.matches("- #").count();
    assert_eq!(entries, 300, "the cast index is capped at 300 entries");
    assert!(
        kickoff.contains("小一") && kickoff.contains("阿一"),
        "aliases are listed so the archivist can resolve them: {kickoff:?}"
    );
    assert!(
        kickoff.contains("#1 "),
        "the first entity is in the index: {kickoff:?}"
    );
    assert!(
        !kickoff.contains("#301 "),
        "entities past the cap are dropped: {kickoff:?}"
    );

    // Stop the session: the test only inspects the kickoff.
    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{session}/abandon"), json!({}))
        .await;
    assert_eq!(status, 200);
}

// ───────────────────────────── finish_maintenance validation ─────────────────────────────

/// `finish_maintenance` in `chapter_maintenance` requires recorded appearances and a summary and
/// names what is missing (spec/05 §6).
#[tokio::test]
async fn finish_maintenance_names_the_missing_required_steps() {
    let runtime = runtime(vec![
        create_character("林凡", "主角"),
        // Too early: neither required step ran.
        step("finish_maintenance", json!({ "report": "done" })),
        step(
            "record_appearances",
            json!({ "appearances": [ { "entity": "林凡", "role": "pov" } ] }),
        ),
        step(
            "set_chapter_summary",
            json!({ "summary": "林凡进入剑冢。" }),
        ),
        step("finish_maintenance", json!({ "report": "done" })),
    ])
    .await;
    runtime.app.put_text(runtime.chapter_id, 0, "正文。").await;
    review_harness::mark_official(&runtime, runtime.chapter_id).await;
    let session = start_archivist(
        &runtime,
        runtime.chapter_id,
        SessionKind::ChapterMaintenance,
    )
    .await;
    runtime
        .wait_status_within(session, SessionStatus::Completed, Duration::from_secs(20))
        .await;

    let calls = runtime.calls(session).await;
    let finishes: Vec<_> = calls
        .iter()
        .filter(|call| call.name == "finish_maintenance")
        .collect();
    assert_eq!(finishes.len(), 2);
    assert!(finishes[0].is_error, "the first finish is rejected");
    let text = finishes[0].raw_result_text.clone().unwrap_or_default();
    assert!(
        text.contains("出场记录") && text.contains("本章摘要"),
        "error.maintenance_incomplete names both missing steps: {text:?}"
    );
    assert!(!finishes[1].is_error, "the second finish completes");
    assert_eq!(chapter_state(&runtime).await.1, "synced");
}

/// If the protagonist appeared and no `update_entity` touched their state, the first
/// `finish_maintenance` withholds completion once; the next call completes (spec/05 §6, spec/06 §6).
#[tokio::test]
async fn finish_maintenance_asks_for_the_protagonist_state_once() {
    let runtime = runtime(vec![
        step(
            "record_appearances",
            json!({ "appearances": [ { "entity": "林凡", "role": "pov" } ] }),
        ),
        step(
            "set_chapter_summary",
            json!({ "summary": "林凡进入剑冢。" }),
        ),
        step("finish_maintenance", json!({ "report": "done" })),
        step(
            "finish_maintenance",
            json!({ "report": "no state change needed" }),
        ),
    ])
    .await;
    create_entity(&runtime, "林凡", true).await;
    runtime.app.put_text(runtime.chapter_id, 0, "正文。").await;
    review_harness::mark_official(&runtime, runtime.chapter_id).await;
    let session = start_archivist(
        &runtime,
        runtime.chapter_id,
        SessionKind::ChapterMaintenance,
    )
    .await;
    runtime
        .wait_status_within(session, SessionStatus::Completed, Duration::from_secs(20))
        .await;

    let calls = runtime.calls(session).await;
    let finishes: Vec<_> = calls
        .iter()
        .filter(|call| call.name == "finish_maintenance")
        .collect();
    assert_eq!(finishes.len(), 2, "the confirmation costs exactly one call");
    assert!(
        !finishes[0].is_error,
        "the withheld completion is a notice, not an error"
    );
    let text = finishes[0].raw_result_text.clone().unwrap_or_default();
    assert!(
        text.contains("主角") && text.contains("state"),
        "the notice asks for the protagonist's state: {text:?}"
    );
    assert!(!finishes[1].is_error);
    assert_eq!(
        chapter_state(&runtime).await.1,
        "synced",
        "the second call completed the maintenance"
    );
}

/// A failed archivist session leaves the canon `failed` so Retry is possible (spec/06 §6, D-048).
#[tokio::test]
async fn a_failed_maintenance_session_marks_canon_failed() {
    use nekowriter_llm::mock::ScriptedProvider;
    use nekowriter_llm::retry::LlmError;

    let mut runtime = runtime(Vec::new()).await;
    let scripted = runtime.app.set_provider(
        ScriptedProvider::new(vec![create_character("林凡", "主角")])
            // A non-retryable failure: the session fails on the first attempt.
            .fail_when_exhausted(LlmError::Auth("boom".to_owned())),
    );
    runtime.scripted = scripted;
    runtime.app.put_text(runtime.chapter_id, 0, "正文。").await;
    review_harness::mark_official(&runtime, runtime.chapter_id).await;
    let session = start_archivist(
        &runtime,
        runtime.chapter_id,
        SessionKind::ChapterMaintenance,
    )
    .await;
    runtime
        .wait_status_within(session, SessionStatus::Failed, Duration::from_secs(40))
        .await;
    assert_eq!(
        chapter_state(&runtime).await.1,
        "failed",
        "a failed maintenance fails the canon in the same transition"
    );
}

/// `error.not_found`'s `what_label` is model-facing text: it must come from `shared.label`
/// (`prompts.label("target_type", …)`), never a literal in code (spec/09 §1, AGENTS §2).
#[tokio::test]
async fn unknown_canon_ids_render_the_target_label_from_the_prompt_pack() {
    let runtime = runtime(vec![
        create_character("林凡", "主角"),
        step(
            "update_timeline_event",
            json!({ "event_id": 999_001, "title": "不存在的事件" }),
        ),
        step(
            "retire_memory_note",
            json!({ "note_id": 999_002, "reason": "不存在" }),
        ),
        step(
            "resolve_foreshadowing",
            json!({ "foreshadowing_id": 999_003, "status": "resolved", "resolution": "不存在" }),
        ),
        step(
            "record_appearances",
            json!({ "appearances": [ { "entity": "林凡", "role": "pov" } ] }),
        ),
        step(
            "set_chapter_summary",
            json!({ "summary": "林凡进入剑冢。" }),
        ),
        step("finish_maintenance", json!({ "report": "done" })),
    ])
    .await;
    runtime.app.put_text(runtime.chapter_id, 0, "正文。").await;
    review_harness::mark_official(&runtime, runtime.chapter_id).await;
    let session = start_archivist(
        &runtime,
        runtime.chapter_id,
        SessionKind::ChapterMaintenance,
    )
    .await;
    runtime
        .wait_status_within(session, SessionStatus::Completed, Duration::from_secs(20))
        .await;

    let calls = runtime.calls(session).await;
    let expect_label = |name: &str, label: &str, id: i64| {
        let call = calls
            .iter()
            .find(|call| call.name == name)
            .unwrap_or_else(|| panic!("{name} ran"));
        assert!(call.is_error, "{name} is an error");
        let text = call.raw_result_text.clone().unwrap_or_default();
        assert!(
            text.contains(label) && text.contains(&id.to_string()),
            "{name} renders the {label} label and id {id}: {text:?}"
        );
    };
    // The labels come from `shared.label` (`target_type`) via the prompt pack.
    expect_label("update_timeline_event", "时间线事件", 999_001);
    expect_label("retire_memory_note", "记忆笔记", 999_002);
    expect_label("resolve_foreshadowing", "伏笔", 999_003);
    assert_eq!(chapter_state(&runtime).await.1, "synced");
}

/// A failed archivist session that is abandoned between `maintain`'s open-session read and its
/// `syncing` write must not wedge canon (M5-04): the status update and the continue validation
/// share one transaction.
#[tokio::test]
async fn maintain_does_not_leave_syncing_when_the_session_is_abandoned() {
    use nekowriter::db::Db;
    use sea_orm::ConnectionTrait;
    use tokio::sync::oneshot;

    let runtime = runtime(vec![
        create_character("林凡", "主角"),
        step("finish_maintenance", json!({ "report": "完成" })),
    ])
    .await;
    runtime
        .app
        .put_text(runtime.chapter_id, 0, "第一段正文。")
        .await;
    review_harness::mark_official(&runtime, runtime.chapter_id).await;
    let session = start_archivist(
        &runtime,
        runtime.chapter_id,
        SessionKind::ChapterMaintenance,
    )
    .await;
    let (status, _) = runtime
        .app
        .post(&format!("/api/sessions/{session}/stop"), json!({}))
        .await;
    assert_eq!(status, 200);
    runtime
        .wait_status_within(session, SessionStatus::Stopped, Duration::from_secs(10))
        .await;
    runtime.app.set_session_status(session, "failed").await;
    runtime
        .app
        .execute(&format!(
            "UPDATE chapters SET canon_status = 'failed' WHERE id = {}",
            runtime.chapter_id
        ))
        .await;

    let (begun_tx, begun_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let db = runtime.app.db().clone();
    let chapter_id = runtime.chapter_id;
    let updater = tokio::spawn(async move {
        Db::write(&db, move |tx| {
            Box::pin(async move {
                tx.execute_unprepared(&format!(
                    "UPDATE agent_sessions SET status = 'abandoned' WHERE id = {session}"
                ))
                .await?;
                let _ = begun_tx.send(());
                let _ = release_rx.await;
                Ok(())
            })
        })
        .await
        .expect("the abandon commits");
    });
    begun_rx.await.expect("the updater holds the write lock");
    let state = runtime.app.state().clone();
    let maintain = tokio::spawn(async move {
        nekowriter::services::maintenance::maintain(&state, chapter_id).await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    release_tx.send(()).expect("release the updater");
    let error = maintain
        .await
        .expect("the maintain task did not panic")
        .expect_err("the abandoned session cannot be continued");
    assert_eq!(
        error.code,
        nekowriter::error::ErrorCode::SessionInvalidState
    );
    updater.await.expect("updater task");
    assert_eq!(
        chapter_state(&runtime).await.1,
        "failed",
        "canon is not left syncing"
    );
    let status: String = runtime
        .app
        .db()
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            format!("SELECT status FROM agent_sessions WHERE id = {session}"),
        ))
        .await
        .expect("query session")
        .expect("row")
        .try_get("", "status")
        .expect("status");
    assert_eq!(status, "abandoned");
}
