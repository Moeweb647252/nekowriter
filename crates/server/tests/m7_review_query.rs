//! M7 review: hybrid-query conformance probes (spec/07 §5, §3, D-030).

use nekowriter::search::{BookCorpus, BookSearch, hybrid_book};
use nekowriter::testing::TestApp;
use sea_orm::ConnectionTrait;

async fn count(app: &TestApp, sql: &str) -> i64 {
    let row = app
        .db()
        .conn()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql,
        ))
        .await
        .expect("count query")
        .expect("count row");
    row.try_get::<i64>("", "c").expect("count value")
}

/// A shared session must not let an isolated track's private documents take FTS result slots
/// (spec/07 §5: the FTS statement applies the track-visibility filter before `LIMIT 30`).
#[tokio::test]
async fn private_documents_do_not_take_fts_slots_of_a_shared_search() {
    let app = TestApp::new().await;
    let book = app.create_book("Slot Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    // A side track with 40 private documents that all match the query, each ranked above the one
    // shared document (the shared one matches the query only in a longer body).
    app.execute(&format!(
        "INSERT INTO tracks(id, book_id, kind, title, canon_mode, ord, created_at, updated_at) \
         VALUES (900, {book_id}, 'side', 'Side', 'isolated', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    let mut sql = String::new();
    for id in 1..=40 {
        sql.push_str(&format!(
            "INSERT INTO search_docs(id, book_id, track_id, kind, ref_id, title, body, content_hash, updated_at) \
             VALUES ({id}, {book_id}, 900, 'entity', {id}, 'Secret{id}', 'secret keeper hidden lore', 'h{id}', '2026-01-01T00:00:00Z'); \
             INSERT INTO fts_book(rowid, title, body) VALUES ({id}, 'secret{id}', 'secret keeper hidden lore');"
        ));
    }
    sql.push_str(&format!(
        "INSERT INTO search_docs(id, book_id, track_id, kind, ref_id, title, body, content_hash, updated_at) \
         VALUES (500, {book_id}, NULL, 'entity', 500, 'Shared', '{padded}', 'h500', '2026-01-01T00:00:00Z'); \
         INSERT INTO fts_book(rowid, title, body) VALUES (500, 'shared', '{padded}')",
        padded = format!("shared secret keeper {}", "filler ".repeat(200)),
    ));
    app.execute(&sql).await;
    assert_eq!(
        count(&app, "SELECT count(*) AS c FROM search_docs").await,
        41
    );

    let hits = hybrid_book(
        app.db().conn(),
        "secret",
        None,
        &BookSearch {
            book_id,
            corpus: BookCorpus::Canon,
            kinds: None,
            isolated_track: None,
            limit: 8,
        },
    )
    .await
    .expect("hybrid");
    println!("shared-session hits: {hits:?}");
    assert!(
        hits.hits.iter().any(|hit| hit.ref_id == 500),
        "the shared document must be found although 40 private documents match: {:?}",
        hits.hits
    );

    // The isolated track's own session sees its private documents too.
    let side = hybrid_book(
        app.db().conn(),
        "secret",
        None,
        &BookSearch {
            book_id,
            corpus: BookCorpus::Canon,
            kinds: None,
            isolated_track: Some(900),
            limit: 8,
        },
    )
    .await
    .expect("hybrid");
    assert!(
        side.hits.iter().any(|hit| hit.track_id == Some(900)),
        "{side:?}"
    );
}

/// `corpus` keeps passages from crowding canon hits out of the vector list (D-030): 40 passages
/// are nearer than the one canon document, yet `search_canon` must still find it.
#[tokio::test]
async fn passages_do_not_crowd_canon_hits_out_of_the_vector_list() {
    use nekowriter::services::indexer;
    use std::sync::Arc;

    struct SameEmbedder;
    #[async_trait::async_trait]
    impl nekowriter_embed::Embedder for SameEmbedder {
        fn model(&self) -> &nekowriter_embed::ModelEntry {
            nekowriter_embed::default_model()
        }
        fn dimension(&self) -> usize {
            384
        }
        async fn embed(
            &self,
            texts: Vec<String>,
            _kind: nekowriter_embed::EmbedKind,
        ) -> Result<Vec<Vec<f32>>, nekowriter_embed::EmbedError> {
            Ok(texts
                .iter()
                .map(|text| {
                    let mut vector = vec![0.0f32; 384];
                    // Passages are closer to the query than the canon document.
                    if text.contains("sword tomb passage") {
                        vector[0] = 1.0;
                    } else {
                        vector[1] = 1.0;
                    }
                    vector
                })
                .collect())
        }
    }

    let app = TestApp::new().await;
    let book = app.create_book("Crowd Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;
    let chapter = app.create_chapter(track_id, "Crowd").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    // 40 rows of `search_docs` passage kind and one canon entity, seeded directly so the corpus
    // split is what the vectors exercise rather than the chunker.
    let mut sql = String::new();
    for index in 1..=40 {
        sql.push_str(&format!(
            "INSERT INTO search_docs(id, book_id, track_id, kind, ref_id, ref_part, title, body, content_hash, updated_at) \
             VALUES ({index}, {book_id}, NULL, 'passage', {chapter_id}, {index}, 'Crowd', 'sword tomb passage {index}', 'ph{index}', \
                     '2026-01-01T00:00:00Z');"
        ));
    }
    sql.push_str(&format!(
        "INSERT INTO search_docs(id, book_id, track_id, kind, ref_id, ref_part, title, body, content_hash, updated_at) \
         VALUES (500, {book_id}, NULL, 'entity', 500, 0, 'Tomb Lord', 'the secret keeper of the tomb', 'eh500', \
                 '2026-01-01T00:00:00Z')"
    ));
    app.execute(&sql).await;
    app.state().search().install(Arc::new(SameEmbedder));
    assert!(app.state().search().ensure_ready().await);
    for _ in 0..3 {
        indexer::run_once(app.state()).await.expect("indexer");
    }
    assert_eq!(
        count(&app, "SELECT count(*) AS c FROM vec_book").await,
        41,
        "every seeded row is embedded"
    );

    let query = vec![1.0f32; 384];
    let canon = hybrid_book(
        app.db().conn(),
        "unrelated-token",
        Some(&query),
        &BookSearch {
            book_id,
            corpus: BookCorpus::Canon,
            kinds: None,
            isolated_track: None,
            limit: 8,
        },
    )
    .await
    .expect("canon");
    println!("canon vector hits: {:?}", canon.hits);
    assert!(
        canon.hits.iter().any(|hit| hit.kind == "entity"),
        "the canon entity must survive 40 nearer passages: {canon:?}"
    );
}
