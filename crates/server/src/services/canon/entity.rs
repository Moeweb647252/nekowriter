//! Entity service (spec/06 §1.1, §2, §3, §9; spec/10 §2.4).
//!
//! Entities are the canon's nouns: characters, organizations, locations, items and everything
//! else that can be named. The rules that live here are the ones M5's review tests:
//!
//! - **visibility** (D-014): the author edits the whole book; an isolated track may create private
//!   rows and update its own, and modifying a shared row from an isolated track is
//!   `error.isolated_readonly`;
//! - **appearance bounds** (`first_chapter_id` / `last_chapter_id`, D-059) recomputed from the
//!   appearances, counting shared-canon chapters only for a shared entity;
//! - **one protagonist per book** (`entities_protagonist`), moved rather than duplicated;
//! - **merge** (spec/06 §9) in one transaction with the language-neutral `#B → #A` note;
//! - **change log** for every real create, update and delete, in the same transaction.

use std::collections::{BTreeMap, HashSet};

use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, Set,
};

use crate::api::dto::canon::{
    AppearanceDto, DesignLinkDto, EntityDto, EntityInput, EntityListItemDto, EntityMergeInput,
    EntityPatch, EntityQuery, RelationDto,
};
use crate::db::entities::{
    appearances, chapters, character_designs, entities, relations, timeline_event_entities, tracks,
};
use crate::db::json::{
    CanonMode, ChangeOp, ChangeTargetType, EntityKind, EntityStatus, RelationStatus,
};
use crate::events::ServerEvent;

use super::{
    CanonError, CanonResult, CanonWrite, ChangeWrite, Visibility, aliases_of, changed_event,
    chapter_orders, clean_aliases, entity_snapshot, stamp, validate_importance,
};

/// How many recent changes the entity detail carries (spec/08 §8.8 history).
pub const DETAIL_CHANGE_LIMIT: usize = 20;

// ───────────────────────────── reads ─────────────────────────────

/// `GET /api/books/:id/entities` (spec/10 §2.4). The list is returned whole and sorted
/// client-side (D-043); `track_id` adds an isolated track's private rows to the shared ones.
pub async fn list(
    conn: &impl ConnectionTrait,
    book_id: i32,
    query: &EntityQuery,
) -> CanonResult<Vec<EntityListItemDto>> {
    super::super::books::get(conn, book_id).await?;
    let visibility = match query.track_id {
        Some(track_id) => Visibility::isolated(book_id, track_id),
        None => Visibility::shared(book_id),
    };
    let mut statement = entities::Entity::find()
        .filter(entities::Column::BookId.eq(book_id))
        .filter(visibility.visible_condition(entities::Column::TrackId))
        .order_by_desc(entities::Column::Importance)
        .order_by_desc(entities::Column::LastChapterId)
        .order_by_asc(entities::Column::Id);
    if let Some(kind) = query.kind {
        statement = statement.filter(entities::Column::Kind.eq(kind.as_str()));
    }
    if let Some(status) = query.status {
        statement = statement.filter(entities::Column::Status.eq(status.as_str()));
    }
    let rows = statement.all(conn).await?;
    let needle = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(needle) = needle
            && !matches_needle(&row, needle)?
        {
            continue;
        }
        items.push(list_item(&row)?);
    }
    Ok(items)
}

/// True when an entity's name, aliases or summary contains `needle` (case-insensitive).
fn matches_needle(row: &entities::Model, needle: &str) -> CanonResult<bool> {
    let needle = needle.to_lowercase();
    if row.name.to_lowercase().contains(&needle) {
        return Ok(true);
    }
    if aliases_of(row)?
        .iter()
        .any(|alias| alias.to_lowercase().contains(&needle))
    {
        return Ok(true);
    }
    Ok(row.summary.to_lowercase().contains(&needle))
}

/// `GET /api/entities/:id` (spec/10 §2.4): the row plus relations, appearances, changes, design.
pub async fn detail(
    conn: &impl ConnectionTrait,
    entity_id: i32,
    history_limit: usize,
) -> CanonResult<EntityDto> {
    let row = super::load_entity(conn, entity_id).await?;
    // The detail shows the relations of the entity's own canon (spec/06 §2): a shared entity
    // never lists another isolated track's private relation, while a private entity may list the
    // shared relations its track reads plus its own.
    let relation_visibility = match row.track_id {
        Some(track_id) => Visibility::isolated(row.book_id, track_id),
        None => Visibility::shared(row.book_id),
    };
    let relation_rows = relations::Entity::find()
        .filter(relations::Column::BookId.eq(row.book_id))
        .filter(relation_visibility.visible_condition(relations::Column::TrackId))
        .filter(
            Condition::any()
                .add(relations::Column::SourceId.eq(entity_id))
                .add(relations::Column::TargetId.eq(entity_id)),
        )
        .order_by_asc(relations::Column::Id)
        .all(conn)
        .await?;
    let appearance_rows = appearances::Entity::find()
        .filter(appearances::Column::EntityId.eq(entity_id))
        .order_by_asc(appearances::Column::ChapterId)
        .all(conn)
        .await?;
    let changes = super::super::change_log::for_target(
        conn,
        ChangeTargetType::Entity,
        entity_id,
        history_limit,
    )
    .await?;
    let design = character_designs::Entity::find()
        .filter(character_designs::Column::EntityId.eq(entity_id))
        .order_by_asc(character_designs::Column::Rank)
        .one(conn)
        .await?;
    Ok(EntityDto {
        id: row.id,
        book_id: row.book_id,
        track_id: row.track_id,
        kind: kind_of(&row)?,
        name: row.name.clone(),
        aliases: aliases_of(&row)?,
        summary: row.summary.clone(),
        description: row.description.clone(),
        state: row.state.clone(),
        attributes: attributes_of(&row)?,
        importance: row.importance,
        status: status_of(&row)?,
        is_protagonist: row.is_protagonist,
        first_chapter_id: row.first_chapter_id,
        last_chapter_id: row.last_chapter_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
        relations: relation_rows
            .iter()
            .map(super::relation::to_dto)
            .collect::<CanonResult<Vec<RelationDto>>>()?,
        appearances: appearance_rows
            .iter()
            .map(appearance_dto)
            .collect::<CanonResult<Vec<AppearanceDto>>>()?,
        recent_changes: changes,
        design: design.map(|row| DesignLinkDto {
            design_id: row.id,
            rank: row.rank,
            role_label: row.role_label,
            summary: row.summary,
        }),
    })
}

