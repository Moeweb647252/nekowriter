//! M5 review regressions for the timeline (spec/06 §1.4, spec/02 §1, spec/10 §2.4).
//!
//! The milestone's acceptance names midpoint insertion, renormalization below the 1e-6 gap, the
//! `after_id`/`before_id` placement, and a story order with no collisions. The suite had no
//! timeline coverage at all, so these tests pin the rules the service implements:
//!
//! - insertion between neighbors uses the midpoint; append/prepend stay ordered;
//! - a gap below 1e-6 renormalizes the whole timeline to multiples of 1024 first;
//! - a gap of exactly 1e-6 is still usable (the spec says "below 1e-6");
//! - `after_id` and `before_id` are mutually exclusive on create, patch and move;
//! - moving an event keeps every position distinct and the list in story order.

mod canon_fixture;

use axum::http::StatusCode;
use nekowriter::testing::TestApp;
use serde_json::{Value, json};

use canon_fixture::book;

/// Creates one timeline event through the API and returns its DTO.
async fn create(
    app: &TestApp,
    book_id: i32,
    title: &str,
    after: Option<i32>,
    before: Option<i32>,
) -> Value {
    let mut body = json!({ "title": title });
    if let Some(after) = after {
        body["after_id"] = json!(after);
    }
    if let Some(before) = before {
        body["before_id"] = json!(before);
    }
    let (status, event) = app
        .post(&format!("/api/books/{book_id}/timeline"), body)
        .await;
    assert_eq!(status, StatusCode::CREATED, "create {title}: {event}");
    event
}

/// Sets one event's position directly (the fixtures need positions the API only produces after
/// many insertions).
async fn set_position(app: &TestApp, event_id: i32, position: f64) {
    app.execute(&format!(
        "UPDATE timeline_events SET position = {position} WHERE id = {event_id}"
    ))
    .await;
}

/// The timeline in story order.
async fn timeline(app: &TestApp, book_id: i32) -> Vec<Value> {
    let (status, body) = app.get(&format!("/api/books/{book_id}/timeline")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body.as_array().expect("events").clone()
}

fn id_of(event: &Value) -> i32 {
    event["id"].as_i64().expect("event id") as i32
}

fn position_of(event: &Value) -> f64 {
    event["position"].as_f64().expect("position")
}

/// Insertion between two neighbors uses the midpoint; prepend and append stay in story order.
#[tokio::test]
async fn midpoint_insertion_keeps_story_order() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let first = create(&app, book_id, "First", None, None).await;
    let second = create(&app, book_id, "Second", None, None).await;
    let third = create(&app, book_id, "Third", None, None).await;
    assert!(
        position_of(&first) < position_of(&second),
        "appends advance the position"
    );
    assert!(position_of(&second) < position_of(&third));

    // Between First and Second: the anchor names one neighbor, the other follows from the order,
    // and the new event takes their midpoint.
    let middle = create(&app, book_id, "Middle", Some(id_of(&first)), None).await;
    let expected = (position_of(&first) + position_of(&second)) / 2.0;
    assert_eq!(position_of(&middle), expected, "the midpoint");

    // Before the first event.
    let start = create(&app, book_id, "Start", None, Some(id_of(&first))).await;
    assert!(position_of(&start) < position_of(&first));

    // After the last event.
    let end = create(&app, book_id, "End", Some(id_of(&third)), None).await;
    assert!(position_of(&third) < position_of(&end));

    let rows = timeline(&app, book_id).await;
    let titles: Vec<&str> = rows
        .iter()
        .map(|event| event["title"].as_str().expect("title"))
        .collect();
    assert_eq!(
        titles,
        vec!["Start", "First", "Middle", "Second", "Third", "End"],
        "story order is the position order"
    );
}

