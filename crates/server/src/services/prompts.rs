//! Prompt override service (spec/09 §4, spec/10 §2.6).
//!
//! Reads assemble the DTO list for the prompts page; writes validate an override before storing
//! it and then invalidate the registry's per-book cache, so a saved change applies to sessions
//! created afterwards (spec/09 §4).

use std::collections::BTreeMap;

use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};

use crate::api::dto::prompts::{
    PromptEntryDto, PromptSaveInput, PromptVariableDocDto, PromptVariableFieldDocDto,
};
use crate::db::entities::{books, prompt_overrides};
use crate::db::json::PromptScope;
use crate::error::AppError;
use crate::prompts::{PromptError, PromptSource};
use crate::state::AppState;

/// Every prompt entry with its metadata, overrides, effective source and variable docs
/// (spec/09 §4).
pub async fn list(state: &AppState, book_id: Option<i32>) -> Result<Vec<PromptEntryDto>, AppError> {
    let registry = state.prompts();
    let (global, book) = load_overrides(state.db().conn(), book_id).await?;
    Ok(registry
        .pack()
        .entries()
        .map(|entry| {
            let global_override = global.get(&entry.key).cloned();
            let book_override = book.get(&entry.key).cloned();
            let source = if book_override.is_some() {
                PromptSource::Book
            } else if global_override.is_some() {
                PromptSource::Global
            } else {
                PromptSource::Default
            };
            PromptEntryDto {
                key: entry.key.clone(),
                kind: entry.kind,
                agent: entry.agent.clone(),
                title_en: entry.title_en.clone(),
                title_zh: entry.title_zh.clone(),
                vars: entry.vars.clone(),
                default_text: entry.text.clone(),
                global_override,
                book_override,
                source,
                variables: registry
                    .pack()
                    .variable_docs(&entry.key)
                    .into_iter()
                    .map(|doc| PromptVariableDocDto {
                        name: doc.name.clone(),
                        en: doc.en.clone(),
                        zh: doc.zh.clone(),
                        fields: doc
                            .fields
                            .iter()
                            .map(|field| PromptVariableFieldDocDto {
                                name: field.name.clone(),
                                en: field.en.clone(),
                                zh: field.zh.clone(),
                            })
                            .collect(),
                    })
                    .collect(),
            }
        })
        .collect())
}

/// Rejects a book override that names no book.
///
/// The override row has a foreign key on `books.id`, so without this check a stale id would
/// surface as a 500 `internal` on the constraint instead of the 404 the API promises (spec/10 §4).
async fn require_book(conn: &impl ConnectionTrait, book_id: i32) -> Result<(), AppError> {
    let exists = books::Entity::find_by_id(book_id)
        .one(conn)
        .await?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err(AppError::not_found(format!("book {book_id}")))
    }
}

/// Reads both override levels for a book (`scope = 'book'` rows of that book only).
async fn load_overrides(
    conn: &impl ConnectionTrait,
    book_id: Option<i32>,
) -> Result<(BTreeMap<String, String>, BTreeMap<String, String>), AppError> {
    let rows = prompt_overrides::Entity::find().all(conn).await?;
    let mut global = BTreeMap::new();
    let mut book = BTreeMap::new();
    for row in rows {
        match row.scope.as_str() {
            "global" => {
                global.insert(row.key, row.content);
            }
            "book" if row.book_id == book_id && book_id.is_some() => {
                book.insert(row.key, row.content);
            }
            _ => {}
        }
    }
    Ok((global, book))
}

/// The entry of one key after the change (spec/09 §4 returns the whole entry).
pub async fn entry(
    state: &AppState,
    key: &str,
    book_id: Option<i32>,
) -> Result<PromptEntryDto, AppError> {
    list(state, book_id)
        .await?
        .into_iter()
        .find(|entry| entry.key == key)
        .ok_or_else(|| AppError::not_found(format!("prompt {key}")))
}

