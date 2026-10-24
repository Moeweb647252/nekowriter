//! Canon endpoints (spec/10 §2.4).
//!
//! Every handler is thin: it parses the path and body, calls one service and returns the DTO.
//! The rules (visibility, change log, revert, appearance bounds) live in `services/canon/*`.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;

use crate::api::dto::canon::{
    AppearanceDto, AppearanceInput, EntityDto, EntityInput, EntityListItemDto, EntityMergeInput,
    EntityPatch, EntityQuery, ForeshadowingDto, ForeshadowingInput, ForeshadowingPatch,
    ForeshadowingQuery, MemoryNoteDto, MemoryNoteInput, MemoryNotePatch, MemoryNoteQuery,
    MemoryQuery, MemoryViewDto, RelationDto, RelationInput, RelationPatch, RelationQuery,
    TimelineEventDto, TimelineEventInput, TimelineEventPatch, TimelineMoveInput, TimelineQuery,
};
use crate::api::dto::changes::ChangeDto;
use crate::error::AppResult;
use crate::services::canon::{self, appearance, entity, foreshadow, note, relation, timeline};
use crate::services::memory;
use crate::state::AppState;

// ───────────────────────────── entities ─────────────────────────────

/// `GET /api/books/:id/entities`.
pub async fn list_entities(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Query(query): Query<EntityQuery>,
) -> AppResult<Json<Vec<EntityListItemDto>>> {
    Ok(Json(
        entity::list(state.db().conn(), book_id, &query).await?,
    ))
}