/// A gap below 1e-6 is renormalized to multiples of 1024 before the insert (spec/02 §1).
#[tokio::test]
async fn a_gap_below_the_threshold_renormalizes_the_timeline() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let first = create(&app, book_id, "First", None, None).await;
    let second = create(&app, book_id, "Second", None, None).await;
    let third = create(&app, book_id, "Third", None, None).await;
    set_position(&app, id_of(&first), 0.0).await;
    set_position(&app, id_of(&second), 1e-7).await;
    set_position(&app, id_of(&third), 1.0).await;

    let inserted = create(&app, book_id, "Between", Some(id_of(&first)), None).await;
    let rows = timeline(&app, book_id).await;
    let positions: Vec<f64> = rows.iter().map(position_of).collect();
    assert_eq!(
        positions.len(),
        4,
        "the timeline gained one event: {rows:?}"
    );
    // Renormalized to 1024, 2048, … with the new event at the midpoint of the first gap.
    assert_eq!(
        position_of(&inserted),
        1536.0,
        "midpoint after renormalizing"
    );
    let others: Vec<f64> = rows
        .iter()
        .filter(|event| event["title"] != json!("Between"))
        .map(position_of)
        .collect();
    for position in others {
        assert_eq!(
            position % 1024.0,
            0.0,
            "renormalized positions are multiples of 1024: {positions:?}"
        );
    }
    let titles: Vec<&str> = rows
        .iter()
        .map(|event| event["title"].as_str().expect("title"))
        .collect();
    assert_eq!(titles, vec!["First", "Between", "Second", "Third"]);
}

/// A gap of exactly 1e-6 is usable: the spec renormalizes only when the gap is *below* 1e-6.
#[tokio::test]
async fn a_gap_of_exactly_the_threshold_is_not_renormalized() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let first = create(&app, book_id, "First", None, None).await;
    let second = create(&app, book_id, "Second", None, None).await;
    set_position(&app, id_of(&first), 0.0).await;
    set_position(&app, id_of(&second), 1e-6).await;

    create(&app, book_id, "Between", Some(id_of(&first)), None).await;
    // Read the stored rows: the DTOs above are snapshots from before the SQL fixture.
    let rows = timeline(&app, book_id).await;
    let position_of_title = |title: &str| {
        position_of(
            rows.iter()
                .find(|event| event["title"] == json!(title))
                .expect("event"),
        )
    };
    assert_eq!(
        position_of_title("First"),
        0.0,
        "the timeline was not renormalized"
    );
    assert_eq!(
        position_of_title("Second"),
        1e-6,
        "the timeline was not renormalized"
    );
    assert_eq!(
        position_of_title("Between"),
        5e-7,
        "the midpoint of an exactly-threshold gap"
    );
}

/// `after_id` and `before_id` cannot be combined on create, patch or move (spec/05 §1.5).
#[tokio::test]
async fn after_and_before_are_mutually_exclusive() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let first = create(&app, book_id, "First", None, None).await;
    let second = create(&app, book_id, "Second", None, None).await;

    let (status, body) = app
        .post(
            &format!("/api/books/{book_id}/timeline"),
            json!({
                "title": "Both",
                "after_id": id_of(&first),
                "before_id": id_of(&second),
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], json!("validation.failed"));
    let fields = body["error"]["details"]["fields"]
        .as_array()
        .expect("fields");
    assert!(fields.contains(&json!("after_id")) && fields.contains(&json!("before_id")));

    let (status, body) = app
        .post(
            &format!("/api/timeline-events/{}/move", id_of(&second)),
            json!({ "after_id": id_of(&first), "before_id": id_of(&first) }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], json!("validation.failed"));

    let (status, body) = app
        .patch(
            &format!("/api/timeline-events/{}", id_of(&second)),
            json!({ "after_id": id_of(&first), "before_id": id_of(&first) }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], json!("validation.failed"));
}

/// Moving an event keeps positions distinct and the returned list in story order (spec/06 §1.4).
#[tokio::test]
async fn moving_an_event_never_collides_and_keeps_story_order() {
    let app = TestApp::new().await;
    let (book_id, _track_id) = book(&app).await;
    let first = create(&app, book_id, "First", None, None).await;
    let second = create(&app, book_id, "Second", None, None).await;
    let third = create(&app, book_id, "Third", None, None).await;

    // Move the last event before the first one.
    let (status, body) = app
        .post(
            &format!("/api/timeline-events/{}/move", id_of(&third)),
            json!({ "before_id": id_of(&first) }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = timeline(&app, book_id).await;
    let titles: Vec<&str> = rows
        .iter()
        .map(|event| event["title"].as_str().expect("title"))
        .collect();
    assert_eq!(titles, vec!["Third", "First", "Second"]);
    let positions: Vec<f64> = rows.iter().map(position_of).collect();
    let mut unique = positions.clone();
    unique.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    unique.dedup();
    assert_eq!(unique.len(), positions.len(), "positions never collide");
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "story order is strictly increasing: {positions:?}"
    );

    // A new event appended after the move does not collide either.
    let appended = create(&app, book_id, "Fourth", Some(id_of(&second)), None).await;
    assert!(position_of(&appended) > position_of(&rows[2]));
    let _ = second;
}
