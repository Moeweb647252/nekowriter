//! Knowledge-base services (spec/07 §1, spec/10 §2.5): scopes, entries, book visibility and the
//! hybrid search endpoint.
//!
//! Chunking runs in the same transaction as an entry change (spec/07 §1.1), so keyword search
//! works immediately and the indexer only has to embed the chunks (spec/07 §4). Deleting a scope
//! cascades to its entries; the schema triggers clean the FTS and vec rows (spec/02 §5.11).

use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Set,
};

use crate::api::dto::knowledge::{
    BookScopesInput, EntryDto, EntryInput, EntryPageDto, EntryPatch, EntryQuery, EntrySummaryDto,
    KnowledgeHitDto, KnowledgeSearchQuery, ScopeDto, ScopeInput, ScopePatch,
};
use crate::db::entities::{book_scopes, knowledge_entries, knowledge_scopes};
use crate::db::json::{ChangeOp, EntrySource};
use crate::error::{AppError, ErrorCode};
use crate::events::ServerEvent;

use super::{field_error, json, now, parse_json};

/// The entries page size (spec/10 §1: default 100, max 500).
pub const ENTRY_PAGE_DEFAULT: i64 = 100;
/// The largest entries page.
pub const ENTRY_PAGE_MAX: i64 = 500;
/// The knowledge search default and maximum (spec/10 §2.5).
pub const SEARCH_DEFAULT: usize = 20;
pub const SEARCH_MAX: usize = 50;

// ───────────────────────────── scopes ─────────────────────────────

/// `GET /api/knowledge/scopes` (with entry counts, spec/10 §2.5).
pub async fn scopes(conn: &impl ConnectionTrait) -> Result<Vec<ScopeDto>, AppError> {
    let rows = knowledge_scopes::Entity::find()
        .order_by_asc(knowledge_scopes::Column::Name)
        .all(conn)
        .await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in &rows {
        items.push(to_scope_dto(conn, row).await?);
    }
    Ok(items)
}

/// `GET /api/knowledge/scopes/:id` (the UI reads it after a rename).
pub async fn get_scope(conn: &impl ConnectionTrait, scope_id: i32) -> Result<ScopeDto, AppError> {
    let row = load_scope(conn, scope_id).await?;
    to_scope_dto(conn, &row).await
}

