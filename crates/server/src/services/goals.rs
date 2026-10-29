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

/// The cap of `block.goals` (plan §4.5).
pub const GOAL_NODE_CAP: usize = 30;

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
