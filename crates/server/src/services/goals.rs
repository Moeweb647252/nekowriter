//! Writing goals (spec/06 §11, plan §4): CRUD, validation, the one evaluator, links, change
//! history helpers and the requirements the runtime switch work already consumes.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, Set,
};
use serde::Deserialize;

use crate::api::dto::goals::{
    GoalConditionStateDto, GoalEntityLinkDto, GoalEventLinkDto, GoalForeshadowingLinkDto,
    GoalNodeDto, GoalNodeInput, GoalQuery,
};
use crate::db::entities::{
    change_log, chapters, entities, foreshadowing, goal_node_entities, goal_node_events,
    goal_node_foreshadowing, goal_nodes, timeline_events, tracks,
};
use crate::db::json::{
    ChangeOp, ChangeTargetType, ForeshadowingStatus, GoalCondition, GoalLevel, GoalStatus,
    JsonText, Snapshot, TrackKind,
};
use crate::error::{AppError, ErrorCode};
use crate::events::ServerEvent;

use super::canon::{
    CanonError, CanonResult, CanonWrite, ChangeWrite, Visibility, changed_event, record_change,
};

/// The cap of `block.goals` (plan §4.5).
pub const GOAL_NODE_CAP: usize = 30;

/// The relevant nodes of one chapter, in render order, plus the number left out by the cap.
#[derive(Debug, Clone, Default)]
pub struct GoalSelection {
    pub nodes: Vec<goal_nodes::Model>,
    pub omitted: i32,
}

/// A PATCH body for one goal node. `parent_id`/`chapter_from`/`chapter_to` use an outer `None`
/// for "leave unchanged" and `Some(None)` for "clear".
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct GoalNodePatch {
    #[serde(default)]
    pub level: Option<GoalLevel>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub track_id: Option<Option<i32>>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub parent_id: Option<Option<i32>>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub status: Option<GoalStatus>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub chapter_from: Option<Option<i32>>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub chapter_to: Option<Option<i32>>,
    #[serde(default)]
    pub conditions: Option<Vec<GoalCondition>>,
    #[serde(default)]
    pub entity_ids: Option<Vec<i32>>,
    #[serde(default)]
    pub event_ids: Option<Vec<i32>>,
    #[serde(default)]
    pub foreshadowing_ids: Option<Vec<i32>>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub confirm_open_children: Option<bool>,
}

/// `Option<Option<T>>` with wire semantics: an absent field is `None` (leave unchanged), while
/// an explicit JSON `null` is `Some(None)` (clear). Without this, serde collapses both to `None`.
fn deserialize_some<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// The derived state of one condition (spec/06 §11.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionEvaluation {
    pub satisfied: bool,
    pub broken: bool,
    pub target_label: Option<String>,
}

/// The derived state of one node (spec/06 §11.3), returned by [`evaluate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalEvaluation {
    pub ready: bool,
    pub closable: bool,
    pub conditions: Vec<ConditionEvaluation>,
}

// ───────────────────────────── list / evaluator ─────────────────────────────

/// `GET /books/:id/goals` (spec/10 §2.4).
pub async fn list(
    conn: &impl ConnectionTrait,
    book_id: i32,
    query: &GoalQuery,
) -> Result<Vec<GoalNodeDto>, AppError> {
    crate::services::books::get(conn, book_id).await?;
    let mut nodes = goal_nodes::Entity::find()
        .filter(goal_nodes::COLUMN.book_id.eq(book_id))
        .order_by_asc(goal_nodes::COLUMN.id)
        .all(conn)
        .await?;
    // Spec/06 §2: no filter means shared canon only; a track filter means shared OR that track.
    let visibility = match query.track_id {
        Some(track_id) => Visibility::isolated(book_id, track_id),
        None => Visibility::shared(book_id),
    };
    nodes.retain(|node| visibility.sees(node.track_id));
    if let Some(status) = query.status {
        nodes.retain(|node| node.status == status);
    }
    if query.entity_id.is_some() || query.event_id.is_some() || query.foreshadowing_id.is_some() {
        let entity_links = if let Some(entity_id) = query.entity_id {
            goal_node_entities::Entity::find()
                .filter(goal_node_entities::COLUMN.entity_id.eq(entity_id))
                .all(conn)
                .await?
                .into_iter()
                .map(|link| link.node_id)
                .collect::<HashSet<_>>()
        } else {
            HashSet::new()
        };
        let event_links = if let Some(event_id) = query.event_id {
            goal_node_events::Entity::find()
                .filter(goal_node_events::COLUMN.event_id.eq(event_id))
                .all(conn)
                .await?
                .into_iter()
                .map(|link| link.node_id)
                .collect::<HashSet<_>>()
        } else {
            HashSet::new()
        };
        let hook_links = if let Some(hook_id) = query.foreshadowing_id {
            goal_node_foreshadowing::Entity::find()
                .filter(goal_node_foreshadowing::COLUMN.foreshadowing_id.eq(hook_id))
                .all(conn)
                .await?
                .into_iter()
                .map(|link| link.node_id)
                .collect::<HashSet<_>>()
        } else {
            HashSet::new()
        };
        nodes.retain(|node| {
            let condition_match = node.conditions.0.iter().any(|condition| match condition {
                GoalCondition::EntityStatusIs { entity_id, .. } => {
                    query.entity_id == Some(*entity_id)
                }
                GoalCondition::ForeshadowingResolved { foreshadowing_id } => {
                    query.foreshadowing_id == Some(*foreshadowing_id)
                }
                GoalCondition::ChapterAtLeast { .. } => false,
            });
            entity_links.contains(&node.id)
                || event_links.contains(&node.id)
                || hook_links.contains(&node.id)
                || condition_match
        });
    }
    load_dtos(conn, book_id, &nodes).await
}

