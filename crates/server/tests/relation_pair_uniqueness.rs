//! One active relation per ordered entity pair (spec/06 §1.2, D-143).
//!
//! The author's report: one entity's relation list showed several near-identical lines for the same
//! pair. The cause was not a missing uniqueness rule for the same wording — that index existed and
//! `ensure_no_active_duplicate` enforced it, returning `409` for an exact
//! `(source, target, kind)` triple. It was that `kind` was **part of the identity**: the archivist
//! re-created a pair it had already recorded, with the bond re-worded, and every re-wording left a
//! new active row. `ended` rows never blocked, so history grew quietly.
//!
//! The rule these tests pin down: `kind` is data about a bond, not part of its identity. One
//! **active** relation per ordered `(source, target)` pair per canon.
//!
//! Four properties, and they pull in different directions:
//!
//! 1. A second active relation for a pair is refused, and the error names the existing row so the
//!    caller can edit it (`conflict.exists`, `details.id`).
//! 2. The pair is **ordered**: `(A, B)` and `(B, A)` are different relations and both may be
//!    active. The author's real canon depends on this (父亲 / 嫡长子, 爱人 in both directions), so a
//!    symmetric rule would reject correct data.
//! 3. `ended` rows are history and never block: a bond that lapses and resumes gets a new active
//!    row.
//! 4. The two canons stay separate: an isolated track's private relation over a pair that already
//!    has a shared active relation is legal, and a shared write never sees a private row.

use axum::http::StatusCode;
use nekowriter::services::canon::{self, CanonWrite, Visibility, relation};
use nekowriter::testing::TestApp;
use serde_json::json;

mod canon_fixture;

use canon_fixture::{book, isolated_track};

/// Creates an entity through the API and returns its id.
async fn character(app: &TestApp, book_id: i32, name: &str) -> i32 {
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/entities"),
            json!({ "kind": "character", "name": name, "summary": format!("{name} summary") }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_i64().expect("id") as i32
}

/// The relations of one pair, whatever their status, as `(id, kind, status)`.
async fn pair_rows(app: &TestApp, book_id: i32, a: i32, b: i32) -> Vec<(i32, String, String)> {
    let (status, body) = app
        .get(&format!("/api/books/{book_id}/relations?entity_id={a}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body.as_array()
        .expect("relations")
        .iter()
        .filter(|row| {
            row["source_id"] == json!(a) && row["target_id"] == json!(b)
                || row["source_id"] == json!(b) && row["target_id"] == json!(a)
        })
        .map(|row| {
            (
                row["id"].as_i64().expect("id") as i32,
                row["kind"].as_str().expect("kind").to_owned(),
                row["status"].as_str().expect("status").to_owned(),
            )
        })
        .collect()
}

// ───────────────────────── 1. the duplicate is refused ─────────────────────────

/// A second active relation for the same ordered pair is `conflict.exists`, even though the kind
/// differs — this is the exact shape the author's data is full of, and the reason the old
/// triple-keyed check did not fire on it.
#[tokio::test]
async fn a_second_active_relation_for_one_pair_is_a_conflict() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let source = character(&app, book_id, "Lin Fan").await;
    let target = character(&app, book_id, "Zhang San").await;

    let (status, created) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": source, "target_id": target, "kind": "senior brother" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let kept = created["id"].as_i64().expect("id") as i32;

    // The re-worded kind. Under the old rule this was a legal second active row.
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": source, "target_id": target, "kind": "senior fellow disciple" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], json!("conflict.exists"), "{body}");
    assert_eq!(
        body["error"]["details"]["id"],
        json!(kept),
        "the error names the existing relation so the caller can edit it: {body}"
    );

    // The wording of the existing row survives; nothing was merged or overwritten.
    let rows = pair_rows(&app, book_id, source, target).await;
    assert_eq!(rows.len(), 1, "one row for the pair: {rows:?}");
    assert_eq!(rows[0].1, "senior brother");
}

/// The same kind is refused too: it always was, and the pair rule must not lose that case.
#[tokio::test]
async fn a_second_active_relation_with_the_same_kind_is_still_a_conflict() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let source = character(&app, book_id, "Lin Fan").await;
    let target = character(&app, book_id, "Zhang San").await;
    let body = json!({ "source_id": source, "target_id": target, "kind": "ally" });

    let (status, _) = app
        .post(&format!("/api/books/{book_id}/relations"), body.clone())
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, response) = app
        .post(&format!("/api/books/{book_id}/relations"), body)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{response}");
    assert_eq!(response["error"]["code"], json!("conflict.exists"));
}

