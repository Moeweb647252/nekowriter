//! Setting proposals end to end (PLAN M6.4, spec/06 §5, spec/10 §2.3).
//!
//! The tests create proposals through a real maintenance session (the only creator, spec/06 §5)
//! and decide them through the API, asserting the applied text, the change log and the conflict
//! rules.

mod review_harness;

use std::time::Duration;

use nekowriter::db::json::SessionStatus;
use nekowriter_llm::mock::ScriptStep;
use review_harness::{
    chapter_state, count, create_entity, prepare_in_review, runtime, sessions_of_kind,
};
use serde_json::json;

fn step(name: &str, arguments: serde_json::Value) -> ScriptStep {
    ScriptStep::new().tool(name, arguments)
}

/// The required maintenance steps, then the given proposal steps, then `finish_maintenance`.
fn maintenance_with(proposal_steps: Vec<ScriptStep>) -> Vec<ScriptStep> {
    let mut steps = vec![
        step(
            "create_entity",
            json!({ "kind": "character", "name": "林凡", "summary": "主角" }),
        ),
        step(
            "record_appearances",
            json!({ "appearances": [ { "entity": "林凡", "role": "pov" } ] }),
        ),
        step(
            "set_chapter_summary",
            json!({ "summary": "林凡进入剑冢。" }),
        ),
    ];
    steps.extend(proposal_steps);
    steps.push(step("finish_maintenance", json!({ "report": "完成" })));
    steps
}

/// Approves the prepared chapter and waits for the maintenance run to finish.
async fn run_maintenance(runtime: &review_harness::base::Runtime) -> i32 {
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/chapters/{}/approve", runtime.chapter_id),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "approve: {body}");
    let session = body["maintenance_session_id"]
        .as_i64()
        .expect("maintenance started") as i32;
    runtime
        .wait_status_within(session, SessionStatus::Completed, Duration::from_secs(20))
        .await;
    session
}

/// Reads the world document.
async fn world_doc(runtime: &review_harness::base::Runtime) -> serde_json::Value {
    let (status, body) = runtime
        .app
        .get(&format!("/api/books/{}/setting-docs", runtime.book_id))
        .await;
    assert_eq!(status, 200, "setting docs: {body}");
    body.as_array()
        .expect("docs")
        .iter()
        .find(|doc| doc["kind"] == "world")
        .cloned()
        .expect("the world doc")
}

/// Writes the world document with the next revision.
async fn write_world(runtime: &review_harness::base::Runtime, content: &str) -> i32 {
    let doc = world_doc(runtime).await;
    let revision = doc["revision"].as_i64().expect("revision") as i32;
    let (status, body) = runtime
        .app
        .put(
            &format!("/api/books/{}/setting-docs/world", runtime.book_id),
            json!({ "content": content, "base_revision": revision }),
        )
        .await;
    assert_eq!(status, 200, "write world: {body}");
    body["revision"].as_i64().expect("revision") as i32
}

async fn proposals(
    runtime: &review_harness::base::Runtime,
    status: &str,
) -> Vec<serde_json::Value> {
    let (code, body) = runtime
        .app
        .get(&format!(
            "/api/books/{}/proposals?status={status}",
            runtime.book_id
        ))
        .await;
    assert_eq!(code, 200, "proposals: {body}");
    body.as_array().expect("proposals").clone()
}

// ───────────────────────────── append → accept ─────────────────────────────

