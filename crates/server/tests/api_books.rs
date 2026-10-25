//! Book, track, setting-material and app-settings API tests (PLAN M1.2).
//!
//! Every state rule and error code of spec/08 §1.1 / §1.3 / §5 that M1 implements is asserted
//! against the real router and a migrated temporary database.

use axum::http::StatusCode;
use nekowriter::testing::{TestApp, assert_error, assert_error_detail};
use serde_json::json;

#[tokio::test]
async fn create_book_makes_the_main_track_and_three_setting_docs() {
    let app = TestApp::new().await;
    let book = app.create_book("青云劫").await;
    let book_id = book["id"].as_i64().expect("book id");

    assert_eq!(book["title"], "青云劫");
    assert_eq!(
        book["status"], "active",
        "start_conception=false starts active"
    );
    assert_eq!(book["writing_language"], "zh-CN");
    // BookSettings defaults come from spec/02 §4.
    assert_eq!(book["settings"]["granularity"], "paragraph");
    assert_eq!(book["settings"]["auto_pause"], false);
    assert_eq!(book["settings"]["pause_after_outline"], true);
    assert_eq!(book["settings"]["target_words_min"], 3000);
    assert_eq!(book["settings"]["target_words_max"], 4000);
    assert_eq!(book["settings"]["recent_summary_count"], 5);
    assert_eq!(book["settings"]["midterm_window"], 10);
    assert_eq!(book["settings"]["auto_maintenance"], true);

    // Invariant 1 (spec/02 §5): exactly one main track, ord 0, empty title.
    let (status, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    assert_eq!(status, StatusCode::OK);
    let tracks = tracks.as_array().expect("tracks array");
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0]["kind"], "main");
    assert_eq!(tracks[0]["title"], "");
    assert_eq!(tracks[0]["canon_mode"], "canon");
    assert_eq!(tracks[0]["anchor_kind"], "none");
    assert_eq!(tracks[0]["ord"], 0);

    // Invariant 2: exactly three setting documents, empty, revision 0.
    let (status, docs) = app.get(&format!("/api/books/{book_id}/setting-docs")).await;
    assert_eq!(status, StatusCode::OK);
    let docs = docs.as_array().expect("docs array");
    assert_eq!(docs.len(), 3);
    let kinds: Vec<&str> = docs
        .iter()
        .map(|doc| doc["kind"].as_str().expect("kind"))
        .collect();
    assert_eq!(kinds, vec!["outline", "world", "protagonist"]);
    for doc in docs {
        assert_eq!(doc["content"], "");
        assert_eq!(doc["revision"], 0);
    }
}