/// Evaluate every requested node against the book's current canon (spec/06 §11.3).
///
/// The returned map is keyed by node id. `nodes` may be a filtered list; the function reads all
/// book nodes for children and target lookup, so a filtered list still gets the same derived
/// `closable` state the full list would show.
pub async fn evaluate(
    conn: &impl ConnectionTrait,
    book_id: i32,
    nodes: &[goal_nodes::Model],
) -> Result<BTreeMap<i32, GoalEvaluation>, AppError> {
    let all_nodes = goal_nodes::Entity::find()
        .filter(goal_nodes::COLUMN.book_id.eq(book_id))
        .all(conn)
        .await?;
    let entity_rows: HashMap<i32, entities::Model> = entities::Entity::find()
        .filter(entities::COLUMN.book_id.eq(book_id))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    let hook_rows: HashMap<i32, foreshadowing::Model> = foreshadowing::Entity::find()
        .filter(foreshadowing::COLUMN.book_id.eq(book_id))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    let track_rows = tracks::Entity::find()
        .filter(tracks::COLUMN.book_id.eq(book_id))
        .all(conn)
        .await?;
    let main_track = track_rows
        .iter()
        .find(|track| track.kind == TrackKind::Main)
        .map(|track| track.id);
    let mut official_by_track: HashMap<i32, BTreeSet<i32>> = HashMap::new();
    for chapter in chapters::Entity::find()
        .filter(chapters::COLUMN.book_id.eq(book_id))
        .filter(
            chapters::COLUMN
                .status
                .eq(crate::db::json::ChapterStatus::Official),
        )
        .all(conn)
        .await?
    {
        official_by_track
            .entry(chapter.track_id)
            .or_default()
            .insert(chapter.ord);
    }
    let by_parent: HashMap<Option<i32>, Vec<&goal_nodes::Model>> = {
        let mut map: HashMap<Option<i32>, Vec<&goal_nodes::Model>> = HashMap::new();
        for node in &all_nodes {
            map.entry(node.parent_id).or_default().push(node);
        }
        map
    };

    let mut out = BTreeMap::new();
    for node in nodes {
        let target_track = node.track_id.or(main_track);
        let conditions = node
            .conditions
            .0
            .iter()
            .map(|condition| {
                evaluate_condition(
                    condition,
                    node.track_id,
                    target_track,
                    &official_by_track,
                    &entity_rows,
                    &hook_rows,
                )
            })
            .collect::<Vec<_>>();
        let ready = node.status == GoalStatus::Waiting
            && !conditions.is_empty()
            && conditions
                .iter()
                .all(|condition| condition.satisfied && !condition.broken);
        let children = by_parent.get(&Some(node.id)).cloned().unwrap_or_default();
        let closable = node.status == GoalStatus::Active
            && !children.is_empty()
            && children
                .iter()
                .all(|child| !matches!(child.status, GoalStatus::Active | GoalStatus::Waiting))
            && children
                .iter()
                .any(|child| child.status == GoalStatus::Done);
        out.insert(
            node.id,
            GoalEvaluation {
                ready,
                closable,
                conditions,
            },
        );
    }
    Ok(out)
}

fn evaluate_condition(
    condition: &GoalCondition,
    node_track: Option<i32>,
    target_track: Option<i32>,
    official_by_track: &HashMap<i32, BTreeSet<i32>>,
    entities: &HashMap<i32, entities::Model>,
    hooks: &HashMap<i32, foreshadowing::Model>,
) -> ConditionEvaluation {
    match condition {
        GoalCondition::ChapterAtLeast { chapter } => {
            let satisfied = target_track
                .and_then(|track| official_by_track.get(&track))
                .is_some_and(|numbers| numbers.iter().any(|number| number >= chapter));
            ConditionEvaluation {
                satisfied,
                broken: false,
                target_label: None,
            }
        }
        GoalCondition::ForeshadowingResolved { foreshadowing_id } => {
            let row = hooks.get(foreshadowing_id);
            let visible = row.is_some_and(|row| visible(node_track, row.track_id));
            let satisfied =
                row.is_some_and(|row| row.status == ForeshadowingStatus::Resolved && visible);
            let broken = row.is_none()
                || !visible
                || row.is_some_and(|row| row.status == ForeshadowingStatus::Abandoned);
            ConditionEvaluation {
                satisfied,
                broken,
                target_label: visible.then(|| row.map(|row| row.title.clone())).flatten(),
            }
        }
        GoalCondition::EntityStatusIs { entity_id, status } => {
            let row = entities.get(entity_id);
            let visible = row.is_some_and(|row| visible(node_track, row.track_id));
            let satisfied = row.is_some_and(|row| visible && row.status == *status);
            let broken = row.is_none() || !visible;
            ConditionEvaluation {
                satisfied,
                broken,
                target_label: visible.then(|| row.map(|row| row.name.clone())).flatten(),
            }
        }
    }
}

fn visible(node_track: Option<i32>, row_track: Option<i32>) -> bool {
    match node_track {
        None => row_track.is_none(),
        Some(track_id) => row_track.is_none() || row_track == Some(track_id),
    }
}

/// Load the DTOs of a node list with one batch per link table and one evaluator pass.
async fn load_dtos(
    conn: &impl ConnectionTrait,
    book_id: i32,
    nodes: &[goal_nodes::Model],
) -> Result<Vec<GoalNodeDto>, AppError> {
    if nodes.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i32> = nodes.iter().map(|node| node.id).collect();
    let entity_links = goal_entity_links(conn, &ids).await?;
    let event_links = goal_event_links(conn, &ids).await?;
    let hook_links = goal_foreshadowing_links(conn, &ids).await?;
    let evaluations = evaluate(conn, book_id, nodes).await?;
    Ok(nodes
        .iter()
        .map(|node| {
            let evaluation = evaluations
                .get(&node.id)
                .cloned()
                .unwrap_or(GoalEvaluation {
                    ready: false,
                    closable: false,
                    conditions: Vec::new(),
                });
            GoalNodeDto {
                id: node.id,
                book_id: node.book_id,
                track_id: node.track_id,
                level: node.level,
                parent_id: node.parent_id,
                title: node.title.clone(),
                purpose: node.purpose.clone(),
                status: node.status,
                chapter_from: node.chapter_from,
                chapter_to: node.chapter_to,
                conditions: node
                    .conditions
                    .0
                    .iter()
                    .cloned()
                    .zip(evaluation.conditions)
                    .map(|(condition, state)| GoalConditionStateDto {
                        condition,
                        satisfied: state.satisfied,
                        broken: state.broken,
                        target_label: state.target_label,
                    })
                    .collect(),
                entities: entity_links.get(&node.id).cloned().unwrap_or_default(),
                events: event_links.get(&node.id).cloned().unwrap_or_default(),
                foreshadowing: hook_links.get(&node.id).cloned().unwrap_or_default(),
                ready: evaluation.ready,
                closable: evaluation.closable,
                created_at: node.created_at,
                updated_at: node.updated_at,
            }
        })
        .collect())
}