/// Accepting an appended proposal edits the document, bumps its revision and records one author
/// change with the rationale as its note (spec/06 §3, §5).
#[tokio::test]
async fn accepting_an_append_applies_it_and_records_an_author_change() {
    let runtime = runtime(maintenance_with(vec![step(
        "propose_setting_change",
        json!({
            "target": "world",
            "append": "## 剑冢\n\n剑冢内无法御剑。",
            "rationale": "本章确立了剑冢的规则。",
        }),
    )]))
    .await;
    let revision = write_world(&runtime, "# 世界观\n\n青云宗立于云岭之上。").await;
    prepare_in_review(&runtime).await;
    let session = run_maintenance(&runtime).await;

    let pending = proposals(&runtime, "pending").await;
    assert_eq!(pending.len(), 1, "one pending proposal: {pending:?}");
    let id = pending[0]["id"].as_i64().expect("id") as i32;
    assert_eq!(pending[0]["target_type"], "setting_doc");
    assert_eq!(pending[0]["target_kind"], "world");
    assert_eq!(pending[0]["session_id"], session);
    assert!(
        pending[0]["preview"]["after"]
            .as_str()
            .unwrap_or_default()
            .contains("剑冢内无法御剑"),
        "preview.after shows the applied text: {pending:?}"
    );

    let changes_before = count(
        &runtime,
        "change_log",
        "target_type = 'setting_doc' AND actor = 'user'",
    )
    .await;
    let (status, body) = runtime
        .app
        .post(&format!("/api/proposals/{id}/accept"), json!({}))
        .await;
    assert_eq!(status, 200, "accept: {body}");
    assert_eq!(body["status"], "accepted");
    assert!(body["decided_at"].is_string());

    let doc = world_doc(&runtime).await;
    assert!(
        doc["content"]
            .as_str()
            .unwrap_or_default()
            .contains("剑冢内无法御剑"),
        "the append was applied: {doc}"
    );
    assert_eq!(
        doc["revision"],
        revision + 1,
        "the revision was bumped once by the accept"
    );
    let changes = count(
        &runtime,
        "change_log",
        "target_type = 'setting_doc' AND actor = 'user'",
    )
    .await;
    assert_eq!(
        changes,
        changes_before + 1,
        "one author change row for the accepted proposal"
    );
    let (_, changes) = runtime
        .app
        .get(&format!(
            "/api/books/{}/changes?target_type=setting_doc",
            runtime.book_id
        ))
        .await;
    let item = &changes["items"][0];
    assert_eq!(item["note"], "本章确立了剑冢的规则。", "note = rationale");
    assert_eq!(item["op"], "update");
    assert_eq!(item["actor"], "user");

    // A decided proposal cannot be accepted again.
    let (status, body) = runtime
        .app
        .post(&format!("/api/proposals/{id}/accept"), json!({}))
        .await;
    assert_eq!(status, 409, "second accept: {body}");
    assert_eq!(body["error"]["code"], "proposal.invalid_state");
    assert_eq!(body["error"]["details"]["status"], "accepted");
}

// ───────────────────────────── stale find → conflict ─────────────────────────────

/// A proposal whose `find` no longer matches becomes `conflict` and applies nothing; Reject then
/// dismisses it (spec/06 §5, D-044).
#[tokio::test]
async fn a_stale_find_becomes_a_conflict_and_reject_dismisses_it() {
    let runtime = runtime(maintenance_with(vec![step(
        "propose_setting_change",
        json!({
            "target": "world",
            "edits": [ { "find": "立于云岭之上", "replace": "立于云岭之巅" } ],
            "rationale": "位置应当更高。",
        }),
    )]))
    .await;
    write_world(&runtime, "# 世界观\n\n青云宗立于云岭之上。").await;
    prepare_in_review(&runtime).await;
    run_maintenance(&runtime).await;
    let pending = proposals(&runtime, "pending").await;
    let id = pending[0]["id"].as_i64().expect("id") as i32;
    assert!(
        pending[0]["preview"]["after"]
            .as_str()
            .unwrap_or_default()
            .contains("立于云岭之巅"),
        "preview applies the edit while it still matches: {pending:?}"
    );

    // The author edits the text the proposal points at.
    write_world(&runtime, "# 世界观\n\n青云宗早已迁离云岭。").await;
    let changes_before = count(
        &runtime,
        "change_log",
        "target_type = 'setting_doc' AND actor = 'user'",
    )
    .await;

    let (status, body) = runtime
        .app
        .post(&format!("/api/proposals/{id}/accept"), json!({}))
        .await;
    assert_eq!(
        status, 200,
        "accept reports the conflict in the body: {body}"
    );
    assert_eq!(body["status"], "conflict");
    let doc = world_doc(&runtime).await;
    assert_eq!(
        doc["content"], "# 世界观\n\n青云宗早已迁离云岭。",
        "nothing was applied"
    );
    assert_eq!(
        count(
            &runtime,
            "change_log",
            "target_type = 'setting_doc' AND actor = 'user'"
        )
        .await,
        changes_before,
        "no change row for a conflict"
    );

    // Reject dismisses the conflict (spec/10 §2.3).
    let (status, body) = runtime
        .app
        .post(&format!("/api/proposals/{id}/reject"), json!({}))
        .await;
    assert_eq!(status, 200, "reject: {body}");
    assert_eq!(body["status"], "rejected");
    let (status, body) = runtime
        .app
        .post(&format!("/api/proposals/{id}/reject"), json!({}))
        .await;
    assert_eq!(
        status, 409,
        "a rejected proposal cannot be rejected again: {body}"
    );
    assert_eq!(body["error"]["code"], "proposal.invalid_state");
}