/// The history of one entity (spec/10 §2.4).
pub async fn history(
    conn: &impl ConnectionTrait,
    entity_id: i32,
    limit: usize,
) -> CanonResult<Vec<crate::api::dto::changes::ChangeDto>> {
    super::load_entity(conn, entity_id).await?;
    super::super::change_log::for_target(conn, ChangeTargetType::Entity, entity_id, limit)
        .await
        .map_err(CanonError::from)
}

// ───────────────────────────── create ─────────────────────────────

/// `POST /api/books/:id/entities` (spec/10 §2.4): the author's create.
pub async fn create(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    book_id: i32,
    input: EntityInput,
) -> CanonResult<EntityDto> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move {
            let write = author_write(book_id, input.track_id);
            create_inner(tx, &write, book_id, input).await
        })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

/// The author's writer: shared canon, or one isolated track when the input names it.
pub fn author_write(book_id: i32, track_id: Option<i32>) -> CanonWrite {
    match track_id {
        Some(track_id) => {
            CanonWrite::author().with_visibility(Visibility::isolated(book_id, track_id))
        }
        None => CanonWrite::author(),
    }
}

/// The create shared by the API and the archivist tool.
pub async fn create_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    book_id: i32,
    input: EntityInput,
) -> CanonResult<(EntityDto, Vec<ServerEvent>)> {
    super::super::books::get(tx, book_id).await?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(CanonError::field("name", "an entity needs a name"));
    }
    let aliases = clean_aliases(&input.aliases);
    let track_id = write.new_row_track();
    if let Some(track_id) = track_id {
        ensure_isolated_track(tx, book_id, track_id).await?;
    }
    validate_importance(input.importance)?;
    ensure_name_free(tx, write, book_id, track_id, name, &aliases, None).await?;
    if input.is_protagonist {
        ensure_protagonist_free(tx, book_id, None).await?;
    }
    let (first_chapter_id, last_chapter_id) = initial_bounds(tx, write, book_id, track_id).await?;
    let timestamp = stamp();
    let model = entities::ActiveModel {
        book_id: Set(book_id),
        track_id: Set(track_id),
        kind: Set(input.kind),
        name: Set(name.to_owned()),
        aliases: Set(crate::services::json_text(aliases.clone())),
        summary: Set(input.summary.trim().to_owned()),
        description: Set(input.description.clone()),
        state: Set(input.state.clone()),
        attributes: Set(crate::services::json_text(input.attributes)),
        importance: Set(input.importance),
        status: Set(input.status),
        is_protagonist: Set(input.is_protagonist),
        first_chapter_id: Set(first_chapter_id),
        last_chapter_id: Set(last_chapter_id),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    let row = entities::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    let after = entity_snapshot(&row)?;
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id,
            target_type: ChangeTargetType::Entity,
            target_id: row.id,
            op: ChangeOp::Create,
            before: None,
            after: Some(&after),
            note: input.note.as_deref().unwrap_or_default().trim(),
            reverts_id: None,
            track_id: row.track_id,
            chapter_id: None,
        },
    )
    .await?;
    // A shared character whose name matches an unlinked design gets linked (spec/05 §6, §7).
    link_design_by_name(tx, &row).await?;
    let events = vec![changed_event(
        book_id,
        ChangeTargetType::Entity,
        row.id,
        ChangeOp::Create,
        change_id,
    )];
    Ok((detail(tx, row.id, DETAIL_CHANGE_LIMIT).await?, events))
}

/// A shared character entity links the character design with the same name (spec/05 §6).
async fn link_design_by_name(tx: &impl ConnectionTrait, row: &entities::Model) -> CanonResult<()> {
    if row.track_id.is_some() || row.kind != EntityKind::Character {
        return Ok(());
    }
    let design = character_designs::Entity::find()
        .filter(character_designs::Column::BookId.eq(row.book_id))
        .filter(character_designs::Column::Name.eq(row.name.clone()))
        .filter(character_designs::Column::EntityId.is_null())
        .one(tx)
        .await?;
    let Some(design) = design else {
        return Ok(());
    };
    character_designs::ActiveModel {
        id: Set(design.id),
        entity_id: Set(Some(row.id)),
        ..Default::default()
    }
    .update(tx)
    .await?;
    Ok(())
}

/// The bounds of a brand-new entity: the chapter it is created in, when an agent creates it
/// (spec/05 §6). For a shared entity an isolated chapter does not count (spec/06 §1.1, D-036).
async fn initial_bounds(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    book_id: i32,
    track_id: Option<i32>,
) -> CanonResult<(Option<i32>, Option<i32>)> {
    let Some(chapter_id) = write.chapter_id else {
        return Ok((None, None));
    };
    let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(tx).await? else {
        return Ok((None, None));
    };
    if chapter.book_id != book_id {
        return Ok((None, None));
    }
    let orders = chapter_orders(tx, book_id).await?;
    let Some(order) = orders.get(&chapter_id) else {
        return Ok((None, None));
    };
    if track_id.is_none() && !order.shared_canon {
        return Ok((None, None));
    }
    Ok((Some(chapter_id), Some(chapter_id)))
}

