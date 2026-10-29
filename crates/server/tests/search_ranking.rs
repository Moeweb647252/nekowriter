//! Hybrid ranking (PLAN 7.3, spec/07 §5): exact names win through FTS, paraphrases through the
//! vectors, RRF fuses the lists, the corpus/kind filters hold, and multi-scope knowledge search
//! merges one KNN per scope and groups chunks per entry.

use std::sync::Arc;

use axum::http::StatusCode;
use nekowriter::db::entities::knowledge_chunks;
use nekowriter::search::{
    BookCorpus, BookSearch, hybrid_book, hybrid_knowledge, index, maintenance,
};
use nekowriter::services::indexer;
use nekowriter::testing::TestApp;
use nekowriter_embed::{EmbedError, EmbedKind, Embedder, ModelEntry};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

/// The fake embedder of the tests maps known texts onto one-hot vectors and everything else onto a
/// different basis vector, so a test controls exactly which document a query vector matches.
struct MapEmbedder {
    entries: Vec<(String, Vec<f32>)>,
    dimension: usize,
}

impl MapEmbedder {
    fn new(entries: &[(&str, usize)]) -> Self {
        Self {
            entries: entries
                .iter()
                .map(|(text, axis)| {
                    let mut vector = vec![0.0f32; 384];
                    vector[*axis] = 1.0;
                    ((*text).to_owned(), vector)
                })
                .collect(),
            dimension: 384,
        }
    }
}

#[async_trait::async_trait]
impl Embedder for MapEmbedder {
    fn model(&self) -> &ModelEntry {
        nekowriter_embed::default_model()
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    async fn embed(
        &self,
        texts: Vec<String>,
        _kind: EmbedKind,
    ) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts
            .iter()
            .map(|text| {
                for (needle, vector) in &self.entries {
                    if text.contains(needle) {
                        return vector.clone();
                    }
                }
                // A distinct fallback basis so unrelated documents stay far apart.
                let mut vector = vec![0.0f32; self.dimension];
                vector[self.dimension - 1] = 1.0;
                vector
            })
            .collect())
    }
}

