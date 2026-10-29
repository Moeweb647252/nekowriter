//! Hybrid retrieval (spec/07 §5): FTS5 keyword hits and vector KNN hits fused with reciprocal
//! rank fusion.
//!
//! ```text
//! hybrid(query, query_vector?, corpus, filters, limit):
//!   fts_hits ← FTS5 query (≤ 30, ranked by bm25)   # empty if no usable tokens
//!   vec_hits ← KNN (≤ 30 per §3)                   # empty if no query vector
//!   fused    ← RRF: score(d) = Σ 1 / (60 + rank_in_list(d))
//!   return top `limit` by fused score, each with a snippet
//! ```
//!
//! The FTS statement joins the source table and applies the filters before `ORDER BY rank LIMIT
//! 30`, so other books' documents never take result slots. Kind subsets and track visibility are
//! applied after KNN by joining `search_docs` (over-fetch `k = max(30, limit × 6)`, keep the best
//! 30, spec/07 §3). When there is no query vector (the embedder is not ready) only keyword
//! matching runs and the caller says so (spec/07 §5).

use std::collections::HashMap;

use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, Statement};

use crate::db::json::SearchDocKind;

use crate::db::entities::{knowledge_chunks, knowledge_entries, knowledge_scopes, search_docs};
use crate::error::AppError;

use super::fts;
use super::index::{self, KnnHit};

/// FTS hits considered per list (spec/07 §5).
pub const FTS_LIMIT: usize = 30;
/// Vector hits kept after the post-KNN filters (spec/07 §3).
pub const VEC_KEEP: usize = 30;
/// Reciprocal-rank-fusion constant (spec/07 §5).
pub const RRF_K: f64 = 60.0;
/// The ± window of a keyword snippet (spec/07 §5).
pub const SNIPPET_RADIUS: usize = 150;
/// The snippet length of a vector-only hit (spec/07 §5).
pub const VECTOR_SNIPPET_CHARS: usize = 300;

/// Which part of the book corpus a query targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookCorpus {
    /// `search_canon`: every kind except `passage` (spec/05 §3).
    Canon,
    /// `search_chapters`: `passage` only (spec/05 §3).
    Passage,
}

impl BookCorpus {
    fn as_filter(self) -> &'static str {
        match self {
            BookCorpus::Canon => "canon",
            BookCorpus::Passage => "passage",
        }
    }
}

/// The filter set of one book query (spec/07 §5).
#[derive(Debug, Clone)]
pub struct BookSearch<'a> {
    pub book_id: i32,
    pub corpus: BookCorpus,
    /// A kind subset (`search_canon`'s `kinds`); `None` means every kind of the corpus.
    pub kinds: Option<&'a [String]>,
    /// The session's isolated track: documents with this `track_id` are visible too. Shared
    /// documents (`track_id IS NULL`) are always visible (spec/06 §2, D-014).
    pub isolated_track: Option<i32>,
    pub limit: usize,
}

/// One fused book hit.
#[derive(Debug, Clone)]
pub struct BookHit {
    pub kind: String,
    pub ref_id: i32,
    pub ref_part: i32,
    pub title: String,
    pub body: String,
    pub track_id: Option<i32>,
    pub meta: String,
    pub snippet: String,
    /// The fused RRF score, for tests and ordering assertions.
    pub score: f64,
}

/// One fused knowledge hit: the best chunk of an entry (spec/07 §5).
#[derive(Debug, Clone)]
pub struct KnowledgeHit {
    pub entry_id: i32,
    pub scope_id: i32,
    pub scope_name: String,
    pub chunk_id: i32,
    pub heading: String,
    pub title: String,
    pub body: String,
    pub snippet: String,
    pub score: f64,
}

/// The fused result of one hybrid query.
#[derive(Debug, Clone)]
pub struct HybridHits<T> {
    pub hits: Vec<T>,
    /// False when no query vector was available: only keyword matching ran (spec/07 §5).
    pub vectors_available: bool,
}

// ───────────────────────────── book corpus ─────────────────────────────

