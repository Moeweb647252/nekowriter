//! Writing skills: schema, catalogue, selection and user-row CRUD (design §4, §8; spec/10 §2.9).
//!
//! The rules under test are the traps of the design:
//! - the built-in row is seeded by migration `0005`, filled by `0007`, and read-only;
//! - a user row never carries a `key`, and user rows may repeat `key = NULL`;
//! - deleting a user row cascades its selections away and tells the caller which books used it;
//! - a book's selection never leaks into another book;
//! - new books select `enabled_by_default = 1` skills.

use nekowriter::services::skills;
use nekowriter::testing::{TestApp, assert_error};
use serde_json::{Value, json};

/// The seeded built-in skill the migration inserts (design §5).
async fn builtin_skill(app: &TestApp) -> Value {
    let (status, body) = app.get("/api/writing-skills").await;
    assert_eq!(status, 200, "list skills: {body}");
    body.as_array()
        .expect("array")
        .iter()
        .find(|row| row["key"] == "basic-writing")
        .cloned()
        .expect("the migration seeds basic-writing")
}

/// Creates a book and returns its id (via the API, so track/docs exist).
async fn book(app: &TestApp, title: &str) -> i32 {
    let created = app.create_book(title).await;
    created["id"].as_i64().expect("id") as i32
}

#[tokio::test]
async fn the_migration_seeds_the_read_only_basic_writing_skill() {
    let app = TestApp::new().await;
    let skill = builtin_skill(&app).await;
    assert_eq!(skill["name"], "基础写作 / Basic Writing");
    assert_eq!(skill["category"], "写作 / Writing");
    assert_eq!(skill["author"], "builtin");
    // The body ships with the first built-in skill (D-156); before that it was empty.
    let body = skill["body"].as_str().expect("body");
    assert!(body.starts_with("### 1. 呼吸与落段"), "{body}");
    assert_eq!(body.chars().count(), 1025, "{body}");
    assert!(body.ends_with("点破背后的逻辑。"), "{body}");
    assert_eq!(
        skill["description"],
        "剥离机器的工整感，用带着企图的观察、错位的对话交锋、单行短句的呼吸节奏与粗粝的生理反馈，写出真人的掌控力。"
    );
    assert_eq!(skill["enabled_by_default"], false);
    assert_eq!(skill["retired"], false);
    assert_eq!(skill["selected"], false);
    assert!(skill["key"].is_string(), "a built-in row has a key");
    assert_eq!(skill["used_by_books"], json!([]));
}

#[tokio::test]
async fn user_rows_are_created_without_a_key_and_round_trip() {
    let app = TestApp::new().await;
    let (status, created) = app
        .post(
            "/api/writing-skills",
            json!({
                "name": "  冷硬文风  ",
                "category": " 文风 ",
                "description": " 短句，少形容词 ",
                "body": "用短句推进。",
            }),
        )
        .await;
    assert_eq!(status, 201, "create skill: {created}");
    assert_eq!(created["name"], "冷硬文风");
    assert_eq!(created["category"], "文风");
    assert_eq!(created["description"], "短句，少形容词");
    assert_eq!(created["body"], "用短句推进。");
    assert_eq!(created["author"], "user");
    assert_eq!(created["key"], Value::Null);

    let (status, all) = app.get("/api/writing-skills").await;
    assert_eq!(status, 200);
    assert_eq!(all.as_array().expect("array").len(), 2);

    // Two user rows both have `key = NULL`; the UNIQUE index must only constrain built-ins.
    let (status, second) = app
        .post(
            "/api/writing-skills",
            json!({ "name": "第二技能", "body": "" }),
        )
        .await;
    assert_eq!(status, 201, "second user row: {second}");
}

#[tokio::test]
async fn a_user_row_with_the_builtin_key_is_rejected_by_the_schema() {
    let app = TestApp::new().await;
    let result = app
        .try_execute(
            "INSERT INTO writing_skills(key, name, category, author, description, body, \
             enabled_by_default, retired, created_at, updated_at) \
             VALUES ('basic-writing', 'copy', '', 'user', '', '', 0, 0, '2026-01-01T00:00:00Z', \
             '2026-01-01T00:00:00Z')",
        )
        .await;
    assert!(result.is_err(), "a duplicate built-in key must fail");
}