// ───────────────────────────── tool validation ─────────────────────────────

/// The tool rejects a `find` that does not match exactly once, a new character whose name is
/// taken, and a call that provides two edit kinds (spec/05 §6).
#[tokio::test]
async fn propose_setting_change_validates_finds_names_and_exactly_one() {
    let runtime = runtime(maintenance_with(vec![
        step(
            "propose_setting_change",
            json!({
                "target": "world",
                "edits": [ { "find": "这句话不在世界观里", "replace": "x" } ],
                "rationale": "a stale find",
            }),
        ),
        step(
            "propose_setting_change",
            json!({
                "target": "new_character",
                "character": { "name": "柳照", "summary": "already a design" },
                "rationale": "a taken name",
            }),
        ),
        step(
            "propose_setting_change",
            json!({
                "target": "world",
                "edits": [ { "find": "# 世界观", "replace": "# 世界" } ],
                "append": "more",
                "rationale": "two kinds at once",
            }),
        ),
    ]))
    .await;
    write_world(&runtime, "# 世界观\n\n青云宗立于云岭之上。").await;
    // A character design the new-character proposal collides with.
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{}/characters", runtime.book_id),
            json!({ "name": "柳照", "role_label": "女主", "summary": "执剑司副使", "profile": "" }),
        )
        .await;
    assert_eq!(status, 201, "create design: {body}");
    prepare_in_review(&runtime).await;
    let session = run_maintenance(&runtime).await;

    let calls = runtime.calls(session).await;
    let proposes: Vec<_> = calls
        .iter()
        .filter(|call| call.name == "propose_setting_change")
        .collect();
    assert_eq!(proposes.len(), 3);
    assert!(proposes[0].is_error, "a missing find is an error");
    let text = proposes[0].raw_result_text.clone().unwrap_or_default();
    assert!(
        text.contains("这句话不在世界观里"),
        "error.find_failed names the find: {text:?}"
    );
    assert!(proposes[1].is_error, "a taken character name is an error");
    let text = proposes[1].raw_result_text.clone().unwrap_or_default();
    assert!(
        text.contains("柳照"),
        "error.character_exists names the character: {text:?}"
    );
    assert!(proposes[2].is_error, "exactly one edit kind is required");
    let text = proposes[2].raw_result_text.clone().unwrap_or_default();
    assert!(
        text.contains("edits") && text.contains("append"),
        "error.exactly_one lists the fields: {text:?}"
    );
    assert!(
        proposals(&runtime, "pending").await.is_empty(),
        "nothing was stored by the rejected proposals"
    );
    // The chapter's canon still synced: maintenance completed.
    assert_eq!(chapter_state(&runtime).await.1, "synced");
}