/// Validates and stores an override (spec/09 §4), then invalidates the cache.
pub async fn save(
    state: &AppState,
    key: &str,
    input: &PromptSaveInput,
) -> Result<PromptEntryDto, AppError> {
    let registry = state.prompts();
    if registry.pack().entry(key).is_none() {
        return Err(AppError::not_found(format!("prompt {key}")));
    }
    let book_id = match input.scope {
        PromptScope::Global => None,
        PromptScope::Book => Some(input.book_id.ok_or_else(|| {
            AppError::validation("a book override needs book_id").with_details(
                serde_json::json!({ "fields": { "book_id": "required for scope=book" } }),
            )
        })?),
    };
    if let Some(book_id) = book_id {
        require_book(state.db().conn(), book_id).await?;
    }
    registry
        .validate_override(key, &input.content)
        .map_err(PromptError::into_app_error)?;

    let key_owned = key.to_owned();
    let content = input.content.clone();
    let scope = input.scope;
    crate::services::write(state.db(), move |tx| {
        Box::pin(async move {
            upsert(tx, scope, book_id, &key_owned, &content).await?;
            Ok(())
        })
    })
    .await?;

    // A global change affects every book, so every cached set is dropped.
    registry.invalidate(book_id);
    entry(state, key, book_id).await
}

/// Deletes one override level, restoring the next level down (spec/09 §4).
pub async fn reset(
    state: &AppState,
    key: &str,
    scope: PromptScope,
    book_id: Option<i32>,
) -> Result<PromptEntryDto, AppError> {
    let registry = state.prompts();
    if registry.pack().entry(key).is_none() {
        return Err(AppError::not_found(format!("prompt {key}")));
    }
    let effective_book = match scope {
        PromptScope::Global => None,
        PromptScope::Book => Some(book_id.ok_or_else(|| {
            AppError::validation("a book override needs book_id").with_details(
                serde_json::json!({ "fields": { "book_id": "required for scope=book" } }),
            )
        })?),
    };
    let key_owned = key.to_owned();
    crate::services::write(state.db(), move |tx| {
        Box::pin(async move {
            let mut query = prompt_overrides::Entity::delete_many()
                .filter(prompt_overrides::Column::Key.eq(key_owned.clone()))
                .filter(prompt_overrides::Column::Scope.eq(scope.as_str()));
            query = match (scope, effective_book) {
                (PromptScope::Book, Some(book_id)) => {
                    query.filter(prompt_overrides::Column::BookId.eq(book_id))
                }
                _ => query.filter(prompt_overrides::Column::BookId.is_null()),
            };
            query.exec(tx).await?;
            Ok(())
        })
    })
    .await?;

    registry.invalidate(effective_book);
    entry(state, key, effective_book).await
}

/// Inserts or replaces one override row inside the caller's transaction.
async fn upsert(
    tx: &impl ConnectionTrait,
    scope: PromptScope,
    book_id: Option<i32>,
    key: &str,
    content: &str,
) -> Result<(), AppError> {
    // The partial unique indexes make "one row per level and key" a schema guarantee, so the
    // existing row (if any) is updated instead of inserted twice.
    let mut query = prompt_overrides::Entity::find().filter(prompt_overrides::Column::Key.eq(key));
    query = match (scope, book_id) {
        (PromptScope::Book, Some(book_id)) => query
            .filter(prompt_overrides::Column::Scope.eq(PromptScope::Book))
            .filter(prompt_overrides::Column::BookId.eq(book_id)),
        _ => query
            .filter(prompt_overrides::Column::Scope.eq(PromptScope::Global))
            .filter(prompt_overrides::Column::BookId.is_null()),
    };
    let existing = query.one(tx).await?;
    let now = chrono::Utc::now();
    match existing {
        Some(row) => {
            let mut active: prompt_overrides::ActiveModel = row.into();
            active.content = Set(content.to_owned());
            active.updated_at = Set(now);
            active.update(tx).await?;
        }
        None => {
            let active = prompt_overrides::ActiveModel {
                scope: Set(scope),
                book_id: Set(book_id),
                key: Set(key.to_owned()),
                content: Set(content.to_owned()),
                updated_at: Set(now),
                ..Default::default()
            };
            active.insert(tx).await?;
        }
    }
    Ok(())
}
