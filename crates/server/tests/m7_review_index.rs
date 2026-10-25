//! M7 review: index-manager and per-kind `search_docs` probes (spec/07 §2, §3).

use axum::http::StatusCode;
use nekowriter::db::entities::search_docs;
use nekowriter::search::{index, maintenance};
use nekowriter::services::indexer;
use nekowriter::testing::TestApp;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde_json::json;

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

async fn doc(app: &TestApp, kind: &str, ref_id: i32) -> Option<search_docs::Model> {
    search_docs::Entity::find()
        .filter(search_docs::Column::Kind.eq(kind))
        .filter(search_docs::Column::RefId.eq(ref_id))
        .one(app.db().conn())
        .await
        .expect("doc query")
}

/// The runtime vec delete triggers must survive a dimension change: deleting the source after a
/// model switch must still remove the vector (spec/07 §3, spec/02 §2).
#[tokio::test]
async fn the_vec_delete_triggers_are_recreated_with_every_dimension() {
    let app = TestApp::new().await;
    let book = app.create_book("Trigger Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "Lin Fan", "summary": "the protagonist" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let entity_id = body["id"].as_i64().expect("entity id") as i32;
    indexer::run_once(app.state()).await.expect("indexer");

    // Switch the model: the vec table is recreated at the new dimension.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "embedding": { "model": "bge-small-zh-v1.5" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(
        count(&app, "SELECT count(*) AS c FROM vec_book").await,
        1,
        "the new dimension stores the vector"
    );

    // The trigger is recreated: deleting the entity removes its vector.
    let (status, _) = app.delete(&format!("/api/entities/{entity_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        count(&app, "SELECT count(*) AS c FROM vec_book").await,
        0,
        "the recreated delete trigger must fire"
    );
    assert_eq!(count(&app, "SELECT count(*) AS c FROM fts_book").await, 0);
}

/// Every kind's `search_docs` title/body/track_id per spec/07 §2.
#[tokio::test]
async fn search_docs_content_matches_the_kind_table() {
    let app = TestApp::new().await;
    let book = app.create_book("Kind Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id") as i32;

    // note: title = first 40 characters of the content, body = content.
    let long_note = format!("{}{}", "字".repeat(40), "tail of the note");
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/memory-notes"),
            json!({ "kind": "fact", "content": long_note }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let note_id = body["id"].as_i64().expect("note id") as i32;
    let note = doc(&app, "note", note_id).await.expect("note doc");
    assert_eq!(note.title, "字".repeat(40), "the first 40 characters");
    assert_eq!(note.body, long_note, "the whole content");
    assert_eq!(note.track_id, None);

    // foreshadowing: title = its title; body = description · planned payoff · resolution.
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/foreshadowing"),
            json!({
                "title": "The broken sword",
                "description": "a sword in the tomb",
                "planned_payoff": "it awakens",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let fore_id = body["id"].as_i64().expect("foreshadowing id") as i32;
    let fore = doc(&app, "foreshadowing", fore_id)
        .await
        .expect("foreshadowing doc");
    assert_eq!(fore.title, "The broken sword");
    assert_eq!(fore.body, "a sword in the tomb · it awakens");

    // event: title = its title; body = story_time · description.
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/timeline"),
            json!({
                "title": "The duel",
                "description": "two swords meet",
                "story_time": "第三天",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let event_id = body["id"].as_i64().expect("event id") as i32;
    let event = doc(&app, "event", event_id).await.expect("event doc");
    assert_eq!(event.title, "The duel");
    assert_eq!(event.body, "第三天 · two swords meet");

    // summary + passage of an official chapter on the main track: track_id NULL.
    let chapter = app.create_chapter(track_id, "Sword Tomb").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.put_text(chapter_id, 0, "Lin Fan enters the tomb.\n\nHe waits.")
        .await;
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/submit"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = app
        .post(&format!("/api/chapters/{chapter_id}/approve"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    app.patch(
        &format!("/api/chapters/{chapter_id}"),
        json!({ "summary": "Lin Fan enters the sword tomb." }),
    )
    .await;
    let summary = doc(&app, "summary", chapter_id).await.expect("summary doc");
    assert_eq!(summary.title, "Sword Tomb");
    assert_eq!(summary.body, "Lin Fan enters the sword tomb.");
    assert_eq!(summary.track_id, None);
    let passage = doc(&app, "passage", chapter_id).await.expect("passage doc");
    assert_eq!(passage.title, "Sword Tomb");
    assert_eq!(passage.track_id, None);

    // An entity carries its own track_id (NULL here).
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": "Su Wan", "summary": "the sword spirit" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let entity_id = body["id"].as_i64().expect("entity id") as i32;
    let entity = doc(&app, "entity", entity_id).await.expect("entity doc");
    assert_eq!(entity.track_id, None);

    // Retiring a note removes its document (spec/07 §2): the rows go through the same hook the
    // archivist's `retire_memory_note` uses.
    app.execute(&format!(
        "UPDATE memory_notes SET status = 'retired' WHERE id = {note_id}"
    ))
    .await;
    nekowriter::db::Db::write(app.db(), move |tx| {
        Box::pin(async move {
            maintenance::sync_note(tx, book_id, note_id)
                .await
                .map_err(|err| sea_orm::DbErr::Custom(err.message))?;
            Ok(())
        })
    })
    .await
    .expect("sync retire");
    assert!(doc(&app, "note", note_id).await.is_none());
}

/// An isolated-track chapter's summary/passage documents carry that track's id (spec/07 §2).
#[tokio::test]
async fn isolated_chapter_docs_carry_the_track_id() {
    let app = TestApp::new().await;
    let book = app.create_book("Isolated Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    app.execute(&format!(
        "INSERT INTO tracks(id, book_id, kind, title, canon_mode, ord, created_at, updated_at) \
         VALUES (900, {book_id}, 'side', 'Side', 'isolated', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'); \
         INSERT INTO chapters(id, book_id, track_id, ord, title, status, summary, content, created_at, updated_at) \
         VALUES (901, {book_id}, 900, 1, 'Side One', 'official', 'a side summary', \
                 '{{\"paragraphs\":[{{\"pid\":\"p1\",\"text\":\"The secret waits.\"}}],\"next_pid\":2,\"tail_open\":false}}', \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    nekowriter::db::Db::write(app.db(), |tx| {
        Box::pin(async move {
            maintenance::sync_chapter(tx, 901)
                .await
                .map_err(|err| sea_orm::DbErr::Custom(err.message))?;
            Ok(())
        })
    })
    .await
    .expect("sync");
    let summary = doc(&app, "summary", 901).await.expect("summary");
    assert_eq!(summary.track_id, Some(900), "isolated summary carries T");
    let passage = doc(&app, "passage", 901).await.expect("passage");
    assert_eq!(passage.track_id, Some(900), "isolated passage carries T");
}

/// A model change writes the meta and resets the states in one transaction; FTS is untouched.
#[tokio::test]
async fn a_model_change_touches_no_fts_row_and_recreates_both_vec_tables() {
    let app = TestApp::new().await;
    let book = app.create_book("Meta Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    app.post(
        &format!("/api/books/{book_id}/entities"),
        json!({ "kind": "character", "name": "Lin Fan", "summary": "the protagonist" }),
    )
    .await;
    indexer::run_once(app.state()).await.expect("indexer");
    let fts_before = count(&app, "SELECT count(*) AS c FROM fts_book").await;
    assert_eq!(fts_before, 1);

    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "embedding": { "model": "embeddinggemma-300m-q" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        index::meta_value(app.db().conn(), index::META_DIM)
            .await
            .expect("meta")
            .as_deref(),
        Some("768")
    );
    // Both vec tables exist at the new dimension and both triggers are installed.
    let triggers = count(
        &app,
        "SELECT count(*) AS c FROM sqlite_master WHERE type = 'trigger' \
         AND name IN ('search_docs_vec_ad', 'knowledge_chunks_vec_ad')",
    )
    .await;
    assert_eq!(triggers, 2, "both delete triggers must be recreated");
    assert_eq!(
        count(&app, "SELECT count(*) AS c FROM fts_book").await,
        fts_before
    );
    // The new table accepts a 768-dim vector only (a 384 one is rejected by vec0).
    assert_eq!(count(&app, "SELECT count(*) AS c FROM vec_book").await, 0);
    let report = indexer::run_once(app.state()).await.expect("indexer");
    assert_eq!(report.stored, 1, "{report:?}");
    assert_eq!(count(&app, "SELECT count(*) AS c FROM vec_book").await, 1);
}

/// spec/01 §6 + PROJECT §8.12: `index.status` carries the resolved model cache path (D-097).
#[tokio::test]
async fn the_index_status_reports_the_model_cache_dir() {
    let app = TestApp::new().await;
    let expected = app
        .config()
        .model_cache_dir(std::env::var_os("HF_HOME").as_deref())
        .display()
        .to_string();
    assert!(!expected.is_empty(), "the resolved path is never empty");

    // Both endpoints that carry `IndexStatusDto` report the same path.
    let (status, body) = app.get("/api/embedding/status").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["model_cache_dir"], serde_json::json!(expected));

    let (status, health) = app.get("/api/health").await;
    assert_eq!(status, StatusCode::OK, "{health}");
    assert_eq!(
        health["embedding"]["model_cache_dir"],
        serde_json::json!(expected)
    );

    // The `index.status` event payload carries it too (spec/10 §3).
    let mut receiver = app.events().subscribe();
    app.state().search().publish_status().await;
    let mut seen: Option<serde_json::Value> = None;
    while let Ok(envelope) = receiver.try_recv() {
        if matches!(
            envelope.event,
            nekowriter::events::ServerEvent::IndexStatus { .. }
        ) {
            seen = Some(serde_json::to_value(&envelope.event).expect("serialize event"));
        }
    }
    let payload = seen.expect("an index.status event");
    assert_eq!(payload["model_cache_dir"], serde_json::json!(expected));
}
