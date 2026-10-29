//! Writing skills (design §4, §6, §8): the catalogue, a book's selection, user-row CRUD and the
//! selected bodies the `block.writing_skills` kickoff block renders.
//!
//! Built-in rows (`author = 'builtin'`) are read-only. The API refuses to change or delete one;
//! user rows are deletable, and the delete response names the books that had selected the row
//! (its cascade removes those selections).

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};

use crate::api::dto::skills::{
    BookWritingSkillsInput, WritingSkillBookDto, WritingSkillDeleteResponse, WritingSkillDto,
    WritingSkillInput, WritingSkillPatch,
};
use crate::db::entities::writing_skills::WritingSkillAuthor;
use crate::db::entities::{book_writing_skills, books, writing_skills};
use crate::error::{AppError, ErrorCode};

use super::{conflict, now};

/// The author value of a built-in row (design §4: `'builtin' | 'user'`, no default).
pub const AUTHOR_BUILTIN: &str = "builtin";
/// The author value of an author-created row.
pub const AUTHOR_USER: &str = "user";

/// `GET /api/writing-skills?book_id=` (design §8): the active catalogue, with `selected` for the
/// named book and the books that use each skill.
pub async fn catalogue(
    conn: &impl ConnectionTrait,
    book_id: Option<i32>,
) -> Result<Vec<WritingSkillDto>, AppError> {
    let selected = match book_id {
        Some(book_id) => {
            ensure_book(conn, book_id).await?;
            selected_ids(conn, book_id).await?
        }
        None => BTreeSet::new(),
    };
    let rows = writing_skills::Entity::find()
        // A retired built-in stops being offered (design §4); selections already made keep
        // working because `selected_skills` below does not filter it out.
        .filter(writing_skills::COLUMN.retired.eq(false))
        .order_by_asc(writing_skills::COLUMN.id)
        .all(conn)
        .await?;
    let usage = usage_map(conn).await?;
    Ok(rows
        .into_iter()
        .map(|row| to_dto(row, &selected, &usage))
        .collect())
}

/// `PUT /api/books/:id/writing-skills` (design §8): replace the book's complete selection.
pub async fn replace_selection(
    state: &crate::state::AppState,
    book_id: i32,
    input: BookWritingSkillsInput,
) -> Result<Vec<WritingSkillDto>, AppError> {
    super::write(state.db(), move |tx| {
        Box::pin(async move {
            ensure_book(tx, book_id).await?;
            let mut ids: Vec<i32> = input.skill_ids;
            ids.sort_unstable();
            ids.dedup();
            if !ids.is_empty() {
                let found = writing_skills::Entity::find()
                    .filter(writing_skills::COLUMN.id.is_in(ids.clone()))
                    .filter(writing_skills::COLUMN.retired.eq(false))
                    .all(tx)
                    .await?;
                if found.len() != ids.len() {
                    return Err(AppError::validation(
                        "every selected skill must exist and must not be retired",
                    ));
                }
            }
            // A retired skill stops being offered but keeps working for the books that selected
            // it (design §4/§12). The catalogue hides it, so a payload built from the visible
            // checkbox list cannot name it; preserve the existing links instead of dropping them
            // on every unrelated toggle.
            let current_ids: Vec<i32> = selected_ids(tx, book_id).await?.into_iter().collect();
            if !current_ids.is_empty() {
                let retired: Vec<i32> = writing_skills::Entity::find()
                    .filter(writing_skills::COLUMN.id.is_in(current_ids))
                    .filter(writing_skills::COLUMN.retired.eq(true))
                    .all(tx)
                    .await?
                    .into_iter()
                    .map(|row| row.id)
                    .collect();
                ids.extend(retired);
                ids.sort_unstable();
                ids.dedup();
            }
            book_writing_skills::Entity::delete_many()
                .filter(book_writing_skills::COLUMN.book_id.eq(book_id))
                .exec(tx)
                .await?;
            let timestamp = now();
            for skill_id in ids {
                book_writing_skills::Entity::insert(book_writing_skills::ActiveModel {
                    book_id: Set(book_id),
                    skill_id: Set(skill_id),
                    created_at: Set(timestamp),
                })
                .exec(tx)
                .await?;
            }
            catalogue(tx, Some(book_id)).await
        })
    })
    .await
}

