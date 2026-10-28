//! Chapter creation is gated on the book being `active` (spec/08 §1.1, D-130).
//!
//! The author's report: "构思未完成时仍可以创建章节" — chapters could be created while the
//! conception was unfinished. `conception/finalize` is the single transition that activates a book
//! and writes the setting materials the writer's kickoff renders, so a chapter created before it
//! is written against an empty book and makes a `conceiving` book look started. The server refuses
//! it with `409 book.invalid_state`, and the same rule covers archived books.

use axum::http::StatusCode;
use nekowriter::testing::{TestApp, assert_error, assert_error_detail};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde_json::{Value, json};

/// The number of chapter rows in a track, with the smallest and largest `ord` (`None` when the
/// track has no chapters).
///
/// Read straight from the columns rather than through the list DTO, because `ord` is `last + 1`:
/// a refused create that still advanced something would show up as a gap here and nowhere else.
/// A typed `try_get` is used instead of `TestApp::query_json`, whose `Option<Value>` conversion
/// cannot decode SQLite integers.
async fn chapter_ords(app: &TestApp, track_id: i32) -> (i64, Option<i32>, Option<i32>) {
    let row = app
        .db()
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "SELECT COUNT(*) AS count, MIN(ord) AS min_ord, MAX(ord) AS max_ord \
                 FROM chapters WHERE track_id = {track_id}"
            ),
        ))
        .await
        .expect("query chapter ords")
        .expect("one row");
    (
        row.try_get::<i64>("", "count").expect("count"),
        row.try_get::<Option<i32>>("", "min_ord").expect("min_ord"),
        row.try_get::<Option<i32>>("", "max_ord").expect("max_ord"),
    )
}