pub async fn to_scope_dto(
    conn: &impl ConnectionTrait,
    row: &knowledge_scopes::Model,
) -> Result<ScopeDto, AppError> {
    let entry_count = knowledge_entries::Entity::find()
        .filter(knowledge_entries::Column::ScopeId.eq(row.id))
        .count(conn)
        .await? as i64;
    Ok(ScopeDto {
        id: row.id,
        name: row.name.clone(),
        description: row.description.clone(),
        entry_count,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// `POST /api/knowledge/scopes`.
pub async fn create_scope(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    input: ScopeInput,
) -> Result<ScopeDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { create_scope_inner(tx, input).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn create_scope_inner(
    tx: &impl ConnectionTrait,
    input: ScopeInput,
) -> Result<(ScopeDto, Vec<ServerEvent>), AppError> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(field_error("name", "a scope needs a name"));
    }
    if let Some(existing) = find_scope_by_name(tx, name).await? {
        return Err(scope_exists(existing.id));
    }
    let timestamp = now();
    let row = knowledge_scopes::Entity::insert(knowledge_scopes::ActiveModel {
        name: Set(name.to_owned()),
        description: Set(input.description.trim().to_owned()),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    })
    .exec_with_returning(tx)
    .await?;
    let events = vec![ServerEvent::KnowledgeChanged {
        scope_id: row.id,
        entry_id: 0,
        op: ChangeOp::Create,
    }];
    Ok((to_scope_dto(tx, &row).await?, events))
}

/// `PATCH /api/knowledge/scopes/:id`.
pub async fn update_scope(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    scope_id: i32,
    patch: ScopePatch,
) -> Result<ScopeDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { update_scope_inner(tx, scope_id, patch).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn update_scope_inner(
    tx: &impl ConnectionTrait,
    scope_id: i32,
    patch: ScopePatch,
) -> Result<(ScopeDto, Vec<ServerEvent>), AppError> {
    let existing = load_scope(tx, scope_id).await?;
    let mut active = knowledge_scopes::ActiveModel {
        id: Set(scope_id),
        updated_at: Set(now()),
        ..Default::default()
    };
    if let Some(name) = &patch.name {
        let name = name.trim();
        if name.is_empty() {
            return Err(field_error("name", "a scope needs a name"));
        }
        if name != existing.name
            && let Some(other) = find_scope_by_name(tx, name).await?
        {
            return Err(scope_exists(other.id));
        }
        active.name = Set(name.to_owned());
    }
    if let Some(description) = &patch.description {
        active.description = Set(description.trim().to_owned());
    }
    active.update(tx).await?;
    let row = load_scope(tx, scope_id).await?;
    let events = vec![ServerEvent::KnowledgeChanged {
        scope_id,
        entry_id: 0,
        op: ChangeOp::Update,
    }];
    Ok((to_scope_dto(tx, &row).await?, events))
}

/// `DELETE /api/knowledge/scopes/:id` — deletes its entries too (spec/07 §1).
pub async fn delete_scope(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    scope_id: i32,
) -> Result<(), AppError> {
    let published = super::write(db, move |tx| {
        Box::pin(async move { delete_scope_inner(tx, scope_id).await })
    })
    .await?;
    events.publish_all(published);
    Ok(())
}

pub async fn delete_scope_inner(
    tx: &impl ConnectionTrait,
    scope_id: i32,
) -> Result<Vec<ServerEvent>, AppError> {
    load_scope(tx, scope_id).await?;
    knowledge_scopes::Entity::delete_by_id(scope_id)
        .exec(tx)
        .await?;
    Ok(vec![ServerEvent::KnowledgeChanged {
        scope_id,
        entry_id: 0,
        op: ChangeOp::Delete,
    }])
}

async fn load_scope(
    conn: &impl ConnectionTrait,
    scope_id: i32,
) -> Result<knowledge_scopes::Model, AppError> {
    knowledge_scopes::Entity::find_by_id(scope_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("knowledge scope #{scope_id}")))
}

async fn find_scope_by_name(
    conn: &impl ConnectionTrait,
    name: &str,
) -> Result<Option<knowledge_scopes::Model>, AppError> {
    Ok(knowledge_scopes::Entity::find()
        .filter(knowledge_scopes::Column::Name.eq(name))
        .one(conn)
        .await?)
}

fn scope_exists(id: i32) -> AppError {
    super::conflict(
        ErrorCode::ConflictExists,
        "a knowledge scope with that name already exists",
        serde_json::json!({ "id": id }),
    )
}

fn entry_exists(id: i32) -> AppError {
    super::conflict(
        ErrorCode::ConflictExists,
        "a knowledge entry with that title already exists in this scope",
        serde_json::json!({ "id": id }),
    )
}

// ───────────────────────────── entries ─────────────────────────────

/// `GET /api/knowledge/scopes/:id/entries?q&cursor&limit`.
pub async fn list_entries(
    conn: &impl ConnectionTrait,
    scope_id: i32,
    query: &EntryQuery,
) -> Result<EntryPageDto, AppError> {
    load_scope(conn, scope_id).await?;
    let limit = query
        .limit
        .unwrap_or(ENTRY_PAGE_DEFAULT)
        .clamp(1, ENTRY_PAGE_MAX);
    let mut statement = knowledge_entries::Entity::find()
        .filter(knowledge_entries::Column::ScopeId.eq(scope_id))
        .order_by_asc(knowledge_entries::Column::Id);
    if let Some(cursor) = &query.cursor {
        let cursor: i32 = cursor
            .parse()
            .map_err(|_| AppError::validation("the cursor is not valid"))?;
        statement = statement.filter(knowledge_entries::Column::Id.gt(cursor));
    }
    if let Some(text) = query.q.as_deref().map(str::trim)
        && !text.is_empty()
    {
        let pattern = format!("%{text}%");
        statement = statement.filter(
            Condition::any()
                .add(knowledge_entries::Column::Title.like(&pattern))
                .add(knowledge_entries::Column::Content.like(&pattern)),
        );
    }
    let rows = statement.limit(limit as u64 + 1).all(conn).await?;
    let mut items: Vec<EntrySummaryDto> = Vec::with_capacity(rows.len().min(limit as usize));
    let mut next_cursor = None;
    for (index, row) in rows.iter().enumerate() {
        if index as i64 == limit {
            next_cursor = items.last().map(|item| item.id.to_string());
            break;
        }
        items.push(to_summary(row)?);
    }
    Ok(EntryPageDto { items, next_cursor })
}

/// `GET /api/knowledge/entries/:id`.
pub async fn get_entry(conn: &impl ConnectionTrait, entry_id: i32) -> Result<EntryDto, AppError> {
    let row = load_entry(conn, entry_id).await?;
    to_dto(&row)
}

/// `POST /api/knowledge/scopes/:id/entries` — chunks in the same transaction (spec/07 §1.1).
pub async fn create_entry(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    scope_id: i32,
    input: EntryInput,
) -> Result<EntryDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { create_entry_inner(tx, scope_id, input).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn create_entry_inner(
    tx: &impl ConnectionTrait,
    scope_id: i32,
    input: EntryInput,
) -> Result<(EntryDto, Vec<ServerEvent>), AppError> {
    load_scope(tx, scope_id).await?;
    let title = input.title.trim();
    if title.is_empty() {
        return Err(field_error("title", "an entry needs a title"));
    }
    if input.content.trim().is_empty() {
        return Err(field_error("content", "an entry needs content"));
    }
    if let Some(existing) = find_entry_by_title(tx, scope_id, title).await? {
        return Err(entry_exists(existing.id));
    }
    let timestamp = now();
    let row = knowledge_entries::Entity::insert(knowledge_entries::ActiveModel {
        scope_id: Set(scope_id),
        title: Set(title.to_owned()),
        content: Set(input.content.clone()),
        source: Set(EntrySource::Manual.as_str().to_owned()),
        source_urls: Set(json(&input.source_urls)?),
        tags: Set(json(&input.tags)?),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    })
    .exec_with_returning(tx)
    .await?;
    // Keyword search works immediately: the chunks and their FTS rows are written here, and the
    // indexer embeds them asynchronously (spec/07 §1.1, §2).
    crate::search::maintenance::sync_knowledge_entry(tx, row.id).await?;
    let events = vec![ServerEvent::KnowledgeChanged {
        scope_id,
        entry_id: row.id,
        op: ChangeOp::Create,
    }];
    Ok((to_dto(&row)?, events))
}

/// `PATCH /api/knowledge/entries/:id`.
pub async fn update_entry(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    entry_id: i32,
    patch: EntryPatch,
) -> Result<EntryDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { update_entry_inner(tx, entry_id, patch).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn update_entry_inner(
    tx: &impl ConnectionTrait,
    entry_id: i32,
    patch: EntryPatch,
) -> Result<(EntryDto, Vec<ServerEvent>), AppError> {
    let existing = load_entry(tx, entry_id).await?;
    let mut active = knowledge_entries::ActiveModel {
        id: Set(entry_id),
        updated_at: Set(now()),
        ..Default::default()
    };
    let mut content_changed = false;
    if let Some(title) = &patch.title {
        let title = title.trim();
        if title.is_empty() {
            return Err(field_error("title", "an entry needs a title"));
        }
        if title != existing.title
            && let Some(other) = find_entry_by_title(tx, existing.scope_id, title).await?
        {
            return Err(entry_exists(other.id));
        }
        active.title = Set(title.to_owned());
    }
    if let Some(content) = &patch.content {
        if content.trim().is_empty() {
            return Err(field_error("content", "an entry needs content"));
        }
        content_changed = *content != existing.content;
        active.content = Set(content.clone());
    }
    if let Some(urls) = &patch.source_urls {
        active.source_urls = Set(json(urls)?);
    }
    if let Some(tags) = &patch.tags {
        active.tags = Set(json(tags)?);
    }
    active.update(tx).await?;
    if content_changed {
        // Re-chunk in the same transaction; unchanged chunks keep their row and vector (D-039).
        crate::search::maintenance::sync_knowledge_entry(tx, entry_id).await?;
    } else if patch.title.is_some() {
        // The FTS title column carries the entry title.
        crate::search::maintenance::sync_knowledge_entry(tx, entry_id).await?;
    }
    let row = load_entry(tx, entry_id).await?;
    let events = vec![ServerEvent::KnowledgeChanged {
        scope_id: row.scope_id,
        entry_id,
        op: ChangeOp::Update,
    }];
    Ok((to_dto(&row)?, events))
}

/// `DELETE /api/knowledge/entries/:id`.
pub async fn delete_entry(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    entry_id: i32,
) -> Result<(), AppError> {
    let published = super::write(db, move |tx| {
        Box::pin(async move { delete_entry_inner(tx, entry_id).await })
    })
    .await?;
    events.publish_all(published);
    Ok(())
}

pub async fn delete_entry_inner(
    tx: &impl ConnectionTrait,
    entry_id: i32,
) -> Result<Vec<ServerEvent>, AppError> {
    let existing = load_entry(tx, entry_id).await?;
    knowledge_entries::Entity::delete_by_id(entry_id)
        .exec(tx)
        .await?;
    Ok(vec![ServerEvent::KnowledgeChanged {
        scope_id: existing.scope_id,
        entry_id,
        op: ChangeOp::Delete,
    }])
}

async fn load_entry(
    conn: &impl ConnectionTrait,
    entry_id: i32,
) -> Result<knowledge_entries::Model, AppError> {
    knowledge_entries::Entity::find_by_id(entry_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("knowledge entry #{entry_id}")))
}

async fn find_entry_by_title(
    conn: &impl ConnectionTrait,
    scope_id: i32,
    title: &str,
) -> Result<Option<knowledge_entries::Model>, AppError> {
    Ok(knowledge_entries::Entity::find()
        .filter(knowledge_entries::Column::ScopeId.eq(scope_id))
        .filter(knowledge_entries::Column::Title.eq(title))
        .one(conn)
        .await?)
}

fn to_dto(row: &knowledge_entries::Model) -> Result<EntryDto, AppError> {
    Ok(EntryDto {
        id: row.id,
        scope_id: row.scope_id,
        title: row.title.clone(),
        content: row.content.clone(),
        source: EntrySource::from_str_opt(&row.source).unwrap_or(EntrySource::Manual),
        source_urls: parse_json("knowledge_entries.source_urls", &row.source_urls)?,
        tags: parse_json("knowledge_entries.tags", &row.tags)?,
        research_task_id: row.research_task_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn to_summary(row: &knowledge_entries::Model) -> Result<EntrySummaryDto, AppError> {
    Ok(EntrySummaryDto {
        id: row.id,
        scope_id: row.scope_id,
        title: row.title.clone(),
        source: EntrySource::from_str_opt(&row.source).unwrap_or(EntrySource::Manual),
        tags: parse_json("knowledge_entries.tags", &row.tags)?,
        updated_at: row.updated_at,
    })
}

// ───────────────────────────── search ─────────────────────────────

/// `GET /api/knowledge/search` (spec/10 §2.5): hybrid search over the given scopes, or every scope
/// when the filter is absent.
pub async fn search(
    state: &crate::state::AppState,
    query: &KnowledgeSearchQuery,
) -> Result<Vec<KnowledgeHitDto>, AppError> {
    let text = query.q.as_deref().unwrap_or_default().trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let scope_ids = match resolve_scope_ids(state, query.scope_ids.as_deref()).await? {
        Some(ids) => ids,
        None => return Ok(Vec::new()),
    };
    let limit = query
        .limit
        .unwrap_or(SEARCH_DEFAULT as i64)
        .clamp(1, SEARCH_MAX as i64) as usize;
    // The query vector is computed outside any transaction; without one only keywords run
    // (spec/07 §5).
    let vector = state.search().query_vector(text).await;
    let hits = crate::search::hybrid_knowledge(
        state.db_conn(),
        text,
        vector.as_deref(),
        &scope_ids,
        limit,
    )
    .await?;
    Ok(hits
        .hits
        .into_iter()
        .map(|hit| KnowledgeHitDto {
            entry_id: hit.entry_id,
            scope_id: hit.scope_id,
            scope_name: hit.scope_name,
            title: hit.title,
            snippet: hit.snippet,
        })
        .collect())
}

/// Parses the `scope_ids` filter; `None` means every scope, `Some(empty)` means no scope.
async fn resolve_scope_ids(
    state: &crate::state::AppState,
    filter: Option<&str>,
) -> Result<Option<Vec<i32>>, AppError> {
    match filter {
        None => {
            let ids: Vec<i32> = knowledge_scopes::Entity::find()
                .order_by_asc(knowledge_scopes::Column::Id)
                .all(state.db_conn())
                .await?
                .into_iter()
                .map(|row| row.id)
                .collect();
            Ok(Some(ids))
        }
        Some(text) => {
            let mut ids = Vec::new();
            for part in text
                .split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
            {
                let id: i32 = part
                    .parse()
                    .map_err(|_| AppError::validation("scope_ids must be comma-separated ids"))?;
                ids.push(id);
            }
            Ok(Some(ids))
        }
    }
}

// ───────────────────────────── book visibility ─────────────────────────────

/// `GET /api/books/:id/scopes` (spec/10 §2.5).
pub async fn book_scopes(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Vec<ScopeDto>, AppError> {
    super::books::get(conn, book_id).await?;
    let rows = book_scopes::Entity::find()
        .filter(book_scopes::Column::BookId.eq(book_id))
        .all(conn)
        .await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let scope = load_scope(conn, row.scope_id).await?;
        items.push(to_scope_dto(conn, &scope).await?);
    }
    items.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(items)
}

/// `PUT /api/books/:id/scopes` — replaces the visible scope set; detaching only removes
/// visibility (spec/07 §1).
pub async fn set_book_scopes(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    book_id: i32,
    input: BookScopesInput,
) -> Result<Vec<ScopeDto>, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { set_book_scopes_inner(tx, book_id, input).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn set_book_scopes_inner(
    tx: &impl ConnectionTrait,
    book_id: i32,
    input: BookScopesInput,
) -> Result<(Vec<ScopeDto>, Vec<ServerEvent>), AppError> {
    super::books::get(tx, book_id).await?;
    let mut ids: Vec<i32> = input.scope_ids;
    ids.sort_unstable();
    ids.dedup();
    for scope_id in &ids {
        load_scope(tx, *scope_id).await?;
    }
    book_scopes::Entity::delete_many()
        .filter(book_scopes::Column::BookId.eq(book_id))
        .exec(tx)
        .await?;
    for scope_id in &ids {
        book_scopes::Entity::insert(book_scopes::ActiveModel {
            book_id: Set(book_id),
            scope_id: Set(*scope_id),
        })
        .exec(tx)
        .await?;
    }
    let mut items = Vec::with_capacity(ids.len());
    for scope_id in &ids {
        let scope = load_scope(tx, *scope_id).await?;
        items.push(to_scope_dto(tx, &scope).await?);
    }
    items.sort_by(|left, right| left.name.cmp(&right.name));
    // The book configuration changed; the library and the book invalidate on this event.
    let events = vec![ServerEvent::BookUpdated {
        book_id,
        op: crate::events::BookOp::Update,
    }];
    Ok((items, events))
}

/// The ids of the scopes a book's writer and reviewer may search (research sessions use their own
/// target scope instead, spec/07 §1).
pub async fn visible_scope_ids(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Vec<i32>, AppError> {
    Ok(book_scopes::Entity::find()
        .filter(book_scopes::Column::BookId.eq(book_id))
        .order_by_asc(book_scopes::Column::ScopeId)
        .all(conn)
        .await?
        .into_iter()
        .map(|row| row.scope_id)
        .collect())
}