/// `POST /api/writing-skills` (design §8): create a user row. `author` is forced to `user` and
/// `key` stays `NULL`, so a user skill can never collide with a future built-in key.
pub async fn create(
    state: &crate::state::AppState,
    input: WritingSkillInput,
) -> Result<WritingSkillDto, AppError> {
    super::write(state.db(), move |tx| {
        Box::pin(async move {
            let name = input.name.trim();
            if name.is_empty() {
                return Err(super::field_error("name", "a skill needs a name"));
            }
            let timestamp = now();
            let row = writing_skills::Entity::insert(writing_skills::ActiveModel {
                key: Set(None),
                name: Set(name.to_owned()),
                category: Set(input.category.trim().to_owned()),
                author: Set(WritingSkillAuthor::User),
                description: Set(input.description.trim().to_owned()),
                body: Set(input.body),
                enabled_by_default: Set(false),
                retired: Set(false),
                created_at: Set(timestamp),
                updated_at: Set(timestamp),
                ..Default::default()
            })
            .exec_with_returning(tx)
            .await?;
            Ok(to_dto(row, &BTreeSet::new(), &BTreeMap::new()))
        })
    })
    .await
}

/// `PATCH /api/writing-skills/:id` (design §8): user rows only.
pub async fn update(
    state: &crate::state::AppState,
    skill_id: i32,
    patch: WritingSkillPatch,
) -> Result<WritingSkillDto, AppError> {
    super::write(state.db(), move |tx| {
        Box::pin(async move {
            load_editable(tx, skill_id).await?;
            let mut active = writing_skills::ActiveModel {
                id: Set(skill_id),
                updated_at: Set(now()),
                ..Default::default()
            };
            if let Some(name) = &patch.name {
                let name = name.trim();
                if name.is_empty() {
                    return Err(super::field_error("name", "a skill needs a name"));
                }
                active.name = Set(name.to_owned());
            }
            if let Some(category) = &patch.category {
                active.category = Set(category.trim().to_owned());
            }
            if let Some(description) = &patch.description {
                active.description = Set(description.trim().to_owned());
            }
            if let Some(body) = &patch.body {
                active.body = Set(body.clone());
            }
            active.update(tx).await?;
            let row = writing_skills::Entity::find_by_id(skill_id)
                .one(tx)
                .await?
                .ok_or_else(|| AppError::internal("the skill vanished during the update"))?;
            Ok(to_dto(row, &BTreeSet::new(), &BTreeMap::new()))
        })
    })
    .await
}

/// `DELETE /api/writing-skills/:id` (design §8): user rows only; names the books that used it.
pub async fn delete(
    state: &crate::state::AppState,
    skill_id: i32,
) -> Result<WritingSkillDeleteResponse, AppError> {
    super::write(state.db(), move |tx| {
        Box::pin(async move {
            let existing = load_editable(tx, skill_id).await?;
            let used_by_books = books_using(tx, &[existing.id])
                .await?
                .get(&existing.id)
                .cloned()
                .unwrap_or_default();
            writing_skills::Entity::delete_by_id(existing.id)
                .exec(tx)
                .await?;
            Ok(WritingSkillDeleteResponse { used_by_books })
        })
    })
    .await
}