/// Hybrid search over the book corpus (spec/07 §5).
pub async fn hybrid_book(
    conn: &impl ConnectionTrait,
    query: &str,
    query_vector: Option<&[f32]>,
    search: &BookSearch<'_>,
) -> Result<HybridHits<BookHit>, AppError> {
    let fts_ids = fts_book_ids(conn, query, search).await?;
    let mut vector_ids: Vec<i32> = Vec::new();
    if let Some(vector) = query_vector {
        vector_ids = knn_book_ids(conn, vector, search).await?;
    }
    let fused = rrf(&fts_ids, &vector_ids);
    if fused.is_empty() {
        return Ok(HybridHits {
            hits: Vec::new(),
            vectors_available: query_vector.is_some(),
        });
    }
    let ids: Vec<i32> = fused.iter().map(|(id, _)| *id).collect();
    let rows: HashMap<i32, search_docs::Model> = search_docs::Entity::find()
        .filter(search_docs::Column::Id.is_in(ids))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    let tokens = fts::query_tokens(query);
    let mut hits: Vec<BookHit> = Vec::new();
    // The grouping key: one result per source item (a chapter's passages collapse to its best
    // chunk, spec/07 §5).
    let mut seen: std::collections::HashSet<(String, i32)> = std::collections::HashSet::new();
    for (id, score) in fused {
        let Some(row) = rows.get(&id) else {
            continue;
        };
        if !matches_filters(row, search) {
            continue;
        }
        if !seen.insert((row.kind.as_str().to_owned(), row.ref_id)) {
            continue;
        }
        let snippet = snippet(&row.body, &tokens);
        hits.push(BookHit {
            kind: row.kind.as_str().to_owned(),
            ref_id: row.ref_id,
            ref_part: row.ref_part,
            title: row.title.clone(),
            body: row.body.clone(),
            track_id: row.track_id,
            meta: serde_json::to_string(&*row.meta).unwrap_or_else(|_| "{}".to_owned()),
            snippet,
            score,
        });
        if hits.len() >= search.limit {
            break;
        }
    }
    Ok(HybridHits {
        hits,
        vectors_available: query_vector.is_some(),
    })
}

/// True when a row passes the post-KNN filters (spec/07 §3).
fn matches_filters(row: &search_docs::Model, search: &BookSearch<'_>) -> bool {
    if row.book_id != search.book_id {
        return false;
    }
    let corpus_ok = match search.corpus {
        BookCorpus::Canon => row.kind != SearchDocKind::Passage,
        BookCorpus::Passage => row.kind == SearchDocKind::Passage,
    };
    if !corpus_ok {
        return false;
    }
    if let Some(kinds) = search.kinds
        && !kinds.iter().any(|kind| kind == row.kind.as_str())
    {
        return false;
    }
    // Track visibility: shared documents always, plus the session's isolated track (spec/06 §2).
    match row.track_id {
        None => true,
        Some(track_id) => search.isolated_track == Some(track_id),
    }
}

/// The FTS5 statement of one book query: filters before the rank limit (spec/07 §5).
async fn fts_book_ids(
    conn: &impl ConnectionTrait,
    query: &str,
    search: &BookSearch<'_>,
) -> Result<Vec<i32>, AppError> {
    let Some(query_string) = fts::query_string(query) else {
        return Ok(Vec::new());
    };
    let mut sql = String::from(
        "SELECT d.id AS id FROM fts_book JOIN search_docs d ON d.id = fts_book.rowid \
         WHERE fts_book MATCH ? AND d.book_id = ?",
    );
    let mut values = vec![query_string.into(), (search.book_id as i64).into()];
    match search.corpus {
        BookCorpus::Canon => sql.push_str(" AND d.kind <> 'passage'"),
        BookCorpus::Passage => sql.push_str(" AND d.kind = 'passage'"),
    }
    if let Some(kinds) = search.kinds
        && !kinds.is_empty()
    {
        let placeholders = vec!["?"; kinds.len()].join(", ");
        sql.push_str(&format!(" AND d.kind IN ({placeholders})"));
        for kind in kinds {
            values.push(kind.clone().into());
        }
    }
    // Track visibility must be applied *before* the rank limit, or another isolated track's private
    // documents take slots a shared session may not even see (spec/06 §2, spec/07 §5).
    match search.isolated_track {
        Some(track_id) => {
            sql.push_str(" AND (d.track_id IS NULL OR d.track_id = ?)");
            values.push((track_id as i64).into());
        }
        None => sql.push_str(" AND d.track_id IS NULL"),
    }
    sql.push_str(&format!(" ORDER BY bm25(fts_book) LIMIT {FTS_LIMIT}"));
    let rows = conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            &sql,
            values,
        ))
        .await?;
    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        ids.push(row.try_get::<i32>("", "id")?);
    }
    Ok(ids)
}

