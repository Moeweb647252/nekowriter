//! M7 review: own boundary probes for chunking and the hash diff (spec/07 §1.1, §2, D-039).

use axum::http::StatusCode;
use nekowriter::db::entities::knowledge_chunks;
use nekowriter::search::chunk;
use nekowriter::services::indexer;
use nekowriter::testing::TestApp;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::json;

async fn scope(app: &TestApp, name: &str) -> i32 {
    let (status, body) = app
        .post("/api/knowledge/scopes", json!({ "name": name }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "scope: {body}");
    body["id"].as_i64().expect("scope id") as i32
}

async fn entry(app: &TestApp, scope_id: i32, title: &str, content: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/knowledge/scopes/{scope_id}/entries"),
            json!({ "title": title, "content": content }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "entry: {body}");
    body["id"].as_i64().expect("entry id") as i32
}

async fn chunks(app: &TestApp, entry_id: i32) -> Vec<knowledge_chunks::Model> {
    knowledge_chunks::Entity::find()
        .filter(knowledge_chunks::Column::EntryId.eq(entry_id))
        .order_by_asc(knowledge_chunks::Column::Ord)
        .all(app.db().conn())
        .await
        .expect("chunks")
}

/// Inserting a section *before* existing ones must keep every unchanged chunk on its own row.
#[tokio::test]
async fn a_new_leading_chunk_keeps_the_unchanged_chunks_on_their_rows() {
    let app = TestApp::new().await;
    let scope_id = scope(&app, "History").await;
    let entry_id = entry(
        &app,
        scope_id,
        "Dress",
        "# Alpha\n\nalpha text\n\n# Beta\n\nbeta text",
    )
    .await;
    indexer::run_once(app.state()).await.expect("indexer");
    let before = chunks(&app, entry_id).await;
    assert_eq!(before.len(), 2, "{before:?}");
    let alpha_id = before[0].id;
    let beta_id = before[1].id;
    assert!(before.iter().all(|row| row.embed_state == "done"));

    // Insert a new section before both existing ones.
    let (status, body) = app
        .patch(
            &format!("/api/knowledge/entries/{entry_id}"),
            json!({ "content": "# New\n\nnew text\n\n# Alpha\n\nalpha text\n\n# Beta\n\nbeta text" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "patch: {body}");

    let after = chunks(&app, entry_id).await;
    assert_eq!(after.len(), 3, "{after:?}");
    let alpha = after
        .iter()
        .find(|row| row.text.contains("alpha text"))
        .expect("the alpha chunk");
    let beta = after
        .iter()
        .find(|row| row.text.contains("beta text"))
        .expect("the beta chunk");
    assert_eq!(
        alpha.id, alpha_id,
        "the unchanged alpha chunk keeps its row: {after:?}"
    );
    assert_eq!(
        beta.id, beta_id,
        "the unchanged beta chunk keeps its row: {after:?}"
    );
    // A row whose text changed must await a new vector; a stale `done` would keep the old one.
    for row in &after {
        assert_eq!(
            row.content_hash,
            chunk::content_hash(&row.heading, &row.text),
            "hash matches its own content: {row:?}"
        );
    }
    let new_row = after
        .iter()
        .find(|row| row.text.contains("new text"))
        .expect("the new chunk");
    assert_eq!(new_row.embed_state, "pending", "new chunks are pending");
    assert_eq!(new_row.ord, 0);
    assert_eq!(alpha.ord, 1);
    assert_eq!(beta.ord, 2);
    assert_eq!(
        alpha.embed_state, "done",
        "the unchanged chunk keeps its vector state"
    );
}

// ───────────────────────────── pure chunking boundaries (spec/07 §1.1) ─────────────────────────────

use nekowriter::search::chunk::{
    CHUNK_TARGET_CHARS, HARD_SPLIT_CHARS, LONG_PARAGRAPH_CHARS, chunk_markdown, chunk_passages,
    content_hash, knowledge_embedding_input,
};

#[test]
fn accumulate_respects_the_target_including_the_join_newline() {
    // Two paragraphs of 349 + 350 characters joined by one newline are exactly the target.
    let a = "字".repeat(349);
    let b = "字".repeat(350);
    let chunks = chunk_markdown(&format!("# H\n\n{a}\n\n{b}"));
    assert_eq!(chunks.len(), 1, "349+1+350 = 700 fits: {chunks:?}");
    assert_eq!(chunks[0].text.chars().count(), CHUNK_TARGET_CHARS);

    // One more character no longer fits.
    let c = "字".repeat(351);
    let chunks = chunk_markdown(&format!("# H\n\n{a}\n\n{c}"));
    assert_eq!(chunks.len(), 2, "initial: {chunks:?}");
    assert_eq!(chunks[0].text.chars().count(), 349);
}

#[test]
fn a_paragraph_of_exactly_the_long_limit_is_not_split() {
    let at_limit = "字".repeat(LONG_PARAGRAPH_CHARS);
    let chunks = chunk_markdown(&at_limit);
    assert_eq!(
        chunks.len(),
        1,
        "exactly {LONG_PARAGRAPH_CHARS} is not split"
    );
    assert_eq!(chunks[0].text.chars().count(), LONG_PARAGRAPH_CHARS);
    assert_eq!(HARD_SPLIT_CHARS, LONG_PARAGRAPH_CHARS);
}

#[test]
fn one_over_the_limit_splits() {
    let over = "字".repeat(LONG_PARAGRAPH_CHARS + 1);
    let chunks = chunk_markdown(&over);
    assert_eq!(chunks.len(), 2, "{chunks:?}");
    assert_eq!(chunks[0].text.chars().count(), HARD_SPLIT_CHARS);
    assert_eq!(chunks[1].text.chars().count(), 1);
}

#[test]
fn a_split_prefers_the_last_sentence_end_and_keeps_the_closing_quote() {
    // A terminator at 700, then 300 filler chars, then more: the cut lands after 「。」
    let mut text = String::new();
    text.push_str(&"字".repeat(699));
    text.push_str("。"); // char 700 (1-based)
    text.push_str(&"字".repeat(400));
    text.push('。');
    text.push_str(&"字".repeat(100));
    let chunks = chunk_markdown(&text);
    assert!(chunks.len() >= 2, "{:?}", chunks.len());
    assert_eq!(
        chunks[0].text.chars().count(),
        700,
        "cut after the terminator"
    );
    assert!(chunks[0].text.ends_with('。'));

    // A closing quote right after the terminator is included in the cut.
    let mut quoted = String::new();
    quoted.push_str(&"字".repeat(698));
    quoted.push_str("。”"); // terminator + closing quote at 699–700
    quoted.push_str(&"字".repeat(400));
    let chunks = chunk_markdown(&quoted);
    assert!(chunks[0].text.ends_with("。”"), "{:?}", chunks[0].text);
    assert_eq!(chunks[0].text.chars().count(), 700);
}

#[test]
fn every_documented_terminator_is_a_split_point() {
    for terminator in ['。', '！', '？', '!', '?', '.', ';', '；'] {
        let mut text = String::new();
        text.push_str(&"a".repeat(500));
        text.push(terminator);
        text.push_str(&"a".repeat(600));
        let chunks = chunk_markdown(&text);
        assert!(chunks.len() >= 2, "{terminator} must split: {chunks:?}");
        assert!(
            chunks[0].text.ends_with(terminator),
            "{terminator}: {:?}",
            chunks[0].text
        );
    }
}

#[test]
fn chunks_have_no_overlap() {
    let text = (0..40)
        .map(|index| format!("sentence{index}。"))
        .collect::<String>();
    let chunks = chunk_markdown(&text);
    let rejoined: String = chunks
        .iter()
        .map(|chunk| chunk.text.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert_eq!(rejoined, text, "no character is repeated or dropped");
}

#[test]
fn heading_paths_nest_and_reset_on_a_higher_level() {
    let chunks =
        chunk_markdown("# A\n\na1\n\n## B\n\nb1\n\n### C\n\nc1\n\n## D\n\nd1\n\n# E\n\ne1");
    let headings: Vec<&str> = chunks.iter().map(|c| c.heading.as_str()).collect();
    assert_eq!(headings, vec!["A", "A > B", "A > B > C", "A > D", "E"]);
}

#[test]
fn the_embedding_input_uses_the_heading_or_the_title() {
    assert_eq!(knowledge_embedding_input("T", "H", "x"), "H\nx");
    assert_eq!(knowledge_embedding_input("T", "", "x"), "T\nx");
    assert_eq!(
        content_hash("H", "x"),
        blake3::hash("Hx".as_bytes()).to_hex().to_string(),
        "content_hash = blake3(heading + text)"
    );
}

#[test]
fn passages_cover_consecutive_paragraphs_without_overlap() {
    let paragraphs: Vec<(String, String)> = (1..=5)
        .map(|index| (format!("p{index}"), "字".repeat(300)))
        .collect();
    let chunks = chunk_passages(&paragraphs);
    let mut seen: Vec<String> = Vec::new();
    for chunk in &chunks {
        let count = chunk.text.chars().count();
        assert!(
            count <= CHUNK_TARGET_CHARS,
            "chunk of {count} chars: {:?}",
            chunk.pids
        );
        for pid in &chunk.pids {
            if seen.last() != Some(pid) {
                seen.push(pid.clone());
            } else {
                panic!("pid {pid} appears twice in a row: {seen:?}");
            }
        }
    }
    let all: Vec<String> = chunks.iter().flat_map(|c| c.pids.clone()).collect();
    let mut unique = all.clone();
    unique.dedup();
    assert_eq!(unique, vec!["p1", "p2", "p3", "p4", "p5"], "{all:?}");
}