// ───────────────────────────── update ─────────────────────────────

/// `PATCH /api/entities/:id` (spec/10 §2.4).
pub async fn update(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    entity_id: i32,
    patch: EntityPatch,
) -> CanonResult<EntityDto> {
    let (result, published) = super::write(db, move |tx| {
        Box::pin(async move { update_inner(tx, &CanonWrite::author(), entity_id, patch).await })
    })
    .await?;
    events.publish_all(published);
    Ok(result)
}

/// The update shared by the API and the archivist tool (spec/05 §6 `update_entity`).
///
/// A patch that stores the same values writes no change row: spec/06 §3 records one row per real
/// change, and a no-op save must not pollute the log. The DTO still comes back so a form settles.
pub async fn update_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    entity_id: i32,
    patch: EntityPatch,
) -> CanonResult<(EntityDto, Vec<ServerEvent>)> {
    let existing = super::load_entity(tx, entity_id).await?;
    write.ensure_writable(existing.track_id, &entity_label(&existing.name, entity_id))?;
    let before = entity_snapshot(&existing)?;

    let aliases = match &patch.aliases {
        Some(aliases) => clean_aliases(aliases),
        None => aliases_of(&existing)?,
    };
    let target_name = match &patch.name {
        Some(name) => {
            let name = name.trim();
            if name.is_empty() {
                return Err(CanonError::field("name", "an entity needs a name"));
            }
            name.to_owned()
        }
        None => existing.name.clone(),
    };
    if patch.name.is_some() || patch.aliases.is_some() {
        ensure_name_free(
            tx,
            write,
            existing.book_id,
            existing.track_id,
            &target_name,
            &aliases,
            Some(entity_id),
        )
        .await?;
    }
    if let Some(importance) = patch.importance {
        validate_importance(importance)?;
    }
    if let Some(is_protagonist) = patch.is_protagonist
        && is_protagonist
        && !existing.is_protagonist
    {
        ensure_protagonist_free(tx, existing.book_id, Some(entity_id)).await?;
    }

    let mut active = entities::ActiveModel {
        id: Set(entity_id),
        updated_at: Set(stamp()),
        ..Default::default()
    };
    if let Some(kind) = patch.kind {
        active.kind = Set(kind);
    }
    if patch.name.is_some() {
        active.name = Set(target_name);
    }
    if patch.aliases.is_some() {
        active.aliases = Set(crate::services::json_text(aliases.clone()));
    }
    if let Some(summary) = &patch.summary {
        active.summary = Set(summary.trim().to_owned());
    }
    if let Some(description) = &patch.description {
        active.description = Set(description.clone());
    }
    if let Some(state) = &patch.state {
        active.state = Set(state.clone());
    }
    if let Some(attributes) = &patch.attributes {
        active.attributes = Set(crate::services::json_text(attributes.clone()));
    }
    if let Some(importance) = patch.importance {
        active.importance = Set(importance);
    }
    if let Some(status) = patch.status {
        active.status = Set(status);
    }
    if let Some(is_protagonist) = patch.is_protagonist {
        active.is_protagonist = Set(is_protagonist);
    }
    active.update(tx).await?;
    let row = super::load_entity(tx, entity_id).await?;
    let after = entity_snapshot(&row)?;
    if before == after {
        return Ok((
            detail(tx, entity_id, DETAIL_CHANGE_LIMIT).await?,
            Vec::new(),
        ));
    }
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id: row.book_id,
            target_type: ChangeTargetType::Entity,
            target_id: row.id,
            op: ChangeOp::Update,
            before: Some(&before),
            after: Some(&after),
            note: patch.note.as_deref().unwrap_or_default().trim(),
            reverts_id: None,
            track_id: row.track_id,
            chapter_id: None,
        },
    )
    .await?;
    let events = vec![changed_event(
        row.book_id,
        ChangeTargetType::Entity,
        row.id,
        ChangeOp::Update,
        change_id,
    )];
    Ok((detail(tx, entity_id, DETAIL_CHANGE_LIMIT).await?, events))
}

// ───────────────────────────── delete ─────────────────────────────

/// `DELETE /api/entities/:id` (spec/10 §2.4): one `delete` change with the full snapshot
/// (spec/02 §5.10).
pub async fn delete(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    entity_id: i32,
) -> CanonResult<()> {
    let (_, published) = super::write(db, move |tx| {
        Box::pin(async move {
            let events = delete_inner(tx, &CanonWrite::author(), entity_id).await?;
            Ok(((), events))
        })
    })
    .await?;
    events.publish_all(published);
    Ok(())
}

pub async fn delete_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    entity_id: i32,
) -> CanonResult<Vec<ServerEvent>> {
    let existing = super::load_entity(tx, entity_id).await?;
    write.ensure_writable(existing.track_id, &entity_label(&existing.name, entity_id))?;
    let before = entity_snapshot(&existing)?;
    // The relations, appearances and join rows cascade (spec/02 §2); a design link is SET NULL.
    entities::Entity::delete_by_id(entity_id).exec(tx).await?;
    let change_id = super::record_change(
        tx,
        write,
        ChangeWrite {
            book_id: existing.book_id,
            target_type: ChangeTargetType::Entity,
            target_id: entity_id,
            op: ChangeOp::Delete,
            before: Some(&before),
            after: None,
            note: "",
            reverts_id: None,
            track_id: existing.track_id,
            chapter_id: None,
        },
    )
    .await?;
    Ok(vec![changed_event(
        existing.book_id,
        ChangeTargetType::Entity,
        entity_id,
        ChangeOp::Delete,
        change_id,
    )])
}

