//! Writing-goal DTOs (spec/10 §2.4, spec/06 §11, plan §6).
//!
//! The rows carry the author's data; the derived `ready`, `closable` and per-condition state are
//! computed on read by the one evaluator, so the API, the overview and the navigator badge cannot
//! disagree (spec/06 §11.3, D-150).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::json::{ForeshadowingStatus, GoalCondition, GoalLevel, GoalStatus};

use super::deserialize_some;

/// One goal node (spec/10 §2.8). The position in the tree comes from `parent_id`; `ready` and
/// `closable` are derived, never stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GoalNodeDto {
    pub id: i32,
    pub book_id: i32,
    /// `None` for a shared node, `Some(track)` for an isolated side track's private node.
    pub track_id: Option<i32>,
    pub level: GoalLevel,
    pub parent_id: Option<i32>,
    pub title: String,
    /// Why the node exists; the agents receive it and never generate it.
    pub purpose: String,
    pub status: GoalStatus,
    pub chapter_from: Option<i32>,
    pub chapter_to: Option<i32>,
    /// One entry per stored condition, with the evaluator's state and the target's name or title.
    pub conditions: Vec<GoalConditionStateDto>,
    pub entities: Vec<GoalEntityLinkDto>,
    pub events: Vec<GoalEventLinkDto>,
    pub foreshadowing: Vec<GoalForeshadowingLinkDto>,
    /// `waiting` and every condition satisfied (spec/06 §11.3).
    pub ready: bool,
    /// Active with at least one done child and no open child (spec/06 §11.3).
    pub closable: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One stored condition and its derived state (spec/10 §2.8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GoalConditionStateDto {
    pub condition: GoalCondition,
    pub satisfied: bool,
    /// The target vanished or is outside the node's scope, so the condition cannot become true
    /// without an edit.
    pub broken: bool,
    /// The stored name or title the UI composes its sentence with (A5: the author's language).
    pub target_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GoalEntityLinkDto {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GoalEventLinkDto {
    pub id: i32,
    pub title: String,
    pub status: crate::db::json::TimelineStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GoalForeshadowingLinkDto {
    pub id: i32,
    pub title: String,
    pub status: ForeshadowingStatus,
}

/// `POST /books/:id/goals` body (spec/10 §2.4, plan §6). `level` is required; the other fields are
/// optional and default as the schema does.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(optional_fields = nullable)]
#[ts(export)]
pub struct GoalNodeInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<i32>,
    pub level: GoalLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<i32>,
    pub title: String,
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<GoalStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_from: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_to: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub conditions: Option<Vec<GoalCondition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub entity_ids: Option<Vec<i32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub event_ids: Option<Vec<i32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub foreshadowing_ids: Option<Vec<i32>>,
}

/// A PATCH body for one goal node (spec/10 §2.4). An absent nullable field leaves it unchanged;
/// an explicit `null` clears it. `confirm_open_children` is the second half of the 409
/// `goal.open_children` handshake.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(optional_fields = nullable)]
#[ts(export)]
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

/// `GET /books/:id/goals` query (spec/10 §2.4). The link filters return the nodes that link to or
/// wait on the row ("Referenced by").
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GoalQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<GoalStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreshadowing_id: Option<i32>,
}

/// The target of a goal proposal (spec/10 §2.8): the node of a `goal_node` proposal, or the parent
/// plan of a `new_goal_task` proposal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GoalTargetDto {
    pub id: i32,
    pub level: GoalLevel,
    pub title: String,
    pub status: GoalStatus,
}

/// Per-system totals of the rows kept in the book, across every scope and status (spec/10 §2.4,
/// spec/06 §10.3). They only inform the Book settings' retained-data line; no action deletes them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CanonCountsDto {
    pub entities: CanonEntityCountsDto,
    pub timeline: CanonTimelineCountsDto,
    pub memory: CanonMemoryCountsDto,
    pub foreshadowing: CanonForeshadowingCountsDto,
    pub goals: CanonGoalCountsDto,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CanonEntityCountsDto {
    pub entities: i32,
    pub relations: i32,
    pub appearances: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CanonTimelineCountsDto {
    pub events: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CanonMemoryCountsDto {
    pub notes: i32,
    pub summaries: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CanonForeshadowingCountsDto {
    pub items: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CanonGoalCountsDto {
    pub nodes: i32,
}