// ───────────────────────────── new character ─────────────────────────────

/// Accepting a new-character proposal creates the design at the end of the ranking; a name taken
/// before the accept turns it into `conflict` (spec/06 §5, D-052).
#[tokio::test]
async fn accepting_a_new_character_creates_the_design_or_conflicts() {
    let runtime = runtime(maintenance_with(vec![
        step(
            "propose_setting_change",
            json!({
                "target": "new_character",
                "character": {
                    "name": "黑衣人",
                    "role_label": "反派",
                    "summary": "章末出现的神秘人",
                    "profile": "# 黑衣人\n\n来历不明。",
                },
                "rationale": "该角色会长期出现。",
            }),
        ),
        step(
            "propose_setting_change",
            json!({
                "target": "new_character",
                "character": { "name": "苏婉", "summary": "后来被占用的名字" },
                "rationale": "将被抢先创建的角色。",
            }),
        ),
    ]))
    .await;
    prepare_in_review(&runtime).await;
    run_maintenance(&runtime).await;
    let pending = proposals(&runtime, "pending").await;
    assert_eq!(pending.len(), 2);
    let first = pending
        .iter()
        .find(|proposal| proposal["edits"]["character"]["name"] == "黑衣人")
        .expect("the first proposal");
    let second = pending
        .iter()
        .find(|proposal| proposal["edits"]["character"]["name"] == "苏婉")
        .expect("the second proposal");

    // Accept the first: a design is created at the end of the ranking.
    let (status, body) = runtime
        .app
        .post(
            &format!(
                "/api/proposals/{}/accept",
                first["id"].as_i64().expect("id")
            ),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "accept: {body}");
    assert_eq!(body["status"], "accepted");
    let (status, characters) = runtime
        .app
        .get(&format!("/api/books/{}/characters", runtime.book_id))
        .await;
    assert_eq!(status, 200, "characters: {characters}");
    let characters = characters.as_array().expect("characters");
    let created = characters
        .iter()
        .find(|character| character["name"] == "黑衣人")
        .expect("the design was created");
    assert_eq!(created["rank"], characters.len(), "rank = last");
    let changes = count(
        &runtime,
        "change_log",
        "target_type = 'character_design' AND actor = 'user'",
    )
    .await;
    assert_eq!(changes, 1);

    // The author takes the second name before the accept: the proposal conflicts.
    let (status, body) = runtime
        .app
        .post(
            &format!("/api/books/{}/characters", runtime.book_id),
            json!({ "name": "苏婉", "summary": "author's own", "profile": "" }),
        )
        .await;
    assert_eq!(status, 201, "create design: {body}");
    let (status, body) = runtime
        .app
        .post(
            &format!(
                "/api/proposals/{}/accept",
                second["id"].as_i64().expect("id")
            ),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "conflict accept: {body}");
    assert_eq!(body["status"], "conflict");
    let (status, characters) = runtime
        .app
        .get(&format!("/api/books/{}/characters", runtime.book_id))
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        characters
            .as_array()
            .expect("characters")
            .iter()
            .filter(|character| character["name"] == "苏婉")
            .count(),
        1,
        "the name is not duplicated"
    );
}

/// The maintenance session finishes before its proposals are decided: the chapter is synced.
#[tokio::test]
async fn maintenance_completes_independently_of_pending_proposals() {
    let runtime = runtime(maintenance_with(vec![step(
        "propose_setting_change",
        json!({ "target": "outline", "append": "## 新卷\n", "rationale": "新的一卷。" }),
    )]))
    .await;
    prepare_in_review(&runtime).await;
    run_maintenance(&runtime).await;
    assert_eq!(chapter_state(&runtime).await.1, "synced");
    assert_eq!(
        sessions_of_kind(&runtime, "chapter_maintenance")
            .await
            .len(),
        1
    );
    assert_eq!(proposals(&runtime, "pending").await.len(), 1);
    let _ = create_entity;
}