#[tokio::test]
async fn create_book_validates_input() {
    let app = TestApp::new().await;
    let (status, body) = app
        .post(
            "/api/books",
            json!({ "title": "  ", "writing_language": "zh-CN" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");

    let (status, body) = app
        .post(
            "/api/books",
            json!({ "title": "ok", "writing_language": "" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");

    // Not a BCP-47 tag.
    let (status, body) = app
        .post(
            "/api/books",
            json!({ "title": "ok", "writing_language": "not a language" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");
}

#[tokio::test]
async fn library_lists_books_and_filters_archived_ones() {
    let app = TestApp::new().await;
    let first = app.create_book("First").await;
    let second = app.create_book("Second").await;

    let (status, list) = app.get("/api/books").await;
    assert_eq!(status, StatusCode::OK);
    let list = list.as_array().expect("books");
    assert_eq!(list.len(), 2);
    // Summaries carry the counts the library cards show (spec/10 §2.2).
    let summary = list
        .iter()
        .find(|book| book["id"] == first["id"])
        .expect("first book");
    assert_eq!(summary["chapter_count"], 0);
    assert_eq!(summary["official_chapter_count"], 0);
    assert_eq!(summary["word_count"], 0);
    assert!(summary["latest_chapter"].is_null());
    assert_eq!(summary["status"], "active");

    // Archive the second: active only, otherwise book.invalid_state (spec/08 §1.3).
    let (status, body) = app
        .patch(
            &format!("/api/books/{}", second["id"]),
            json!({ "status": "archived" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "archived");

    let (_, list) = app.get("/api/books").await;
    assert_eq!(
        list.as_array().expect("books").len(),
        2,
        "archiving only hides"
    );
    let (_, archived) = app.get("/api/books?status=archived").await;
    assert_eq!(archived.as_array().expect("archived").len(), 1);
    let (_, active) = app.get("/api/books?status=active").await;
    assert_eq!(active.as_array().expect("active").len(), 1);

    // Archiving an archived book is rejected.
    let (status, body) = app
        .patch(
            &format!("/api/books/{}", second["id"]),
            json!({ "status": "archived" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error_detail(&body, "book.invalid_state", "status");

    // Unarchive it.
    let (status, body) = app
        .patch(
            &format!("/api/books/{}", second["id"]),
            json!({ "status": "active" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "active");
    let _ = first;
}

#[tokio::test]
async fn delete_book_removes_its_tracks_chapters_and_settings() {
    let app = TestApp::new().await;
    let book = app.create_book("To delete").await;
    let book_id = book["id"].as_i64().expect("book id");
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id");
    let chapter = app.create_chapter(track_id as i32, "ch1").await;
    app.put_text(
        chapter["id"].as_i64().expect("chapter id") as i32,
        0,
        "one\n\ntwo",
    )
    .await;

    let (status, _) = app.delete(&format!("/api/books/{book_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = app.get(&format!("/api/books/{book_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&body, "not_found");
    let (status, _) = app.get(&format!("/api/tracks/{track_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app.get(&format!("/api/chapters/{}", chapter["id"])).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn book_patch_edits_fields_and_keeps_others() {
    let app = TestApp::new().await;
    let book = app.create_book("Original").await;
    let book_id = book["id"].as_i64().expect("book id");

    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "title": "Renamed", "genre": "thriller" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["title"], "Renamed");
    assert_eq!(body["genre"], "thriller");
    assert_eq!(body["idea"], "an idea", "omitted fields are unchanged");
    assert_eq!(body["writing_language"], "zh-CN");

    // Settings can be replaced, and are validated.
    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "settings": {
                "granularity": "sentence",
                "auto_pause": true,
                "pause_after_outline": false,
                "target_words_min": 1200,
                "target_words_max": 1800,
                "recent_summary_count": 3,
                "midterm_window": 6,
                "auto_maintenance": false
            }}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["settings"]["granularity"], "sentence");
    assert_eq!(body["settings"]["target_words_max"], 1800);

    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "settings": { "target_words_min": 5000, "target_words_max": 100 } }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");

    // `conceiving` is not settable by a patch.
    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "status": "conceiving" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");
}

#[tokio::test]
async fn missing_books_and_tracks_are_not_found() {
    let app = TestApp::new().await;
    let (status, body) = app.get("/api/books/999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&body, "not_found");
    let (status, body) = app.get("/api/books/999/tracks").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&body, "not_found");
    let (status, body) = app.get("/api/tracks/999/chapters").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&body, "not_found");
    let (status, body) = app.get("/api/books/999/overview").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&body, "not_found");
}

#[tokio::test]
async fn main_track_cannot_be_deleted_and_only_its_synopsis_is_editable() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id");
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id");

    // Spec/08 §1.3: the main track cannot be deleted (`validation.failed`).
    let (status, body) = app.delete(&format!("/api/tracks/{track_id}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");

    // A second main track is refused (invariant 1).
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "kind": "main" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&body, "conflict.exists");

    // A side track is created by the same endpoint (spec/08 §6; M10 replaced the M1 deferral
    // that answered `validation.failed` here).
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "title": "Side" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["kind"], "side");
    assert_eq!(body["title"], "Side");

    // The main track's title is localized by the UI (DECISIONS D-037): only a synopsis is legal.
    let (status, body) = app
        .patch(
            &format!("/api/tracks/{track_id}"),
            json!({ "title": "Main" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");
    let (status, body) = app
        .patch(
            &format!("/api/tracks/{track_id}"),
            json!({ "synopsis": "The main story" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["synopsis"], "The main story");
    assert_eq!(body["title"], "");
}

#[tokio::test]
async fn setting_docs_are_saved_with_base_revision_and_write_a_change() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id");

    let (status, doc) = app
        .put(
            &format!("/api/books/{book_id}/setting-docs/outline"),
            json!({ "content": "# Outline\n\nAct one.", "base_revision": 0, "note": "first pass" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc["revision"], 1);
    assert_eq!(doc["content"], "# Outline\n\nAct one.");

    // A stale base_revision is conflict.version with the current revision (spec/10 §4).
    let (status, body) = app
        .put(
            &format!("/api/books/{book_id}/setting-docs/outline"),
            json!({ "content": "should not apply", "base_revision": 0 }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&body, "conflict.version");
    assert_eq!(body["error"]["details"]["revision"], 1);

    // A no-op save keeps the revision.
    let (status, doc) = app
        .put(
            &format!("/api/books/{book_id}/setting-docs/outline"),
            json!({ "content": "# Outline\n\nAct one.", "base_revision": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["revision"], 1);

    // The change log records the author's note and the before/after snapshots (spec/06 §3).
    let (status, changes) = app
        .get(&format!(
            "/api/books/{book_id}/changes?target_type=setting_doc"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    let items = changes["items"].as_array().expect("changes");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["actor"], "user");
    assert_eq!(items[0]["op"], "update");
    assert_eq!(items[0]["note"], "first pass");
    assert_eq!(items[0]["before"]["content"], "");
    assert_eq!(items[0]["after"]["content"], "# Outline\n\nAct one.");

    // The history endpoint filters to this document.
    let (status, history) = app
        .get(&format!(
            "/api/books/{book_id}/setting-docs/outline/history"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(history.as_array().expect("history").len(), 1);
}

#[tokio::test]
async fn unknown_setting_doc_kind_is_validation_failed() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id");
    let (status, body) = app
        .get(&format!("/api/books/{book_id}/setting-docs/characters"))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");
}

#[tokio::test]
async fn characters_are_ranked_renumbered_and_unique() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id");

    let (status, first) = app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "林凡", "role_label": "主角", "summary": "the protagonist" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["rank"], 1);
    assert_eq!(first["name"], "林凡");
    assert_eq!(first["role_label"], "主角");

    let (status, second) = app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "苏婉", "role_label": "女主" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(second["rank"], 2);

    // A duplicate name is conflict.exists with the existing id (spec/10 §4).
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "林凡" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error_detail(&body, "conflict.exists", "id");

    // Reorder: most important first, ranks renumbered 1..n.
    let first_id = first["id"].as_i64().expect("first id");
    let second_id = second["id"].as_i64().expect("second id");
    let (status, list) = app
        .post(
            &format!("/api/books/{book_id}/characters/reorder"),
            json!({ "ids": [second_id, first_id] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let list = list.as_array().expect("characters");
    assert_eq!(list[0]["name"], "苏婉");
    assert_eq!(list[0]["rank"], 1);
    assert_eq!(list[1]["name"], "林凡");
    assert_eq!(list[1]["rank"], 2);

    // A reorder list that omits a character is rejected.
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/characters/reorder"),
            json!({ "ids": [first_id] }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");

    // Patching a name to a taken one is conflict.exists; patching other fields is fine.
    let (status, body) = app
        .patch(
            &format!("/api/characters/{first_id}"),
            json!({ "name": "苏婉" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&body, "conflict.exists");
    let (status, body) = app
        .patch(
            &format!("/api/characters/{first_id}"),
            json!({ "profile": "## Profile\n\nBorn in the valley.", "note": "expanded" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["profile"], "## Profile\n\nBorn in the valley.");
    assert_eq!(body["name"], "林凡", "omitted fields are unchanged");

    // Deleting closes up the ranking.
    let (status, _) = app.delete(&format!("/api/characters/{second_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, list) = app.get(&format!("/api/books/{book_id}/characters")).await;
    let list = list.as_array().expect("characters");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["rank"], 1);

    // Every change is logged with actor user and the right op.
    let (_, changes) = app
        .get(&format!(
            "/api/books/{book_id}/changes?target_type=character_design"
        ))
        .await;
    let ops: Vec<&str> = changes["items"]
        .as_array()
        .expect("changes")
        .iter()
        .map(|change| change["op"].as_str().expect("op"))
        .collect();
    assert_eq!(ops, vec!["delete", "update", "create", "create"]);
    assert!(
        changes["items"]
            .as_array()
            .expect("changes")
            .iter()
            .all(|change| change["actor"] == "user")
    );
}

#[tokio::test]
async fn overview_reports_progress_per_track() {
    let app = TestApp::new().await;
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id");
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks[0]["id"].as_i64().expect("track id");
    let chapter = app.create_chapter(track_id as i32, "ch1").await;
    let chapter_id = chapter["id"].as_i64().expect("chapter id") as i32;
    app.put_text(chapter_id, 0, "风吹过山谷\n\nThe wind blew")
        .await;

    let (status, overview) = app.get(&format!("/api/books/{book_id}/overview")).await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert_eq!(overview["book"]["title"], "Book");
    assert_eq!(overview["progress"]["total_chapters"], 1);
    assert_eq!(overview["progress"]["official_chapters"], 0);
    assert_eq!(overview["progress"]["total_words"], 8);
    assert_eq!(overview["progress"]["official_words"], 0);
    let track = &overview["progress"]["tracks"][0];
    assert_eq!(track["track_id"], track_id);
    assert_eq!(track["kind"], "main");
    assert_eq!(track["chapters"], 1);
    assert_eq!(track["words"], 8);
    // A manual edit moves `planned` into `drafting` (the lifecycle diagram of spec/08 §1), so the
    // chapter counts under `drafting` here and `planned` stays at 0.
    let drafting = track["by_status"]
        .as_array()
        .expect("by_status")
        .iter()
        .find(|entry| entry["status"] == "drafting")
        .expect("drafting entry");
    assert_eq!(drafting["count"], 1);
    let planned = track["by_status"]
        .as_array()
        .expect("by_status")
        .iter()
        .find(|entry| entry["status"] == "planned")
        .expect("planned entry");
    assert_eq!(planned["count"], 0);
    assert_eq!(overview["usage"]["requests"], 0);
}

#[tokio::test]
async fn app_settings_defaults_and_patch() {
    let app = TestApp::new().await;
    // Defaults from spec/02 §4.1 before anything is written.
    let (status, settings) = app.get("/api/app-settings").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings["book_defaults"]["granularity"], "paragraph");
    assert_eq!(settings["search"]["provider"], "none");
    assert_eq!(settings["search"]["max_results"], 6);
    assert_eq!(settings["embedding"]["model"], "multilingual-e5-small");
    assert!(settings["roles_defaults"]["writer"].is_null());

    // New books inherit the updated defaults.
    let (status, updated) = app
        .patch(
            "/api/app-settings",
            json!({ "book_defaults": {
                "granularity": "free",
                "auto_pause": true,
                "pause_after_outline": false,
                "target_words_min": 2000,
                "target_words_max": 2500,
                "recent_summary_count": 2,
                "midterm_window": 4,
                "auto_maintenance": false
            }}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["book_defaults"]["granularity"], "free");

    let book = app.create_book("Inherits defaults").await;
    assert_eq!(book["settings"]["granularity"], "free");
    assert_eq!(book["settings"]["target_words_min"], 2000);
    assert_eq!(book["settings"]["auto_maintenance"], false);

    // Role defaults round-trip.
    let (status, updated) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": { "writer": 7 } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["roles_defaults"]["writer"], 7);
    assert!(updated["roles_defaults"]["reviewer"].is_null());

    // The embedding model must be one of the supported keys.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "embedding": { "model": "nope" } }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");
    let (status, updated) = app
        .patch(
            "/api/app-settings",
            json!({ "embedding": { "model": "bge-small-zh-v1.5" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["embedding"]["model"], "bge-small-zh-v1.5");

    // A provider that needs configuration demands it.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "provider": "searxng" } }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");

    // The key is masked in responses, and a round-tripped mask does not erase it.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "provider": "tavily", "tavily_api_key": "tvly-secret-9876" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["search"]["tavily_api_key"], "…9876");
    assert!(!body.to_string().contains("tvly-secret-9876"));
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "tavily_api_key": "…9876" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["search"]["tavily_api_key"], "…9876");
}