/// The vector hits of one book query, filtered after KNN and cut to the best 30 (spec/07 §3).
async fn knn_book_ids(
    conn: &impl ConnectionTrait,
    vector: &[f32],
    search: &BookSearch<'_>,
) -> Result<Vec<i32>, AppError> {
    let over_fetch = (search.limit * 6).max(VEC_KEEP);
    let hits = index::knn_book(
        conn,
        vector,
        search.book_id,
        search.corpus.as_filter(),
        over_fetch,
    )
    .await?;
    filter_knn(conn, hits, |row| matches_filters(row, search)).await
}

/// Loads the rows of the KNN hits, drops those that fail the filters, and keeps the best
/// [`VEC_KEEP`] in distance order (spec/07 §3).
async fn filter_knn(
    conn: &impl ConnectionTrait,
    hits: Vec<KnnHit>,
    keep: impl Fn(&search_docs::Model) -> bool,
) -> Result<Vec<i32>, AppError> {
    if hits.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i32> = hits.iter().map(|hit| hit.id).collect();
    let rows: HashMap<i32, search_docs::Model> = search_docs::Entity::find()
        .filter(search_docs::Column::Id.is_in(ids))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    let mut kept = Vec::new();
    for hit in hits {
        let Some(row) = rows.get(&hit.id) else {
            continue;
        };
        if !keep(row) {
            continue;
        }
        kept.push(hit.id);
        if kept.len() == VEC_KEEP {
            break;
        }
    }
    Ok(kept)
}

// ───────────────────────────── knowledge ─────────────────────────────

/// Hybrid search over the visible knowledge scopes. Partition keys support only `=`, so the KNN
/// runs once per scope and the results are merged (spec/07 §3).
pub async fn hybrid_knowledge(
    conn: &impl ConnectionTrait,
    query: &str,
    query_vector: Option<&[f32]>,
    scope_ids: &[i32],
    limit: usize,
) -> Result<HybridHits<KnowledgeHit>, AppError> {
    if scope_ids.is_empty() {
        return Ok(HybridHits {
            hits: Vec::new(),
            vectors_available: query_vector.is_some(),
        });
    }
    let fts_ids = fts_knowledge_ids(conn, query, scope_ids).await?;
    let mut vector_ids: Vec<i32> = Vec::new();
    if let Some(vector) = query_vector {
        let over_fetch = (limit * 6).max(VEC_KEEP);
        let mut merged: Vec<KnnHit> = Vec::new();
        for scope_id in scope_ids {
            merged.extend(index::knn_knowledge(conn, vector, *scope_id, over_fetch).await?);
        }
        // One KNN per scope; the merged list is the best hits across scopes (spec/07 §3).
        merged.sort_by(|a, b| {
            a.distance
                .partial_cmp(&b.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        merged.truncate(VEC_KEEP);
        vector_ids = merged.into_iter().map(|hit| hit.id).collect();
    }
    let fused = rrf(&fts_ids, &vector_ids);
    if fused.is_empty() {
        return Ok(HybridHits {
            hits: Vec::new(),
            vectors_available: query_vector.is_some(),
        });
    }
    let ids: Vec<i32> = fused.iter().map(|(id, _)| *id).collect();
    let chunks: HashMap<i32, knowledge_chunks::Model> = knowledge_chunks::Entity::find()
        .filter(knowledge_chunks::Column::Id.is_in(ids))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    let entry_ids: Vec<i32> = chunks.values().map(|row| row.entry_id).collect();
    let entries: HashMap<i32, knowledge_entries::Model> = knowledge_entries::Entity::find()
        .filter(knowledge_entries::Column::Id.is_in(entry_ids))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    let scopes: HashMap<i32, String> = knowledge_scopes::Entity::find()
        .filter(knowledge_scopes::Column::Id.is_in(scope_ids.to_vec()))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| (row.id, row.name))
        .collect();
    let tokens = fts::query_tokens(query);
    let mut hits: Vec<KnowledgeHit> = Vec::new();
    let mut seen: std::collections::HashSet<i32> = std::collections::HashSet::new();
    for (id, score) in fused {
        let Some(chunk) = chunks.get(&id) else {
            continue;
        };
        if !scope_ids.contains(&chunk.scope_id) {
            continue;
        }
        let Some(entry) = entries.get(&chunk.entry_id) else {
            continue;
        };
        if !seen.insert(entry.id) {
            continue;
        }
        let Some(scope_name) = scopes.get(&chunk.scope_id) else {
            continue;
        };
        let body = searchable_chunk_body(chunk);
        hits.push(KnowledgeHit {
            entry_id: entry.id,
            scope_id: chunk.scope_id,
            scope_name: scope_name.clone(),
            chunk_id: chunk.id,
            heading: chunk.heading.clone(),
            title: entry.title.clone(),
            body,
            snippet: snippet(&searchable_chunk_body(chunk), &tokens),
            score,
        });
        if hits.len() >= limit {
            break;
        }
    }
    Ok(HybridHits {
        hits,
        vectors_available: query_vector.is_some(),
    })
}

/// The FTS5 statement of a knowledge query (spec/07 §5).
async fn fts_knowledge_ids(
    conn: &impl ConnectionTrait,
    query: &str,
    scope_ids: &[i32],
) -> Result<Vec<i32>, AppError> {
    let Some(query_string) = fts::query_string(query) else {
        return Ok(Vec::new());
    };
    let placeholders = vec!["?"; scope_ids.len()].join(", ");
    let sql = format!(
        "SELECT c.id AS id FROM fts_knowledge JOIN knowledge_chunks c ON c.id = fts_knowledge.rowid \
         WHERE fts_knowledge MATCH ? AND c.scope_id IN ({placeholders}) \
         ORDER BY bm25(fts_knowledge) LIMIT {FTS_LIMIT}"
    );
    let mut values = vec![query_string.into()];
    for scope_id in scope_ids {
        values.push((*scope_id as i64).into());
    }
    let rows = conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            &sql,
            values,
        ))
        .await?;
    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        ids.push(row.try_get::<i32>("", "id")?);
    }
    Ok(ids)
}