#[tokio::test]
async fn the_catalogue_marks_the_books_selection_and_never_leaks_between_books() {
    let app = TestApp::new().await;
    let first_book = book(&app, "A").await;
    let second_book = book(&app, "B").await;
    let (_, created) = app
        .post(
            "/api/writing-skills",
            json!({ "name": "本书技能", "body": "只属于 A。" }),
        )
        .await;
    let skill_id = created["id"].as_i64().expect("id") as i32;
    let builtin = builtin_skill(&app).await["id"].as_i64().expect("id") as i32;

    let (status, selected) = app
        .put(
            &format!("/api/books/{first_book}/writing-skills"),
            json!({ "skill_ids": [skill_id, builtin] }),
        )
        .await;
    assert_eq!(status, 200, "replace selection: {selected}");
    let selected_rows = selected.as_array().expect("array");
    assert_eq!(
        selected_rows
            .iter()
            .filter(|row| row["selected"] == true)
            .count(),
        2
    );

    let (_, first_catalogue) = app
        .get(&format!("/api/writing-skills?book_id={first_book}"))
        .await;
    assert!(
        first_catalogue
            .as_array()
            .expect("array")
            .iter()
            .filter(|row| row["selected"] == true)
            .count()
            == 2
    );
    let (_, second_catalogue) = app
        .get(&format!("/api/writing-skills?book_id={second_book}"))
        .await;
    assert!(
        second_catalogue
            .as_array()
            .expect("array")
            .iter()
            .all(|row| row["selected"] == false)
    );

    // Replacing with an empty list clears the selection; a retired or missing id is refused.
    let (status, cleared) = app
        .put(
            &format!("/api/books/{first_book}/writing-skills"),
            json!({ "skill_ids": [] }),
        )
        .await;
    assert_eq!(status, 200);
    assert!(
        cleared
            .as_array()
            .expect("array")
            .iter()
            .all(|row| row["selected"] == false)
    );
    let (status, body) = app
        .put(
            &format!("/api/books/{first_book}/writing-skills"),
            json!({ "skill_ids": [9999] }),
        )
        .await;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");
}

#[tokio::test]
async fn built_in_rows_refuse_patch_and_delete_with_a_conflict() {
    let app = TestApp::new().await;
    let builtin = builtin_skill(&app).await["id"].as_i64().expect("id") as i32;

    let (status, body) = app
        .patch(
            &format!("/api/writing-skills/{builtin}"),
            json!({ "name": "改名" }),
        )
        .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT);
    assert_error(&body, "writing_skill.builtin");
    assert_eq!(body["error"]["details"]["id"], builtin);

    let (status, body) = app.delete(&format!("/api/writing-skills/{builtin}")).await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT);
    assert_error(&body, "writing_skill.builtin");

    // Still there and unchanged.
    let (_, all) = app.get("/api/writing-skills").await;
    assert_eq!(all.as_array().expect("array").len(), 1);
    assert_eq!(all[0]["name"], "基础写作 / Basic Writing");
}

#[tokio::test]
async fn patching_a_user_row_updates_only_what_was_sent() {
    let app = TestApp::new().await;
    let (_, created) = app
        .post(
            "/api/writing-skills",
            json!({ "name": "原名", "category": "原类", "description": "原说明", "body": "原正文" }),
        )
        .await;
    let skill_id = created["id"].as_i64().expect("id") as i32;
    let (status, patched) = app
        .patch(
            &format!("/api/writing-skills/{skill_id}"),
            json!({ "body": "新正文" }),
        )
        .await;
    assert_eq!(status, 200, "patch: {patched}");
    assert_eq!(patched["name"], "原名");
    assert_eq!(patched["category"], "原类");
    assert_eq!(patched["description"], "原说明");
    assert_eq!(patched["body"], "新正文");
}

#[tokio::test]
async fn deleting_a_user_skill_names_the_books_that_used_it_and_cascades() {
    let app = TestApp::new().await;
    let first_book = book(&app, "使用它的书").await;
    let second_book = book(&app, "另一本").await;
    let (_, created) = app
        .post(
            "/api/writing-skills",
            json!({ "name": "待删技能", "body": "x" }),
        )
        .await;
    let skill_id = created["id"].as_i64().expect("id") as i32;
    for book_id in [first_book, second_book] {
        let (status, _) = app
            .put(
                &format!("/api/books/{book_id}/writing-skills"),
                json!({ "skill_ids": [skill_id] }),
            )
            .await;
        assert_eq!(status, 200);
    }

    // The catalogue already carries the list, so the UI can name the books before confirming.
    let (_, catalogue) = app
        .get(&format!("/api/writing-skills?book_id={first_book}"))
        .await;
    let row = catalogue
        .as_array()
        .expect("array")
        .iter()
        .find(|row| row["id"] == skill_id)
        .expect("skill row");
    assert_eq!(
        row["used_by_books"],
        json!([
            { "id": first_book, "title": "使用它的书" },
            { "id": second_book, "title": "另一本" },
        ])
    );

    let (status, deleted) = app.delete(&format!("/api/writing-skills/{skill_id}")).await;
    assert_eq!(status, 200, "delete: {deleted}");
    assert_eq!(
        deleted["used_by_books"],
        json!([
            { "id": first_book, "title": "使用它的书" },
            { "id": second_book, "title": "另一本" },
        ])
    );

    // The selections cascaded away and the row is gone.
    let (_, first_catalogue) = app
        .get(&format!("/api/writing-skills?book_id={first_book}"))
        .await;
    assert!(
        first_catalogue
            .as_array()
            .expect("array")
            .iter()
            .all(|row| row["id"] != skill_id)
    );
}