// ───────────────────────────── merge (spec/06 §9) ─────────────────────────────

/// `POST /api/entities/:id/merge { from_id }`: A ← B (spec/06 §9).
///
/// One transaction moves B's records to A, drops the ones that would break a constraint, unions
/// the aliases, deletes B, and records two changes: an `update` on A and a `delete` on B, both
/// with the language-neutral note `#B → #A` (D-032). A merge is not revertible as a whole; the UI
/// warns.
pub async fn merge(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    entity_id: i32,
    input: EntityMergeInput,
) -> CanonResult<EntityDto> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { merge_inner(tx, entity_id, input).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn merge_inner(
    tx: &impl ConnectionTrait,
    into_id: i32,
    input: EntityMergeInput,
) -> CanonResult<(EntityDto, Vec<ServerEvent>)> {
    let write = CanonWrite::author();
    if into_id == input.from_id {
        return Err(CanonError::field(
            "from_id",
            "an entity cannot be merged into itself",
        ));
    }
    let target = super::load_entity(tx, into_id).await?;
    let source = super::load_entity(tx, input.from_id).await?;
    if target.book_id != source.book_id {
        return Err(CanonError::validation(
            "the entities belong to different books",
        ));
    }
    // "A shared entity cannot be merged into an isolated track's private entity (400
    // validation.failed); a private one can be merged into a shared one, and its moved private
    // rows keep their track_id" (spec/06 §9).
    if source.track_id.is_none() && target.track_id.is_some() {
        return Err(CanonError::validation(
            "a shared entity cannot be merged into an isolated track's private entity",
        ));
    }
    // Moving another isolated track's private rows would leave them pointing at a row their own
    // track cannot see (spec/06 §2): the two private canons are separate.
    if let (Some(source_track), Some(target_track)) = (source.track_id, target.track_id)
        && source_track != target_track
    {
        return Err(CanonError::validation(
            "entities of two isolated tracks belong to different canons and cannot be merged",
        ));
    }
    let target_before = entity_snapshot(&target)?;
    let source_before = entity_snapshot(&source)?;
    let note = merge_note(&source.name, &target.name);

    move_relations(tx, &target, &source).await?;
    move_appearances(tx, &target, &source).await?;
    move_link_rows(tx, super::LinkTable::TimelineEvent, into_id, input.from_id).await?;
    move_link_rows(tx, super::LinkTable::MemoryNote, into_id, input.from_id).await?;
    move_link_rows(tx, super::LinkTable::Foreshadowing, into_id, input.from_id).await?;
    move_design_link(tx, &target, &source).await?;
    move_track_focus(tx, &target, &source).await?;
    // The protagonist flag moves with the merge (D-046).
    let target_is_protagonist = target.is_protagonist || source.is_protagonist;
    // Union the aliases; B's name becomes an alias of A.
    let mut aliases = aliases_of(&target)?;
    aliases.push(source.name.clone());
    aliases.extend(aliases_of(&source)?);
    let aliases = clean_aliases(&aliases);
    // A never loses the more central importance to a minor duplicate.
    let importance = target.importance.max(source.importance);
    if target_is_protagonist && source.is_protagonist && !target.is_protagonist {
        // The partial unique index allows one protagonist per book: clear B's flag before A
        // takes it (D-046).
        entities::ActiveModel {
            id: Set(source.id),
            is_protagonist: Set(false),
            ..Default::default()
        }
        .update(tx)
        .await?;
    }
    entities::ActiveModel {
        id: Set(into_id),
        aliases: Set(crate::services::json_text(aliases.clone())),
        importance: Set(importance),
        is_protagonist: Set(target_is_protagonist),
        updated_at: Set(stamp()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    entities::Entity::delete_by_id(input.from_id)
        .exec(tx)
        .await?;
    // B's appearances are gone with it: recompute every bound (D-059).
    recompute_bounds(tx, target.book_id, &[into_id]).await?;
    let merged = super::load_entity(tx, into_id).await?;
    let target_after = entity_snapshot(&merged)?;
    let update_change = super::record_change(
        tx,
        &write,
        ChangeWrite {
            book_id: merged.book_id,
            target_type: ChangeTargetType::Entity,
            target_id: into_id,
            op: ChangeOp::Update,
            before: Some(&target_before),
            after: Some(&target_after),
            note: &note,
            reverts_id: None,
            track_id: merged.track_id,
            chapter_id: None,
        },
    )
    .await?;
    let delete_change = super::record_change(
        tx,
        &write,
        ChangeWrite {
            book_id: source.book_id,
            target_type: ChangeTargetType::Entity,
            target_id: input.from_id,
            op: ChangeOp::Delete,
            before: Some(&source_before),
            after: None,
            note: &note,
            reverts_id: None,
            track_id: source.track_id,
            chapter_id: None,
        },
    )
    .await?;
    let events = vec![
        changed_event(
            merged.book_id,
            ChangeTargetType::Entity,
            into_id,
            ChangeOp::Update,
            update_change,
        ),
        changed_event(
            source.book_id,
            ChangeTargetType::Entity,
            input.from_id,
            ChangeOp::Delete,
            delete_change,
        ),
    ];
    Ok((detail(tx, into_id, DETAIL_CHANGE_LIMIT).await?, events))
}

/// The language-neutral merge note (spec/06 §9, D-032).
pub fn merge_note(from_name: &str, into_name: &str) -> String {
    format!("#{from_name} → #{into_name}")
}

/// Moves B's relations to A, dropping ones between A and B and active duplicates (spec/06 §9).
async fn move_relations(
    tx: &impl ConnectionTrait,
    target: &entities::Model,
    source: &entities::Model,
) -> CanonResult<()> {
    let rows = relations::Entity::find()
        .filter(relations::Column::BookId.eq(source.book_id))
        .filter(
            Condition::any()
                .add(relations::Column::SourceId.eq(source.id))
                .add(relations::Column::TargetId.eq(source.id)),
        )
        .all(tx)
        .await?;
    // The active relations A already has (with anyone but B), in **both** directions: a moved
    // relation that lands on one of them is dropped (spec/06 §9). A relation `X → B` becomes
    // `X → A`, so A as the *target* counts too — checking only A as the source let that shape
    // reach the unique index and abort the merge.
    let existing = relations::Entity::find()
        .filter(relations::Column::BookId.eq(target.book_id))
        .filter(
            Condition::any()
                .add(relations::Column::SourceId.eq(target.id))
                .add(relations::Column::TargetId.eq(target.id)),
        )
        .filter(relations::Column::SourceId.ne(source.id))
        .filter(relations::Column::TargetId.ne(source.id))
        .filter(relations::Column::Status.eq(RelationStatus::Active))
        .all(tx)
        .await?;
    // The key is the ordered pair alone. `kind` is data on a relation, not part of its identity
    // (spec/06 §1.2), so a moved row whose wording differs is a second active relation on a pair
    // A already has and has to go, exactly like an identically-worded one.
    let mut taken: HashSet<(i32, i32, Option<i32>)> = existing
        .iter()
        .map(|row| (row.source_id, row.target_id, row.track_id))
        .collect();
    for row in rows {
        // A relation between A and B would become a self-relation (the table has a CHECK).
        if row.source_id == source.id && row.target_id == target.id
            || row.source_id == target.id && row.target_id == source.id
        {
            relations::Entity::delete_by_id(row.id).exec(tx).await?;
            continue;
        }
        let source_id = if row.source_id == source.id {
            target.id
        } else {
            row.source_id
        };
        let target_id = if row.target_id == source.id {
            target.id
        } else {
            row.target_id
        };
        if source_id == target_id {
            relations::Entity::delete_by_id(row.id).exec(tx).await?;
            continue;
        }
        // One end is A after the rewrite, so `taken` holds exactly the pairs that already carry an
        // active relation of A; a collision is dropped, a free pair is recorded. A row with a
        // *different* kind is still a collision: the pair can only keep one active relation, and
        // the merge cannot invent a combined wording, so A's existing row wins.
        if row.status == RelationStatus::Active
            && !taken.insert((source_id, target_id, row.track_id))
        {
            relations::Entity::delete_by_id(row.id).exec(tx).await?;
            continue;
        }
        relations::ActiveModel {
            id: Set(row.id),
            source_id: Set(source_id),
            target_id: Set(target_id),
            updated_at: Set(stamp()),
            ..Default::default()
        }
        .update(tx)
        .await?;
    }
    Ok(())
}

/// Moves B's appearances to A; where both appear, A keeps the more prominent role (spec/06 §9).
async fn move_appearances(
    tx: &impl ConnectionTrait,
    target: &entities::Model,
    source: &entities::Model,
) -> CanonResult<()> {
    let rows = appearances::Entity::find()
        .filter(appearances::Column::EntityId.eq(source.id))
        .all(tx)
        .await?;
    for row in rows {
        let existing = appearances::Entity::find()
            .filter(appearances::Column::ChapterId.eq(row.chapter_id))
            .filter(appearances::Column::EntityId.eq(target.id))
            .one(tx)
            .await?;
        match existing {
            None => {
                appearances::ActiveModel {
                    id: Set(row.id),
                    entity_id: Set(target.id),
                    ..Default::default()
                }
                .update(tx)
                .await?;
            }
            Some(existing) => {
                // A keeps the more prominent role: pov > major > minor > mentioned.
                if super::appearance::role_rank(row.role)
                    > super::appearance::role_rank(existing.role)
                {
                    appearances::ActiveModel {
                        id: Set(existing.id),
                        role: Set(row.role),
                        note: Set(row.note.clone()),
                        ..Default::default()
                    }
                    .update(tx)
                    .await?;
                }
                appearances::Entity::delete_by_id(row.id).exec(tx).await?;
            }
        }
    }
    Ok(())
}

/// Moves the join rows of one link table from B to A (deleting B cascades them otherwise).
async fn move_link_rows(
    tx: &impl ConnectionTrait,
    table: super::LinkTable,
    target_id: i32,
    source_id: i32,
) -> CanonResult<()> {
    use sea_orm::{DatabaseBackend, Statement};
    // `INSERT OR IGNORE` keeps an owner already linked to A from failing the primary key.
    let sql = format!(
        "INSERT OR IGNORE INTO {table}({owner}, entity_id) SELECT {owner}, ? FROM {table} WHERE \
         entity_id = ?",
        table = table.table(),
        owner = table.owner_column()
    );
    tx.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        sql,
        vec![target_id.into(), source_id.into()],
    ))
    .await?;
    Ok(())
}

/// Moves B's character-design link to A (spec/06 §9).
async fn move_design_link(
    tx: &impl ConnectionTrait,
    target: &entities::Model,
    source: &entities::Model,
) -> CanonResult<()> {
    let designs = character_designs::Entity::find()
        .filter(character_designs::Column::EntityId.eq(source.id))
        .order_by_asc(character_designs::Column::Rank)
        .all(tx)
        .await?;
    let target_linked = character_designs::Entity::find()
        .filter(character_designs::Column::EntityId.eq(target.id))
        .one(tx)
        .await?
        .is_some();
    for (index, design) in designs.into_iter().enumerate() {
        // A keeps its own link; B's first link moves only when A has none (a design has one
        // entity, so any extra B design is unlinked rather than pointed at A twice).
        let entity_id = if index == 0 && !target_linked {
            Some(target.id)
        } else {
            None
        };
        character_designs::ActiveModel {
            id: Set(design.id),
            entity_id: Set(entity_id),
            ..Default::default()
        }
        .update(tx)
        .await?;
    }
    Ok(())
}

/// Moves side tracks' `focus_entity_id` from B to A (spec/06 §9, D-046).
async fn move_track_focus(
    tx: &impl ConnectionTrait,
    target: &entities::Model,
    source: &entities::Model,
) -> CanonResult<()> {
    let rows = tracks::Entity::find()
        .filter(tracks::Column::FocusEntityId.eq(source.id))
        .all(tx)
        .await?;
    for row in rows {
        tracks::ActiveModel {
            id: Set(row.id),
            focus_entity_id: Set(Some(target.id)),
            updated_at: Set(stamp()),
            ..Default::default()
        }
        .update(tx)
        .await?;
    }
    Ok(())
}

// ───────────────────────────── protagonist (spec/06 §1.1) ─────────────────────────────

/// `POST /api/entities/:id/protagonist`: moves the flag to this entity (spec/06 §9).
pub async fn set_protagonist(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    entity_id: i32,
) -> CanonResult<EntityDto> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { set_protagonist_inner(tx, entity_id).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn set_protagonist_inner(
    tx: &impl ConnectionTrait,
    entity_id: i32,
) -> CanonResult<(EntityDto, Vec<ServerEvent>)> {
    let write = CanonWrite::author();
    let existing = super::load_entity(tx, entity_id).await?;
    write.ensure_writable(existing.track_id, &entity_label(&existing.name, entity_id))?;
    let mut events = Vec::new();
    // The flag moves: whoever held it loses it in the same transaction (one per book, §5 (7)).
    let current = entities::Entity::find()
        .filter(entities::Column::BookId.eq(existing.book_id))
        .filter(entities::Column::IsProtagonist.eq(1))
        .all(tx)
        .await?;
    for row in current {
        if row.id == entity_id {
            continue;
        }
        let before = entity_snapshot(&row)?;
        entities::ActiveModel {
            id: Set(row.id),
            is_protagonist: Set(false),
            updated_at: Set(stamp()),
            ..Default::default()
        }
        .update(tx)
        .await?;
        let updated = super::load_entity(tx, row.id).await?;
        let after = entity_snapshot(&updated)?;
        let change_id = super::record_change(
            tx,
            &write,
            ChangeWrite {
                book_id: row.book_id,
                target_type: ChangeTargetType::Entity,
                target_id: row.id,
                op: ChangeOp::Update,
                before: Some(&before),
                after: Some(&after),
                note: "",
                reverts_id: None,
                track_id: row.track_id,
                chapter_id: None,
            },
        )
        .await?;
        events.push(changed_event(
            row.book_id,
            ChangeTargetType::Entity,
            row.id,
            ChangeOp::Update,
            change_id,
        ));
    }
    if !existing.is_protagonist {
        let before = entity_snapshot(&existing)?;
        entities::ActiveModel {
            id: Set(entity_id),
            is_protagonist: Set(true),
            updated_at: Set(stamp()),
            ..Default::default()
        }
        .update(tx)
        .await?;
        let updated = super::load_entity(tx, entity_id).await?;
        let after = entity_snapshot(&updated)?;
        let change_id = super::record_change(
            tx,
            &write,
            ChangeWrite {
                book_id: existing.book_id,
                target_type: ChangeTargetType::Entity,
                target_id: entity_id,
                op: ChangeOp::Update,
                before: Some(&before),
                after: Some(&after),
                note: "",
                reverts_id: None,
                track_id: existing.track_id,
                chapter_id: None,
            },
        )
        .await?;
        events.push(changed_event(
            existing.book_id,
            ChangeTargetType::Entity,
            entity_id,
            ChangeOp::Update,
            change_id,
        ));
    }
    Ok((detail(tx, entity_id, DETAIL_CHANGE_LIMIT).await?, events))
}

/// At most one protagonist per book (spec/02 §5 (7)): the partial unique index is book-wide, so
/// a private row cannot carry a second one either. A side story's protagonist is its track's
/// `focus_entity_id` (D-043), not a second `is_protagonist`.
async fn ensure_protagonist_free(
    tx: &impl ConnectionTrait,
    book_id: i32,
    except_id: Option<i32>,
) -> CanonResult<()> {
    let mut statement = entities::Entity::find()
        .filter(entities::Column::BookId.eq(book_id))
        .filter(entities::Column::IsProtagonist.eq(1));
    if let Some(except_id) = except_id {
        statement = statement.filter(entities::Column::Id.ne(except_id));
    }
    if let Some(existing) = statement.one(tx).await? {
        return Err(super::conflict(
            format!(
                "{} is already the protagonist; use set_protagonist to move the flag",
                existing.name
            ),
            existing.id,
        ));
    }
    Ok(())
}

// ───────────────────────────── names (spec/05 §1.4) ─────────────────────────────

/// Rejects a duplicate name or alias among visible entities of the same canon (spec/05 §1.4).
async fn ensure_name_free(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    book_id: i32,
    track_id: Option<i32>,
    name: &str,
    aliases: &[String],
    except_id: Option<i32>,
) -> CanonResult<()> {
    let visibility = write
        .visibility
        .unwrap_or_else(|| Visibility::shared(book_id));
    let mut statement = entities::Entity::find()
        .filter(entities::Column::BookId.eq(book_id))
        .filter(visibility.visible_condition(entities::Column::TrackId));
    if let Some(except_id) = except_id {
        statement = statement.filter(entities::Column::Id.ne(except_id));
    }
    let rows = statement.all(tx).await?;
    let wanted: Vec<String> = std::iter::once(name.to_owned())
        .chain(aliases.iter().cloned())
        .map(|value| value.to_lowercase())
        .collect();
    for row in rows {
        // Another track's private row never collides with a shared or own row.
        if row.track_id != track_id {
            continue;
        }
        let mut candidates = vec![row.name.clone()];
        candidates.extend(aliases_of(&row)?);
        for candidate in candidates {
            if wanted.contains(&candidate.to_lowercase()) {
                return Err(super::conflict(
                    format!(
                        "an entity named or aliased {candidate:?} already exists (#{})",
                        row.id
                    ),
                    row.id,
                ));
            }
        }
    }
    Ok(())
}

/// The track an isolated-row create belongs to must be one of the book's isolated tracks.
async fn ensure_isolated_track(
    tx: &impl ConnectionTrait,
    book_id: i32,
    track_id: i32,
) -> CanonResult<()> {
    let track = tracks::Entity::find_by_id(track_id).one(tx).await?;
    let Some(track) = track else {
        return Err(CanonError::field("track_id", "the track does not exist"));
    };
    if track.book_id != book_id {
        return Err(CanonError::field(
            "track_id",
            "the track belongs to another book",
        ));
    }
    if track.canon_mode != CanonMode::Isolated {
        // Canon rows with a non-NULL track_id belong to isolated tracks only (spec/02 §5 (8)).
        return Err(CanonError::field(
            "track_id",
            "only an isolated track can own private canon rows",
        ));
    }
    Ok(())
}

// ───────────────────────────── appearance bounds (D-059) ─────────────────────────────

/// Recomputes `first_chapter_id` / `last_chapter_id` of the given entities from `appearances`
/// (spec/06 §1.1, D-059).
///
/// The order is (track `ord`, chapter `ord`), and a shared entity counts only shared-canon
/// chapters (the main track and canon-mode side tracks), so an isolated chapter never changes a
/// shared entity's bounds (D-036).
pub async fn recompute_bounds(
    tx: &impl ConnectionTrait,
    book_id: i32,
    entity_ids: &[i32],
) -> CanonResult<()> {
    let mut unique: Vec<i32> = entity_ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    if unique.is_empty() {
        return Ok(());
    }
    let orders = chapter_orders(tx, book_id).await?;
    for entity_id in unique {
        let Some(row) = entities::Entity::find_by_id(entity_id).one(tx).await? else {
            continue;
        };
        let rows = appearances::Entity::find()
            .filter(appearances::Column::EntityId.eq(entity_id))
            .all(tx)
            .await?;
        let mut first: Option<(i32, super::ChapterOrder)> = None;
        let mut last: Option<(i32, super::ChapterOrder)> = None;
        for appearance in &rows {
            let Some(order) = orders.get(&appearance.chapter_id).copied() else {
                continue;
            };
            // A shared entity counts only shared-canon chapters (D-036).
            if row.track_id.is_none() && !order.shared_canon {
                continue;
            }
            match first {
                None => first = Some((appearance.chapter_id, order)),
                Some((_, current)) if order.key() < current.key() => {
                    first = Some((appearance.chapter_id, order));
                }
                _ => {}
            }
            match last {
                None => last = Some((appearance.chapter_id, order)),
                Some((_, current)) if order.key() > current.key() => {
                    last = Some((appearance.chapter_id, order));
                }
                _ => {}
            }
        }
        let first_id = first.map(|(chapter_id, _)| chapter_id);
        let last_id = last.map(|(chapter_id, _)| chapter_id);
        if row.first_chapter_id == first_id && row.last_chapter_id == last_id {
            continue;
        }
        entities::ActiveModel {
            id: Set(entity_id),
            first_chapter_id: Set(first_id),
            last_chapter_id: Set(last_id),
            ..Default::default()
        }
        .update(tx)
        .await?;
    }
    Ok(())
}

// ───────────────────────────── DTO helpers ─────────────────────────────

pub fn kind_of(row: &entities::Model) -> CanonResult<EntityKind> {
    Ok(row.kind)
}

pub fn status_of(row: &entities::Model) -> CanonResult<EntityStatus> {
    Ok(row.status)
}

pub fn attributes_of(row: &entities::Model) -> CanonResult<BTreeMap<String, String>> {
    Ok(row.attributes.clone().into_inner())
}

pub fn list_item(row: &entities::Model) -> CanonResult<EntityListItemDto> {
    Ok(EntityListItemDto {
        id: row.id,
        book_id: row.book_id,
        track_id: row.track_id,
        kind: kind_of(row)?,
        name: row.name.clone(),
        aliases: aliases_of(row)?,
        summary: row.summary.clone(),
        importance: row.importance,
        status: status_of(row)?,
        is_protagonist: row.is_protagonist,
        first_chapter_id: row.first_chapter_id,
        last_chapter_id: row.last_chapter_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn appearance_dto(row: &appearances::Model) -> CanonResult<AppearanceDto> {
    Ok(AppearanceDto {
        id: row.id,
        chapter_id: row.chapter_id,
        entity_id: row.entity_id,
        role: row.role,
        note: row.note.clone(),
    })
}

/// `Name(#12)`, the form every tool and change note uses for an entity (spec/05 §1.5).
pub fn entity_label(name: &str, id: i32) -> String {
    format!("{name}(#{id})")
}

/// Unused-import guard for the join-table entity of the timeline links merged above.
#[allow(dead_code)]
fn _join_tables() -> &'static str {
    std::any::type_name::<timeline_event_entities::Entity>()
}

// ───────────────────────────── revert support (spec/06 §3) ─────────────────────────────

/// The current snapshot of an entity, when it still exists.
pub async fn snapshot_now(
    conn: &impl ConnectionTrait,
    entity_id: i32,
) -> CanonResult<Option<crate::db::json::Snapshot>> {
    match entities::Entity::find_by_id(entity_id).one(conn).await? {
        Some(row) => Ok(Some(entity_snapshot(&row)?)),
        None => Ok(None),
    }
}

/// Restores an entity from a `before` snapshot (the revert of an update).
pub async fn restore_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    entity_id: i32,
    before: &crate::db::json::Snapshot,
) -> CanonResult<crate::db::json::Snapshot> {
    let crate::db::json::Snapshot::Entity {
        kind,
        name,
        aliases,
        summary,
        description,
        state,
        attributes,
        importance,
        status,
        is_protagonist,
        ..
    } = before
    else {
        return Err(CanonError::internal(
            "an entity revert needs an entity snapshot",
        ));
    };
    let existing = super::load_entity(tx, entity_id).await?;
    write.ensure_writable(existing.track_id, &entity_label(&existing.name, entity_id))?;
    // Restoring the protagonist flag must not collide with another holder (spec/02 §5 (7)).
    if *is_protagonist && !existing.is_protagonist {
        let other = entities::Entity::find()
            .filter(entities::Column::BookId.eq(existing.book_id))
            .filter(entities::Column::IsProtagonist.eq(1))
            .filter(entities::Column::Id.ne(entity_id))
            .one(tx)
            .await?;
        if let Some(other) = other {
            entities::ActiveModel {
                id: Set(other.id),
                is_protagonist: Set(false),
                updated_at: Set(stamp()),
                ..Default::default()
            }
            .update(tx)
            .await?;
        }
    }
    entities::ActiveModel {
        id: Set(entity_id),
        kind: Set(*kind),
        name: Set(name.clone()),
        aliases: Set(crate::services::json_text(aliases.clone())),
        summary: Set(summary.clone()),
        description: Set(description.clone()),
        state: Set(state.clone()),
        attributes: Set(crate::services::json_text(attributes.clone())),
        importance: Set(*importance),
        status: Set(*status),
        is_protagonist: Set(*is_protagonist),
        updated_at: Set(stamp()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    let row = super::load_entity(tx, entity_id).await?;
    entity_snapshot(&row)
}

/// Recreates a deleted entity from its `before` snapshot, with a **new id** (spec/02 §5.10).
///
/// The relations of the deleted entity are **not** restored (a documented limitation: the UI
/// states it), because they would need their other ends to still exist and match by name.
pub async fn recreate_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    before: &crate::db::json::Snapshot,
) -> CanonResult<(i32, crate::db::json::Snapshot)> {
    let crate::db::json::Snapshot::Entity {
        kind,
        name,
        aliases,
        summary,
        description,
        state,
        attributes,
        importance,
        status,
        is_protagonist,
        track_id,
    } = before
    else {
        return Err(CanonError::internal(
            "an entity revert needs an entity snapshot",
        ));
    };
    let book_id = write
        .visibility
        .map(|visibility| visibility.book_id)
        .ok_or_else(|| CanonError::internal("an entity revert needs a book"))?;
    super::super::books::get(tx, book_id).await?;
    // A name that another entity took meanwhile would violate the unique index: the author gets
    // a `conflict.exists` they can resolve, rather than an internal error.
    if let Some(other) = find_by_name(tx, book_id, *track_id, name).await? {
        return Err(super::conflict(
            format!("an entity named {name:?} exists again (#{})", other.id),
            other.id,
        ));
    }
    if *is_protagonist {
        let other = entities::Entity::find()
            .filter(entities::Column::BookId.eq(book_id))
            .filter(entities::Column::IsProtagonist.eq(1))
            .one(tx)
            .await?;
        if let Some(other) = other {
            entities::ActiveModel {
                id: Set(other.id),
                is_protagonist: Set(false),
                updated_at: Set(stamp()),
                ..Default::default()
            }
            .update(tx)
            .await?;
        }
    }
    let timestamp = stamp();
    // SQLite would hand the recreated row the deleted one's rowid; spec/02 §5.10 wants a NEW id.
    let new_id = super::fresh_id(tx, "entities", ChangeTargetType::Entity).await?;
    let model = entities::ActiveModel {
        id: Set(new_id),
        book_id: Set(book_id),
        track_id: Set(*track_id),
        kind: Set(*kind),
        name: Set(name.clone()),
        aliases: Set(crate::services::json_text(aliases.clone())),
        summary: Set(summary.clone()),
        description: Set(description.clone()),
        state: Set(state.clone()),
        attributes: Set(crate::services::json_text(attributes.clone())),
        importance: Set(*importance),
        status: Set(*status),
        is_protagonist: Set(*is_protagonist),
        first_chapter_id: Set(None),
        last_chapter_id: Set(None),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
    };
    let row = entities::Entity::insert(model)
        .exec_with_returning(tx)
        .await?;
    link_design_by_name(tx, &row).await?;
    Ok((row.id, entity_snapshot(&row)?))
}

async fn find_by_name(
    conn: &impl ConnectionTrait,
    book_id: i32,
    track_id: Option<i32>,
    name: &str,
) -> CanonResult<Option<entities::Model>> {
    let mut statement = entities::Entity::find()
        .filter(entities::Column::BookId.eq(book_id))
        .filter(entities::Column::Name.eq(name));
    statement = match track_id {
        Some(track_id) => statement.filter(entities::Column::TrackId.eq(track_id)),
        None => statement.filter(entities::Column::TrackId.is_null()),
    };
    Ok(statement.one(conn).await?)
}