// ───────────────────── 2. the pair is ordered, both ways work ─────────────────────

/// `(A, B)` and `(B, A)` are different relations: each direction keeps one active row. The author's
/// canon holds `(3,4) 父亲` and `(4,3) 嫡长子` as two correct rows, so a symmetric guard would
/// destroy real data.
#[tokio::test]
async fn the_two_directions_of_a_pair_are_separate_relations() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let father = character(&app, book_id, "Cheng Zu").await;
    let son = character(&app, book_id, "Zhu Gaochi").await;

    let (status, forward) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": father, "target_id": son, "kind": "father" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{forward}");

    let (status, backward) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": son, "target_id": father, "kind": "eldest son" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the reverse direction is a separate relation: {backward}"
    );
    assert_ne!(forward["id"], backward["id"]);

    // Both stay active: the pair keeps one relation in each direction.
    let (status, relations) = app
        .get(&format!(
            "/api/books/{book_id}/relations?entity_id={father}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    let rows: Vec<&serde_json::Value> = relations
        .as_array()
        .expect("relations")
        .iter()
        .filter(|row| row["status"] == json!("active"))
        .collect();
    assert_eq!(
        rows.len(),
        2,
        "one active relation per direction: {relations}"
    );
    assert!(rows.iter().any(|row| row["kind"] == json!("father")));
    assert!(rows.iter().any(|row| row["kind"] == json!("eldest son")));
}

/// The reverse direction is still one relation, not two: a second `(B, A)` row with different
/// wording collides with `(B, A)`, not with `(A, B)`.
#[tokio::test]
async fn a_reworded_reverse_relation_collides_with_its_own_direction_only() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let father = character(&app, book_id, "Cheng Zu").await;
    let son = character(&app, book_id, "Zhu Gaochi").await;
    for (source, target, kind) in [(father, son, "father"), (son, father, "eldest son")] {
        let (status, _) = app
            .post(
                &format!("/api/books/{book_id}/relations"),
                json!({ "source_id": source, "target_id": target, "kind": kind }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": son, "target_id": father, "kind": "heir" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "only the (son, father) direction is taken: {body}"
    );
    assert_eq!(
        body["error"]["details"]["id"],
        json!(2),
        "the reverse row is #2"
    );
}

// ─────────────────── 3. ended history never blocks a new bond ───────────────────

/// An `ended` relation is history. A bond that lapses and resumes gets a **new** active row and the
/// old one stays `ended` — this is how the author's resumed pairs already look (id 1 `ended` and id
/// 61 active, same kind).
#[tokio::test]
async fn an_ended_relation_does_not_block_a_new_active_one() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let source = character(&app, book_id, "Lin Fan").await;
    let target = character(&app, book_id, "Zhang San").await;
    let (status, first) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": source, "target_id": target, "kind": "ally" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let first_id = first["id"].as_i64().expect("id");

    let (status, _) = app
        .patch(
            &format!("/api/relations/{first_id}"),
            json!({ "status": "ended" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "end the bond");

    let (status, second) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": source, "target_id": target, "kind": "sworn ally" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "a resumed bond gets a new active row: {second}"
    );
    assert_ne!(second["id"], json!(first_id));

    let rows = pair_rows(&app, book_id, source, target).await;
    assert_eq!(rows.len(), 2, "the history plus the new bond: {rows:?}");
    assert_eq!(
        rows.iter().filter(|row| row.2 == "active").count(),
        1,
        "exactly one active relation for the pair"
    );
    assert_eq!(
        rows.iter().filter(|row| row.2 == "ended").count(),
        1,
        "the original row stays as history"
    );
}

/// Reverting the delete of an `ended` relation brings it back: history is never blocked by the
/// pair rule, because an `ended` row is not active.
#[tokio::test]
async fn reverting_the_delete_of_an_ended_relation_recreates_it() {
    use nekowriter::db::entities::change_log;
    use nekowriter::db::json::ChangeTargetType;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let source = character(&app, book_id, "Lin Fan").await;
    let target = character(&app, book_id, "Zhang San").await;
    let (status, created) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": source, "target_id": target, "kind": "ally" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let relation_id = created["id"].as_i64().expect("id") as i32;
    // End it, then a new bond takes the pair's single active slot.
    let (status, _) = app
        .patch(
            &format!("/api/relations/{relation_id}"),
            json!({ "status": "ended" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, second) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": source, "target_id": target, "kind": "sworn ally" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");

    // Delete the ended row and revert that delete: the recreated row is `ended`, so it does not
    // collide with the active bond the pair now carries.
    let (status, _) = app.delete(&format!("/api/relations/{relation_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let delete_change = change_log::Entity::find()
        .filter(change_log::Column::TargetType.eq(ChangeTargetType::Relation.as_str()))
        .filter(change_log::Column::TargetId.eq(relation_id))
        .filter(change_log::Column::Op.eq("delete"))
        .order_by_desc(change_log::Column::Id)
        .one(app.db().conn())
        .await
        .expect("query")
        .expect("the delete change");

    let (status, reverted) = app
        .post(
            &format!("/api/changes/{}/revert", delete_change.id),
            json!({}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "recreating an ended relation is always allowed: {reverted}"
    );
    let recreated = reverted["target_id"].as_i64().expect("new id") as i32;
    assert_ne!(recreated, relation_id, "a recreated row takes a new id");
    let (status, row) = app.get(&format!("/api/relations/{recreated}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(row["status"], json!("ended"), "history stays ended");

    let rows = pair_rows(&app, book_id, source, target).await;
    assert_eq!(rows.len(), 2, "history plus the active bond: {rows:?}");
    assert_eq!(
        rows.iter().filter(|row| row.2 == "active").count(),
        1,
        "still exactly one active relation for the pair"
    );
}

// ──────────────── 4. the two canons keep their own pair rules ────────────────

/// An isolated track may hold its own private relation over a pair that already has a shared active
/// relation: the shared row belongs to a canon the track cannot write, and its private copy is a
/// different row (spec/06 §2). The pair rule must not conflate them.
#[tokio::test]
async fn an_isolated_track_may_relate_a_pair_that_shared_canon_already_related() {
    use nekowriter::db::entities::relations;
    use sea_orm::{DatabaseConnection, EntityTrait};

    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let first = character(&app, book_id, "Shared One").await;
    let second = character(&app, book_id, "Shared Two").await;
    let isolated = isolated_track(&app, book_id, "Side story").await;

    let (status, shared) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": first, "target_id": second, "kind": "ally" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{shared}");

    let write = CanonWrite::author().with_visibility(Visibility::isolated(book_id, isolated));
    let private = canon::write(app.db(), move |tx| {
        Box::pin(async move {
            relation::create_inner(
                tx,
                &write,
                book_id,
                nekowriter::api::dto::canon::RelationInput {
                    source_id: first,
                    target_id: second,
                    kind: "secret ally".to_owned(),
                    inverse_kind: String::new(),
                    description: String::new(),
                    since_chapter_id: None,
                    until_chapter_id: None,
                    note: None,
                },
            )
            .await
        })
    })
    .await;
    let private = private.expect("an isolated track keeps its own relation for the pair");

    // The two rows are separate: one shared, one private, and both active.
    let conn: &DatabaseConnection = app.db().conn();
    let rows = relations::Entity::find()
        .all(conn)
        .await
        .expect("relations");
    let active: Vec<_> = rows.iter().filter(|row| row.status == "active").collect();
    assert_eq!(
        active.len(),
        2,
        "one shared and one private row: {active:?}"
    );
    assert_eq!(
        active.iter().filter(|row| row.track_id.is_none()).count(),
        1,
        "the shared row stays the only shared one"
    );
    assert_eq!(
        active
            .iter()
            .filter(|row| row.track_id == Some(isolated))
            .count(),
        1,
        "the track's private row carries the track id"
    );
    assert_eq!(private.0.track_id, Some(isolated));
}

/// The private row is invisible to the shared canon: a shared write still sees its own row and
/// rejects a duplicate, and the private wording never leaks into the shared error.
#[tokio::test]
async fn the_shared_canon_does_not_see_a_private_relation_for_the_pair() {
    use nekowriter::api::dto::canon::RelationInput;

    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let first = character(&app, book_id, "Shared One").await;
    let second = character(&app, book_id, "Shared Two").await;
    let isolated = isolated_track(&app, book_id, "Side story").await;

    // A private row only, over two shared entities.
    let write = CanonWrite::author().with_visibility(Visibility::isolated(book_id, isolated));
    canon::write(app.db(), move |tx| {
        Box::pin(async move {
            relation::create_inner(
                tx,
                &write,
                book_id,
                RelationInput {
                    source_id: first,
                    target_id: second,
                    kind: "secret ally".to_owned(),
                    inverse_kind: String::new(),
                    description: String::new(),
                    since_chapter_id: None,
                    until_chapter_id: None,
                    note: None,
                },
            )
            .await
        })
    })
    .await
    .expect("private relation");

    // The shared canon has no active relation for the pair, so a shared write is accepted.
    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": first, "target_id": second, "kind": "ally" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the shared canon is independent of the track: {body}"
    );

    // And the shared canon now has exactly one active row for the pair.
    let (status, relations) = app.get(&format!("/api/books/{book_id}/relations")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        relations
            .as_array()
            .expect("relations")
            .iter()
            .filter(|row| row["status"] == json!("active"))
            .count(),
        1,
        "the shared list shows only the shared row: {relations}"
    );
}

// ───────────────────────── 5. merging keeps the pair rule ─────────────────────────

/// Merging B into A rewrites B's relations to point at A. A moved row that lands on a pair A already
/// has must be dropped **even when the wording differs**: `kind` is not part of the identity, so
/// keeping it would leave the merged entity with two active relations for one pair — the very shape
/// the author reported, produced by a merge instead of by the archivist.
#[tokio::test]
async fn merging_drops_a_moved_relation_that_rewords_a_taken_pair() {
    let app = TestApp::new().await;
    let (book_id, _track) = book(&app).await;
    let x = character(&app, book_id, "Zhang San").await;
    let a = character(&app, book_id, "Lin Fan").await;
    let b = character(&app, book_id, "Lin Fann").await;
    // A already has `x → a`. B has `x → b` with *different* wording: after the merge both become
    // `x → a`, so one has to go.
    let (status, kept) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": x, "target_id": a, "kind": "mentor" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{kept}");
    let (status, _) = app
        .post(
            &format!("/api/books/{book_id}/relations"),
            json!({ "source_id": x, "target_id": b, "kind": "old mentor" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = app
        .post(&format!("/api/entities/{a}/merge"), json!({ "from_id": b }))
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the reworded duplicate is dropped instead of failing the merge: {body}"
    );

    let (status, relations) = app
        .get(&format!("/api/books/{book_id}/relations?entity_id={a}"))
        .await;
    assert_eq!(status, StatusCode::OK);
    let active: Vec<&serde_json::Value> = relations
        .as_array()
        .expect("relations")
        .iter()
        .filter(|row| row["status"] == json!("active"))
        .collect();
    assert_eq!(
        active.len(),
        1,
        "one active relation for the merged pair: {relations}"
    );
    assert_eq!(
        active[0]["id"],
        json!(kept["id"]),
        "A's own relation is the survivor: {relations}"
    );
    assert_eq!(active[0]["kind"], json!("mentor"), "with A's wording");
}