/// The searchable text of a chunk: its heading path plus the text (spec/07 §1.1).
pub fn searchable_chunk_body(chunk: &knowledge_chunks::Model) -> String {
    if chunk.heading.is_empty() {
        chunk.text.clone()
    } else {
        format!("{}\n{}", chunk.heading, chunk.text)
    }
}

// ───────────────────────────── fusion and snippets ─────────────────────────────

/// Reciprocal rank fusion with `k = 60` (spec/07 §5). Rank is 1-based.
pub fn rrf(fts_ids: &[i32], vector_ids: &[i32]) -> Vec<(i32, f64)> {
    let mut scores: HashMap<i32, f64> = HashMap::new();
    for (rank, id) in fts_ids.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (RRF_K + (rank + 1) as f64);
    }
    for (rank, id) in vector_ids.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (RRF_K + (rank + 1) as f64);
    }
    let mut fused: Vec<(i32, f64)> = scores.into_iter().collect();
    // A stable order: descending score, then ascending id so equal scores are deterministic.
    fused.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    fused
}

/// A snippet computed from the original text (spec/07 §5): the first occurrence of the longest
/// matching query token with ±150 characters, snapped to sentence boundaries when possible; for
/// a vector-only hit, the first 300 characters.
pub fn snippet(body: &str, tokens: &[String]) -> String {
    let chars: Vec<char> = body.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    let mut ordered: Vec<&String> = tokens.iter().collect();
    ordered.sort_by_key(|token| std::cmp::Reverse(token.chars().count()));
    let mut found: Option<(usize, usize)> = None;
    for token in ordered {
        if let Some(position) = find_token(&chars, token) {
            found = Some((position, token.chars().count()));
            break;
        }
    }
    match found {
        Some((position, length)) => {
            let mut start = position.saturating_sub(SNIPPET_RADIUS);
            let mut end = (position + length + SNIPPET_RADIUS).min(chars.len());
            // Snap to the nearest sentence boundary when one is close (spec/07 §5).
            if let Some(boundary) = sentence_before(&chars, start, 80) {
                start = boundary;
            }
            if let Some(boundary) = sentence_after(&chars, end, 80) {
                end = boundary;
            }
            clean(&chars[start..end])
        }
        None => clean(&chars[..chars.len().min(VECTOR_SNIPPET_CHARS)]),
    }
}

/// The char index of the first case-insensitive (ASCII) occurrence of `token`.
fn find_token(chars: &[char], token: &str) -> Option<usize> {
    let needle: Vec<char> = token.chars().collect();
    if needle.is_empty() || needle.len() > chars.len() {
        return None;
    }
    for start in 0..=chars.len() - needle.len() {
        let candidate = &chars[start..start + needle.len()];
        if candidate
            .iter()
            .zip(&needle)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
        {
            return Some(start);
        }
    }
    None
}