/// Creates a book through the API with an explicit `start_conception` and returns its DTO.
///
/// The harness's `TestApp::create_book` always passes `start_conception: false`, so a conceiving
/// book has to be created by hand. No model is configured, so creation succeeds with the
/// documented `model.missing_role` warning and no session (D-046).
async fn book_with_status(app: &TestApp, title: &str, start_conception: bool) -> Value {
    let (status, body) = app
        .post(
            "/api/books",
            json!({
                "title": title,
                "idea": "an idea",
                "genre": "xianxia",
                "writing_language": "zh-CN",
                "start_conception": start_conception,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create book: {body}");
    body["book"].clone()
}

/// The id of the book's main track.
async fn main_track(app: &TestApp, book_id: i64) -> i32 {
    let (status, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    assert_eq!(status, StatusCode::OK, "tracks: {tracks}");
    tracks[0]["id"].as_i64().expect("track id") as i32
}

/// `POST /api/tracks/:id/chapters` without asserting success.
async fn try_create_chapter(app: &TestApp, track_id: i32) -> (StatusCode, Value) {
    app.post(
        &format!("/api/tracks/{track_id}/chapters"),
        json!({ "title": "Chapter", "guidance": "" }),
    )
    .await
}

/// Seeds what finalize requires: the three non-empty setting documents and one character design
/// (spec/08 §5 step 1).
async fn complete_materials(app: &TestApp, book_id: i64) {
    for (kind, content) in [
        ("outline", "# Outline\n\nAct one."),
        ("world", "# World\n\nThe gate keeps its own law."),
        ("protagonist", "# Protagonist\n\nShen Yan."),
    ] {
        let (status, body) = app
            .put(
                &format!("/api/books/{book_id}/setting-docs/{kind}"),
                json!({ "content": content, "base_revision": 0, "note": "" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "seed {kind}: {body}");
    }
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/characters"),
            json!({ "name": "Shen Yan", "role_label": "protagonist" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "seed character: {body}");
}

/// Assigns one Demo model profile to the writer role only, so a write session can start.
///
/// The archivist role stays unassigned deliberately: `conception/finalize` then reports
/// `model.missing_role` and starts no `canon_seed` session, which keeps these tests about chapter
/// creation rather than about seeding.
async fn assign_writer_model(app: &TestApp) {
    let (status, provider) = app
        .post(
            "/api/providers",
            json!({
                "name": "Demo",
                "protocol": "mock",
                "base_url": "mock://demo",
                "extra_headers": {},
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "create provider: {provider}");
    let provider_id = provider["id"].as_i64().expect("provider id");
    let (status, profile) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider_id,
                "display_name": "Demo model",
                "model": "demo-model",
                "context_window": 200_000,
                "max_output_tokens": 8_192,
                "options": {},
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "create model profile: {profile}"
    );
    let profile_id = profile["id"].as_i64().expect("profile id");
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": { "writer": profile_id } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "assign writer: {body}");
}

/// Seeds a `planned`, empty chapter straight into the database.
///
/// This is the shape a book reaches when the gate was missing: the row exists because nothing
/// refused it, not because an API call succeeded. Seeding it through SQL keeps that provenance
/// explicit, since the endpoint now refuses to produce it.
async fn seed_existing_chapter(app: &TestApp, book_id: i64, track_id: i32) -> i32 {
    app.execute(&format!(
        "INSERT INTO chapters(book_id, track_id, ord, status, content, created_at, updated_at) \
         VALUES ({book_id}, {track_id}, 1, 'planned', \
                 '{{\"paragraphs\":[],\"next_pid\":1,\"tail_open\":false}}', \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    ))
    .await;
    let (status, list) = app.get(&format!("/api/tracks/{track_id}/chapters")).await;
    assert_eq!(status, StatusCode::OK, "chapters: {list}");
    let first = list
        .as_array()
        .and_then(|rows| rows.first())
        .expect("the seeded chapter");
    first["id"].as_i64().expect("chapter id") as i32
}

/// A `conceiving` book refuses a chapter with `409 book.invalid_state` and `details.status`, and
/// the refusal leaves the track untouched (no row, no consumed number).
#[tokio::test]
async fn a_conceiving_book_refuses_a_new_chapter() {
    let app = TestApp::new().await;
    let book = book_with_status(&app, "Conceiving", true).await;
    let book_id = book["id"].as_i64().expect("book id");
    assert_eq!(book["status"], "conceiving");
    let track_id = main_track(&app, book_id).await;

    let (status, body) = try_create_chapter(&app, track_id).await;
    assert_eq!(status, StatusCode::CONFLICT, "conceiving book: {body}");
    assert_error_detail(&body, "book.invalid_state", "status");
    assert_eq!(
        body["error"]["details"]["status"], "conceiving",
        "the envelope names the current status so the UI can word the reason: {body}"
    );

    // Nothing was inserted: the chapter list is still empty, so no `ord` was consumed and the
    // Navigator's numbering will start at 1 once the book is finalized.
    let (status, list) = app.get(&format!("/api/tracks/{track_id}/chapters")).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list.as_array().expect("chapters").len(), 0);

    // Asserted against the column itself, not only through the list DTO: `ord` is `last + 1`, so
    // a refused attempt that still bumped something would leave a gap once the book is active.
    for _ in 0..3 {
        let (status, body) = try_create_chapter(&app, track_id).await;
        assert_eq!(status, StatusCode::CONFLICT, "repeated refusal: {body}");
    }
    let (count, min_ord, max_ord) = chapter_ords(&app, track_id).await;
    assert_eq!(count, 0, "no chapter row exists");
    assert_eq!(
        (min_ord, max_ord),
        (None, None),
        "repeated refusals consumed no `ord`"
    );
}

/// An archived book refuses a chapter by the same rule (spec/08 §1.3 archives only `active`
/// books, and an archived book must not accumulate new chapters either).
#[tokio::test]
async fn an_archived_book_refuses_a_new_chapter() {
    let app = TestApp::new().await;
    let book = app.create_book("Archived").await;
    let book_id = book["id"].as_i64().expect("book id");
    let track_id = main_track(&app, book_id).await;
    // A chapter created while the book was active keeps working (archiving only hides).
    let kept = app.create_chapter(track_id, "Kept").await;

    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "status": "archived" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "archive: {body}");

    let (status, body) = try_create_chapter(&app, track_id).await;
    assert_eq!(status, StatusCode::CONFLICT, "archived book: {body}");
    assert_error_detail(&body, "book.invalid_state", "status");
    assert_eq!(body["error"]["details"]["status"], "archived", "{body}");

    let (_, list) = app.get(&format!("/api/tracks/{track_id}/chapters")).await;
    let list = list.as_array().expect("chapters");
    assert_eq!(list.len(), 1, "the existing chapter is untouched: {list:?}");
    assert_eq!(list[0]["id"], kept["id"]);

    // Unarchiving makes the book active again, and creation resumes.
    let (status, body) = app
        .patch(
            &format!("/api/books/{book_id}"),
            json!({ "status": "active" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "unarchive: {body}");
    let (status, body) = try_create_chapter(&app, track_id).await;
    assert_eq!(status, StatusCode::CREATED, "after unarchive: {body}");
    assert_eq!(body["number"], 2);
}

/// The transition the rule is about: refused while `conceiving` → `conception/finalize` succeeds
/// and activates the book → creation works and numbers the chapter as the first one.
///
/// Asserting only the refusal would let a bug where finalize forgets to set `active` pass silently.
#[tokio::test]
async fn finalize_is_what_makes_chapter_creation_possible() {
    let app = TestApp::new().await;
    let book = book_with_status(&app, "Transition", true).await;
    let book_id = book["id"].as_i64().expect("book id");
    let track_id = main_track(&app, book_id).await;

    let (status, body) = try_create_chapter(&app, track_id).await;
    assert_eq!(status, StatusCode::CONFLICT, "before finalize: {body}");
    assert_error(&body, "book.invalid_state");
    // Two more refusals, so the "no consumed number" assertion below is not a single-shot fluke.
    for _ in 0..2 {
        let (status, body) = try_create_chapter(&app, track_id).await;
        assert_eq!(status, StatusCode::CONFLICT, "still conceiving: {body}");
    }

    complete_materials(&app, book_id).await;
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "finalize: {body}");
    assert_eq!(
        body["book"]["status"], "active",
        "finalize is the activation: {body}"
    );

    let (status, chapter) = try_create_chapter(&app, track_id).await;
    assert_eq!(status, StatusCode::CREATED, "after finalize: {chapter}");
    assert_eq!(chapter["status"], "planned");
    // The numbering starts at 1 even though three creates were refused first: the refusals happen
    // before `next_ord` can consume anything, so the Navigator shows no gap.
    assert_eq!(
        chapter["number"], 1,
        "the refused attempts consumed no number"
    );
    let (count, min_ord, max_ord) = chapter_ords(&app, track_id).await;
    assert_eq!(count, 1, "exactly one chapter");
    assert_eq!(min_ord, Some(1), "numbering starts at 1");
    assert_eq!(max_ord, Some(1), "no gap from the refused attempts");
}

/// The real-world shape the gate has to handle: a `conceiving` book that *already* has a chapter,
/// because it was created while the gate was missing (the author's own book is in exactly this
/// state). A second chapter is refused, and the existing one is left alone: the rule stops new
/// chapters, it does not retroactively break what is already there.
#[tokio::test]
async fn a_conceiving_book_with_an_existing_chapter_refuses_only_the_second_one() {
    let app = TestApp::new().await;
    let book = book_with_status(&app, "Legacy", true).await;
    let book_id = book["id"].as_i64().expect("book id");
    let track_id = main_track(&app, book_id).await;
    let existing_id = seed_existing_chapter(&app, book_id, track_id).await;

    let (status, body) = try_create_chapter(&app, track_id).await;
    assert_eq!(status, StatusCode::CONFLICT, "second chapter: {body}");
    assert_error_detail(&body, "book.invalid_state", "status");
    assert_eq!(body["error"]["details"]["status"], "conceiving", "{body}");

    // The pre-existing chapter is intact and still readable through the API — the refusal did not
    // cascade into the row that was already there.
    let (status, list) = app.get(&format!("/api/tracks/{track_id}/chapters")).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let list = list.as_array().expect("chapters");
    assert_eq!(
        list.len(),
        1,
        "still exactly the one legacy chapter: {list:?}"
    );
    assert_eq!(list[0]["id"], existing_id);
    assert_eq!(list[0]["status"], "planned");
    let (status, chapter) = app.get(&format!("/api/chapters/{existing_id}")).await;
    assert_eq!(status, StatusCode::OK, "read the legacy chapter: {chapter}");
    assert_eq!(chapter["number"], 1);
    assert_eq!(chapter["word_count"], 0);
}

/// The nastier failure mode, asserted: the chapter that predates the gate must not be stranded
/// after `conception/finalize`. It is `planned` with no text, so a write session must start on it
/// exactly as it would for a chapter created after finalize.
#[tokio::test]
async fn a_chapter_that_predates_finalize_is_writable_once_the_book_is_active() {
    let app = TestApp::new().await;
    assign_writer_model(&app).await;
    let book = book_with_status(&app, "Stranded", true).await;
    let book_id = book["id"].as_i64().expect("book id");
    let track_id = main_track(&app, book_id).await;
    let existing_id = seed_existing_chapter(&app, book_id, track_id).await;

    // Before finalize the book is conceiving; finalize is the only transition that activates it.
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    // Finalize needs its materials; seed them and finalize for real rather than flipping the
    // status behind the API's back.
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "finalize before materials: {body}"
    );
    assert_error(&body, "conception.incomplete");
    complete_materials(&app, book_id).await;
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/conception/finalize"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "finalize: {body}");
    assert_eq!(body["book"]["status"], "active", "{body}");

    // The legacy chapter is unchanged by the transition...
    let (_, chapter) = app.get(&format!("/api/chapters/{existing_id}")).await;
    assert_eq!(
        chapter["status"], "planned",
        "finalize does not rewrite chapters"
    );
    assert_eq!(chapter["number"], 1);

    // ...and a writer session starts on it normally once it has an accepted outline. This is what
    // "not stranded" means: the chapter predates the gate but is a first-class chapter once the
    // book is active.
    app.execute(&format!(
        "UPDATE chapters SET outline = '细纲' WHERE id = {existing_id}"
    ))
    .await;
    let (status, body) = app
        .post(&format!("/api/chapters/{existing_id}/write"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "write the legacy chapter: {body}"
    );
    let session_id = body["session_id"].as_i64().expect("session id");
    let (status, session) = app.get(&format!("/api/sessions/{session_id}")).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    assert_eq!(session["session"]["kind"], "chapter_write");
    assert_eq!(session["session"]["chapter_id"], existing_id);

    // A chapter created after finalize still numbers after it, with no gap from the refusals.
    let (status, next) = try_create_chapter(&app, track_id).await;
    assert_eq!(status, StatusCode::CREATED, "next chapter: {next}");
    assert_eq!(next["number"], 2);
}

/// A side track of a conceiving book is refused by the same rule: the gate is on the book, not on
/// the kind of track the chapter would belong to (spec/08 §6).
#[tokio::test]
async fn a_side_track_of_a_conceiving_book_gets_no_chapter() {
    let app = TestApp::new().await;
    let book = book_with_status(&app, "Side", true).await;
    let book_id = book["id"].as_i64().expect("book id");

    let (status, side) = app
        .post(
            &format!("/api/books/{book_id}/tracks"),
            json!({ "title": "Side story" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "side track: {side}");
    let side_id = side["id"].as_i64().expect("side track id") as i32;

    let (status, body) = try_create_chapter(&app, side_id).await;
    assert_eq!(status, StatusCode::CONFLICT, "side track: {body}");
    assert_error_detail(&body, "book.invalid_state", "status");
    assert_eq!(body["error"]["details"]["status"], "conceiving", "{body}");
}

/// An `active` book still accepts chapters — the rule adds no friction to the normal path.
#[tokio::test]
async fn an_active_book_still_accepts_chapters() {
    let app = TestApp::new().await;
    let book = book_with_status(&app, "Active", false).await;
    let book_id = book["id"].as_i64().expect("book id");
    assert_eq!(book["status"], "active");
    let track_id = main_track(&app, book_id).await;

    let (status, first) = try_create_chapter(&app, track_id).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["number"], 1);
    let (status, second) = try_create_chapter(&app, track_id).await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    assert_eq!(second["number"], 2);
}
