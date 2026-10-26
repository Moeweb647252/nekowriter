//! The library list carries every book status, and the `?status=` filter stays exact
//! (spec/10 §2.2, PROJECT §8.3, D-129).
//!
//! Why this exists: the library page used to ask for `?status=active` whenever "Show archived"
//! was off. `books::list` filters server-side, so a book in **conception** never reached the
//! browser and the page's own archived filter could not bring it back — the book was unreachable
//! from the library from creation until finalize. The fix is client-side (the page fetches every
//! status once), which means the endpoint's unfiltered response has to be the full list, not just
//! the active books it happened to be asked for. Both halves are pinned here: no `status` returns
//! all three, `?status=active` still returns active only.
//!
//! A conceiving book is created through the API with `start_conception: true`. With no conception
//! model configured the session is not started (the response carries a `model.missing_role`
//! warning), but the book is still `conceiving` — which is precisely the state an author is left
//! in when they close the studio or the provider refuses the first turn.

use axum::http::StatusCode;
use nekowriter::testing::TestApp;
use serde_json::{Value, json};

/// A book whose status the caller controls, through `POST /api/books`.
async fn create_book(app: &TestApp, title: &str, start_conception: bool) -> Value {
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
    assert_eq!(status, StatusCode::CREATED, "create {title}: {body}");
    body["book"].clone()
}