fn is_sentence_end(ch: char) -> bool {
    matches!(ch, '。' | '！' | '？' | '!' | '?' | '.' | ';' | '；')
}

fn is_closing_quote(ch: char) -> bool {
    matches!(ch, '”' | '"' | '』' | '」' | '’' | '）')
}

/// The index just after the nearest sentence end at or before `position`, when within `window`.
fn sentence_before(chars: &[char], position: usize, window: usize) -> Option<usize> {
    let floor = position.saturating_sub(window);
    let mut found = None;
    let mut index = floor;
    while index < position {
        if is_sentence_end(chars[index]) {
            let mut end = index + 1;
            if end < chars.len() && is_closing_quote(chars[end]) {
                end += 1;
            }
            found = Some(end);
        }
        index += 1;
    }
    found
}

/// The index just after the nearest sentence end at or after `position`, when within `window`.
fn sentence_after(chars: &[char], position: usize, window: usize) -> Option<usize> {
    let ceiling = (position + window).min(chars.len());
    let mut index = position;
    while index < ceiling {
        if is_sentence_end(chars[index]) {
            let mut end = index + 1;
            if end < chars.len() && is_closing_quote(chars[end]) {
                end += 1;
            }
            return Some(end);
        }
        index += 1;
    }
    None
}

/// Collapses whitespace runs so a snippet stays one line in the result template.
fn clean(chars: &[char]) -> String {
    let mut text = String::new();
    let mut last_space = false;
    for ch in chars {
        if ch.is_whitespace() {
            if !last_space && !text.is_empty() {
                text.push(' ');
            }
            last_space = true;
        } else {
            text.push(*ch);
            last_space = false;
        }
    }
    text.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(query: &str) -> Vec<String> {
        fts::query_tokens(query)
    }

    #[test]
    fn rrf_ranks_a_hit_in_both_lists_first() {
        let fused = rrf(&[7, 2, 9], &[2, 7, 4]);
        assert_eq!(
            fused[0].0, 2,
            "a hit first in one list and second in the other wins"
        );
        assert_eq!(fused[1].0, 7);
        // 1/61 + 1/63 vs 1/62 + 1/61.
        assert!((fused[0].1 - (1.0 / 62.0 + 1.0 / 61.0)).abs() < 1e-12);
    }

    #[test]
    fn rrf_scores_a_single_list_by_rank() {
        let fused = rrf(&[], &[5, 6]);
        assert_eq!(fused[0].0, 5);
        assert!((fused[0].1 - 1.0 / 61.0).abs() < 1e-12);
        assert!((fused[1].1 - 1.0 / 62.0).abs() < 1e-12);
    }

    #[test]
    fn snippet_centres_on_the_longest_matching_token() {
        let body = format!(
            "{}剑冢{}",
            "前面的文字。".repeat(20),
            "后面的文字。".repeat(20)
        );
        let snippet = snippet(&body, &tokens("剑冢 禁制"));
        assert!(snippet.contains("剑冢"), "{snippet}");
        assert!(snippet.chars().count() <= 2 * SNIPPET_RADIUS + 10);
    }

    #[test]
    fn snippet_falls_back_to_the_first_300_characters() {
        let body = "字".repeat(500);
        let snippet = snippet(&body, &tokens("没有出现"));
        assert_eq!(snippet.chars().count(), VECTOR_SNIPPET_CHARS);
    }

    #[test]
    fn snippet_is_case_insensitive_for_ascii() {
        let body = format!("{}Sword Tomb{}", "x ".repeat(200), " y".repeat(200));
        let snippet = snippet(&body, &tokens("sword"));
        assert!(snippet.to_lowercase().contains("sword tomb"), "{snippet}");
    }

    #[test]
    fn snippet_snaps_to_sentence_boundaries() {
        let body = "第一句。第二句提到剑冢。第三句。";
        let snippet = snippet(body, &tokens("剑冢"));
        assert!(
            snippet.starts_with("第一句。") || snippet.starts_with("第二句"),
            "{snippet}"
        );
        assert!(snippet.ends_with('。'), "{snippet}");
    }

    #[test]
    fn empty_body_has_an_empty_snippet() {
        assert_eq!(snippet("", &tokens("anything")), "");
    }
}