#[tokio::test]
async fn toggling_another_skill_preserves_a_retired_selection() {
    let app = TestApp::new().await;
    let book_id = book(&app, "退役技能").await;
    let (_, created) = app
        .post(
            "/api/writing-skills",
            json!({ "name": "旧技能", "body": "旧正文。" }),
        )
        .await;
    let retired_id = created["id"].as_i64().expect("id") as i32;
    let (status, body) = app
        .put(
            &format!("/api/books/{book_id}/writing-skills"),
            json!({ "skill_ids": [retired_id] }),
        )
        .await;
    assert_eq!(status, 200, "select the future retired skill: {body}");
    // A later pack update retires the skill: the catalogue stops offering it, but the books that
    // already selected it keep working (design §4/§12).
    app.execute(&format!(
        "UPDATE writing_skills SET retired = 1 WHERE id = {retired_id}"
    ))
    .await;
    let (_, catalogue) = app
        .get(&format!("/api/writing-skills?book_id={book_id}"))
        .await;
    assert!(
        catalogue
            .as_array()
            .expect("array")
            .iter()
            .all(|row| row["id"] != retired_id),
        "a retired row is hidden from the active catalogue"
    );
    let builtin = builtin_skill(&app).await["id"].as_i64().expect("id") as i32;

    // Book settings derives its payload from the visible catalogue, so toggling the built-in sends
    // only the built-in id. The replacement must not silently drop the retired selection.
    let (status, body) = app
        .put(
            &format!("/api/books/{book_id}/writing-skills"),
            json!({ "skill_ids": [builtin] }),
        )
        .await;
    assert_eq!(status, 200, "toggle the built-in: {body}");
    let selected = skills::selected_skills(app.db().conn(), book_id)
        .await
        .expect("selected skills");
    let ids: Vec<i32> = selected.iter().map(|row| row.id).collect();
    assert_eq!(
        ids,
        vec![builtin.min(retired_id), builtin.max(retired_id)],
        "the retired selection survives another toggle"
    );
    assert!(ids.contains(&retired_id), "the retired row still injects");

    // An explicit retired id in the payload is still refused (spec/10 §2.9): the preservation
    // rule only covers rows the book already selected before the request.
    let (status, body) = app
        .put(
            &format!("/api/books/{book_id}/writing-skills"),
            json!({ "skill_ids": [retired_id] }),
        )
        .await;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    assert_error(&body, "validation.failed");
}

#[tokio::test]
async fn a_new_book_selects_enabled_by_default_skills_only() {
    let app = TestApp::new().await;
    // Turn the seeded built-in into a default-selected skill. The migration seeds `0`; this pins
    // the creation rule the design states for a future built-in that does opt in.
    app.execute("UPDATE writing_skills SET enabled_by_default = 1 WHERE key = 'basic-writing'")
        .await;
    let selected_book = book(&app, "默认选中").await;
    let (_, catalogue) = app
        .get(&format!("/api/writing-skills?book_id={selected_book}"))
        .await;
    assert_eq!(
        catalogue
            .as_array()
            .expect("array")
            .iter()
            .filter(|row| row["selected"] == true)
            .count(),
        1
    );

    app.execute("UPDATE writing_skills SET enabled_by_default = 0 WHERE key = 'basic-writing'")
        .await;
    let plain_book = book(&app, "不默认").await;
    let (_, catalogue) = app
        .get(&format!("/api/writing-skills?book_id={plain_book}"))
        .await;
    assert!(
        catalogue
            .as_array()
            .expect("array")
            .iter()
            .all(|row| row["selected"] == false)
    );
}