#[tokio::test]
async fn the_unfiltered_list_contains_every_book_status() {
    let app = TestApp::new().await;
    let conceiving = create_book(&app, "带着蓝色大肥鱼穿洪武", true).await;
    let active = create_book(&app, "Manual Check Book", false).await;
    let archived = create_book(&app, "Sessions Check", false).await;
    assert_eq!(conceiving["status"], "conceiving");
    assert_eq!(active["status"], "active");

    // Archive the third one so all three wire values are present at once.
    let (status, body) = app
        .patch(
            &format!("/api/books/{}", archived["id"]),
            json!({ "status": "archived" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "archive: {body}");
    assert_eq!(body["status"], "archived");

    // The unfiltered list is what the library now asks for: three books, three statuses. Before
    // the fix this test failed at the conceiving entry — the page never saw it because it always
    // sent `?status=active`.
    let (status, list) = app.get("/api/books").await;
    assert_eq!(status, StatusCode::OK);
    let list = list.as_array().expect("books");
    assert_eq!(list.len(), 3, "the unfiltered list must hold every book");
    let statuses: Vec<&str> = list
        .iter()
        .map(|book| book["status"].as_str().expect("status"))
        .collect();
    assert!(
        statuses.contains(&"conceiving"),
        "a conceiving book must be in the unfiltered list, got {statuses:?}"
    );
    assert!(statuses.contains(&"active"), "got {statuses:?}");
    assert!(statuses.contains(&"archived"), "got {statuses:?}");

    // The conceiving summary is a normal one: zero chapters and no latest chapter, which is what
    // makes the card fall back to the studio link instead of a chapter line.
    let conceiving_row = list
        .iter()
        .find(|book| book["id"] == conceiving["id"])
        .expect("the conceiving book");
    assert_eq!(conceiving_row["chapter_count"], 0);
    assert!(conceiving_row["latest_chapter"].is_null());
}

#[tokio::test]
async fn the_status_filter_stays_exact() {
    let app = TestApp::new().await;
    let conceiving = create_book(&app, "Conceiving", true).await;
    let active = create_book(&app, "Active", false).await;
    let archived = create_book(&app, "Archived", false).await;
    let (status, _) = app
        .patch(
            &format!("/api/books/{}", archived["id"]),
            json!({ "status": "archived" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // `?status=active` is the contract other screens rely on and it must not regress: it excludes
    // both the conceiving book and the archived one.
    let (status, active_only) = app.get("/api/books?status=active").await;
    assert_eq!(status, StatusCode::OK);
    let active_only = active_only.as_array().expect("books");
    assert_eq!(active_only.len(), 1, "active only: {active_only:?}");
    assert_eq!(active_only[0]["id"], active["id"]);

    // The other two values filter just as exactly.
    let (_, conceiving_only) = app.get("/api/books?status=conceiving").await;
    let conceiving_only = conceiving_only.as_array().expect("books");
    assert_eq!(conceiving_only.len(), 1);
    assert_eq!(conceiving_only[0]["id"], conceiving["id"]);

    let (_, archived_only) = app.get("/api/books?status=archived").await;
    let archived_only = archived_only.as_array().expect("books");
    assert_eq!(archived_only.len(), 1);
    assert_eq!(archived_only[0]["id"], archived["id"]);

    // `status=` and `status=all` mean "no filter" (spec/10 §2.2), used by the switcher pages.
    for path in ["/api/books?status=", "/api/books?status=all"] {
        let (status, all) = app.get(path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(all.as_array().expect("books").len(), 3, "{path}");
    }

    // An unknown value is still rejected rather than silently ignored.
    let (status, body) = app.get("/api/books?status=finished").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "validation.failed");
}

/// Gives a book one chapter by inserting the row directly.
///
/// This is the shape the author's own database has: book 3 is `conceiving` **and** owns a
/// `planned` chapter, created before `chapters::create` began refusing non-active books (D-130).
/// The API can no longer produce that state, so the fixture writes the row itself — a test that
/// went through `POST /tracks/:id/chapters` would be pinned to a rule the historical data predates.
async fn seed_chapter(app: &TestApp, book_id: i64) {
    let (_, tracks) = app.get(&format!("/api/books/{book_id}/tracks")).await;
    let track_id = tracks
        .as_array()
        .and_then(|tracks| tracks.first())
        .and_then(|track| track["id"].as_i64())
        .expect("the book's main track");
    let sql = format!(
        "INSERT INTO chapters (book_id, track_id, ord, title, status, content, created_at, updated_at) \
         VALUES ({book_id}, {track_id}, 1, '', 'planned', '{{\"paragraphs\":[],\"next_pid\":1,\
\"tail_open\":false}}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
    );
    app.db().write_unprepared(&sql).await.expect("seed chapter");
}

#[tokio::test]
async fn a_conceiving_book_that_already_has_a_chapter_stays_in_the_default_view() {
    // The exact combination that produced the bug report: one book per status, and the conceiving
    // one is **not empty** — it already has a chapter. A fixture where the conceiving book has no
    // chapters would still pass a "show conceiving books" fix while a card that hides the counts
    // of a conceiving book would go unnoticed, which is the failure this test exists to prevent.
    let app = TestApp::new().await;
    let conceiving = create_book(&app, "带着蓝色大肥鱼穿洪武", true).await;
    let active = create_book(&app, "Manual Check Book", false).await;
    let archived = create_book(&app, "Old Draft", false).await;
    seed_chapter(&app, conceiving["id"].as_i64().expect("id")).await;
    let (status, _) = app
        .patch(
            &format!("/api/books/{}", archived["id"]),
            json!({ "status": "archived" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // The unfiltered list — what the library asks for — holds all three rows.
    let (status, list) = app.get("/api/books").await;
    assert_eq!(status, StatusCode::OK);
    let list = list.as_array().expect("books");
    assert_eq!(list.len(), 3, "every status is in the unfiltered list");

    // The conceiving book carries its real chapter through the API, so the card can show it. This
    // is the assertion that keeps the card from replacing the counts with a "nothing here" block.
    let conceiving_row = list
        .iter()
        .find(|book| book["id"] == conceiving["id"])
        .expect("the conceiving book");
    assert_eq!(conceiving_row["status"], "conceiving");
    assert_eq!(
        conceiving_row["chapter_count"], 1,
        "a conceiving book can already own a chapter: {conceiving_row}"
    );
    assert_eq!(conceiving_row["word_count"], 0);
    assert_eq!(
        conceiving_row["latest_chapter"]["number"], 1,
        "the latest chapter travels with the summary: {conceiving_row}"
    );

    // The default view the author sees ("Show archived" off) is conceiving + active: the row that
    // was missing before the fix is present, and only the archived one is hidden.
    let default_view: Vec<&str> = list
        .iter()
        .filter(|book| book["status"] != "archived")
        .map(|book| book["status"].as_str().expect("status"))
        .collect();
    assert_eq!(default_view.len(), 2);
    assert!(default_view.contains(&"conceiving"), "{default_view:?}");
    assert!(default_view.contains(&"active"), "{default_view:?}");
    assert!(!default_view.contains(&"archived"), "{default_view:?}");
    // And the archived book is in the payload the toggle reveals, not lost by the fetch.
    assert!(
        list.iter().any(|book| book["status"] == "archived"),
        "the archived row must be fetched so the toggle can reveal it"
    );

    // The existing contract is untouched: an exact status filter still excludes the other two.
    let (_, active_only) = app.get("/api/books?status=active").await;
    let active_only = active_only.as_array().expect("books");
    assert_eq!(active_only.len(), 1);
    assert_eq!(active_only[0]["id"], active["id"]);

    let (_, conceiving_only) = app.get("/api/books?status=conceiving").await;
    let conceiving_only = conceiving_only.as_array().expect("books");
    assert_eq!(conceiving_only.len(), 1);
    assert_eq!(conceiving_only[0]["id"], conceiving["id"]);
    assert_eq!(conceiving_only[0]["chapter_count"], 1);
}