async fn goal_entity_links(
    conn: &impl ConnectionTrait,
    ids: &[i32],
) -> Result<BTreeMap<i32, Vec<GoalEntityLinkDto>>, AppError> {
    let links = goal_node_entities::Entity::find()
        .filter(goal_node_entities::COLUMN.node_id.is_in(ids.to_vec()))
        .all(conn)
        .await?;
    let entity_ids: Vec<i32> = links.iter().map(|link| link.entity_id).collect();
    let names: HashMap<i32, String> = entities::Entity::find()
        .filter(entities::COLUMN.id.is_in(entity_ids))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| (row.id, row.name))
        .collect();
    let mut out: BTreeMap<i32, Vec<GoalEntityLinkDto>> = BTreeMap::new();
    for link in links {
        if let Some(name) = names.get(&link.entity_id) {
            out.entry(link.node_id)
                .or_default()
                .push(GoalEntityLinkDto {
                    id: link.entity_id,
                    name: name.clone(),
                });
        }
    }
    Ok(out)
}

async fn goal_event_links(
    conn: &impl ConnectionTrait,
    ids: &[i32],
) -> Result<BTreeMap<i32, Vec<GoalEventLinkDto>>, AppError> {
    let links = goal_node_events::Entity::find()
        .filter(goal_node_events::COLUMN.node_id.is_in(ids.to_vec()))
        .all(conn)
        .await?;
    let event_ids: Vec<i32> = links.iter().map(|link| link.event_id).collect();
    let titles: HashMap<i32, String> = timeline_events::Entity::find()
        .filter(timeline_events::COLUMN.id.is_in(event_ids))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| (row.id, row.title))
        .collect();
    let mut out: BTreeMap<i32, Vec<GoalEventLinkDto>> = BTreeMap::new();
    for link in links {
        if let Some(title) = titles.get(&link.event_id) {
            out.entry(link.node_id).or_default().push(GoalEventLinkDto {
                id: link.event_id,
                title: title.clone(),
            });
        }
    }
    Ok(out)
}

async fn goal_foreshadowing_links(
    conn: &impl ConnectionTrait,
    ids: &[i32],
) -> Result<BTreeMap<i32, Vec<GoalForeshadowingLinkDto>>, AppError> {
    let links = goal_node_foreshadowing::Entity::find()
        .filter(goal_node_foreshadowing::COLUMN.node_id.is_in(ids.to_vec()))
        .all(conn)
        .await?;
    let hook_ids: Vec<i32> = links.iter().map(|link| link.foreshadowing_id).collect();
    let hooks: HashMap<i32, foreshadowing::Model> = foreshadowing::Entity::find()
        .filter(foreshadowing::COLUMN.id.is_in(hook_ids))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    let mut out: BTreeMap<i32, Vec<GoalForeshadowingLinkDto>> = BTreeMap::new();
    for link in links {
        if let Some(hook) = hooks.get(&link.foreshadowing_id) {
            out.entry(link.node_id)
                .or_default()
                .push(GoalForeshadowingLinkDto {
                    id: hook.id,
                    title: hook.title.clone(),
                    status: hook.status,
                });
        }
    }
    Ok(out)
}

// ───────────────────────────── mutations ─────────────────────────────