async fn book_with_track(app: &TestApp) -> (i32, i32) {
    let book = app.create_book("Ranking Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (status, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    assert_eq!(status, StatusCode::OK, "tracks: {tracks}");
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    (book_id, track_id)
}

async fn create_entity(app: &TestApp, book_id: i32, name: &str, summary: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            serde_json::json!({
                "kind": "character",
                "name": name,
                "summary": summary,
                "description": "",
                "importance": 3,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create entity: {body}");
    body["id"].as_i64().expect("entity id") as i32
}

async fn official_chapter(app: &TestApp, track_id: i32, title: &str, text: &str) -> i32 {
    let chapter = app.create_chapter(track_id, title).await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.put_text(chapter_id, 0, text).await;
    let (status, body) = app
        .post(
            &format!("/api/chapters/{chapter_id}/submit"),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "submit: {body}");
    let (status, body) = app
        .post(
            &format!("/api/chapters/{chapter_id}/approve"),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "approve: {body}");
    chapter_id
}

fn book_search(book_id: i32, corpus: BookCorpus, limit: usize) -> BookSearch<'static> {
    BookSearch {
        book_id,
        corpus,
        kinds: None,
        isolated_track: None,
        limit,
    }
}

/// An exact name is found by the keyword list when no query vector exists (spec/07 §5).
#[tokio::test]
async fn an_exact_name_wins_through_fts_without_an_embedder() {
    let mut app = TestApp::new().await;
    let (book_id, _track) = book_with_track(&app).await;
    let lin = create_entity(&app, book_id, "林凡", "the young cultivator").await;
    create_entity(&app, book_id, "苏婉", "the sword spirit").await;
    app.without_embedder();

    let query = "林凡";
    assert!(app.state().search().query_vector(query).await.is_none());
    let result = hybrid_book(
        app.db().conn(),
        query,
        None,
        &book_search(book_id, BookCorpus::Canon, 5),
    )
    .await
    .expect("hybrid");
    assert!(!result.vectors_available, "only keyword matching ran");
    assert!(!result.hits.is_empty(), "FTS found the name");
    assert_eq!(result.hits[0].kind, "entity");
    assert_eq!(result.hits[0].ref_id, lin);
    assert_eq!(result.hits[0].title, "林凡");
    assert!(
        result.hits[0].snippet.contains("林凡") || result.hits[0].snippet.contains("cultivator")
    );
}

/// A query whose tokens are absent from the index is found through the vector list only.
#[tokio::test]
async fn a_paraphrase_is_found_through_the_vectors() {
    let app = TestApp::new().await;
    let (book_id, _track) = book_with_track(&app).await;
    // The embedder maps the Chinese paraphrase onto the same axis as the English summary, which
    // is what a real multilingual model does; FTS cannot match across the languages.
    app.state().search().install(Arc::new(MapEmbedder::new(&[
        ("年轻剑修", 1),
        ("the young cultivator", 1),
        ("the sword spirit", 2),
    ])));
    assert!(
        app.state().search().ensure_ready().await,
        "the test embedder loads"
    );
    let lin = create_entity(&app, book_id, "Lin Fan", "the young cultivator").await;
    create_entity(&app, book_id, "Su Wan", "the sword spirit").await;
    let report = indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(report.stored, 2, "{report:?}");

    let query = "年轻剑修";
    let vector = app
        .state()
        .search()
        .query_vector(query)
        .await
        .expect("query vector");
    assert_eq!(
        index::meta_value(app.db().conn(), index::META_DIM)
            .await
            .expect("meta")
            .as_deref(),
        Some("384")
    );
    let result = hybrid_book(
        app.db().conn(),
        query,
        Some(&vector),
        &book_search(book_id, BookCorpus::Canon, 5),
    )
    .await
    .expect("hybrid");
    assert!(result.vectors_available);
    assert_eq!(
        result.hits.len(),
        2,
        "KNN returns the near-list; the ordering decides"
    );
    assert_eq!(
        result.hits[0].ref_id, lin,
        "the paraphrase matches Lin Fan first"
    );
    assert!(
        result.hits[0].snippet.contains("young cultivator"),
        "{}",
        result.hits[0].snippet
    );
    assert!(
        result.hits[0].score >= result.hits[1].score,
        "fused scores order the hits"
    );
}

/// A document that appears in both lists outranks a document that appears in one (RRF).
#[tokio::test]
async fn rrf_puts_a_document_ranked_by_both_lists_first() {
    let app = TestApp::new().await;
    let (book_id, _track) = book_with_track(&app).await;
    app.state().search().install(Arc::new(MapEmbedder::new(&[
        ("cultivator", 1),
        ("the young cultivator", 1),
        ("sword spirit", 2),
    ])));
    assert!(
        app.state().search().ensure_ready().await,
        "the test embedder loads"
    );
    let lin = create_entity(&app, book_id, "Lin Fan", "the young cultivator").await;
    let su = create_entity(&app, book_id, "Su Wan", "the cultivator's rival").await;
    let _ = su;
    indexer::run_once(app.state()).await.expect("indexer");
    let query = "cultivator";
    let vector = app
        .state()
        .search()
        .query_vector(query)
        .await
        .expect("vector");
    let result = hybrid_book(
        app.db().conn(),
        query,
        Some(&vector),
        &book_search(book_id, BookCorpus::Canon, 5),
    )
    .await
    .expect("hybrid");
    assert!(result.hits.len() >= 2, "{result:?}");
    assert_eq!(result.hits[0].ref_id, lin, "both lists name Lin Fan first");
}

/// The corpus split keeps passages out of `search_canon` and everything else out of
/// `search_chapters`; a kind subset narrows the keyword list too (spec/07 §5, D-030).
#[tokio::test]
async fn corpus_and_kind_filters_hold() {
    let app = TestApp::new().await;
    let (book_id, track_id) = book_with_track(&app).await;
    let entity = create_entity(&app, book_id, "Tomb Lord", "keeper of the ancient tomb").await;
    let chapter = official_chapter(
        &app,
        track_id,
        "The Tomb",
        "Lin Fan enters the ancient tomb and the swords hum.",
    )
    .await;
    indexer::run_once(app.state()).await.expect("indexer");
    let query = "tomb";

    let canon = hybrid_book(
        app.db().conn(),
        query,
        None,
        &book_search(book_id, BookCorpus::Canon, 10),
    )
    .await
    .expect("canon");
    assert!(
        canon.hits.iter().all(|hit| hit.kind != "passage"),
        "{:?}",
        canon.hits
    );
    assert!(canon.hits.iter().any(|hit| hit.ref_id == entity));

    let chapters = hybrid_book(
        app.db().conn(),
        query,
        None,
        &book_search(book_id, BookCorpus::Passage, 10),
    )
    .await
    .expect("chapters");
    assert!(chapters.hits.iter().all(|hit| hit.kind == "passage"));
    assert!(chapters.hits.iter().any(|hit| hit.ref_id == chapter));
    assert_eq!(chapters.hits[0].ref_part, 0);

    let kinds = vec!["entity".to_owned()];
    let filtered = hybrid_book(
        app.db().conn(),
        query,
        None,
        &BookSearch {
            kinds: Some(&kinds),
            ..book_search(book_id, BookCorpus::Canon, 10)
        },
    )
    .await
    .expect("kinds");
    assert!(
        filtered.hits.iter().all(|hit| hit.kind == "entity"),
        "{:?}",
        filtered.hits
    );
}

/// Knowledge search runs one KNN per scope, groups chunks per entry and reports the scope name
/// (spec/07 §5, §3).
#[tokio::test]
async fn knowledge_hits_are_grouped_per_entry_and_scoped() {
    let app = TestApp::new().await;
    app.execute(
        "INSERT INTO knowledge_scopes(id, name, description, created_at, updated_at) VALUES \
           (1, 'History', 'Song dynasty daily life', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'), \
           (2, 'Secret', 'Hidden tomb lore', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'); \
         INSERT INTO knowledge_entries(id, scope_id, title, content, source, created_at, updated_at) VALUES \
           (1, 1, 'Song dress', '# Song dress\n\nMen wore round-collar robes in the Song dynasty.\n\n## Colors\n\nBlue and white were common.', 'manual', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'), \
           (2, 2, 'Tomb secret', 'The tomb hides a sword spirit.', 'manual', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .await;
    for entry in [1, 2] {
        nekowriter::db::Db::write(app.db(), move |tx| {
            Box::pin(async move {
                maintenance::sync_knowledge_entry(tx, entry)
                    .await
                    .map_err(|err| sea_orm::DbErr::Custom(err.message))?;
                Ok(())
            })
        })
        .await
        .expect("chunk");
    }
    indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(
        knowledge_chunks::Entity::find()
            .filter(knowledge_chunks::COLUMN.embed_state.eq("done"))
            .count(app.db().conn())
            .await
            .expect("count"),
        3
    );

    // Query "Song dress" matches the FTS title of both chunks of entry 1; grouping keeps one hit.
    let result = hybrid_knowledge(app.db().conn(), "Song dress", None, &[1], 5)
        .await
        .expect("knowledge");
    assert_eq!(result.hits.len(), 1, "{result:?}");
    let hit = &result.hits[0];
    assert_eq!(hit.entry_id, 1);
    assert_eq!(hit.scope_name, "History");
    assert_eq!(hit.title, "Song dress");
    assert!(!hit.snippet.is_empty());
    assert!(hit.score > 0.0);

    // The hidden scope is not searched unless it is visible.
    let visible = hybrid_knowledge(app.db().conn(), "sword spirit", None, &[1], 5)
        .await
        .expect("knowledge");
    assert!(
        visible.hits.iter().all(|hit| hit.scope_id == 1),
        "{visible:?}"
    );
    let all = hybrid_knowledge(app.db().conn(), "sword spirit", None, &[1, 2], 5)
        .await
        .expect("knowledge");
    assert!(all.hits.iter().any(|hit| hit.scope_id == 2), "{all:?}");
}

/// Without an embedder the result survives on FTS alone and says so (spec/07 §5).
#[tokio::test]
async fn hybrid_search_works_without_an_embedder() {
    let mut app = TestApp::new().await;
    let (book_id, _track) = book_with_track(&app).await;
    create_entity(&app, book_id, "Lin Fan", "the protagonist").await;
    app.without_embedder();
    let result = hybrid_book(
        app.db().conn(),
        "Lin Fan",
        None,
        &book_search(book_id, BookCorpus::Canon, 5),
    )
    .await
    .expect("hybrid");
    assert!(!result.hits.is_empty());
    assert!(!result.vectors_available);
}