/// `POST /api/books/:id/entities`.
pub async fn create_entity(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Json(input): Json<EntityInput>,
) -> AppResult<(StatusCode, Json<EntityDto>)> {
    let dto = entity::create(state.db(), state.events(), book_id, input).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// `GET /api/entities/:id`.
pub async fn get_entity(
    State(state): State<AppState>,
    Path(entity_id): Path<i32>,
) -> AppResult<Json<EntityDto>> {
    Ok(Json(
        entity::detail(state.db().conn(), entity_id, entity::DETAIL_CHANGE_LIMIT).await?,
    ))
}

/// `PATCH /api/entities/:id`.
pub async fn patch_entity(
    State(state): State<AppState>,
    Path(entity_id): Path<i32>,
    Json(patch): Json<EntityPatch>,
) -> AppResult<Json<EntityDto>> {
    Ok(Json(
        entity::update(state.db(), state.events(), entity_id, patch).await?,
    ))
}

/// `DELETE /api/entities/:id`.
pub async fn delete_entity(
    State(state): State<AppState>,
    Path(entity_id): Path<i32>,
) -> AppResult<StatusCode> {
    entity::delete(state.db(), state.events(), entity_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/entities/:id/merge`.
pub async fn merge_entity(
    State(state): State<AppState>,
    Path(entity_id): Path<i32>,
    Json(input): Json<EntityMergeInput>,
) -> AppResult<Json<EntityDto>> {
    Ok(Json(
        entity::merge(state.db(), state.events(), entity_id, input).await?,
    ))
}

/// `POST /api/entities/:id/protagonist`.
pub async fn set_protagonist(
    State(state): State<AppState>,
    Path(entity_id): Path<i32>,
) -> AppResult<Json<EntityDto>> {
    Ok(Json(
        entity::set_protagonist(state.db(), state.events(), entity_id).await?,
    ))
}

/// `GET /api/entities/:id/history`.
pub async fn entity_history(
    State(state): State<AppState>,
    Path(entity_id): Path<i32>,
) -> AppResult<Json<Vec<ChangeDto>>> {
    Ok(Json(
        entity::history(state.db().conn(), entity_id, 100).await?,
    ))
}

// ───────────────────────────── relations ─────────────────────────────

/// `GET /api/books/:id/relations`.
pub async fn list_relations(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Query(query): Query<RelationQuery>,
) -> AppResult<Json<Vec<RelationDto>>> {
    Ok(Json(
        relation::list(state.db().conn(), book_id, &query).await?,
    ))
}

/// `POST /api/books/:id/relations`.
pub async fn create_relation(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Json(input): Json<RelationInput>,
) -> AppResult<(StatusCode, Json<RelationDto>)> {
    let dto = relation::create(state.db(), state.events(), book_id, input).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// `GET /api/relations/:id`.
pub async fn get_relation(
    State(state): State<AppState>,
    Path(relation_id): Path<i32>,
) -> AppResult<Json<RelationDto>> {
    Ok(Json(relation::get(state.db().conn(), relation_id).await?))
}

/// `PATCH /api/relations/:id`.
pub async fn patch_relation(
    State(state): State<AppState>,
    Path(relation_id): Path<i32>,
    Json(patch): Json<RelationPatch>,
) -> AppResult<Json<RelationDto>> {
    Ok(Json(
        relation::update(state.db(), state.events(), relation_id, patch).await?,
    ))
}

/// `DELETE /api/relations/:id`.
pub async fn delete_relation(
    State(state): State<AppState>,
    Path(relation_id): Path<i32>,
) -> AppResult<StatusCode> {
    relation::delete(state.db(), state.events(), relation_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────────── appearances ─────────────────────────────

/// `GET /api/chapters/:id/appearances`.
pub async fn chapter_appearances(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
) -> AppResult<Json<Vec<AppearanceDto>>> {
    let rows = appearance::for_chapter(state.db().conn(), chapter_id).await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(appearance::to_dto(&row)?);
    }
    Ok(Json(items))
}

/// `PUT /api/chapters/:id/appearances`: replaces the chapter's appearance set
/// (spec/05 §6 `record_appearances`).
pub async fn put_appearances(
    State(state): State<AppState>,
    Path(chapter_id): Path<i32>,
    Json(inputs): Json<Vec<AppearanceInput>>,
) -> AppResult<Json<Vec<AppearanceDto>>> {
    let (rows, published) = canon::write(state.db(), move |tx| {
        Box::pin(async move {
            let (rows, events) =
                appearance::record_set_inner(tx, &canon::CanonWrite::author(), chapter_id, &inputs)
                    .await?;
            Ok((rows, events))
        })
    })
    .await?;
    state.events().publish_all(published);
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(appearance::to_dto(&row)?);
    }
    Ok(Json(items))
}

// ───────────────────────────── timeline ─────────────────────────────

/// `GET /api/books/:id/timeline` (story order).
pub async fn list_timeline(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Query(query): Query<TimelineQuery>,
) -> AppResult<Json<Vec<TimelineEventDto>>> {
    Ok(Json(
        timeline::list(state.db().conn(), book_id, &query).await?,
    ))
}

/// `POST /api/books/:id/timeline`.
pub async fn create_timeline_event(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Json(input): Json<TimelineEventInput>,
) -> AppResult<(StatusCode, Json<TimelineEventDto>)> {
    let dto = timeline::create(state.db(), state.events(), book_id, input).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// `GET /api/timeline-events/:id`.
pub async fn get_timeline_event(
    State(state): State<AppState>,
    Path(event_id): Path<i32>,
) -> AppResult<Json<TimelineEventDto>> {
    Ok(Json(timeline::get(state.db().conn(), event_id).await?))
}

/// `PATCH /api/timeline-events/:id`.
pub async fn patch_timeline_event(
    State(state): State<AppState>,
    Path(event_id): Path<i32>,
    Json(patch): Json<TimelineEventPatch>,
) -> AppResult<Json<TimelineEventDto>> {
    Ok(Json(
        timeline::update(state.db(), state.events(), event_id, patch).await?,
    ))
}

/// `DELETE /api/timeline-events/:id`.
pub async fn delete_timeline_event(
    State(state): State<AppState>,
    Path(event_id): Path<i32>,
) -> AppResult<StatusCode> {
    timeline::delete(state.db(), state.events(), event_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/timeline-events/:id/move`.
pub async fn move_timeline_event(
    State(state): State<AppState>,
    Path(event_id): Path<i32>,
    Json(input): Json<TimelineMoveInput>,
) -> AppResult<Json<TimelineEventDto>> {
    Ok(Json(
        timeline::move_event(state.db(), state.events(), event_id, input).await?,
    ))
}

// ───────────────────────────── memory notes ─────────────────────────────

/// `GET /api/books/:id/memory-notes`.
pub async fn list_memory_notes(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Query(query): Query<MemoryNoteQuery>,
) -> AppResult<Json<Vec<MemoryNoteDto>>> {
    Ok(Json(note::list(state.db().conn(), book_id, &query).await?))
}

/// `POST /api/books/:id/memory-notes`.
pub async fn create_memory_note(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Json(input): Json<MemoryNoteInput>,
) -> AppResult<(StatusCode, Json<MemoryNoteDto>)> {
    let dto = note::create(state.db(), state.events(), book_id, input).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// `GET /api/memory-notes/:id`.
pub async fn get_memory_note(
    State(state): State<AppState>,
    Path(note_id): Path<i32>,
) -> AppResult<Json<MemoryNoteDto>> {
    Ok(Json(note::get(state.db().conn(), note_id).await?))
}

/// `PATCH /api/memory-notes/:id`.
pub async fn patch_memory_note(
    State(state): State<AppState>,
    Path(note_id): Path<i32>,
    Json(patch): Json<MemoryNotePatch>,
) -> AppResult<Json<MemoryNoteDto>> {
    Ok(Json(
        note::update(state.db(), state.events(), note_id, patch).await?,
    ))
}

/// `DELETE /api/memory-notes/:id`.
pub async fn delete_memory_note(
    State(state): State<AppState>,
    Path(note_id): Path<i32>,
) -> AppResult<StatusCode> {
    note::delete(state.db(), state.events(), note_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────────── foreshadowing ─────────────────────────────

/// `GET /api/books/:id/foreshadowing`.
pub async fn list_foreshadowing(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Query(query): Query<ForeshadowingQuery>,
) -> AppResult<Json<Vec<ForeshadowingDto>>> {
    Ok(Json(
        foreshadow::list(state.db().conn(), book_id, &query).await?,
    ))
}

/// `POST /api/books/:id/foreshadowing`.
pub async fn create_foreshadowing(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Json(input): Json<ForeshadowingInput>,
) -> AppResult<(StatusCode, Json<ForeshadowingDto>)> {
    let dto = foreshadow::create(state.db(), state.events(), book_id, input).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// `GET /api/foreshadowing/:id`.
pub async fn get_foreshadowing(
    State(state): State<AppState>,
    Path(item_id): Path<i32>,
) -> AppResult<Json<ForeshadowingDto>> {
    Ok(Json(foreshadow::get(state.db().conn(), item_id).await?))
}

/// `PATCH /api/foreshadowing/:id`.
pub async fn patch_foreshadowing(
    State(state): State<AppState>,
    Path(item_id): Path<i32>,
    Json(patch): Json<ForeshadowingPatch>,
) -> AppResult<Json<ForeshadowingDto>> {
    Ok(Json(
        foreshadow::update(state.db(), state.events(), item_id, patch).await?,
    ))
}

/// `DELETE /api/foreshadowing/:id`.
pub async fn delete_foreshadowing(
    State(state): State<AppState>,
    Path(item_id): Path<i32>,
) -> AppResult<StatusCode> {
    foreshadow::delete(state.db(), state.events(), item_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────────── memory snapshot (spec/06 §4) ─────────────────────────────

/// `GET /api/books/:id/memory` (spec/10 §2.4): the snapshot plus the exact `block.memory` text.
pub async fn memory_snapshot(
    State(state): State<AppState>,
    Path(book_id): Path<i32>,
    Query(query): Query<MemoryQuery>,
) -> AppResult<Json<MemoryViewDto>> {
    Ok(Json(memory::view(&state, book_id, &query).await?))
}

// ───────────────────────────── revert (spec/06 §3) ─────────────────────────────

/// `POST /api/changes/:id/revert` (the body carries nothing; spec/06 §3, spec/10 §2.4).
pub async fn revert_change(
    State(state): State<AppState>,
    Path(change_id): Path<i32>,
) -> AppResult<Json<ChangeDto>> {
    Ok(Json(
        canon::revert::revert(state.db(), state.events(), change_id).await?,
    ))
}