pub async fn create(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    book_id: i32,
    input: GoalNodeInput,
) -> CanonResult<GoalNodeDto> {
    let (dto, published) = super::canon::write(db, move |tx| {
        Box::pin(async move { create_inner(tx, &CanonWrite::author(), book_id, input).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn create_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    book_id: i32,
    input: GoalNodeInput,
) -> CanonResult<(GoalNodeDto, Vec<ServerEvent>)> {
    crate::services::books::get(tx, book_id).await?;
    let track_id = write.new_row_track().or(input.track_id);
    if let Some(track_id) = track_id {
        ensure_isolated_track(tx, book_id, track_id).await?;
    }
    let title = clean_title(input.title)?;
    let purpose = clean_purpose(input.purpose)?;
    let status = input.status.unwrap_or(GoalStatus::Active);
    let conditions = clean_conditions(input.conditions.unwrap_or_default())?;
    validate_status_conditions(status, &conditions)?;
    validate_range(input.chapter_from, input.chapter_to)?;
    let parent = match input.parent_id {
        Some(parent_id) => {
            Some(validate_parent(tx, book_id, parent_id, track_id, input.level).await?)
        }
        None => None,
    };
    validate_links(
        tx,
        book_id,
        track_id,
        input.entity_ids.clone().unwrap_or_default(),
        input.event_ids.clone().unwrap_or_default(),
        input.foreshadowing_ids.clone().unwrap_or_default(),
    )
    .await?;
    validate_condition_targets(tx, book_id, track_id, &conditions).await?;
    let timestamp = Utc::now();
    let new_id = super::canon::fresh_id(tx, "goal_nodes", target_type(input.level)).await?;
    let node = goal_nodes::ActiveModel {
        id: Set(new_id),
        book_id: Set(book_id),
        track_id: Set(track_id),
        level: Set(input.level),
        parent_id: Set(parent.as_ref().map(|parent| parent.id)),
        title: Set(title),
        purpose: Set(purpose),
        status: Set(status),
        conditions: Set(JsonText(conditions.clone())),
        chapter_from: Set(input.chapter_from),
        chapter_to: Set(input.chapter_to),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
    }
    .insert(tx)
    .await?;
    let entity_ids = input.entity_ids.unwrap_or_default();
    let event_ids = input.event_ids.unwrap_or_default();
    let foreshadowing_ids = input.foreshadowing_ids.unwrap_or_default();
    replace_links(tx, node.id, &entity_ids, &event_ids, &foreshadowing_ids).await?;
    let after = snapshot(tx, &node).await?;
    let change = record_goal_change(
        tx,
        write,
        node.book_id,
        node.level,
        node.id,
        ChangeOp::Create,
        None,
        Some(&after),
        "",
        node.track_id,
    )
    .await?;
    let dto = load_dtos(tx, book_id, std::slice::from_ref(&node))
        .await?
        .pop()
        .ok_or_else(|| CanonError::internal("the created goal did not load"))?;
    Ok((
        dto,
        vec![changed_event(
            node.book_id,
            target_type(node.level),
            node.id,
            ChangeOp::Create,
            change,
        )],
    ))
}

pub async fn update(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    node_id: i32,
    patch: GoalNodePatch,
) -> CanonResult<GoalNodeDto> {
    let (dto, published) = super::canon::write(db, move |tx| {
        Box::pin(async move { update_inner(tx, &CanonWrite::author(), node_id, patch).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn update_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    node_id: i32,
    patch: GoalNodePatch,
) -> CanonResult<(GoalNodeDto, Vec<ServerEvent>)> {
    let existing = load_node(tx, node_id).await?;
    write.ensure_writable(existing.track_id, &format!("goal #{node_id}"))?;
    if let Some(level) = patch.level
        && level != existing.level
    {
        return Err(CanonError::field("level", "level cannot change"));
    }
    if patch.track_id.is_some() {
        return Err(CanonError::field("track_id", "track_id cannot change"));
    }
    let title = match patch.title {
        Some(title) => clean_title(title)?,
        None => existing.title.clone(),
    };
    let purpose = match patch.purpose {
        Some(purpose) => clean_purpose(purpose)?,
        None => existing.purpose.clone(),
    };
    let status = patch.status.unwrap_or(existing.status);
    let conditions_written = patch.conditions.is_some();
    let conditions = match patch.conditions {
        Some(conditions) => clean_conditions(conditions)?,
        None => existing.conditions.0.clone(),
    };
    validate_status_conditions(status, &conditions)?;
    let chapter_from = patch.chapter_from.unwrap_or(existing.chapter_from);
    let chapter_to = patch.chapter_to.unwrap_or(existing.chapter_to);
    validate_range(chapter_from, chapter_to)?;
    let parent_id = match patch.parent_id {
        Some(parent_id) => parent_id,
        None => existing.parent_id,
    };
    let parent = match parent_id {
        Some(parent_id) => Some(
            validate_parent(
                tx,
                existing.book_id,
                parent_id,
                existing.track_id,
                existing.level,
            )
            .await?,
        ),
        None => None,
    };
    let (existing_entities, existing_events, existing_hooks) =
        current_link_ids(tx, existing.id).await?;
    let entity_ids = patch.entity_ids.unwrap_or(existing_entities);
    let event_ids = patch.event_ids.unwrap_or(existing_events);
    let foreshadowing_ids = patch.foreshadowing_ids.unwrap_or(existing_hooks);
    validate_links(
        tx,
        existing.book_id,
        existing.track_id,
        entity_ids.clone(),
        event_ids.clone(),
        foreshadowing_ids.clone(),
    )
    .await?;
    if conditions_written {
        validate_condition_targets(tx, existing.book_id, existing.track_id, &conditions).await?;
    }
    if matches!(status, GoalStatus::Done | GoalStatus::Dropped)
        && !matches!(existing.status, GoalStatus::Done | GoalStatus::Dropped)
        && patch.confirm_open_children != Some(true)
    {
        let open = open_descendants(tx, existing.id).await?;
        if !open.is_empty() {
            return Err(CanonError::App(
                AppError::new(ErrorCode::GoalOpenChildren, "the goal has open descendants")
                    .with_details(serde_json::json!({
                        "children": open.iter().map(|child| serde_json::json!({
                            "id": child.id,
                            "level": child.level.as_str(),
                            "title": child.title,
                            "status": child.status.as_str(),
                        })).collect::<Vec<_>>()
                    })),
            ));
        }
    }
    let before = snapshot(tx, &existing).await?;
    let node = goal_nodes::ActiveModel {
        id: Set(existing.id),
        parent_id: Set(parent.as_ref().map(|parent| parent.id)),
        title: Set(title),
        purpose: Set(purpose),
        status: Set(status),
        conditions: Set(JsonText(conditions)),
        chapter_from: Set(chapter_from),
        chapter_to: Set(chapter_to),
        updated_at: Set(Utc::now()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    replace_links(tx, node.id, &entity_ids, &event_ids, &foreshadowing_ids).await?;
    let after = snapshot(tx, &node).await?;
    if before == after {
        let dto = load_dtos(tx, existing.book_id, std::slice::from_ref(&node))
            .await?
            .pop()
            .ok_or_else(|| CanonError::internal("the goal did not load"))?;
        return Ok((dto, Vec::new()));
    }
    let change = record_goal_change(
        tx,
        write,
        node.book_id,
        node.level,
        node.id,
        ChangeOp::Update,
        Some(&before),
        Some(&after),
        patch.note.as_deref().unwrap_or(""),
        node.track_id,
    )
    .await?;
    let dto = load_dtos(tx, existing.book_id, std::slice::from_ref(&node))
        .await?
        .pop()
        .ok_or_else(|| CanonError::internal("the updated goal did not load"))?;
    Ok((
        dto,
        vec![changed_event(
            node.book_id,
            target_type(node.level),
            node.id,
            ChangeOp::Update,
            change,
        )],
    ))
}

pub async fn delete(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    node_id: i32,
) -> CanonResult<()> {
    let (_, published) = super::canon::write(db, move |tx| {
        Box::pin(async move {
            let events = delete_inner(tx, &CanonWrite::author(), node_id).await?;
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
    node_id: i32,
) -> CanonResult<Vec<ServerEvent>> {
    let existing = load_node(tx, node_id).await?;
    write.ensure_writable(existing.track_id, &format!("goal #{node_id}"))?;
    let children_count = goal_nodes::Entity::find()
        .filter(goal_nodes::COLUMN.parent_id.eq(node_id))
        .count(tx)
        .await?;
    if children_count > 0 {
        return Err(CanonError::App(
            AppError::new(ErrorCode::GoalHasChildren, "the goal has children")
                .with_details(serde_json::json!({ "children_count": children_count })),
        ));
    }
    let before = snapshot(tx, &existing).await?;
    goal_nodes::Entity::delete_by_id(node_id).exec(tx).await?;
    let change = record_goal_change(
        tx,
        write,
        existing.book_id,
        existing.level,
        node_id,
        ChangeOp::Delete,
        Some(&before),
        None,
        "",
        existing.track_id,
    )
    .await?;
    Ok(vec![changed_event(
        existing.book_id,
        target_type(existing.level),
        node_id,
        ChangeOp::Delete,
        change,
    )])
}
/// The `block.goals` set of a chapter (plan §4.5).
pub async fn relevant_nodes(
    conn: &impl ConnectionTrait,
    book_id: i32,
    track: &tracks::Model,
    chapter: &chapters::Model,
) -> Result<GoalSelection, AppError> {
    let nodes = goal_nodes::Entity::find()
        .filter(goal_nodes::COLUMN.book_id.eq(book_id))
        .all(conn)
        .await?;
    if nodes.is_empty() {
        return Ok(GoalSelection::default());
    }
    let by_id: BTreeMap<i32, &goal_nodes::Model> =
        nodes.iter().map(|node| (node.id, node)).collect();
    // Only the nodes that pass the relevance gates are candidates. Ancestors are collected
    // later, per *selected* task: pre-seeding them here would leave the ancestor of a task that
    // the cap drops without a rendered descendant (task 13.4 review S2-1).
    let candidates: Vec<&goal_nodes::Model> = nodes
        .iter()
        .filter(|node| {
            node.status == GoalStatus::Active
                && node.chapter_from.is_none_or(|from| from <= chapter.ord)
                && in_scope(node, track)
                && !has_closed_ancestor(node, &by_id, &mut BTreeSet::new())
        })
        .collect();
    if candidates.is_empty() {
        return Ok(GoalSelection::default());
    }
    // The universe used for the "left out" count: candidates plus the active ancestors they
    // would bring. It is deliberately not used to fill the render list.
    let mut relevant_ids: BTreeSet<i32> = candidates.iter().map(|node| node.id).collect();
    for candidate in &candidates {
        collect_active_ancestor_ids(
            candidate,
            &by_id,
            track,
            &mut relevant_ids,
            &mut BTreeSet::new(),
        );
    }

    // §11.5 cap: keep tasks by earliest `chapter_from`, each with its active ancestors; then
    // fill the remaining slots with the other candidates. A skipped task brings no ancestor.
    let mut tasks: Vec<&goal_nodes::Model> = candidates
        .iter()
        .copied()
        .filter(|node| node.level == GoalLevel::Task)
        .collect();
    tasks.sort_by_key(|node| (node.chapter_from.unwrap_or(i32::MAX), node.id));
    let mut chosen: BTreeSet<i32> = BTreeSet::new();
    for task in tasks {
        let mut with_ancestors = BTreeSet::new();
        with_ancestors.insert(task.id);
        collect_active_ancestor_ids(
            task,
            &by_id,
            track,
            &mut with_ancestors,
            &mut BTreeSet::new(),
        );
        if chosen.len() + with_ancestors.difference(&chosen).count() > GOAL_NODE_CAP {
            continue;
        }
        chosen.extend(with_ancestors);
    }
    for node in &candidates {
        if chosen.len() >= GOAL_NODE_CAP {
            break;
        }
        if node.level == GoalLevel::Task || chosen.contains(&node.id) {
            continue;
        }
        let mut with_ancestors = BTreeSet::new();
        with_ancestors.insert(node.id);
        collect_active_ancestor_ids(
            node,
            &by_id,
            track,
            &mut with_ancestors,
            &mut BTreeSet::new(),
        );
        if chosen.len() + with_ancestors.difference(&chosen).count() <= GOAL_NODE_CAP {
            chosen.extend(with_ancestors);
        }
    }
    let omitted = relevant_ids.len().saturating_sub(chosen.len()) as i32;
    let mut nodes: Vec<goal_nodes::Model> = chosen
        .into_iter()
        .filter_map(|id| by_id.get(&id).map(|node| (*node).clone()))
        .collect();
    nodes.sort_by_key(|node| (level_rank(node.level), node.chapter_from, node.id));
    Ok(GoalSelection { nodes, omitted })
}

/// True when the chapter has at least one node in the `block.goals` set (plan §4.5).
pub async fn chapter_has_relevant_goals(
    conn: &impl ConnectionTrait,
    book_id: i32,
    track: &tracks::Model,
    chapter_id: i32,
) -> Result<bool, AppError> {
    let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(conn).await? else {
        return Ok(false);
    };
    Ok(!relevant_nodes(conn, book_id, track, &chapter)
        .await?
        .nodes
        .is_empty())
}

fn level_rank(level: GoalLevel) -> i32 {
    match level {
        GoalLevel::Goal => 0,
        GoalLevel::Plan => 1,
        GoalLevel::Task => 2,
    }
}

fn in_scope(node: &goal_nodes::Model, track: &tracks::Model) -> bool {
    if track.kind == TrackKind::Main {
        return node.track_id.is_none();
    }
    if node.track_id == Some(track.id) {
        return true;
    }
    // Book-wide goals reach side stories: shared, level `goal`, and no chapter range.
    node.track_id.is_none()
        && node.level == GoalLevel::Goal
        && node.chapter_from.is_none()
        && node.chapter_to.is_none()
}

fn has_closed_ancestor(
    node: &goal_nodes::Model,
    by_id: &BTreeMap<i32, &goal_nodes::Model>,
    seen: &mut BTreeSet<i32>,
) -> bool {
    let Some(parent_id) = node.parent_id else {
        return false;
    };
    if !seen.insert(parent_id) {
        return true;
    }
    let Some(parent) = by_id.get(&parent_id) else {
        return false;
    };
    matches!(parent.status, GoalStatus::Done | GoalStatus::Dropped)
        || has_closed_ancestor(parent, by_id, seen)
}

/// Adds every active ancestor in the same track scope, even when its own `chapter_from` is
/// outside the target chapter: a selected task brings its plan and goal with it (plan §4.5).
fn collect_active_ancestor_ids(
    node: &goal_nodes::Model,
    by_id: &BTreeMap<i32, &goal_nodes::Model>,
    track: &tracks::Model,
    ids: &mut BTreeSet<i32>,
    seen: &mut BTreeSet<i32>,
) {
    let Some(parent_id) = node.parent_id else {
        return;
    };
    if !seen.insert(parent_id) {
        return;
    }
    let Some(parent) = by_id.get(&parent_id) else {
        return;
    };
    if parent.status != GoalStatus::Active || !in_scope(parent, track) {
        return;
    }
    ids.insert(parent.id);
    collect_active_ancestor_ids(parent, by_id, track, ids, seen);
}

// ───────────────────────────── validation and helpers ─────────────────────────────

fn clean_title(value: String) -> CanonResult<String> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(CanonError::field("title", "a goal needs a title"));
    }
    if value.chars().count() > 120 {
        return Err(CanonError::field(
            "title",
            "a title may have at most 120 characters",
        ));
    }
    Ok(value)
}

fn clean_purpose(value: String) -> CanonResult<String> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(CanonError::field("purpose", "a goal needs a purpose"));
    }
    if value.chars().count() > 1000 {
        return Err(CanonError::field(
            "purpose",
            "a purpose may have at most 1000 characters",
        ));
    }
    Ok(value)
}

fn clean_conditions(conditions: Vec<GoalCondition>) -> CanonResult<Vec<GoalCondition>> {
    let mut seen = HashSet::new();
    let mut cleaned = Vec::new();
    for condition in conditions {
        let key = serde_json::to_string(&condition)
            .map_err(|_| CanonError::field("conditions", "invalid condition"))?;
        if seen.insert(key) {
            cleaned.push(condition);
        }
    }
    if cleaned.len() > 10 {
        return Err(CanonError::field(
            "conditions",
            "a goal may have at most 10 conditions",
        ));
    }
    for condition in &cleaned {
        if let GoalCondition::ChapterAtLeast { chapter } = condition
            && *chapter < 1
        {
            return Err(CanonError::field(
                "conditions",
                "chapter must be at least 1",
            ));
        }
    }
    Ok(cleaned)
}

fn validate_status_conditions(status: GoalStatus, conditions: &[GoalCondition]) -> CanonResult<()> {
    if status == GoalStatus::Waiting && conditions.is_empty() {
        return Err(CanonError::field(
            "conditions",
            "a waiting goal needs at least one condition",
        ));
    }
    Ok(())
}

fn validate_range(from: Option<i32>, to: Option<i32>) -> CanonResult<()> {
    if from.is_some_and(|from| from < 1) || to.is_some_and(|to| to < 1) {
        return Err(CanonError::field(
            "chapter_from",
            "chapter numbers start at 1",
        ));
    }
    if let (Some(from), Some(to)) = (from, to)
        && to < from
    {
        return Err(CanonError::field(
            "chapter_to",
            "chapter_to must not be before chapter_from",
        ));
    }
    Ok(())
}

async fn ensure_isolated_track(
    tx: &impl ConnectionTrait,
    book_id: i32,
    track_id: i32,
) -> CanonResult<()> {
    let track = tracks::Entity::find_by_id(track_id)
        .one(tx)
        .await?
        .ok_or_else(|| CanonError::not_found(format!("track #{track_id}")))?;
    if track.book_id != book_id
        || track.kind != TrackKind::Side
        || track.canon_mode != crate::db::json::CanonMode::Isolated
    {
        return Err(CanonError::field(
            "track_id",
            "a private goal needs an isolated side track of this book",
        ));
    }
    Ok(())
}

async fn validate_parent(
    tx: &impl ConnectionTrait,
    book_id: i32,
    parent_id: i32,
    track_id: Option<i32>,
    level: GoalLevel,
) -> CanonResult<goal_nodes::Model> {
    let parent = load_node(tx, parent_id).await?;
    if parent.book_id != book_id || parent.track_id != track_id {
        return Err(CanonError::field(
            "parent_id",
            "the parent must belong to the same book and scope",
        ));
    }
    let expected = match level {
        GoalLevel::Goal => None,
        GoalLevel::Plan => Some(GoalLevel::Goal),
        GoalLevel::Task => Some(GoalLevel::Plan),
    };
    if expected != Some(parent.level) {
        return Err(CanonError::field(
            "parent_id",
            match level {
                GoalLevel::Goal => "a goal cannot have a parent",
                GoalLevel::Plan => "a plan's parent must be a goal",
                GoalLevel::Task => "a task's parent must be a plan",
            },
        ));
    }
    Ok(parent)
}

async fn validate_links(
    tx: &impl ConnectionTrait,
    book_id: i32,
    track_id: Option<i32>,
    entity_ids: Vec<i32>,
    event_ids: Vec<i32>,
    foreshadowing_ids: Vec<i32>,
) -> CanonResult<()> {
    for entity_id in dedupe_ids(entity_ids) {
        let row = entities::Entity::find_by_id(entity_id)
            .one(tx)
            .await?
            .ok_or_else(|| CanonError::not_found(format!("entity #{entity_id}")))?;
        if row.book_id != book_id || !visible(track_id, row.track_id) {
            return Err(CanonError::not_found(format!("entity #{entity_id}")));
        }
    }
    for event_id in dedupe_ids(event_ids) {
        let row = timeline_events::Entity::find_by_id(event_id)
            .one(tx)
            .await?
            .ok_or_else(|| CanonError::not_found(format!("timeline event #{event_id}")))?;
        if row.book_id != book_id || !visible(track_id, row.track_id) {
            return Err(CanonError::not_found(format!("timeline event #{event_id}")));
        }
    }
    for hook_id in dedupe_ids(foreshadowing_ids) {
        let row = foreshadowing::Entity::find_by_id(hook_id)
            .one(tx)
            .await?
            .ok_or_else(|| CanonError::not_found(format!("foreshadowing #{hook_id}")))?;
        if row.book_id != book_id || !visible(track_id, row.track_id) {
            return Err(CanonError::not_found(format!("foreshadowing #{hook_id}")));
        }
    }
    Ok(())
}

async fn validate_condition_targets(
    tx: &impl ConnectionTrait,
    book_id: i32,
    track_id: Option<i32>,
    conditions: &[GoalCondition],
) -> CanonResult<()> {
    for condition in conditions {
        match condition {
            GoalCondition::ChapterAtLeast { .. } => {}
            GoalCondition::ForeshadowingResolved { foreshadowing_id } => {
                let row = foreshadowing::Entity::find_by_id(*foreshadowing_id)
                    .one(tx)
                    .await?
                    .ok_or_else(|| {
                        CanonError::not_found(format!("foreshadowing #{foreshadowing_id}"))
                    })?;
                if row.book_id != book_id || !visible(track_id, row.track_id) {
                    return Err(CanonError::not_found(format!(
                        "foreshadowing #{foreshadowing_id}"
                    )));
                }
            }
            GoalCondition::EntityStatusIs { entity_id, status } => {
                let row = entities::Entity::find_by_id(*entity_id)
                    .one(tx)
                    .await?
                    .ok_or_else(|| CanonError::not_found(format!("entity #{entity_id}")))?;
                if row.book_id != book_id || !visible(track_id, row.track_id) {
                    return Err(CanonError::not_found(format!("entity #{entity_id}")));
                }
                let _ = status;
            }
        }
    }
    Ok(())
}

fn dedupe_ids(ids: Vec<i32>) -> Vec<i32> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

async fn load_node(tx: &impl ConnectionTrait, node_id: i32) -> CanonResult<goal_nodes::Model> {
    goal_nodes::Entity::find_by_id(node_id)
        .one(tx)
        .await?
        .ok_or_else(|| CanonError::not_found(format!("goal #{node_id}")))
}

async fn open_descendants(
    tx: &impl ConnectionTrait,
    node_id: i32,
) -> CanonResult<Vec<goal_nodes::Model>> {
    let all = goal_nodes::Entity::find().all(tx).await?;
    let mut out = Vec::new();
    let mut stack = vec![node_id];
    while let Some(parent) = stack.pop() {
        for node in all.iter().filter(|node| node.parent_id == Some(parent)) {
            stack.push(node.id);
            if matches!(node.status, GoalStatus::Active | GoalStatus::Waiting) {
                out.push(node.clone());
            }
        }
    }
    Ok(out)
}

#[derive(Clone, Copy)]
enum LinkKind {
    Entity,
    Event,
    Foreshadowing,
}

async fn current_link_ids(
    tx: &impl ConnectionTrait,
    node_id: i32,
) -> CanonResult<(Vec<i32>, Vec<i32>, Vec<i32>)> {
    let entity_ids = goal_node_entities::Entity::find()
        .filter(goal_node_entities::COLUMN.node_id.eq(node_id))
        .all(tx)
        .await?
        .into_iter()
        .map(|link| link.entity_id)
        .collect();
    let event_ids = goal_node_events::Entity::find()
        .filter(goal_node_events::COLUMN.node_id.eq(node_id))
        .all(tx)
        .await?
        .into_iter()
        .map(|link| link.event_id)
        .collect();
    let hook_ids = goal_node_foreshadowing::Entity::find()
        .filter(goal_node_foreshadowing::COLUMN.node_id.eq(node_id))
        .all(tx)
        .await?
        .into_iter()
        .map(|link| link.foreshadowing_id)
        .collect();
    Ok((entity_ids, event_ids, hook_ids))
}

async fn replace_links(
    tx: &impl ConnectionTrait,
    node_id: i32,
    entity_ids: &[i32],
    event_ids: &[i32],
    foreshadowing_ids: &[i32],
) -> CanonResult<()> {
    goal_node_entities::Entity::delete_many()
        .filter(goal_node_entities::COLUMN.node_id.eq(node_id))
        .exec(tx)
        .await?;
    goal_node_events::Entity::delete_many()
        .filter(goal_node_events::COLUMN.node_id.eq(node_id))
        .exec(tx)
        .await?;
    goal_node_foreshadowing::Entity::delete_many()
        .filter(goal_node_foreshadowing::COLUMN.node_id.eq(node_id))
        .exec(tx)
        .await?;
    for entity_id in dedupe_ids(entity_ids.to_vec()) {
        goal_node_entities::ActiveModel {
            node_id: Set(node_id),
            entity_id: Set(entity_id),
        }
        .insert(tx)
        .await?;
    }
    for event_id in dedupe_ids(event_ids.to_vec()) {
        goal_node_events::ActiveModel {
            node_id: Set(node_id),
            event_id: Set(event_id),
        }
        .insert(tx)
        .await?;
    }
    for hook_id in dedupe_ids(foreshadowing_ids.to_vec()) {
        goal_node_foreshadowing::ActiveModel {
            node_id: Set(node_id),
            foreshadowing_id: Set(hook_id),
        }
        .insert(tx)
        .await?;
    }
    Ok(())
}

async fn snapshot(tx: &impl ConnectionTrait, node: &goal_nodes::Model) -> CanonResult<Snapshot> {
    let (entity_ids, event_ids, foreshadowing_ids) = current_link_ids(tx, node.id).await?;
    Ok(Snapshot::GoalNode {
        level: node.level,
        parent_id: node.parent_id,
        title: node.title.clone(),
        purpose: node.purpose.clone(),
        status: node.status,
        conditions: node.conditions.0.clone(),
        chapter_from: node.chapter_from,
        chapter_to: node.chapter_to,
        entity_ids,
        event_ids,
        foreshadowing_ids,
        track_id: node.track_id,
    })
}

pub fn target_type(level: GoalLevel) -> ChangeTargetType {
    match level {
        GoalLevel::Goal => ChangeTargetType::Goal,
        GoalLevel::Plan => ChangeTargetType::Plan,
        GoalLevel::Task => ChangeTargetType::Task,
    }
}

// The change writer needs the target identity and both snapshots; grouping them would only move
// the same fields into a struct (same rationale as `revert::record_revert`).
#[allow(clippy::too_many_arguments)]
async fn record_goal_change(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    book_id: i32,
    level: GoalLevel,
    node_id: i32,
    op: ChangeOp,
    before: Option<&Snapshot>,
    after: Option<&Snapshot>,
    note: &str,
    track_id: Option<i32>,
) -> CanonResult<i32> {
    record_change(
        tx,
        write,
        ChangeWrite {
            book_id,
            target_type: target_type(level),
            target_id: node_id,
            op,
            before,
            after,
            note,
            reverts_id: None,
            track_id,
            chapter_id: None,
        },
    )
    .await
}

/// Restore an `update` snapshot in place (revert of an update; spec/06 §11.6).
///
/// A parent that vanished or no longer fits the node becomes `None` (top-level). Link rows are
/// restored only for targets that still exist and are visible; conditions are restored exactly as
/// stored, so a vanished target stays visible to the evaluator as broken. A restored `done` or
/// `dropped` is an explicit undo and therefore needs no open-children confirmation.
pub async fn restore_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    node_id: i32,
    before: &Snapshot,
) -> CanonResult<()> {
    let Snapshot::GoalNode {
        level,
        parent_id,
        title,
        purpose,
        status,
        conditions,
        chapter_from,
        chapter_to,
        entity_ids,
        event_ids,
        foreshadowing_ids,
        track_id,
    } = before
    else {
        return Err(CanonError::internal("a goal revert needs a goal snapshot"));
    };
    let current = load_node(tx, node_id).await?;
    write.ensure_writable(current.track_id, &format!("goal #{node_id}"))?;
    if current.level != *level || current.track_id != *track_id {
        return Err(CanonError::internal(
            "a goal snapshot changed level or scope",
        ));
    }
    let cutoff = revert_cutoff(tx, write).await?;
    let parent = parent_if_valid(tx, current.book_id, *parent_id, *track_id, *level).await?;
    let entity_ids = surviving_links(
        tx,
        current.book_id,
        *track_id,
        entity_ids,
        LinkKind::Entity,
        cutoff,
    )
    .await?;
    let event_ids = surviving_links(
        tx,
        current.book_id,
        *track_id,
        event_ids,
        LinkKind::Event,
        cutoff,
    )
    .await?;
    let foreshadowing_ids = surviving_links(
        tx,
        current.book_id,
        *track_id,
        foreshadowing_ids,
        LinkKind::Foreshadowing,
        cutoff,
    )
    .await?;
    let before_current = snapshot(tx, &current).await?;
    let restored = goal_nodes::ActiveModel {
        id: Set(current.id),
        parent_id: Set(parent.as_ref().map(|parent| parent.id)),
        title: Set(title.clone()),
        purpose: Set(purpose.clone()),
        status: Set(*status),
        conditions: Set(JsonText(conditions.clone())),
        chapter_from: Set(*chapter_from),
        chapter_to: Set(*chapter_to),
        updated_at: Set(Utc::now()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    replace_links(tx, node_id, &entity_ids, &event_ids, &foreshadowing_ids).await?;
    let after = snapshot(tx, &restored).await?;
    record_goal_change(
        tx,
        write,
        restored.book_id,
        restored.level,
        restored.id,
        ChangeOp::Update,
        Some(&before_current),
        Some(&after),
        "",
        restored.track_id,
    )
    .await?;
    Ok(())
}

/// Recreate a deleted node with a new id (revert of a delete; spec/06 §11.6).
pub async fn recreate_inner(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    before: &Snapshot,
) -> CanonResult<(i32, Snapshot)> {
    let Snapshot::GoalNode {
        level,
        parent_id,
        title,
        purpose,
        status,
        conditions,
        chapter_from,
        chapter_to,
        entity_ids,
        event_ids,
        foreshadowing_ids,
        track_id,
    } = before
    else {
        return Err(CanonError::internal("a goal revert needs a goal snapshot"));
    };
    let book_id = write
        .visibility
        .map(|visibility| visibility.book_id)
        .ok_or_else(|| CanonError::internal("a goal revert needs a book"))?;
    let cutoff = revert_cutoff(tx, write).await?;
    let parent = parent_if_valid(tx, book_id, *parent_id, *track_id, *level).await?;
    let entity_ids =
        surviving_links(tx, book_id, *track_id, entity_ids, LinkKind::Entity, cutoff).await?;
    let event_ids =
        surviving_links(tx, book_id, *track_id, event_ids, LinkKind::Event, cutoff).await?;
    let foreshadowing_ids = surviving_links(
        tx,
        book_id,
        *track_id,
        foreshadowing_ids,
        LinkKind::Foreshadowing,
        cutoff,
    )
    .await?;
    let new_id = super::canon::fresh_id(tx, "goal_nodes", target_type(*level)).await?;
    let timestamp = Utc::now();
    let node = goal_nodes::ActiveModel {
        id: Set(new_id),
        book_id: Set(book_id),
        track_id: Set(*track_id),
        level: Set(*level),
        parent_id: Set(parent.as_ref().map(|parent| parent.id)),
        title: Set(title.clone()),
        purpose: Set(purpose.clone()),
        status: Set(*status),
        conditions: Set(JsonText(conditions.clone())),
        chapter_from: Set(*chapter_from),
        chapter_to: Set(*chapter_to),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
    }
    .insert(tx)
    .await?;
    replace_links(tx, new_id, &entity_ids, &event_ids, &foreshadowing_ids).await?;
    let after = snapshot(tx, &node).await?;
    record_goal_change(
        tx,
        write,
        book_id,
        node.level,
        new_id,
        ChangeOp::Create,
        None,
        Some(&after),
        "",
        node.track_id,
    )
    .await?;
    Ok((new_id, after))
}

async fn parent_if_valid(
    tx: &impl ConnectionTrait,
    book_id: i32,
    parent_id: Option<i32>,
    track_id: Option<i32>,
    level: GoalLevel,
) -> CanonResult<Option<goal_nodes::Model>> {
    let Some(parent_id) = parent_id else {
        return Ok(None);
    };
    let Some(parent) = goal_nodes::Entity::find_by_id(parent_id).one(tx).await? else {
        return Ok(None);
    };
    if parent.book_id != book_id || parent.track_id != track_id {
        return Ok(None);
    }
    let expected = match level {
        GoalLevel::Goal => None,
        GoalLevel::Plan => Some(GoalLevel::Goal),
        GoalLevel::Task => Some(GoalLevel::Plan),
    };
    Ok((expected == Some(parent.level)).then_some(parent))
}

async fn revert_cutoff(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
) -> CanonResult<Option<chrono::DateTime<chrono::Utc>>> {
    let Some(revert) = write.revert else {
        return Ok(None);
    };
    Ok(change_log::Entity::find_by_id(revert.of_change)
        .one(tx)
        .await?
        .map(|change| change.created_at))
}

async fn surviving_links(
    tx: &impl ConnectionTrait,
    book_id: i32,
    track_id: Option<i32>,
    ids: &[i32],
    kind: LinkKind,
    cutoff: Option<chrono::DateTime<chrono::Utc>>,
) -> CanonResult<Vec<i32>> {
    let mut out = Vec::new();
    for id in dedupe_ids(ids.to_vec()) {
        let (exists, row_track, row_created_at) = match kind {
            LinkKind::Entity => entities::Entity::find_by_id(id)
                .one(tx)
                .await?
                .map(|row| (row.book_id == book_id, row.track_id, row.created_at))
                .unwrap_or((false, None, chrono::DateTime::<chrono::Utc>::MIN_UTC)),
            LinkKind::Event => timeline_events::Entity::find_by_id(id)
                .one(tx)
                .await?
                .map(|row| (row.book_id == book_id, row.track_id, row.created_at))
                .unwrap_or((false, None, chrono::DateTime::<chrono::Utc>::MIN_UTC)),
            LinkKind::Foreshadowing => foreshadowing::Entity::find_by_id(id)
                .one(tx)
                .await?
                .map(|row| (row.book_id == book_id, row.track_id, row.created_at))
                .unwrap_or((false, None, chrono::DateTime::<chrono::Utc>::MIN_UTC)),
        };
        let existed = cutoff.is_none_or(|cutoff| row_created_at <= cutoff);
        if exists && existed && visible(track_id, row_track) {
            out.push(id);
        }
    }
    Ok(out)
}
