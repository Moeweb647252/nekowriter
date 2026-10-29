//! Writing-goal queries shared by the runtime switch gates (spec/06 §11, plan §4.5).
//!
//! The full goals service (validation, mutations, evaluator) lands with M13 task 13.5. This module
//! owns the read the switch work needs: the relevance set behind `block.goals`, the maintenance
//! goals step and the `propose_goal_change` gate.

use std::collections::{BTreeMap, BTreeSet};

use sea_orm::{ConnectionTrait, EntityTrait, QueryFilter};

use crate::db::entities::{chapters, goal_nodes, tracks};
use crate::db::json::{GoalLevel, GoalStatus, TrackKind};
use crate::error::AppError;

/// The relevant nodes of one chapter, in render order, plus the number left out by the cap.
#[derive(Debug, Clone, Default)]
pub struct GoalSelection {
    pub nodes: Vec<goal_nodes::Model>,
    pub omitted: i32,
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
    let mut selected: BTreeSet<i32> = BTreeSet::new();
    for node in &nodes {
        if node.status != GoalStatus::Active
            || node.chapter_from.is_some_and(|from| from > chapter.ord)
            || !in_scope(node, track)
            || has_closed_ancestor(node, &by_id, &mut BTreeSet::new())
        {
            continue;
        }
        selected.insert(node.id);
        add_active_ancestors(node, &by_id, &mut selected, &mut BTreeSet::new());
    }
    let mut selected: Vec<goal_nodes::Model> = selected
        .into_iter()
        .filter_map(|id| by_id.get(&id).map(|node| (*node).clone()))
        .collect();
    selected.sort_by_key(|node| (level_rank(node.level), node.chapter_from, node.id));
    let omitted = selected.len().saturating_sub(30) as i32;
    selected.truncate(30);
    Ok(GoalSelection {
        nodes: selected,
        omitted,
    })
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

fn add_active_ancestors(
    node: &goal_nodes::Model,
    by_id: &BTreeMap<i32, &goal_nodes::Model>,
    selected: &mut BTreeSet<i32>,
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
    if parent.status == GoalStatus::Active {
        selected.insert(parent.id);
        add_active_ancestors(parent, by_id, selected, seen);
    }
}