/// The selected skills of a book, in catalogue order (design §6.2: full bodies are injected).
///
/// Retired rows are **not** filtered: a built-in that disappears from the pack stops being offered
/// but the books that already selected it keep working (design §4/§12).
pub async fn selected_skills(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Vec<writing_skills::Model>, AppError> {
    let ids = selected_ids(conn, book_id).await?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(writing_skills::Entity::find()
        .filter(
            writing_skills::COLUMN
                .id
                .is_in(ids.into_iter().collect::<Vec<_>>()),
        )
        .order_by_asc(writing_skills::COLUMN.id)
        .all(conn)
        .await?)
}

/// Adds every `enabled_by_default = 1` skill to a freshly created book (design §4.1). The seeded
/// `basic-writing` row is `0`, so this is normally a no-op; the rule is implemented so a future
/// built-in can opt in without touching the create service.
pub async fn insert_defaults_for_book(
    tx: &impl ConnectionTrait,
    book_id: i32,
    timestamp: DateTime<Utc>,
) -> Result<(), AppError> {
    let defaults = writing_skills::Entity::find()
        .filter(writing_skills::COLUMN.enabled_by_default.eq(true))
        .filter(writing_skills::COLUMN.retired.eq(false))
        .all(tx)
        .await?;
    for row in defaults {
        book_writing_skills::Entity::insert(book_writing_skills::ActiveModel {
            book_id: Set(book_id),
            skill_id: Set(row.id),
            created_at: Set(timestamp),
        })
        .exec(tx)
        .await?;
    }
    Ok(())
}

/// The skill ids a book has selected.
async fn selected_ids(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<BTreeSet<i32>, AppError> {
    Ok(book_writing_skills::Entity::find()
        .filter(book_writing_skills::COLUMN.book_id.eq(book_id))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| row.skill_id)
        .collect())
}

/// One row per skill: the books that selected it, ordered by title for a stable confirmation list.
async fn usage_map(
    conn: &impl ConnectionTrait,
) -> Result<BTreeMap<i32, Vec<WritingSkillBookDto>>, AppError> {
    let links = book_writing_skills::Entity::find().all(conn).await?;
    let skill_ids: Vec<i32> = {
        let mut ids: Vec<i32> = links.iter().map(|link| link.skill_id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    books_using(conn, &skill_ids).await
}

async fn books_using(
    conn: &impl ConnectionTrait,
    skill_ids: &[i32],
) -> Result<BTreeMap<i32, Vec<WritingSkillBookDto>>, AppError> {
    let mut map: BTreeMap<i32, Vec<WritingSkillBookDto>> = BTreeMap::new();
    if skill_ids.is_empty() {
        return Ok(map);
    }
    let links = book_writing_skills::Entity::find()
        .filter(
            book_writing_skills::COLUMN
                .skill_id
                .is_in(skill_ids.to_vec()),
        )
        .all(conn)
        .await?;
    let book_ids: Vec<i32> = {
        let mut ids: Vec<i32> = links.iter().map(|link| link.book_id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let mut titles: BTreeMap<i32, String> = BTreeMap::new();
    if !book_ids.is_empty() {
        for book in books::Entity::find()
            .filter(books::COLUMN.id.is_in(book_ids))
            .all(conn)
            .await?
        {
            titles.insert(book.id, book.title);
        }
    }
    for link in links {
        if let Some(title) = titles.get(&link.book_id) {
            map.entry(link.skill_id)
                .or_default()
                .push(WritingSkillBookDto {
                    id: link.book_id,
                    title: title.clone(),
                });
        }
    }
    for rows in map.values_mut() {
        rows.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.id.cmp(&b.id)));
    }
    Ok(map)
}

async fn ensure_book(conn: &impl ConnectionTrait, book_id: i32) -> Result<(), AppError> {
    if books::Entity::find_by_id(book_id)
        .one(conn)
        .await?
        .is_none()
    {
        return Err(AppError::not_found(format!("book #{book_id}")));
    }
    Ok(())
}

/// Loads a row and refuses a built-in row with a documented conflict (design §8: 409 chosen over
/// 403 because the request is well-formed and authenticated; the row's kind is what forbids it).
async fn load_editable(
    conn: &impl ConnectionTrait,
    skill_id: i32,
) -> Result<writing_skills::Model, AppError> {
    let row = writing_skills::Entity::find_by_id(skill_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("writing skill #{skill_id}")))?;
    if row.author != WritingSkillAuthor::User {
        return Err(conflict(
            ErrorCode::WritingSkillBuiltin,
            "built-in writing skills are read-only",
            serde_json::json!({ "id": skill_id }),
        ));
    }
    Ok(row)
}

fn to_dto(
    row: writing_skills::Model,
    selected: &BTreeSet<i32>,
    usage: &BTreeMap<i32, Vec<WritingSkillBookDto>>,
) -> WritingSkillDto {
    WritingSkillDto {
        id: row.id,
        key: row.key,
        name: row.name,
        category: row.category,
        author: row.author.as_str().to_owned(),
        description: row.description,
        body: row.body,
        enabled_by_default: row.enabled_by_default,
        retired: row.retired,
        selected: selected.contains(&row.id),
        used_by_books: usage.get(&row.id).cloned().unwrap_or_default(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}
